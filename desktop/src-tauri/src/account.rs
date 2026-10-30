//! Desktop account IPC. Credential ownership and refresh remain in the Python sidecar.
use crate::gateway::commands::CommandError;
use crate::gateway::service::GatewayService;
use crate::gateway::types::GatewayState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::State;

/// Public snapshot only: no token, client secret, raw error or credential path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub email: Option<String>,
    pub provider: String,
    pub subscription: Option<String>,
    pub used: Option<f64>,
    pub total: Option<f64>,
    pub remaining: Option<f64>,
    pub usage_percent: Option<f64>,
    pub reset_at: Option<String>,
    pub overage_enabled: bool,
    pub overage_used: Option<f64>,
    pub token_expires_at: Option<String>,
    pub can_refresh_token: bool,
    pub checked_at: String,
    pub token_refreshed_at: Option<String>,
    pub usage_error: Option<String>,
}

fn error(code: &'static str, message: &str) -> CommandError {
    CommandError { code, message: message.into() }
}

/// Talk exclusively to loopback; neither redirects nor system proxies may see the key.
fn fetch(port: u16, secret: &str, refresh: bool, timeout: Duration) -> Result<AccountSnapshot, CommandError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout)).proxy(None).http_status_as_error(false)
        .max_redirects(0).max_redirects_will_error(false).build().into();
    let base = format!("http://127.0.0.1:{port}/desktop/account");
    let response = if refresh {
        agent.post(format!("{base}/refresh-token")).header("Authorization", format!("Bearer {secret}")).send_empty()
    } else {
        agent.get(base).header("Authorization", format!("Bearer {secret}")).call()
    }.map_err(|_| error("gateway_unreachable", "账号操作未完成，请稍后刷新额度确认状态"))?;
    match response.status().as_u16() {
        200 => {
            let body = response.into_body().with_config().limit(64 * 1024).read_to_string()
                .map_err(|_| error("gateway_unreachable", "无法读取账号信息，请重试"))?;
            serde_json::from_str(&body).map_err(|_| error("gateway_unreachable", "账号信息格式异常，请更新应用后重试"))
        }
        409 => Err(error("env_not_ready", "CLI 登录已变化，请重启网关同步当前账号")),
        404 => Err(error("gateway_unreachable", "当前网关不支持账号信息，请重启或更新应用")),
        502 => Err(error("env_not_ready", "账号操作失败，请检查网络或重新登录 kiro-cli 后重试")),
        _ => Err(error("gateway_unreachable", "账号信息暂时不可用，请稍后重试")),
    }
}

async fn request(service: Arc<GatewayService>, refresh: bool) -> Result<AccountSnapshot, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let before = service.status();
        if before.state != GatewayState::Running {
            return Err(error("gateway_not_running", "启动网关后可查看账号和刷新凭证"));
        }
        let snapshot = fetch(before.port, &service.config().secret(), refresh, Duration::from_secs(75))?;
        let after = service.status();
        if after.state != GatewayState::Running || before.pid != after.pid || before.since != after.since {
            return Err(error("gateway_not_running", "网关状态已变化，请刷新账号信息"));
        }
        Ok(snapshot)
    }).await.map_err(|_| error("internal", "账号操作未完成，请重试"))?
}

/// Fetch live quota and the account used by the running gateway.
#[tauri::command]
pub async fn account_get(service: State<'_, Arc<GatewayService>>) -> Result<AccountSnapshot, CommandError> {
    request(service.inner().clone(), false).await
}

/// Explicitly renew the running gateway's credentials, then update its quota snapshot.
#[tauri::command]
pub async fn account_refresh_token(service: State<'_, Arc<GatewayService>>) -> Result<AccountSnapshot, CommandError> {
    request(service.inner().clone(), true).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::testing::TestServer;

    #[test]
    fn refresh_is_an_authenticated_post_and_provider_errors_are_not_exposed() {
        let server = TestServer::start(|_| Some(TestServer::json(502, "secret-token-and-provider-error")));
        let failure = fetch(server.port, "local-key", true, Duration::from_secs(1)).unwrap_err();
        assert!(!failure.message.contains("secret-token"));
        let request = server.requests.lock().unwrap()[0].clone();
        assert!(request.starts_with("POST /desktop/account/refresh-token "));
        assert!(request.to_lowercase().contains("authorization: bearer local-key"));
    }

    #[test]
    fn rejects_redirects_and_maps_changed_source() {
        for (status, code) in [(302, "gateway_unreachable"), (409, "env_not_ready"), (404, "gateway_unreachable")] {
            let server = TestServer::start(move |_| Some(TestServer::json(status, "{}")));
            assert_eq!(fetch(server.port, "local-key", false, Duration::from_secs(1)).unwrap_err().code, code);
        }
    }

    #[test]
    fn snapshot_discards_unknown_secret_fields() {
        let body = r#"{"email":null,"provider":"AWS SSO","subscription":null,"used":null,"total":null,"remaining":null,"usagePercent":null,"resetAt":null,"overageEnabled":false,"overageUsed":null,"tokenExpiresAt":null,"canRefreshToken":false,"checkedAt":"2026-10-30T00:00:00Z","tokenRefreshedAt":null,"usageError":null,"accessToken":"must-not-cross-ipc"}"#;
        let server = TestServer::start(move |_| Some(TestServer::json(200, body)));
        let value = fetch(server.port, "local-key", false, Duration::from_secs(1)).unwrap();
        assert!(value.remaining.is_none());
        assert!(!serde_json::to_string(&value).unwrap().contains("must-not-cross-ipc"));
    }
}
