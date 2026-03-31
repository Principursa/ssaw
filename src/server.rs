use std::io::{self, BufRead, Write};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Paths;
use crate::wallet;

#[derive(Debug, Deserialize)]
struct Request {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct AddressParams {
    index: Option<u32>,
    alias: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ListAddressesParams {
    count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SignMessageParams {
    message: String,
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SignTypedDataParams {
    typed_data: Value,
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SendTransactionParams {
    to: String,
    value_wei: String,
    chain: Value,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    wait: bool,
    #[serde(default = "default_timeout_secs")]
    timeout_secs: u64,
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReadContractParams {
    address: String,
    chain: Value,
    function: String,
    abi: Value,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WriteContractParams {
    address: String,
    chain: Value,
    function: String,
    abi: Value,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    value_wei: Option<String>,
    #[serde(default)]
    wait: bool,
    #[serde(default = "default_timeout_secs")]
    timeout_secs: u64,
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Debug, Serialize)]
struct Response {
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub async fn run(paths: &Paths) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line.context("failed to read request line")?;
        if line.trim().is_empty() {
            continue;
        }

        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle_request(paths, &request).await,
            Err(error) => Response {
                id: Value::Null,
                result: None,
                error: Some(format!("invalid request: {error}")),
            },
        };

        serde_json::to_writer(&mut output, &reply).context("failed to serialize response")?;
        writeln!(&mut output).context("failed to write response newline")?;
        output.flush().context("failed to flush response")?;
    }

    Ok(())
}

async fn handle_request(paths: &Paths, request: &Request) -> Response {
    let result = match request.method.as_str() {
        "get_address" => parse_params::<AddressParams>(&request.params).and_then(|params| {
            let target =
                wallet::resolve_address_target(paths, params.index, params.alias.as_deref())?;
            wallet::derive_address(paths, target.index)
                .map(|address| json!({ "address": address, "index": target.index, "alias": target.alias }))
        }),
        "list_addresses" => {
            parse_params::<ListAddressesParams>(&request.params).and_then(|params| {
                wallet::list_addresses(paths, params.count)
                    .map(|addresses| json!({ "addresses": addresses }))
            })
        }
        "sign_message" => parse_params::<SignMessageParams>(&request.params).and_then(|params| {
            let target =
                wallet::resolve_address_target(paths, params.index, params.alias.as_deref())?;
            wallet::sign_message(paths, &params.message, target.index).map(|signed| {
                json!({ "address": signed.address, "signature": signed.signature, "index": target.index, "alias": target.alias })
            })
        }),
        "sign_typed_data" => {
            parse_params::<SignTypedDataParams>(&request.params).and_then(|params| {
                let typed_data = serde_json::to_string(&params.typed_data)
                    .context("failed to serialize typed data payload")?;
                let target =
                    wallet::resolve_address_target(paths, params.index, params.alias.as_deref())?;
                wallet::sign_typed_data(paths, &typed_data, target.index).map(|signed| {
                    json!({ "address": signed.address, "signature": signed.signature, "index": target.index, "alias": target.alias })
                })
            })
        }
        "send_transaction" => {
            let params = parse_params::<SendTransactionParams>(&request.params);
            match params.and_then(|params| {
                chain_selector_from_json(&params.chain).map(|selector| (params, selector))
            }) {
                Ok((params, selector)) => {
                    let target =
                        wallet::resolve_address_target(paths, params.index, params.alias.as_deref());
                    match target {
                        Ok(target) => wallet::send_transaction(
                            paths,
                            &selector,
                            &params.to,
                            &params.value_wei,
                            params.data.as_deref(),
                            wallet::WaitOptions::from_flag(params.wait, params.timeout_secs),
                            target.index,
                        )
                        .await
                        .map(|sent| serde_json::to_value(&sent).expect("sent tx serializes")),
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        }
        "read_contract" => {
            let params = parse_params::<ReadContractParams>(&request.params);
            match params.and_then(|params| {
                chain_selector_from_json(&params.chain).map(|selector| (params, selector))
            }) {
                Ok((params, selector)) => {
                    let abi_json = serde_json::to_string(&params.abi)
                        .context("failed to serialize contract ABI payload");
                    match abi_json {
                        Ok(abi_json) => wallet::read_contract(
                            paths,
                            &selector,
                            &params.address,
                            &abi_json,
                            &params.function,
                            &params.args,
                        )
                        .await
                        .map(|output| json!({ "outputs": output.outputs })),
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        }
        "write_contract" => {
            let params = parse_params::<WriteContractParams>(&request.params);
            match params.and_then(|params| {
                chain_selector_from_json(&params.chain).map(|selector| (params, selector))
            }) {
                Ok((params, selector)) => {
                    let abi_json = serde_json::to_string(&params.abi)
                        .context("failed to serialize contract ABI payload");
                    match abi_json {
                        Ok(abi_json) => {
                            let target = wallet::resolve_address_target(
                                paths,
                                params.index,
                                params.alias.as_deref(),
                            );
                            match target {
                                Ok(target) => wallet::write_contract(
                                    paths,
                                    &selector,
                                    &params.address,
                                    &abi_json,
                                    &params.function,
                                    &params.args,
                                    params.value_wei.as_deref(),
                                    wallet::WaitOptions::from_flag(
                                        params.wait,
                                        params.timeout_secs,
                                    ),
                                    target.index,
                                )
                                .await
                                .map(|sent| serde_json::to_value(&sent).expect("sent tx serializes")),
                                Err(error) => Err(error),
                            }
                        }
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        }
        _ => Err(anyhow::anyhow!("unknown method `{}`", request.method)),
    };

    match result {
        Ok(value) => Response {
            id: request.id.clone(),
            result: Some(value),
            error: None,
        },
        Err(error) => Response {
            id: request.id.clone(),
            result: None,
            error: Some(error.to_string()),
        },
    }
}

fn parse_params<T>(params: &Value) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(params.clone()).context("invalid params")
}

fn chain_selector_from_json(value: &Value) -> Result<crate::chain::ChainSelector> {
    match value {
        Value::String(name) => Ok(crate::chain::ChainSelector::parse(name)),
        Value::Number(number) => number
            .as_u64()
            .map(crate::chain::ChainSelector::ChainId)
            .context("chain number must be an unsigned integer"),
        _ => bail!("chain must be a string name or numeric chain id"),
    }
}

fn default_timeout_secs() -> u64 {
    60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_method() {
        let paths = Paths::discover().expect("paths");
        let request = Request {
            id: json!(1),
            method: "nope".to_owned(),
            params: json!({}),
        };

        let response = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(handle_request(&paths, &request));
        assert!(response.error.unwrap().contains("unknown method"));
    }

    #[test]
    fn parses_chain_selector_from_json() {
        let named = chain_selector_from_json(&json!("base-sepolia")).expect("named selector");
        assert!(matches!(named, crate::chain::ChainSelector::Name(_)));

        let numeric = chain_selector_from_json(&json!(84532)).expect("numeric selector");
        assert!(matches!(
            numeric,
            crate::chain::ChainSelector::ChainId(84532)
        ));
    }
}
