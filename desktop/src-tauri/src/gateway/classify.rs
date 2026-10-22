//! Failure reason from the sidecar's own log (tasks 5.4).
//!
//! The patterns are the real texts the gateway writes (`kiro/auth.py`,
//! `kiro/account_manager.py`, uvicorn); the tests use lines captured from real runs.
//! Lines are expected in chronological order.

use super::redact::strip_ansi;
use super::types::FailureKind;

/// How many trailing log lines are inspected.
pub const CLASSIFY_WINDOW: usize = 200;

/// A recognised failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub kind: FailureKind,
    /// One Chinese sentence, safe to show.
    pub message: String,
    /// The port named in the log (`port_in_use` only).
    pub port: Option<u16>,
}

/// The last `max` lines of `lines`.
pub fn window(lines: &[String], max: usize) -> &[String] {
    &lines[lines.len().saturating_sub(max)..]
}

fn login_expired() -> Classification {
    Classification { kind: FailureKind::LoginExpired, message: "kiro-cli 的登录已失效".to_string(), port: None }
}

fn port_in_use(port: Option<u16>) -> Classification {
    let message = match port {
        Some(p) => format!("{p} 端口被别的程序占用"),
        None => "端口被别的程序占用".to_string(),
    };
    Classification { kind: FailureKind::PortInUse, message, port }
}

/// A token endpoint URL: AWS SSO OIDC or the Kiro desktop refresh endpoint.
fn is_token_endpoint(line: &str) -> bool {
    (line.contains("://oidc.") && line.contains("/token")) || line.contains("/refreshToken")
}

/// `kiro/auth.py` and `kiro/account_manager.py` texts that mean "the login is no longer valid".
fn is_login_failure(line: &str) -> bool {
    // auth.py: logger.error(f"AWS SSO OIDC refresh failed: status={code}, body=...")
    if line.contains("AWS SSO OIDC refresh failed: status=401") || line.contains("AWS SSO OIDC refresh failed: status=403") {
        return true;
    }
    // httpx's raise_for_status text, surfaced by account_manager ("Failed to initialize account ...").
    if (line.contains("Client error '401") || line.contains("Client error '403")) && is_token_endpoint(line) {
        return true;
    }
    // auth.py get_access_token(): refresh failed with 400 and the stored access token is gone too.
    line.contains("Token expired and refresh failed") && line.contains("kiro-cli login")
}

/// Success lines of `auth.py` that make an earlier refresh failure obsolete.
fn is_refresh_success(line: &str) -> bool {
    line.contains("Token refreshed via AWS SSO OIDC") || line.contains("Token refreshed via Kiro Desktop Auth")
}

/// uvicorn: `[Errno 48] error while attempting to bind on address ('127.0.0.1', 8150): address already in use`.
fn bind_failure(line: &str) -> Option<Option<u16>> {
    if !line.contains("error while attempting to bind on address") {
        return None;
    }
    let lower = line.to_ascii_lowercase();
    if lower.contains("permission denied") || lower.contains("errno 13") {
        return None;
    }
    let port = line
        .split_once("bind on address")
        .and_then(|(_, rest)| rest.split_once(','))
        .map(|(_, after)| after.trim_start())
        .map(|after| after.chars().take_while(char::is_ascii_digit).collect::<String>())
        .and_then(|digits| digits.parse::<u16>().ok());
    Some(port)
}

/// Decide why the gateway failed from its log lines.
///
/// The most recent recognised event wins. A token refresh that succeeded after a login
/// failure cancels it (the gateway recovered). A bind failure is never cancelled: it
/// always comes after account setup and ends the process.
///
/// Args:
///     lines: Log lines in chronological order (colored or plain).
///
/// Returns:
///     The classification, or `None` when nothing recognisable is in the lines.
pub fn classify(lines: &[String]) -> Option<Classification> {
    let mut current: Option<Classification> = None;
    for raw in lines {
        let line = strip_ansi(raw);
        if let Some(port) = bind_failure(&line) {
            current = Some(port_in_use(port));
        } else if is_login_failure(&line) {
            current = Some(login_expired());
        } else if is_refresh_success(&line) && current.as_ref().is_some_and(|c| c.kind == FailureKind::LoginExpired) {
            current = None;
        }
    }
    current
}

#[cfg(test)]
mod tests;
