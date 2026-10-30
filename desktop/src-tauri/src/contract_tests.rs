//! Rust ⇄ frontend contract guard.
//!
//! Serialises the structures the commands and events return and compares their JSON field
//! names, one by one, with the interfaces in `src/lib/ipc-types.ts` (the file is read and
//! parsed here). A field renamed on one side only fails this test. It also checks that
//! every command and event name of the TypeScript registry exists on the Rust side and
//! that every error code Rust can produce is known to the frontend.

use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use crate::app_config::commands::ConfigUpdateResult;
use crate::app_config::model::AppConfig;
use crate::env_check::login::{LoginInfo, LoginMethod, LoginProbe};
use crate::env_check::report::{build_report, ReportInputs};
use crate::gateway::service::GatewayModel;
use crate::gateway::types::{FailureKind, GatewayFailure, GatewayState, GatewayStatus, PortOccupant};
use crate::kclaude::types::{
    ErrorCode as KclaudeErrorCode, FixAction as KclaudeFix, KclaudeState, KclaudeStatus, KclaudeVerify, ShellKind,
    VerifyStatus,
};
use crate::request_log::model::{
    ApiKind, ModelUsage, MonthUsage, Outcome, RequestLogAppended, RequestLogSnapshot, RequestRecord, UsageSummary,
    UsageTotal,
};
use crate::update::types::{DownloadProgress, UpdateErrorCode, UpdateState, UpdateStatus};

fn ts_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/lib/ipc-types.ts");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Top-level body of `export interface <name> ... { ... }` (braces balanced).
fn block_after(source: &str, header: &str) -> String {
    let start = source.find(header).unwrap_or_else(|| panic!("`{header}` not found in ipc-types.ts"));
    let open = start + source[start..].find('{').expect("opening brace");
    let mut depth = 0usize;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return source[open + 1..open + offset].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces after `{header}`");
}

/// Field names declared directly in a TS object body (nested `{ ... }` are skipped).
fn top_level_keys(body: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let mut depth = 0i32;
    for line in body.lines() {
        let trimmed = line.trim();
        let at_top = depth == 0;
        if at_top && !trimmed.starts_with("//") && !trimmed.starts_with('*') && !trimmed.starts_with("/*") {
            // `a: x; b: y` on one line is allowed (inline variants), so split on ';'.
            for part in trimmed.split(';') {
                if let Some((name, _)) = part.trim().split_once(':') {
                    let name = name.trim().trim_end_matches('?');
                    if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        keys.insert(name.to_string());
                    }
                }
            }
        }
        depth += trimmed.matches('{').count() as i32 - trimmed.matches('}').count() as i32;
    }
    keys
}

/// Keys of `export interface <name>` including those of the interfaces it extends.
fn interface_keys(source: &str, name: &str) -> BTreeSet<String> {
    let header_start = source
        .find(&format!("export interface {name} "))
        .unwrap_or_else(|| panic!("interface {name} not found"));
    let header_line = source[header_start..].lines().next().unwrap();
    let mut keys = top_level_keys(&block_after(source, &format!("export interface {name} ")));
    if let Some((_, parents)) = header_line.split_once(" extends ") {
        for parent in parents.trim_end_matches('{').split(',') {
            keys.extend(interface_keys(source, parent.trim()));
        }
    }
    keys
}

/// Keys of the union member whose discriminator `tag: 'value'` matches.
fn variant_keys(source: &str, type_name: &str, tag: &str, value: &str) -> BTreeSet<String> {
    let start = source.find(&format!("export type {type_name} =")).unwrap_or_else(|| panic!("{type_name} missing"));
    let rest = &source[start..];
    let end = rest.find("\n\n").unwrap_or(rest.len());
    let body = &rest[..end];
    let marker = format!("{tag}: '{value}'");
    let at = body.find(&marker).unwrap_or_else(|| panic!("{type_name} has no variant {tag}={value}"));
    let open = body[..at].rfind('{').unwrap();
    let mut depth = 0usize;
    for (offset, ch) in body[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return top_level_keys(&body[open + 1..open + offset]);
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced variant");
}

fn json_keys(value: &Value) -> BTreeSet<String> {
    value.as_object().unwrap_or_else(|| panic!("not an object: {value}")).keys().cloned().collect()
}

#[track_caller]
fn same_fields(what: &str, rust: &Value, ts: BTreeSet<String>) {
    assert_eq!(json_keys(rust), ts, "{what}: Rust JSON keys differ from ipc-types.ts");
}

#[track_caller]
fn matches_interface(source: &str, name: &str, rust: &Value) {
    same_fields(name, rust, interface_keys(source, name));
}

fn to_json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap()
}

fn at() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-10-22T14:41:08+08:00").unwrap()
}

#[test]
fn config_types_match() {
    let ts = ts_source();
    let config = to_json(&AppConfig::default());
    matches_interface(&ts, "AppConfig", &config);
    assert_eq!(
        json_keys(&config["models"]),
        ["opus", "sonnet", "haiku"].map(String::from).into_iter().collect()
    );
    let result = to_json(&ConfigUpdateResult { config: AppConfig::default(), restart_required: true });
    matches_interface(&ts, "ConfigUpdateResult", &result);
    // The patch the frontend sends deserialises, and stray keys do not.
    let patch: crate::app_config::ConfigPatch =
        serde_json::from_str(r#"{"port":8100,"commandName":"kclaude-test","models":{"opus":"x"},"autostart":true}"#)
            .unwrap();
    assert_eq!(patch.port, Some(8100));
    assert_eq!(patch.command_name.as_deref(), Some("kclaude-test"));
    // Every key of the TS patch is a key the Rust patch understands, and vice versa.
    assert_eq!(
        interface_keys(&ts, "ConfigPatch"),
        ["autostart", "commandName", "models", "port"].map(String::from).into_iter().collect()
    );
    let full = to_json(&crate::app_config::ConfigPatch {
        port: Some(1),
        models: Some(Default::default()),
        autostart: Some(true),
        command_name: Some("cclaude".into()),
    });
    assert_eq!(json_keys(&full), interface_keys(&ts, "ConfigPatch"), "ConfigPatch: Rust keys differ from ipc-types.ts");
    // The documented defaults.
    assert_eq!(config["port"], 6868);
    assert_eq!(config["commandName"], "cclaude");
}

#[test]
fn gateway_types_match() {
    let ts = ts_source();
    let failure = GatewayFailure {
        kind: FailureKind::PortInUse,
        message: "6868 端口被别的程序占用".into(),
        occupant: Some(PortOccupant { name: Some("python3.13".into()), pid: Some(4242) }),
        log_excerpt: vec!["line".into()],
        at: at().to_rfc3339(),
    };
    let status = GatewayStatus {
        state: GatewayState::StartFailed,
        port: 6868,
        pid: Some(1),
        since: Some(at().to_rfc3339()),
        failure: Some(failure),
    };
    let value = to_json(&status);
    matches_interface(&ts, "GatewayStatus", &value);
    matches_interface(&ts, "GatewayFailure", &value["failure"]);
    matches_interface(&ts, "PortOccupant", &value["failure"]["occupant"]);
    // Optional fields are `null`, never omitted.
    let stopped = to_json(&GatewayStatus::stopped(6868));
    matches_interface(&ts, "GatewayStatus", &stopped);
    assert!(stopped["failure"].is_null() && stopped["pid"].is_null() && stopped["since"].is_null());
    matches_interface(&ts, "GatewayModel", &to_json(&GatewayModel { id: "x".into() }));
}

#[test]
fn gateway_enums_use_the_contract_spellings() {
    let ts = ts_source();
    for (state, text) in [
        (GatewayState::Stopped, "stopped"),
        (GatewayState::Starting, "starting"),
        (GatewayState::Running, "running"),
        (GatewayState::StartFailed, "start_failed"),
        (GatewayState::Crashed, "crashed"),
    ] {
        assert_eq!(to_json(&state), text);
        assert!(ts.contains(&format!("'{text}'")), "{text} missing from ipc-types.ts");
    }
    for (kind, text) in [
        (FailureKind::PortInUse, "port_in_use"),
        (FailureKind::LoginExpired, "login_expired"),
        (FailureKind::Timeout, "timeout"),
        (FailureKind::Other, "other"),
    ] {
        assert_eq!(to_json(&kind), text);
        assert!(ts.contains(&format!("'{text}'")), "{text} missing from ipc-types.ts");
    }
}

#[test]
fn env_report_matches() {
    let ts = ts_source();
    let login = LoginProbe::Found(LoginInfo { method: LoginMethod::AwsSso, expires_at: Some(at()), refreshable: true });
    let report = build_report(&ReportInputs {
        kiro_cli: Some(PathBuf::from("/Users/x/.local/bin/kiro-cli")),
        claude: None,
        login,
        home: Some(PathBuf::from("/Users/x")),
        now: at(),
    });
    let value = to_json(&report);
    matches_interface(&ts, "EnvReport", &value);
    let items = value["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    for item in items {
        matches_interface(&ts, "CheckItem", item);
    }
    // A failing item carries a fix whose variant matches the TS union.
    let failing = items.iter().find(|i| !i["fix"].is_null()).expect("claude missing gives a fix");
    let kind = failing["fix"]["kind"].as_str().unwrap();
    same_fields("FixAction", &failing["fix"], variant_keys(&ts, "FixAction", "kind", kind));
    // No kiro-cli at all: the fix is an open_url.
    let missing = to_json(&build_report(&ReportInputs {
        kiro_cli: None,
        claude: None,
        login: LoginProbe::NoDatabase,
        home: None,
        now: at(),
    }));
    let fix = missing["items"][0]["fix"].clone();
    assert_eq!(fix["kind"], "open_url");
    same_fields("FixAction(open_url)", &fix, variant_keys(&ts, "FixAction", "kind", "open_url"));
    assert!(missing["loginCommand"].is_null() && missing["kiroCliPath"].is_null());
}

#[test]
fn kclaude_status_matches() {
    let ts = ts_source();
    let status = KclaudeStatus {
        command_name: "cclaude".into(),
        state: KclaudeState::Foreign,
        shell: ShellKind::Zsh,
        rc_files: vec!["~/.zshrc".into()],
        claude_installed: true,
        verify: KclaudeVerify {
            status: VerifyStatus::Fail,
            detail: "新开的终端里找不到 cclaude".into(),
            fix: Some(KclaudeFix::CopyCommand { label: "复制".into(), command: "cmd".into() }),
        },
    };
    let value = to_json(&status);
    matches_interface(&ts, "KclaudeStatus", &value);
    assert_eq!(value["commandName"], "cclaude");
    matches_interface(&ts, "KclaudeVerify", &value["verify"]);
    same_fields("FixAction(copy_command)", &value["verify"]["fix"], variant_keys(&ts, "FixAction", "kind", "copy_command"));
    let skipped = to_json(&KclaudeVerify::skipped("cclaude"));
    matches_interface(&ts, "KclaudeVerify", &skipped);
    assert!(skipped["fix"].is_null());
}

fn record() -> RequestRecord {
    RequestRecord {
        ts: at().to_rfc3339(),
        id: None,
        api: ApiKind::Anthropic,
        model: None,
        stream: true,
        status: 200,
        outcome: Outcome::Ok,
        duration_ms: 12,
        credits: 0.5,
        error: None,
    }
}

fn summary() -> UsageSummary {
    UsageSummary {
        today: UsageTotal { credits: 1.0, requests: 2 },
        month: MonthUsage { credits: 3.0, requests: 4, month: "2026-10".into() },
        by_model: vec![ModelUsage { model: "m".into(), credits: 3.0, requests: 4 }],
    }
}

#[test]
fn request_log_types_match() {
    let ts = ts_source();
    let snapshot = to_json(&RequestLogSnapshot { summary: summary(), records: vec![record()], generated_at: at().to_rfc3339() });
    matches_interface(&ts, "RequestLogSnapshot", &snapshot);
    matches_interface(&ts, "RequestRecord", &snapshot["records"][0]);
    matches_interface(&ts, "ModelUsage", &snapshot["byModel"][0]);
    matches_interface(&ts, "UsageTotal", &snapshot["today"]);
    // `month` is `UsageTotal & { month: string }`.
    same_fields("month", &snapshot["month"], ["credits", "month", "requests"].map(String::from).into_iter().collect());
    assert!(ts.contains("month: UsageTotal & { month: string }"));
    let appended = to_json(&RequestLogAppended { summary: summary(), records: vec![record()], reset: false });
    matches_interface(&ts, "RequestLogAppended", &appended);
    assert!(snapshot["records"][0]["id"].is_null() && snapshot["records"][0]["model"].is_null());
}

#[test]
fn update_status_matches_every_variant() {
    let ts = ts_source();
    let progress = DownloadProgress { downloaded_bytes: 1, total_bytes: None };
    let states = [
        UpdateState::None,
        UpdateState::Downloading { version: "0.0.2".into(), progress: progress.clone() },
        UpdateState::Ready { version: "0.0.2".into() },
        UpdateState::Installing { version: "0.0.2".into() },
        UpdateState::Forced {
            version: "0.0.3".into(),
            progress: Some(progress.clone()),
            error: None,
            manual_url: "https://x/y.dmg".into(),
        },
        UpdateState::InstallFailed { version: "0.0.2".into(), error: "e".into(), manual_url: "https://x/y.dmg".into() },
    ];
    let bookkeeping: BTreeSet<String> = interface_keys_of_status(&ts);
    for state in states {
        let value = to_json(&UpdateStatus { state, current_version: "0.0.1".into(), last_checked_at: None });
        let tag = value["state"].as_str().unwrap().to_string();
        let mut expected = variant_keys(&ts, "UpdateState", "state", &tag);
        expected.extend(bookkeeping.clone());
        same_fields(&format!("UpdateStatus({tag})"), &value, expected);
        if let Some(p) = value.get("progress").filter(|p| !p.is_null()) {
            matches_interface(&ts, "DownloadProgress", p);
        }
        if tag == "downloading" {
            matches_interface(&ts, "DownloadProgress", &value["progress"]);
        }
    }
}

/// `type UpdateStatus = UpdateState & { currentVersion: string; lastCheckedAt: string | null }`
fn interface_keys_of_status(source: &str) -> BTreeSet<String> {
    let start = source.find("export type UpdateStatus =").expect("UpdateStatus");
    let rest = &source[start..];
    let end = rest.find("\n}\n").map(|i| i + 2).unwrap_or(rest.len());
    top_level_keys(&block_after(&rest[..end], "UpdateState &"))
}

#[test]
fn every_command_of_the_registry_is_registered() {
    let ts = ts_source();
    let main = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")).unwrap();
    let registered: BTreeSet<String> = main
        .split("generate_handler![")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("generate_handler! list")
        .split(',')
        .filter_map(|entry| entry.trim().rsplit("::").next().map(str::to_string))
        .filter(|name| !name.is_empty())
        .collect();
    // One entry per line: `name: { args: ...; result: ... }`.
    let declared: BTreeSet<String> = block_after(&ts, "export interface CommandMap ")
        .lines()
        .filter_map(|l| l.trim().split_once(": {").map(|(name, _)| name.trim().to_string()))
        .collect();
    assert_eq!(declared.len(), 20);
    assert_eq!(registered, declared, "commands in main.rs vs CommandMap in ipc-types.ts");
}

#[test]
fn every_event_name_matches_the_registry() {
    let ts = ts_source();
    let block = block_after(&ts, "export interface EventMap ");
    let declared: BTreeSet<String> = block
        .lines()
        .filter_map(|l| l.trim().strip_prefix('\''))
        .filter_map(|l| l.split('\'').next())
        .map(str::to_string)
        .collect();
    let rust: BTreeSet<String> = [
        crate::gateway::commands::STATUS_EVENT,
        crate::request_log::commands::APPENDED_EVENT,
        crate::update::commands::UPDATE_STATUS_EVENT,
    ]
    .map(String::from)
    .into_iter()
    .collect();
    assert_eq!(rust, declared);
}

#[test]
fn rust_error_codes_are_known_to_the_frontend() {
    let ts = ts_source();
    let list = ts
        .split("export const IPC_ERROR_CODES")
        .nth(1)
        .and_then(|r| r.split("= [").nth(1))
        .and_then(|r| r.split(']').next())
        .expect("IPC_ERROR_CODES");
    let known: BTreeSet<String> = list.split('\'').skip(1).step_by(2).map(str::to_string).collect();
    assert_eq!(known.len(), 10);

    let mut produced: BTreeSet<String> = BTreeSet::new();
    for code in [
        KclaudeErrorCode::InvalidArgument,
        KclaudeErrorCode::ForeignPresent,
        KclaudeErrorCode::BackupFailed,
        KclaudeErrorCode::WriteFailed,
        KclaudeErrorCode::Internal,
    ] {
        produced.insert(to_json(&code).as_str().unwrap().to_string());
    }
    for code in [
        UpdateErrorCode::InvalidArgument,
        UpdateErrorCode::UpgradeRequired,
        UpdateErrorCode::NoPendingUpdate,
        UpdateErrorCode::Internal,
    ] {
        produced.insert(to_json(&code).as_str().unwrap().to_string());
    }
    use crate::app_config::model::ConfigError;
    use crate::gateway::health::ModelsError;
    use crate::gateway::service::ServiceError;
    for e in [ConfigError::InvalidArgument(String::new()), ConfigError::WriteFailed(String::new()), ConfigError::Internal(String::new())] {
        produced.insert(e.code().to_string());
    }
    for e in [ServiceError::UpgradeRequired, ServiceError::EnvNotReady(String::new()), ServiceError::Internal(String::new())] {
        produced.insert(e.code().to_string());
    }
    for e in [ModelsError::NotRunning, ModelsError::Unreachable, ModelsError::BadResponse] {
        produced.insert(e.code().to_string());
    }
    let unknown: Vec<_> = produced.difference(&known).collect();
    assert!(unknown.is_empty(), "Rust produces error codes the frontend does not know: {unknown:?}");
}

#[test]
fn the_ts_parser_itself_catches_a_missing_field() {
    // Guard for the guard: a made-up Rust shape must not pass for GatewayModel.
    let source = ts_source();
    let wrong = serde_json::json!({ "id": "x", "extra": 1 });
    let result = std::panic::catch_unwind(|| matches_interface(&source, "GatewayModel", &wrong));
    assert!(result.is_err());
}


#[test]
fn account_snapshot_matches_the_frontend_contract() {
    let value = serde_json::json!({
        "email": null, "provider": "AWS SSO", "subscription": null,
        "used": null, "total": null, "remaining": null, "usagePercent": null,
        "resetAt": null, "overageEnabled": false, "overageUsed": null,
        "tokenExpiresAt": null, "canRefreshToken": false,
        "checkedAt": "2026-10-30T00:00:00Z", "tokenRefreshedAt": null, "usageError": null
    });
    let snapshot: crate::account::AccountSnapshot = serde_json::from_value(value).unwrap();
    matches_interface(&ts_source(), "AccountSnapshot", &to_json(&snapshot));
}
