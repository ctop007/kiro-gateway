use super::*;
use crate::gateway::types::FailureKind;

/// Builds a line in the exact shape main.py configures for loguru (colorize=True).
fn colored(level: &str, location: &str, message: &str) -> String {
    let (open, close) = match level {
        "ERROR" => ("\x1b[31m\x1b[1m", "\x1b[0m"),
        "WARNING" => ("\x1b[33m\x1b[1m", "\x1b[0m"),
        _ => ("\x1b[1m", "\x1b[0m"),
    };
    let mut parts = location.splitn(3, ':');
    let (name, func, line) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
    format!(
        "\x1b[32m2026-10-22 22:49:08\x1b[0m | {open}{level: <8}{close} | \x1b[36m{name}\x1b[0m:\x1b[36m{func}\x1b[0m:\x1b[36m{line}\x1b[0m - {open}{message}{close}"
    )
}

fn plain(level: &str, location: &str, message: &str) -> String {
    format!("2026-10-22 22:49:08 | {level: <8} | {location} - {message}")
}

// The messages below are copied from real gateway output (see progress/config-gateway.md).
const SSO_403: &str = "AWS SSO OIDC refresh failed: status=403, body={\"message\":\"forbidden\"}";
const SSO_401: &str = "AWS SSO OIDC refresh failed: status=401, body={\"error\":\"invalid_client\",\"error_description\":\"Client authentication failed\"}";
const INIT_403: &str = "Failed to initialize account /Users/x/.aws/sso/cache/kiro-auth-token.json: Client error '403 Forbidden' for url 'https://oidc.us-east-1.amazonaws.com/token'";
const INIT_401_DESKTOP: &str = "Failed to initialize account /Users/x/.aws/sso/cache/kiro-auth-token.json: Client error '401 Unauthorized' for url 'https://prod.us-east-1.auth.desktop.kiro.dev/refreshToken'";
const INIT_EXPIRED: &str = "Failed to initialize account /Users/x/data.sqlite3: Token expired and refresh failed. Please run 'kiro-cli login' to refresh your credentials.";
const BIND: &str = "[Errno 48] error while attempting to bind on address ('127.0.0.1', 8150): address already in use";

#[test]
fn sso_refresh_403_and_401_mean_login_expired() {
    for msg in [SSO_403, SSO_401] {
        let lines = vec![colored("ERROR", "kiro.auth:_do_aws_sso_oidc_refresh:840", msg)];
        assert_eq!(classify(&lines).unwrap().kind, FailureKind::LoginExpired, "{msg}");
    }
}

#[test]
fn account_initialization_failing_with_401_or_403_on_a_token_url_is_login_expired() {
    for msg in [INIT_403, INIT_401_DESKTOP] {
        let lines = vec![colored("ERROR", "kiro.account_manager:_initialize_account:578", msg)];
        assert_eq!(classify(&lines).unwrap().kind, FailureKind::LoginExpired, "{msg}");
    }
}

#[test]
fn refresh_failing_with_400_then_asking_for_kiro_cli_login_is_login_expired() {
    let lines = vec![colored("ERROR", "kiro.account_manager:_initialize_account:578", INIT_EXPIRED)];
    assert_eq!(classify(&lines).unwrap().kind, FailureKind::LoginExpired);
}

#[test]
fn works_on_plain_uncolored_lines_too() {
    let lines = vec![plain("ERROR", "kiro.auth:_do_aws_sso_oidc_refresh:840", SSO_403)];
    assert_eq!(classify(&lines).unwrap().kind, FailureKind::LoginExpired);
}

#[test]
fn real_startup_failure_sequence_is_login_expired() {
    // Order and content of a real failed start: refresh error, details, init error,
    // uvicorn startup abort, traceback.
    let lines = vec![
        colored("INFO", "kiro.auth:_do_aws_sso_oidc_refresh:809", "Refreshing Kiro token via AWS SSO OIDC..."),
        colored("ERROR", "kiro.auth:_do_aws_sso_oidc_refresh:840", SSO_403),
        colored("ERROR", "kiro.auth:_do_aws_sso_oidc_refresh:847", "AWS SSO OIDC error details: error=unknown, description=no description"),
        colored("ERROR", "kiro.account_manager:_initialize_account:578", INIT_403),
        "For more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/403".to_string(),
        colored("ERROR", "main:lifespan:499", "Failed to initialize any account. Check your credentials."),
        "RuntimeError: Failed to initialize any account".to_string(),
    ];
    let c = classify(&lines).unwrap();
    assert_eq!(c.kind, FailureKind::LoginExpired);
    assert_eq!(c.message, "kiro-cli 的登录已失效");
}

#[test]
fn uvicorn_bind_error_means_port_in_use_on_every_platform_wording() {
    for msg in [
        BIND.to_string(),
        "[Errno 98] error while attempting to bind on address ('127.0.0.1', 8000): address already in use".to_string(),
        "[Errno 10048] error while attempting to bind on address ('127.0.0.1', 8000): [winerror 10048] only one usage of each socket address is normally permitted".to_string(),
    ] {
        let lines = vec![colored("ERROR", "logging:callHandlers:1737", &msg)];
        let c = classify(&lines).unwrap();
        assert_eq!(c.kind, FailureKind::PortInUse, "{msg}");
    }
}

#[test]
fn port_in_use_message_has_the_port_taken_from_the_log() {
    let lines = vec![plain("ERROR", "logging:callHandlers:1737", BIND)];
    let c = classify(&lines).unwrap();
    assert_eq!(c.port, Some(8150));
    assert_eq!(c.message, "8150 端口被别的程序占用");
}

#[test]
fn bind_error_without_a_parsable_port_still_classifies() {
    let lines = vec!["error while attempting to bind on address".to_string()];
    let c = classify(&lines).unwrap();
    assert_eq!(c.kind, FailureKind::PortInUse);
    assert_eq!(c.port, None);
}

#[test]
fn unrelated_errors_are_not_classified() {
    for msg in [
        "Traceback (most recent call last):",
        "ModuleNotFoundError: No module named 'main'",
        "Received 403, refreshing token (attempt 1/3)",
        "Client error '403 Forbidden' for url 'https://runtime.us-east-1.kiro.dev/generateAssistantResponse'",
        "Client error '404 Not Found' for url 'https://oidc.us-east-1.amazonaws.com/token'",
        "Client error '429 Too Many Requests' for url 'https://oidc.us-east-1.amazonaws.com/token'",
        "Token refresh failed with 400 after SQLite reload. This may happen if kiro-cli refreshed tokens in memory without persisting.",
        "Using existing access_token until it expires. Run 'kiro-cli login' when convenient to refresh credentials.",
    ] {
        let lines = vec![plain("ERROR", "kiro.x:y:1", msg)];
        assert_eq!(classify(&lines), None, "{msg}");
    }
}

#[test]
fn empty_input_is_not_classified() {
    assert_eq!(classify(&[]), None);
}

#[test]
fn a_refresh_that_succeeded_after_an_old_failure_is_not_login_expired() {
    let lines = vec![
        colored("ERROR", "kiro.auth:_do_aws_sso_oidc_refresh:840", SSO_403),
        colored("INFO", "kiro.auth:_do_aws_sso_oidc_refresh:871", "Token refreshed via AWS SSO OIDC, expires: 2026-10-22T15:00:00+00:00"),
        colored("INFO", "main:lifespan:510", "Account system initialized successfully"),
    ];
    assert_eq!(classify(&lines), None);
}

#[test]
fn a_failure_after_an_earlier_success_is_login_expired() {
    let lines = vec![
        colored("INFO", "kiro.auth:_do_aws_sso_oidc_refresh:871", "Token refreshed via AWS SSO OIDC, expires: 2026-10-22T15:00:00+00:00"),
        colored("ERROR", "kiro.auth:_do_aws_sso_oidc_refresh:840", SSO_403),
    ];
    assert_eq!(classify(&lines).unwrap().kind, FailureKind::LoginExpired);
}

#[test]
fn the_kiro_desktop_refresh_success_line_also_clears_an_old_failure() {
    let lines = vec![
        colored("ERROR", "kiro.account_manager:_initialize_account:578", INIT_401_DESKTOP),
        colored("INFO", "kiro.auth:_refresh_token_kiro_desktop:744", "Token refreshed via Kiro Desktop Auth, expires: 2026-10-22T15:00:00+00:00"),
    ];
    assert_eq!(classify(&lines), None);
}

#[test]
fn the_most_recent_matching_event_wins_between_port_and_login() {
    let port_last = vec![
        colored("ERROR", "kiro.auth:_do_aws_sso_oidc_refresh:840", SSO_403),
        colored("ERROR", "logging:callHandlers:1737", BIND),
    ];
    assert_eq!(classify(&port_last).unwrap().kind, FailureKind::PortInUse);
    let login_last = vec![
        colored("ERROR", "logging:callHandlers:1737", BIND),
        colored("ERROR", "kiro.auth:_do_aws_sso_oidc_refresh:840", SSO_403),
    ];
    assert_eq!(classify(&login_last).unwrap().kind, FailureKind::LoginExpired);
}

#[test]
fn only_the_given_window_is_inspected() {
    let mut lines = vec![colored("ERROR", "kiro.auth:_do_aws_sso_oidc_refresh:840", SSO_403)];
    lines.extend((0..60).map(|n| plain("INFO", "kiro.x:y:1", &format!("request {n}"))));
    assert_eq!(classify(&lines).unwrap().kind, FailureKind::LoginExpired);
    assert_eq!(classify(window(&lines, 40)), None);
    assert_eq!(window(&lines, 1000).len(), lines.len());
}

#[test]
fn messages_never_contain_log_text_urls_or_paths() {
    let lines = vec![colored("ERROR", "kiro.account_manager:_initialize_account:578", INIT_403)];
    let c = classify(&lines).unwrap();
    assert!(!c.message.contains("http"));
    assert!(!c.message.contains('/'));
}
