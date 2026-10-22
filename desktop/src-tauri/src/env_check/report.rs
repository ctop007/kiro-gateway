//! Compose the `EnvReport` of `docs/ipc-contract.md` section 3 from probe results.
//!
//! [`build_report`] is a pure function of [`ReportInputs`]; [`run_env_check`]
//! gathers the real inputs. User-visible wording follows `docs/mockup/index.html`.
//!
//! The report never contains token material: the only login facts it can carry
//! come from `LoginInfo` (method, expiry, refreshable).

use super::login::{self, LoginInfo, LoginProbe};
use super::which::{self, Tool};
use super::{Platform, UserDirs};
use chrono::{DateTime, FixedOffset, Local, SecondsFormat};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// kiro-cli official install page.
pub const KIRO_CLI_INSTALL_URL: &str = "https://kiro.dev/cli/";
/// Claude Code install instructions. Not opened from this sandbox, so the exact
/// page is unverified; it must stay an https URL (the opener capability only allows https).
pub const CLAUDE_CODE_INSTALL_URL: &str = "https://docs.claude.com/en/docs/claude-code/setup";

/// Which of the four checks an item belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckId {
    KiroCliInstalled,
    KiroCliLoggedIn,
    LoginValid,
    ClaudeInstalled,
}

/// Result of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Fail,
    Skipped,
}

/// Button the user can press to repair a failed check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FixAction {
    OpenUrl { label: String, url: String },
    CopyCommand { label: String, command: String },
}

/// One row of the check list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckItem {
    pub id: CheckId,
    pub status: CheckStatus,
    pub title: String,
    pub detail: String,
    pub fix: Option<FixAction>,
}

/// Response of the `env_check` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvReport {
    pub checked_at: String,
    pub items: Vec<CheckItem>,
    /// `~`-abbreviated, display only.
    pub kiro_cli_path: Option<String>,
    /// `~`-abbreviated, display only.
    pub claude_path: Option<String>,
    pub login_expires_at: Option<String>,
    /// `<absolute kiro-cli path> login`, quoted for the user's shell when needed.
    pub login_command: Option<String>,
}

/// Whether the gateway switch may be turned on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayGate {
    Allowed,
    /// Switch stays off; `reason` is shown next to it.
    Blocked { reason: String },
}

/// Everything [`build_report`] needs.
#[derive(Debug, Clone)]
pub struct ReportInputs {
    pub kiro_cli: Option<PathBuf>,
    pub claude: Option<PathBuf>,
    pub login: LoginProbe,
    /// Used only to abbreviate paths for display.
    pub home: Option<PathBuf>,
    pub now: DateTime<FixedOffset>,
}

/// Abbreviate the home directory as `~` (display only).
///
/// `/Users/meow` is not inside `/Users/me`: matching is per path component.
pub fn display_path(path: &Path, home: Option<&PathBuf>) -> String {
    if let Some(home) = home {
        if let Ok(rest) = path.strip_prefix(home) {
            let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
            return if parts.is_empty() { "~".to_string() } else { format!("~/{}", parts.join("/")) };
        }
    }
    path.to_string_lossy().to_string()
}

fn needs_quoting(text: &str, windows: bool) -> bool {
    if text.is_empty() {
        return true;
    }
    text.chars().any(|c| {
        let safe = c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '+' | '@' | ',' | '=');
        let safe_windows = windows && (c == '\\');
        !(safe || safe_windows)
    })
}

/// `<absolute kiro-cli path> login`, with quotes when the path needs them.
///
/// POSIX shells get single quotes (`'` is written as `'\''`); Windows gets double
/// quotes. Note for Windows: PowerShell needs a leading `& ` to run a quoted
/// path, cmd.exe does not accept it; the plain double-quoted form works in cmd.exe.
pub fn login_command(kiro_cli: &Path, windows: bool) -> String {
    let path = kiro_cli.to_string_lossy();
    if !needs_quoting(&path, windows) {
        return format!("{path} login");
    }
    if windows {
        format!("\"{path}\" login")
    } else {
        format!("'{}' login", path.replace('\'', "'\\''"))
    }
}

fn item(id: CheckId, status: CheckStatus, title: &str, detail: &str, fix: Option<FixAction>) -> CheckItem {
    CheckItem { id, status, title: title.to_string(), detail: detail.to_string(), fix }
}

fn copy_login(command: &str) -> FixAction {
    FixAction::CopyCommand { label: "复制".to_string(), command: command.to_string() }
}

fn format_expiry_for_detail(expiry: DateTime<FixedOffset>) -> String {
    expiry.format("%Y-%m-%d %H:%M").to_string()
}

fn as_rfc3339(t: DateTime<FixedOffset>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, false)
}

/// Build the four-item report.
///
/// Args:
///     inputs: Tool locations, the database probe, home directory and clock.
///
/// Returns:
///     The report; items follow the order of the IPC contract.
pub fn build_report(inputs: &ReportInputs) -> EnvReport {
    let windows = cfg!(windows);
    let home = inputs.home.as_ref();
    let tz = *inputs.now.offset();
    let mut items = Vec::with_capacity(4);
    let mut login_expires_at = None;
    let mut login_cmd = None;

    match &inputs.kiro_cli {
        None => {
            items.push(item(
                CheckId::KiroCliInstalled,
                CheckStatus::Fail,
                "没找到 kiro-cli",
                "网关要借用 kiro-cli 的登录",
                Some(FixAction::OpenUrl { label: "打开安装页面".to_string(), url: KIRO_CLI_INSTALL_URL.to_string() }),
            ));
            items.push(item(CheckId::KiroCliLoggedIn, CheckStatus::Skipped, "kiro-cli 登录", "装好 kiro-cli 后再检测", None));
            items.push(item(CheckId::LoginValid, CheckStatus::Skipped, "登录有效", "登录后再检测", None));
        }
        Some(path) => {
            let command = login_command(path, windows);
            login_cmd = Some(command.clone());
            items.push(item(
                CheckId::KiroCliInstalled,
                CheckStatus::Ok,
                "kiro-cli 已安装",
                &display_path(path, home),
                None,
            ));
            match &inputs.login {
                LoginProbe::Found(info) => {
                    login_expires_at = info.expires_at.map(|t| as_rfc3339(t.with_timezone(&tz)));
                    items.push(item(CheckId::KiroCliLoggedIn, CheckStatus::Ok, "kiro-cli 已登录", info.method.label(), None));
                    items.push(validity_item(info, inputs.now, &command));
                }
                other => {
                    let detail = if matches!(other, LoginProbe::Unreadable) {
                        "读不到 kiro-cli 的登录数据。在终端运行这条命令，浏览器里完成登录后点「重新检测」"
                    } else {
                        "在终端运行这条命令，浏览器里完成登录后点「重新检测」"
                    };
                    items.push(item(
                        CheckId::KiroCliLoggedIn,
                        CheckStatus::Fail,
                        "kiro-cli 还没登录",
                        detail,
                        Some(copy_login(&command)),
                    ));
                    items.push(item(CheckId::LoginValid, CheckStatus::Skipped, "登录有效", "登录后再检测", None));
                }
            }
        }
    }

    match &inputs.claude {
        Some(path) => items.push(item(
            CheckId::ClaudeInstalled,
            CheckStatus::Ok,
            "Claude Code 已安装",
            &display_path(path, home),
            None,
        )),
        None => items.push(item(
            CheckId::ClaudeInstalled,
            CheckStatus::Fail,
            "没找到 Claude Code",
            "网关照样能开，但 kclaude 要等装好 Claude Code 才能用",
            Some(FixAction::OpenUrl { label: "查看安装方法".to_string(), url: CLAUDE_CODE_INSTALL_URL.to_string() }),
        )),
    }

    EnvReport {
        checked_at: as_rfc3339(inputs.now),
        items,
        kiro_cli_path: inputs.kiro_cli.as_ref().map(|p| display_path(p, home)),
        claude_path: inputs.claude.as_ref().map(|p| display_path(p, home)),
        login_expires_at,
        login_command: login_cmd,
    }
}

fn validity_item(info: &LoginInfo, now: DateTime<FixedOffset>, command: &str) -> CheckItem {
    if !info.is_valid(now) {
        return item(
            CheckId::LoginValid,
            CheckStatus::Fail,
            "登录已失效",
            "重新登录一次就好",
            Some(copy_login(command)),
        );
    }
    let detail = match (info.refreshable, info.expires_at) {
        (true, _) => "到期后会自动续期".to_string(),
        (false, Some(expiry)) => format!("有效期至 {}", format_expiry_for_detail(expiry.with_timezone(now.offset()))),
        (false, None) => "到期后会自动续期".to_string(),
    };
    item(CheckId::LoginValid, CheckStatus::Ok, "登录有效", &detail, None)
}

/// Decide whether the gateway switch may be turned on.
///
/// The three kiro-cli items must all be `ok`; `skipped` and missing items block
/// as well. The Claude Code item never matters here.
pub fn gateway_gate(report: &EnvReport) -> GatewayGate {
    const REQUIRED: [(CheckId, &str); 3] = [
        (CheckId::KiroCliInstalled, "先安装 kiro-cli 才能打开"),
        (CheckId::KiroCliLoggedIn, "先登录 kiro-cli 才能打开"),
        (CheckId::LoginValid, "kiro-cli 的登录已失效，重新登录后才能打开"),
    ];
    for (id, reason) in REQUIRED {
        let passed = report.items.iter().any(|i| i.id == id && i.status == CheckStatus::Ok);
        if !passed {
            return GatewayGate::Blocked { reason: reason.to_string() };
        }
    }
    GatewayGate::Allowed
}

/// Text for the kclaude area when Claude Code is missing; `None` when it is usable.
#[cfg(test)]
pub fn kclaude_unavailable_reason(report: &EnvReport) -> Option<String> {
    let installed = report.items.iter().any(|i| i.id == CheckId::ClaudeInstalled && i.status == CheckStatus::Ok);
    if installed {
        None
    } else {
        Some("需先安装 Claude Code".to_string())
    }
}

/// Run all checks against this machine.
///
/// Probes the login shell PATH (cached, at most 3 s the first time) and reads the
/// kiro-cli database read-only.
pub fn run_env_check() -> EnvReport {
    let platform = Platform::current();
    let dirs = UserDirs::from_process();
    let inputs = ReportInputs {
        kiro_cli: which::locate(Tool::KiroCli),
        claude: which::locate(Tool::Claude),
        login: login::probe_default(platform, &dirs),
        home: dirs.home.clone(),
        now: Local::now().fixed_offset(),
    };
    build_report(&inputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env_check::login::{LoginInfo, LoginMethod, LoginProbe};
    use chrono::{DateTime, FixedOffset, TimeZone};
    use std::path::PathBuf;

    fn now() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2026, 10, 22, 12, 0, 0).unwrap()
    }

    fn expiry(year: i32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap()
    }

    fn login(method: LoginMethod, year: i32, refreshable: bool) -> LoginProbe {
        LoginProbe::Found(LoginInfo { method, expires_at: Some(expiry(year)), refreshable })
    }

    fn inputs(kiro: Option<&str>, claude: Option<&str>, probe: LoginProbe) -> ReportInputs {
        ReportInputs {
            kiro_cli: kiro.map(PathBuf::from),
            claude: claude.map(PathBuf::from),
            login: probe,
            home: Some(PathBuf::from("/Users/me")),
            now: now(),
        }
    }

    fn statuses(r: &EnvReport) -> Vec<CheckStatus> {
        r.items.iter().map(|i| i.status).collect()
    }

    fn all_good() -> EnvReport {
        build_report(&inputs(
            Some("/Users/me/.local/bin/kiro-cli"),
            Some("/Users/me/.local/bin/claude"),
            login(LoginMethod::AwsSso, 2030, true),
        ))
    }

    // ---------- shape ----------

    #[test]
    fn report_has_four_items_in_contract_order() {
        let r = all_good();
        let ids: Vec<CheckId> = r.items.iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            vec![CheckId::KiroCliInstalled, CheckId::KiroCliLoggedIn, CheckId::LoginValid, CheckId::ClaudeInstalled]
        );
    }

    #[test]
    fn everything_ok_report() {
        let r = all_good();
        assert_eq!(statuses(&r), vec![CheckStatus::Ok; 4]);
        assert!(r.items.iter().all(|i| i.fix.is_none()));
        assert_eq!(r.items[0].title, "kiro-cli 已安装");
        assert_eq!(r.items[0].detail, "~/.local/bin/kiro-cli");
        assert_eq!(r.items[1].title, "kiro-cli 已登录");
        assert_eq!(r.items[1].detail, "AWS SSO 登录");
        assert_eq!(r.items[2].title, "登录有效");
        assert_eq!(r.items[2].detail, "到期后会自动续期");
        assert_eq!(r.items[3].title, "Claude Code 已安装");
        assert_eq!(r.items[3].detail, "~/.local/bin/claude");
        assert_eq!(r.kiro_cli_path.as_deref(), Some("~/.local/bin/kiro-cli"));
        assert_eq!(r.claude_path.as_deref(), Some("~/.local/bin/claude"));
        assert_eq!(r.login_command.as_deref(), Some("/Users/me/.local/bin/kiro-cli login"));
        assert_eq!(r.login_expires_at.as_deref(), Some("2030-01-01T00:00:00+08:00"));
        assert_eq!(r.checked_at, "2026-10-22T12:00:00+08:00");
    }

    #[test]
    fn json_shape_matches_ipc_contract() {
        let r = build_report(&inputs(None, None, LoginProbe::NoDatabase));
        let v = serde_json::to_value(&r).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["checkedAt", "claudePath", "items", "kiroCliPath", "loginCommand", "loginExpiresAt"]
        );
        // Optional values are null, never omitted.
        assert!(v["kiroCliPath"].is_null() && v["claudePath"].is_null());
        assert!(v["loginExpiresAt"].is_null() && v["loginCommand"].is_null());
        let first = &v["items"][0];
        assert_eq!(first["id"], "kiro_cli_installed");
        assert_eq!(first["status"], "fail");
        assert_eq!(first["fix"]["kind"], "open_url");
        assert_eq!(first["fix"]["label"], "打开安装页面");
        assert_eq!(first["fix"]["url"], KIRO_CLI_INSTALL_URL);
        assert_eq!(v["items"][1]["status"], "skipped");
        assert!(v["items"][1]["fix"].is_null());
    }

    #[test]
    fn copy_command_fix_serializes_with_kind_label_command() {
        let r = build_report(&inputs(Some("/bin/kiro-cli"), None, LoginProbe::NoCredentials));
        let v = serde_json::to_value(&r.items[1].fix).unwrap();
        assert_eq!(v["kind"], "copy_command");
        assert_eq!(v["label"], "复制");
        assert_eq!(v["command"], "/bin/kiro-cli login");
    }

    // ---------- kiro-cli not installed ----------

    #[test]
    fn missing_kiro_cli_fails_and_skips_both_login_items() {
        let r = build_report(&inputs(None, Some("/usr/local/bin/claude"), login(LoginMethod::Social, 2030, true)));
        assert_eq!(
            statuses(&r),
            vec![CheckStatus::Fail, CheckStatus::Skipped, CheckStatus::Skipped, CheckStatus::Ok]
        );
        assert_eq!(r.items[0].title, "没找到 kiro-cli");
        assert_eq!(r.items[0].detail, "网关要借用 kiro-cli 的登录");
        assert_eq!(r.items[1].title, "kiro-cli 登录");
        assert_eq!(r.items[1].detail, "装好 kiro-cli 后再检测");
        assert_eq!(r.items[2].title, "登录有效");
        assert_eq!(r.items[2].detail, "登录后再检测");
        assert_eq!(r.login_command, None);
        assert_eq!(r.login_expires_at, None, "a database without kiro-cli must not leak a stale expiry");
        assert!(matches!(&r.items[0].fix, Some(FixAction::OpenUrl { url, .. }) if url.starts_with("https://")));
    }

    // ---------- not logged in ----------

    #[test]
    fn every_probe_without_a_usable_row_means_not_logged_in() {
        for probe in [LoginProbe::NoDatabase, LoginProbe::NoCredentials, LoginProbe::Corrupt, LoginProbe::Unreadable] {
            let r = build_report(&inputs(Some("/Users/me/.local/bin/kiro-cli"), None, probe.clone()));
            assert_eq!(r.items[0].status, CheckStatus::Ok, "{probe:?}");
            assert_eq!(r.items[1].status, CheckStatus::Fail, "{probe:?}");
            assert_eq!(r.items[2].status, CheckStatus::Skipped, "{probe:?}");
            assert_eq!(r.items[1].title, "kiro-cli 还没登录");
            assert!(r.items[1].detail.ends_with("在终端运行这条命令，浏览器里完成登录后点「重新检测」"));
            match &r.items[1].fix {
                Some(FixAction::CopyCommand { label, command }) => {
                    assert_eq!(label, "复制");
                    assert_eq!(command, "/Users/me/.local/bin/kiro-cli login");
                }
                other => panic!("expected copy_command, got {other:?}"),
            }
            assert_eq!(r.login_expires_at, None);
        }
    }

    #[test]
    fn unreadable_database_says_so_in_the_detail() {
        let r = build_report(&inputs(Some("/bin/kiro-cli"), None, LoginProbe::Unreadable));
        assert_ne!(r.items[1].detail, build_report(&inputs(Some("/bin/kiro-cli"), None, LoginProbe::NoDatabase)).items[1].detail);
        assert!(r.items[1].detail.contains("读不到"));
    }

    // ---------- login validity ----------

    #[test]
    fn expired_but_refreshable_login_passes() {
        let r = build_report(&inputs(Some("/bin/kiro-cli"), None, login(LoginMethod::Social, 2020, true)));
        assert_eq!(r.items[2].status, CheckStatus::Ok);
        assert_eq!(r.items[1].detail, "社交账号登录");
        assert_eq!(r.login_expires_at.as_deref(), Some("2020-01-01T00:00:00+08:00"));
    }

    #[test]
    fn expired_without_refresh_fails_with_relogin_command() {
        let r = build_report(&inputs(Some("/bin/kiro-cli"), None, login(LoginMethod::AwsSso, 2020, false)));
        assert_eq!(r.items[1].status, CheckStatus::Ok);
        assert_eq!(r.items[2].status, CheckStatus::Fail);
        assert_eq!(r.items[2].title, "登录已失效");
        assert_eq!(r.items[2].detail, "重新登录一次就好");
        assert!(matches!(&r.items[2].fix, Some(FixAction::CopyCommand { command, .. }) if command == "/bin/kiro-cli login"));
    }

    #[test]
    fn unexpired_login_without_refresh_shows_expiry_in_detail() {
        let r = build_report(&inputs(Some("/bin/kiro-cli"), None, login(LoginMethod::AwsSso, 2030, false)));
        assert_eq!(r.items[2].status, CheckStatus::Ok);
        assert_eq!(r.items[2].detail, "有效期至 2030-01-01 00:00");
    }

    #[test]
    fn unknown_expiry_without_refresh_is_invalid() {
        let probe = LoginProbe::Found(LoginInfo { method: LoginMethod::Social, expires_at: None, refreshable: false });
        let r = build_report(&inputs(Some("/bin/kiro-cli"), None, probe));
        assert_eq!(r.items[2].status, CheckStatus::Fail);
        assert_eq!(r.login_expires_at, None);
    }

    // ---------- claude ----------

    #[test]
    fn missing_claude_fails_independently_of_kiro_cli_items() {
        let r = build_report(&inputs(Some("/bin/kiro-cli"), None, login(LoginMethod::Social, 2030, true)));
        assert_eq!(
            statuses(&r),
            vec![CheckStatus::Ok, CheckStatus::Ok, CheckStatus::Ok, CheckStatus::Fail]
        );
        assert_eq!(r.items[3].title, "没找到 Claude Code");
        assert_eq!(r.items[3].detail, "网关照样能开，但 kclaude 要等装好 Claude Code 才能用");
        match &r.items[3].fix {
            Some(FixAction::OpenUrl { label, url }) => {
                assert_eq!(label, "查看安装方法");
                assert_eq!(url, CLAUDE_CODE_INSTALL_URL);
            }
            other => panic!("expected open_url, got {other:?}"),
        }
        assert_eq!(r.claude_path, None);
    }

    #[test]
    fn missing_claude_and_missing_kiro_cli_report_both() {
        let r = build_report(&inputs(None, None, LoginProbe::NoDatabase));
        assert_eq!(
            statuses(&r),
            vec![CheckStatus::Fail, CheckStatus::Skipped, CheckStatus::Skipped, CheckStatus::Fail]
        );
    }

    // ---------- fix actions only on fail ----------

    #[test]
    fn fix_is_present_only_when_status_is_fail() {
        let probes = [
            LoginProbe::NoDatabase,
            LoginProbe::Corrupt,
            login(LoginMethod::Social, 2020, false),
            login(LoginMethod::Social, 2030, true),
        ];
        for kiro in [None, Some("/bin/kiro-cli")] {
            for claude in [None, Some("/bin/claude")] {
                for probe in &probes {
                    let r = build_report(&inputs(kiro, claude, probe.clone()));
                    for item in &r.items {
                        assert_eq!(item.fix.is_some(), item.status == CheckStatus::Fail, "{item:?}");
                    }
                }
            }
        }
    }

    // ---------- gateway gate ----------

    #[test]
    fn gate_allows_when_all_kiro_cli_items_pass() {
        assert_eq!(gateway_gate(&all_good()), GatewayGate::Allowed);
    }

    #[test]
    fn gate_blocks_with_reason_for_each_kiro_cli_failure() {
        let missing = build_report(&inputs(None, Some("/bin/claude"), LoginProbe::NoDatabase));
        let not_logged_in = build_report(&inputs(Some("/bin/kiro-cli"), None, LoginProbe::NoCredentials));
        let expired = build_report(&inputs(Some("/bin/kiro-cli"), None, login(LoginMethod::Social, 2020, false)));
        assert_eq!(gateway_gate(&missing), GatewayGate::Blocked { reason: "先安装 kiro-cli 才能打开".into() });
        assert_eq!(gateway_gate(&not_logged_in), GatewayGate::Blocked { reason: "先登录 kiro-cli 才能打开".into() });
        assert_eq!(gateway_gate(&expired), GatewayGate::Blocked { reason: "kiro-cli 的登录已失效，重新登录后才能打开".into() });
    }

    #[test]
    fn gate_blocks_on_skipped_kiro_cli_item_too() {
        // A hand-built report with a skipped item must not slip through.
        let mut r = all_good();
        r.items[2].status = CheckStatus::Skipped;
        assert!(matches!(gateway_gate(&r), GatewayGate::Blocked { .. }));
    }

    #[test]
    fn gate_ignores_claude_status() {
        let r = build_report(&inputs(Some("/bin/kiro-cli"), None, login(LoginMethod::Social, 2030, true)));
        assert_eq!(r.items[3].status, CheckStatus::Fail);
        assert_eq!(gateway_gate(&r), GatewayGate::Allowed);
    }

    #[test]
    fn gate_blocks_on_malformed_report() {
        let mut r = all_good();
        r.items.truncate(2);
        assert!(matches!(gateway_gate(&r), GatewayGate::Blocked { .. }));
        r.items.clear();
        assert!(matches!(gateway_gate(&r), GatewayGate::Blocked { .. }));
    }

    #[test]
    fn kclaude_note_only_when_claude_missing() {
        assert_eq!(kclaude_unavailable_reason(&all_good()), None);
        let r = build_report(&inputs(Some("/bin/kiro-cli"), None, login(LoginMethod::Social, 2030, true)));
        assert_eq!(kclaude_unavailable_reason(&r).as_deref(), Some("需先安装 Claude Code"));
        let mut malformed = all_good();
        malformed.items.clear();
        assert_eq!(kclaude_unavailable_reason(&malformed).as_deref(), Some("需先安装 Claude Code"));
    }

    // ---------- login command quoting ----------

    #[test]
    fn login_command_plain_path_is_unquoted() {
        assert_eq!(login_command(Path::new("/Users/me/.local/bin/kiro-cli"), false), "/Users/me/.local/bin/kiro-cli login");
    }

    #[test]
    fn login_command_quotes_path_with_spaces() {
        assert_eq!(
            login_command(Path::new("/Users/me/My Tools/kiro-cli"), false),
            "'/Users/me/My Tools/kiro-cli' login"
        );
    }

    #[test]
    fn login_command_unix_quoting_escapes_single_quote_and_neutralizes_shell_chars() {
        assert_eq!(login_command(Path::new("/Users/o'neil/bin/kiro-cli"), false), "'/Users/o'\\''neil/bin/kiro-cli' login");
        assert_eq!(login_command(Path::new("/Users/me/$HOME/kiro-cli"), false), "'/Users/me/$HOME/kiro-cli' login");
        assert_eq!(login_command(Path::new("/a;b/kiro-cli"), false), "'/a;b/kiro-cli' login");
    }

    #[test]
    fn login_command_windows_uses_double_quotes_when_needed() {
        assert_eq!(
            login_command(Path::new(r"C:\Users\me\AppData\Local\Programs\kiro-cli\kiro-cli.exe"), true),
            r"C:\Users\me\AppData\Local\Programs\kiro-cli\kiro-cli.exe login"
        );
        assert_eq!(
            login_command(Path::new(r"C:\Program Files\Kiro CLI\kiro-cli.exe"), true),
            r#""C:\Program Files\Kiro CLI\kiro-cli.exe" login"#
        );
    }

    #[test]
    fn login_command_in_report_uses_absolute_path_not_tilde() {
        let r = build_report(&inputs(Some("/Users/me/My Tools/kiro-cli"), None, LoginProbe::NoDatabase));
        let cmd = r.login_command.unwrap();
        assert!(!cmd.contains('~'));
        assert_eq!(cmd, "'/Users/me/My Tools/kiro-cli' login");
        assert_eq!(r.kiro_cli_path.as_deref(), Some("~/My Tools/kiro-cli"));
    }

    // ---------- display path ----------

    #[test]
    fn display_path_abbreviates_home_only_at_a_component_boundary() {
        let home = PathBuf::from("/Users/me");
        assert_eq!(display_path(Path::new("/Users/me/.local/bin/claude"), Some(&home)), "~/.local/bin/claude");
        assert_eq!(display_path(Path::new("/Users/me"), Some(&home)), "~");
        assert_eq!(display_path(Path::new("/Users/meow/bin/claude"), Some(&home)), "/Users/meow/bin/claude");
        assert_eq!(display_path(Path::new("/opt/homebrew/bin/claude"), Some(&home)), "/opt/homebrew/bin/claude");
        assert_eq!(display_path(Path::new("/opt/homebrew/bin/claude"), None), "/opt/homebrew/bin/claude");
    }

    // ---------- no credential material anywhere ----------

    #[test]
    fn report_json_has_no_token_fields_or_values() {
        let r = all_good();
        let text = serde_json::to_string(&r).unwrap().to_lowercase();
        for forbidden in ["access_token", "refresh_token", "client_secret", "accesstoken", "refreshtoken", "secret"] {
            assert!(!text.contains(forbidden), "report mentions {forbidden}");
        }
    }
}
