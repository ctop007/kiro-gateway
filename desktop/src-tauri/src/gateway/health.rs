//! Talking to the local gateway over HTTP: the `/health` probe and the `/v1/models`
//! request made on behalf of the frontend (the frontend's CSP forbids direct access).
//!
//! Plain HTTP on `127.0.0.1` only. System proxy settings are ignored on purpose: a
//! configured HTTP proxy must never see the access key or be asked for a loopback URL.

use std::time::Duration;
use ureq::Agent;

/// Whether the gateway answers on a port. A trait so the manager can be tested with scripts.
pub trait HealthProbe: Send + Sync {
    /// True when `GET http://127.0.0.1:<port>/health` answers 200 in time.
    fn is_healthy(&self, port: u16) -> bool;
}

fn agent(timeout: Duration) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(timeout))
        .proxy(None)
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .into()
}

/// Real probe.
pub struct HttpHealth {
    agent: Agent,
}

impl HttpHealth {
    /// Args:
    ///     timeout: Limit for one whole request (connect, send, receive).
    pub fn new(timeout: Duration) -> Self {
        Self { agent: agent(timeout) }
    }
}

impl HealthProbe for HttpHealth {
    fn is_healthy(&self, port: u16) -> bool {
        match self.agent.get(format!("http://127.0.0.1:{port}/health")).call() {
            Ok(response) => response.status().as_u16() == 200,
            Err(_) => false,
        }
    }
}

/// Why the model list could not be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsError {
    /// The gateway is not running (checked by the caller before asking).
    NotRunning,
    /// No answer in time, connection refused, or a non-200 status (including a wrong key).
    Unreachable,
    /// Something answered 200 but it is not a model list.
    BadResponse,
}

impl ModelsError {
    /// The `code` of the IPC error shape.
    pub fn code(self) -> &'static str {
        match self {
            ModelsError::NotRunning => "gateway_not_running",
            ModelsError::Unreachable | ModelsError::BadResponse => "gateway_unreachable",
        }
    }

    /// One Chinese sentence, safe to show: no URL, port or key.
    pub fn message(self) -> &'static str {
        match self {
            ModelsError::NotRunning => "网关没有运行，启动后才能读取模型列表",
            ModelsError::Unreachable => "网关没有响应，稍后重试",
            ModelsError::BadResponse => "网关返回的模型列表无法识别",
        }
    }
}

/// Ask the gateway for its model ids.
///
/// Args:
///     port: Gateway port on `127.0.0.1`.
///     secret: The access key, sent as `Authorization: Bearer`.
///     timeout: Limit for the whole request (the contract says 5 seconds).
///
/// Returns:
///     Ids sorted and without duplicates, or why not. Entries without a non-empty string
///     `id` are skipped.
pub fn fetch_models(port: u16, secret: &str, timeout: Duration) -> Result<Vec<String>, ModelsError> {
    let response = agent(timeout)
        .get(format!("http://127.0.0.1:{port}/v1/models"))
        .header("Authorization", format!("Bearer {secret}"))
        .call()
        .map_err(|_| ModelsError::Unreachable)?;
    if response.status().as_u16() != 200 {
        return Err(ModelsError::Unreachable);
    }
    let text = response.into_body().read_to_string().map_err(|_| ModelsError::Unreachable)?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| ModelsError::BadResponse)?;
    let entries = value.get("data").and_then(|d| d.as_array()).ok_or(ModelsError::BadResponse)?;
    let mut ids: Vec<String> = entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()))
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests;
