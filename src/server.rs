use std::io::{self, BufRead, Write};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    index: u32,
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
    index: u32,
}

#[derive(Debug, Deserialize)]
struct SignTypedDataParams {
    typed_data: Value,
    #[serde(default)]
    index: u32,
}

#[derive(Debug, Deserialize)]
struct SendTransactionParams {
    to: String,
    value_wei: String,
    chain: Value,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    index: u32,
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
        "get_address" => parse_params::<AddressParams>(&request.params)
            .and_then(|params| wallet::derive_address(paths, params.index).map(|address| json!({ "address": address }))),
        "list_addresses" => parse_params::<ListAddressesParams>(&request.params).and_then(|params| {
            wallet::list_addresses(paths, params.count).map(|addresses| json!({ "addresses": addresses }))
        }),
        "sign_message" => parse_params::<SignMessageParams>(&request.params).and_then(|params| {
            wallet::sign_message(paths, &params.message, params.index)
                .map(|signed| json!({ "address": signed.address, "signature": signed.signature }))
        }),
        "sign_typed_data" => parse_params::<SignTypedDataParams>(&request.params).and_then(|params| {
            let typed_data = serde_json::to_string(&params.typed_data)
                .context("failed to serialize typed data payload")?;
            wallet::sign_typed_data(paths, &typed_data, params.index)
                .map(|signed| json!({ "address": signed.address, "signature": signed.signature }))
        }),
        "send_transaction" => {
            let params = parse_params::<SendTransactionParams>(&request.params);
            match params.and_then(|params| {
                chain_selector_from_json(&params.chain).map(|selector| (params, selector))
            }) {
                Ok((params, selector)) => wallet::send_transaction(
                    paths,
                    &selector,
                    &params.to,
                    &params.value_wei,
                    params.data.as_deref(),
                    params.index,
                )
                .await
                .map(|sent| json!({ "tx_hash": sent.tx_hash })),
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
        assert!(matches!(numeric, crate::chain::ChainSelector::ChainId(84532)));
    }
}
