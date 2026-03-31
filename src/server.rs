use std::io::{self, BufRead, Write};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Paths;
use crate::wallet;

const JSONRPC_VERSION: &str = "2.0";
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const ERROR_PARSE: i64 = -32700;
const ERROR_INVALID_REQUEST: i64 = -32600;
const ERROR_METHOD_NOT_FOUND: i64 = -32601;
const ERROR_INVALID_PARAMS: i64 = -32602;
const ERROR_SERVER_NOT_INITIALIZED: i64 = -32002;

#[derive(Debug, Deserialize)]
struct LegacyRequest {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct LegacyResponse {
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize)]
struct CallToolParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct AddressParams {
    project: Option<String>,
    index: Option<u32>,
    alias: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ListAddressesParams {
    project: Option<String>,
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

#[derive(Default)]
struct ServerState {
    initialized: bool,
}

pub async fn run(paths: &Paths) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut state = ServerState::default();

    for line in stdin.lock().lines() {
        let line = line.context("failed to read request line")?;
        if line.trim().is_empty() {
            continue;
        }

        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(error) => {
                let reply = jsonrpc_error_response(
                    Value::Null,
                    ERROR_PARSE,
                    format!("parse error: {error}"),
                );
                write_json_line(&mut output, &reply)?;
                continue;
            }
        };

        if let Some(reply) = handle_message(paths, &mut state, &message).await {
            write_json_line(&mut output, &reply)?;
        }
    }

    Ok(())
}

async fn handle_message(paths: &Paths, state: &mut ServerState, message: &Value) -> Option<Value> {
    match message {
        Value::Array(items) => {
            if items.is_empty() {
                return Some(jsonrpc_error_response(
                    Value::Null,
                    ERROR_INVALID_REQUEST,
                    "batch requests cannot be empty",
                ));
            }

            let mut replies = Vec::new();
            for item in items {
                if let Some(reply) = handle_single_message(paths, state, item).await {
                    replies.push(reply);
                }
            }

            if replies.is_empty() {
                None
            } else {
                Some(Value::Array(replies))
            }
        }
        _ => handle_single_message(paths, state, message).await,
    }
}

async fn handle_single_message(
    paths: &Paths,
    state: &mut ServerState,
    message: &Value,
) -> Option<Value> {
    if let Some(legacy_request) = parse_legacy_request(message) {
        let reply = handle_legacy_request(paths, &legacy_request).await;
        return Some(serde_json::to_value(reply).expect("legacy response serializes"));
    }

    let request = match serde_json::from_value::<JsonRpcRequest>(message.clone()) {
        Ok(request) => request,
        Err(error) => {
            return Some(jsonrpc_error_response(
                Value::Null,
                ERROR_INVALID_REQUEST,
                format!("invalid request: {error}"),
            ));
        }
    };

    if request.jsonrpc.as_deref() != Some(JSONRPC_VERSION) {
        return Some(jsonrpc_error_response(
            request.id.unwrap_or(Value::Null),
            ERROR_INVALID_REQUEST,
            "jsonrpc must be `2.0`",
        ));
    }

    handle_jsonrpc_request(paths, state, request)
        .await
        .map(|reply| serde_json::to_value(reply).expect("jsonrpc response serializes"))
}

fn parse_legacy_request(message: &Value) -> Option<LegacyRequest> {
    let object = message.as_object()?;
    if object.contains_key("jsonrpc") {
        return None;
    }

    serde_json::from_value(message.clone()).ok()
}

async fn handle_legacy_request(paths: &Paths, request: &LegacyRequest) -> LegacyResponse {
    match dispatch_wallet_method(paths, &request.method, &request.params).await {
        Ok(result) => LegacyResponse {
            id: request.id.clone(),
            result: Some(result),
            error: None,
        },
        Err(error) => LegacyResponse {
            id: request.id.clone(),
            result: None,
            error: Some(error.to_string()),
        },
    }
}

async fn handle_jsonrpc_request(
    paths: &Paths,
    state: &mut ServerState,
    request: JsonRpcRequest,
) -> Option<JsonRpcResponse> {
    let id = request.id.clone();

    match request.method.as_str() {
        "initialize" => {
            state.initialized = true;
            let result = json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": "ssaw",
                    "title": "Shark's Secure Agent Wallet",
                    "version": env!("CARGO_PKG_VERSION")
                }
            });
            id.map(|id| jsonrpc_result_response(id, result))
        }
        "notifications/initialized" => None,
        "ping" => id.map(|id| jsonrpc_result_response(id, json!({}))),
        "tools/list" => {
            if !state.initialized {
                return id.map(|id| {
                    jsonrpc_error(id, ERROR_SERVER_NOT_INITIALIZED, "server not initialized")
                });
            }

            id.map(|id| jsonrpc_result_response(id, json!({ "tools": tool_definitions() })))
        }
        "tools/call" => {
            if !state.initialized {
                return id.map(|id| {
                    jsonrpc_error(id, ERROR_SERVER_NOT_INITIALIZED, "server not initialized")
                });
            }

            let params = match parse_params::<CallToolParams>(&request.params) {
                Ok(params) => params,
                Err(error) => {
                    return id.map(|id| jsonrpc_error(id, ERROR_INVALID_PARAMS, error.to_string()));
                }
            };

            if !tool_exists(&params.name) {
                return id.map(|id| {
                    jsonrpc_error(
                        id,
                        ERROR_INVALID_PARAMS,
                        format!("unknown tool `{}`", params.name),
                    )
                });
            }

            let result = match dispatch_wallet_method(paths, &params.name, &params.arguments).await
            {
                Ok(result) => mcp_tool_success(result),
                Err(error) => mcp_tool_error(error),
            };

            id.map(|id| jsonrpc_result_response(id, result))
        }
        _ => id.map(|id| {
            jsonrpc_error(
                id,
                ERROR_METHOD_NOT_FOUND,
                format!("unknown method `{}`", request.method),
            )
        }),
    }
}

fn jsonrpc_result_response(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION,
        id,
        result: Some(result),
        error: None,
    }
}

fn jsonrpc_error(id: Value, code: i64, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION,
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
        }),
    }
}

fn jsonrpc_error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    serde_json::to_value(jsonrpc_error(id, code, message)).expect("jsonrpc error serializes")
}

fn mcp_tool_success(result: Value) -> Value {
    let text = serde_json::to_string_pretty(&result).expect("tool result serializes");
    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "structuredContent": result,
        "isError": false
    })
}

fn mcp_tool_error(error: anyhow::Error) -> Value {
    let message = error.to_string();
    json!({
        "content": [
            {
                "type": "text",
                "text": message
            }
        ],
        "structuredContent": {
            "error": message
        },
        "isError": true
    })
}

fn tool_exists(name: &str) -> bool {
    matches!(
        name,
        "get_address"
            | "list_addresses"
            | "sign_message"
            | "sign_typed_data"
            | "send_transaction"
            | "read_contract"
            | "write_contract"
    )
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "get_address",
            "description": "Derive a single wallet address from the selected project by index or alias.",
            "inputSchema": address_target_schema("Optional project override plus either an address index or a project-local alias.")
        }),
        json!({
            "name": "list_addresses",
            "description": "List derived wallet addresses for the selected project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Optional project override." },
                    "count": { "type": "integer", "minimum": 1, "maximum": 20, "description": "How many addresses to derive." }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "sign_message",
            "description": "Sign an arbitrary UTF-8 message with a project wallet address.",
            "inputSchema": sign_schema(json!({
                "message": { "type": "string", "description": "Plain message text to sign." }
            }), vec!["message"])
        }),
        json!({
            "name": "sign_typed_data",
            "description": "Sign EIP-712 typed data with a project wallet address.",
            "inputSchema": sign_schema(json!({
                "typed_data": {
                    "type": "object",
                    "description": "Full EIP-712 typed data payload."
                }
            }), vec!["typed_data"])
        }),
        json!({
            "name": "send_transaction",
            "description": "Send a native ETH transaction or raw calldata transaction on a configured chain.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Optional project override." },
                    "chain": chain_schema(),
                    "to": { "type": "string", "description": "Destination address." },
                    "value_wei": { "type": "string", "description": "Native value in wei as a decimal string." },
                    "data": { "type": "string", "description": "Optional calldata as a 0x-prefixed hex string." },
                    "wait": { "type": "boolean", "description": "Wait for a receipt before returning." },
                    "timeout_secs": { "type": "integer", "minimum": 1, "description": "Wait timeout in seconds." },
                    "index": { "type": "integer", "minimum": 0, "description": "Signer derivation index." },
                    "alias": { "type": "string", "description": "Signer alias within the selected project." }
                },
                "required": ["chain", "to", "value_wei"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "read_contract",
            "description": "Run an eth_call against a contract and decode outputs using a provided ABI.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Optional project override." },
                    "chain": chain_schema(),
                    "address": { "type": "string", "description": "Contract address." },
                    "function": { "type": "string", "description": "Function name to call." },
                    "abi": { "type": "array", "description": "Contract ABI entries." },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Function arguments encoded as Solidity-like string literals."
                    }
                },
                "required": ["chain", "address", "function", "abi"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "write_contract",
            "description": "Sign and submit a contract write transaction using a provided ABI.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Optional project override." },
                    "chain": chain_schema(),
                    "address": { "type": "string", "description": "Contract address." },
                    "function": { "type": "string", "description": "Function name to call." },
                    "abi": { "type": "array", "description": "Contract ABI entries." },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Function arguments encoded as Solidity-like string literals."
                    },
                    "value_wei": { "type": "string", "description": "Optional native value in wei for payable calls." },
                    "wait": { "type": "boolean", "description": "Wait for a receipt before returning." },
                    "timeout_secs": { "type": "integer", "minimum": 1, "description": "Wait timeout in seconds." },
                    "index": { "type": "integer", "minimum": 0, "description": "Signer derivation index." },
                    "alias": { "type": "string", "description": "Signer alias within the selected project." }
                },
                "required": ["chain", "address", "function", "abi"],
                "additionalProperties": false
            }
        }),
    ]
}

fn address_target_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "properties": {
            "project": { "type": "string", "description": "Optional project override." },
            "index": { "type": "integer", "minimum": 0, "description": "Signer derivation index." },
            "alias": { "type": "string", "description": "Project-local alias for the signer." }
        },
        "additionalProperties": false
    })
}

fn sign_schema(extra_properties: Value, required: Vec<&str>) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "project".to_owned(),
        json!({ "type": "string", "description": "Optional project override." }),
    );
    properties.insert(
        "index".to_owned(),
        json!({ "type": "integer", "minimum": 0, "description": "Signer derivation index." }),
    );
    properties.insert(
        "alias".to_owned(),
        json!({ "type": "string", "description": "Project-local alias for the signer." }),
    );

    let extra = extra_properties
        .as_object()
        .expect("extra properties must be an object");
    for (key, value) in extra {
        properties.insert(key.clone(), value.clone());
    }

    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn chain_schema() -> Value {
    json!({
        "description": "Chain name or numeric chain id.",
        "oneOf": [
            { "type": "string" },
            { "type": "integer", "minimum": 0 }
        ]
    })
}

async fn dispatch_wallet_method(paths: &Paths, method: &str, params: &Value) -> Result<Value> {
    let paths = request_paths(paths, params)?;

    match method {
        "get_address" => parse_params::<AddressParams>(params).and_then(|params| {
            let target =
                wallet::resolve_address_target(&paths, params.index, params.alias.as_deref())?;
            let aliases = wallet::aliases_for_index(&paths, target.index)?;
            wallet::derive_address(&paths, target.index).map(|address| {
                json!({ "address": address, "index": target.index, "alias": target.alias, "aliases": aliases })
            })
        }),
        "list_addresses" => parse_params::<ListAddressesParams>(params)
            .and_then(|params| wallet::list_addresses(&paths, params.count))
            .map(|addresses| json!({ "addresses": addresses })),
        "sign_message" => parse_params::<SignMessageParams>(params).and_then(|params| {
            let target =
                wallet::resolve_address_target(&paths, params.index, params.alias.as_deref())?;
            wallet::sign_message(&paths, &params.message, target.index).map(|signed| {
                json!({ "address": signed.address, "signature": signed.signature, "index": target.index, "alias": target.alias })
            })
        }),
        "sign_typed_data" => parse_params::<SignTypedDataParams>(params).and_then(|params| {
            let typed_data = serde_json::to_string(&params.typed_data)
                .context("failed to serialize typed data payload")?;
            let target =
                wallet::resolve_address_target(&paths, params.index, params.alias.as_deref())?;
            wallet::sign_typed_data(&paths, &typed_data, target.index).map(|signed| {
                json!({ "address": signed.address, "signature": signed.signature, "index": target.index, "alias": target.alias })
            })
        }),
        "send_transaction" => {
            let params = parse_params::<SendTransactionParams>(params)?;
            let selector = chain_selector_from_json(&params.chain)?;
            let target =
                wallet::resolve_address_target(&paths, params.index, params.alias.as_deref())?;
            wallet::send_transaction(
                &paths,
                &selector,
                &params.to,
                &params.value_wei,
                params.data.as_deref(),
                wallet::WaitOptions::from_flag(params.wait, params.timeout_secs),
                target.index,
            )
            .await
            .map(|sent| serde_json::to_value(&sent).expect("sent tx serializes"))
        }
        "read_contract" => {
            let params = parse_params::<ReadContractParams>(params)?;
            let selector = chain_selector_from_json(&params.chain)?;
            let abi_json = serde_json::to_string(&params.abi)
                .context("failed to serialize contract ABI payload")?;
            wallet::read_contract(
                &paths,
                &selector,
                &params.address,
                &abi_json,
                &params.function,
                &params.args,
            )
            .await
            .map(|output| json!({ "outputs": output.outputs }))
        }
        "write_contract" => {
            let params = parse_params::<WriteContractParams>(params)?;
            let selector = chain_selector_from_json(&params.chain)?;
            let abi_json = serde_json::to_string(&params.abi)
                .context("failed to serialize contract ABI payload")?;
            let target =
                wallet::resolve_address_target(&paths, params.index, params.alias.as_deref())?;
            wallet::write_contract(
                &paths,
                &selector,
                &params.address,
                &abi_json,
                &params.function,
                &params.args,
                params.value_wei.as_deref(),
                wallet::WaitOptions::from_flag(params.wait, params.timeout_secs),
                target.index,
            )
            .await
            .map(|sent| serde_json::to_value(&sent).expect("sent tx serializes"))
        }
        _ => bail!("unknown method `{method}`"),
    }
}

fn write_json_line(output: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *output, value).context("failed to serialize response")?;
    writeln!(&mut *output).context("failed to write response newline")?;
    output.flush().context("failed to flush response")
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

fn request_paths(base_paths: &Paths, params: &Value) -> Result<Paths> {
    match params.get("project") {
        Some(Value::String(project_name)) => Paths::discover_with_project(Some(project_name)),
        Some(_) => bail!("project must be a string"),
        None => Ok(base_paths.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_unknown_legacy_method() {
        let paths = Paths::discover().expect("paths");
        let request = LegacyRequest {
            id: json!(1),
            method: "nope".to_owned(),
            params: json!({}),
        };

        let response = handle_legacy_request(&paths, &request).await;
        assert!(
            response
                .error
                .expect("legacy error")
                .contains("unknown method")
        );
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

    #[test]
    fn rejects_non_string_project_override() {
        let paths = Paths::discover().expect("paths");
        let error = request_paths(&paths, &json!({ "project": 1 })).expect_err("project error");
        assert!(error.to_string().contains("project must be a string"));
    }

    #[tokio::test]
    async fn mcp_requires_initialize_before_tools() {
        let paths = Paths::discover().expect("paths");
        let mut state = ServerState::default();
        let request = JsonRpcRequest {
            jsonrpc: Some(JSONRPC_VERSION.to_owned()),
            id: Some(json!(1)),
            method: "tools/list".to_owned(),
            params: json!({}),
        };

        let response = handle_jsonrpc_request(&paths, &mut state, request)
            .await
            .expect("jsonrpc response");
        assert_eq!(
            response.error.expect("error").code,
            ERROR_SERVER_NOT_INITIALIZED
        );
    }

    #[tokio::test]
    async fn initialize_advertises_tools_capability() {
        let paths = Paths::discover().expect("paths");
        let mut state = ServerState::default();
        let request = JsonRpcRequest {
            jsonrpc: Some(JSONRPC_VERSION.to_owned()),
            id: Some(json!(1)),
            method: "initialize".to_owned(),
            params: json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.1.0" }
            }),
        };

        let response = handle_jsonrpc_request(&paths, &mut state, request)
            .await
            .expect("jsonrpc response");
        assert_eq!(
            response.result.expect("result")["capabilities"]["tools"]["listChanged"],
            false
        );
    }

    #[test]
    fn tool_list_contains_wallet_tools() {
        let tools = tool_definitions();
        assert!(tools.iter().any(|tool| tool["name"] == "get_address"));
        assert!(tools.iter().any(|tool| tool["name"] == "write_contract"));
    }
}
