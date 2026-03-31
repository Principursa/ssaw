use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

pub struct RpcClient {
    rpc_url: String,
}

impl RpcClient {
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
        }
    }

    pub fn request<T>(&self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let response: Value = ureq::post(&self.rpc_url)
            .set("content-type", "application/json")
            .send_json(body)
            .with_context(|| format!("rpc request `{method}` failed"))?
            .into_json()
            .context("failed to decode rpc response JSON")?;

        if let Some(error) = response.get("error") {
            bail!("rpc `{method}` error: {error}");
        }

        let result = response
            .get("result")
            .cloned()
            .with_context(|| format!("rpc `{method}` response was missing result"))?;

        serde_json::from_value(result)
            .with_context(|| format!("failed to parse rpc `{method}` result"))
    }
}
