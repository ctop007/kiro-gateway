//! `gateway_start` / `gateway_stop` / `gateway_models` behaviour around the manager.

use super::*;
use crate::app_config::{AppPaths, ConfigPatch, ConfigStore};
use crate::gateway::manager::GatewayManager;
use crate::gateway::sidecar::exe_name;
use crate::gateway::testing::*;
use crate::gateway::types::{FailureKind, GatewayState};
use chrono::NaiveDate;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::{tempdir, TempDir};

struct FakeGate {
    answer: Mutex<Result<PathBuf, String>>,
    calls: AtomicUsize,
}

impl FakeGate {
    fn ok(db: &Path) -> Arc<Self> {
        Arc::new(Self { answer: Mutex::new(Ok(db.to_path_buf())), calls: AtomicUsize::new(0) })
    }
    fn blocked(reason: &str) -> Arc<Self> {
        Arc::new(Self { answer: Mutex::new(Err(reason.to_string())), calls: AtomicUsize::new(0) })
    }
}

impl EnvGate for FakeGate {
    fn check(&self) -> Result<PathBuf, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.lock().unwrap().clone()
    }
}

struct Fixture {
    _dir: TempDir,
    data: PathBuf,
    resources: PathBuf,
    db: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let data = dir.path().join("data");
    let resources = dir.path().join("resources");
    let db = dir.path().join("kiro-cli").join("data.sqlite3");
    fs::create_dir_all(&data).unwrap();
    Fixture { _dir: dir, data, resources, db }
}

fn install_sidecar(f: &Fixture, mode: &str) -> PathBuf {
    let sidecar = write_fake_sidecar(&f.resources.join("gateway"), mode);
    assert_eq!(sidecar.exe.file_name().unwrap(), exe_name(false));
    sidecar.exe
}

fn service(f: &Fixture, gate: Arc<dyn EnvGate>, port: u16) -> GatewayService {
    let now = NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_opt(14, 41, 8).unwrap();
    let (config, _) = ConfigStore::open(AppPaths::new(&f.data), now).unwrap();
    config.update(&ConfigPatch { port: Some(i64::from(port)), ..Default::default() }, false).unwrap();
    let config = Arc::new(config);
    let manager = GatewayManager::new(AppPaths::new(&f.data), system_deps(), fast_settings(), port);
    GatewayService::new(
        config,
        manager,
        gate,
        f.resources.clone(),
        None,
        Box::new(|| {
            vec![
                (OsString::from("PATH"), std::env::var_os("PATH").unwrap_or_default()),
                (OsString::from("ANTHROPIC_BASE_URL"), OsString::from("https://leaked.invalid")),
            ]
        }),
    )
}

fn wait_state(s: &GatewayService, state: GatewayState) -> crate::gateway::types::GatewayStatus {
    assert!(wait_until(Duration::from_secs(15), || s.status().state == state), "wanted {state:?}, got {:?}", s.status());
    s.status()
}

#[test]
fn start_with_everything_in_place_runs_and_remembers_the_switch() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let port = free_port();
    let s = service(&f, FakeGate::ok(&f.db), port);
    assert!(!s.config().get().gateway_wanted);

    let first = s.start().unwrap();
    assert_eq!(first.state, GatewayState::Starting);
    assert!(s.config().get().gateway_wanted, "gatewayWanted is set once the start was accepted");
    wait_state(&s, GatewayState::Running);
    s.shutdown();
}

#[test]
fn the_sidecar_receives_the_stored_secret_the_db_from_the_gate_and_the_configured_port() {
    let f = fixture();
    let dump = f.data.join("dump.json");
    install_sidecar(&f, "serve");
    let port = free_port();
    let s = service(&f, FakeGate::ok(&f.db), port);
    // The dump path travels through the inherited environment of the service.
    let s = s.with_inherited_env(Box::new({
        let dump = dump.clone();
        move || {
            vec![
                (OsString::from("PATH"), std::env::var_os("PATH").unwrap_or_default()),
                (OsString::from("KGW_FAKE_DUMP"), dump.clone().into_os_string()),
                (OsString::from("ANTHROPIC_BASE_URL"), OsString::from("https://leaked.invalid")),
            ]
        }
    }));
    s.start().unwrap();
    wait_state(&s, GatewayState::Running);

    let seen = read_dump(&dump);
    let env = seen["env"].as_object().unwrap();
    assert_eq!(env["PROXY_API_KEY"].as_str().unwrap(), s.config().secret());
    assert_eq!(env["KIRO_CLI_DB_FILE"].as_str().unwrap(), f.db.to_str().unwrap());
    assert_eq!(env["SERVER_PORT"].as_str().unwrap(), port.to_string());
    assert_eq!(env["SERVER_HOST"], "127.0.0.1");
    assert!(!env.contains_key("ANTHROPIC_BASE_URL"));
    assert_eq!(
        PathBuf::from(env["REQUEST_LOG_DIR"].as_str().unwrap()),
        AppPaths::new(&f.data).request_log_dir()
    );
    s.shutdown();
}

#[test]
fn the_forced_upgrade_flag_rejects_start_before_anything_else() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let gate = FakeGate::ok(&f.db);
    let s = service(&f, gate.clone(), free_port());
    s.config().set_upgrade_required(true).unwrap();
    let err = s.start().unwrap_err();
    assert_eq!(err.code(), "upgrade_required");
    assert_eq!(err.message(), "需要先升级到新版本才能使用");
    assert_eq!(gate.calls.load(Ordering::SeqCst), 0, "no environment check once the upgrade is forced");
    assert_eq!(s.status().state, GatewayState::Stopped);
    assert!(!s.config().get().gateway_wanted);
}

#[test]
fn a_blocked_environment_is_env_not_ready_with_the_reason_and_changes_nothing() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let s = service(&f, FakeGate::blocked("先登录 kiro-cli 才能打开"), free_port());
    let err = s.start().unwrap_err();
    assert_eq!(err.code(), "env_not_ready");
    assert_eq!(err.message(), "先登录 kiro-cli 才能打开");
    assert_eq!(s.status().state, GatewayState::Stopped);
    assert!(!s.config().get().gateway_wanted);
}

#[test]
fn the_environment_is_checked_again_on_every_start() {
    let f = fixture();
    install_sidecar(&f, "exit:1");
    let gate = FakeGate::ok(&f.db);
    let s = service(&f, gate.clone(), free_port());
    s.start().unwrap();
    wait_state(&s, GatewayState::StartFailed);
    s.start().unwrap();
    assert_eq!(gate.calls.load(Ordering::SeqCst), 2);
    s.shutdown();
}

#[test]
fn start_on_an_active_gateway_does_not_check_the_environment_or_launch_again() {
    let f = fixture();
    let launches = f.data.join("launches.txt");
    install_sidecar(&f, "serve");
    let port = free_port();
    let s = service(&f, FakeGate::ok(&f.db), port).with_inherited_env(Box::new({
        let launches = launches.clone();
        move || {
            vec![
                (OsString::from("PATH"), std::env::var_os("PATH").unwrap_or_default()),
                (OsString::from("KGW_FAKE_LAUNCHES"), launches.clone().into_os_string()),
            ]
        }
    }));
    s.start().unwrap();
    wait_state(&s, GatewayState::Running);
    let again = s.start().unwrap();
    assert_eq!(again.state, GatewayState::Running);
    assert_eq!(launch_count(&launches), 1);
    s.shutdown();
}

#[test]
fn a_missing_sidecar_is_a_start_failure_state_with_a_reinstall_hint() {
    let f = fixture();
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    let status = s.start().unwrap();
    assert_eq!(status.state, GatewayState::StartFailed);
    let failure = status.failure.clone().unwrap();
    assert_eq!(failure.kind, FailureKind::Other);
    assert!(failure.message.contains("重新安装"), "{}", failure.message);
    assert!(!failure.message.contains('/'));
    assert_eq!(s.status(), status);
}

#[test]
fn the_sidecar_override_variable_points_the_service_elsewhere() {
    let f = fixture();
    let elsewhere = f._dir.path().join("elsewhere");
    let custom = write_fake_sidecar(&elsewhere, "serve").exe;
    let port = free_port();
    let s = service(&f, FakeGate::ok(&f.db), port).with_sidecar_override(Some(custom.clone().into_os_string()));
    s.start().unwrap();
    wait_state(&s, GatewayState::Running);
    s.shutdown();

    let s2 = service(&f, FakeGate::ok(&f.db), free_port()).with_sidecar_override(Some(OsString::from("/no/such/file")));
    let status = s2.start().unwrap();
    assert_eq!(status.state, GatewayState::StartFailed);
    assert!(status.failure.unwrap().message.contains("KIRO_GATEWAY_SIDECAR"));
}

#[test]
fn a_taken_port_is_a_state_with_the_occupant_and_the_switch_is_still_remembered() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let port = free_port();
    let holder = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let s = service(&f, FakeGate::ok(&f.db), port);
    let status = s.start().unwrap();
    assert_eq!(status.state, GatewayState::StartFailed);
    assert_eq!(status.failure.as_ref().unwrap().kind, FailureKind::PortInUse);
    assert!(s.config().get().gateway_wanted, "the user did ask for the gateway; a later resume retries");
    drop(holder);
}

#[test]
fn stop_clears_the_switch_and_returns_stopped() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    s.start().unwrap();
    wait_state(&s, GatewayState::Running);
    let status = s.stop().unwrap();
    assert_eq!(status.state, GatewayState::Stopped);
    assert!(!s.config().get().gateway_wanted);
}

#[test]
fn stop_works_after_a_failure_and_clears_it() {
    let f = fixture();
    install_sidecar(&f, "exit:1");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    s.start().unwrap();
    wait_state(&s, GatewayState::StartFailed);
    let status = s.stop().unwrap();
    assert_eq!(status.state, GatewayState::Stopped);
    assert_eq!(status.failure, None);
}

#[test]
fn a_crash_keeps_the_switch_remembered() {
    let f = fixture();
    install_sidecar(&f, "crash_after:0.3");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    s.start().unwrap();
    wait_state(&s, GatewayState::Crashed);
    assert!(s.config().get().gateway_wanted, "an unexpected exit must not change gatewayWanted");
}

#[test]
fn shutdown_stops_the_gateway_but_keeps_the_switch_for_the_next_launch() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    s.start().unwrap();
    wait_state(&s, GatewayState::Running);
    s.shutdown();
    assert_eq!(s.status().state, GatewayState::Stopped);
    assert!(s.config().get().gateway_wanted, "quitting the App is not the user turning the switch off");
}

#[test]
fn resume_starts_only_when_the_switch_was_on() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    s.resume_if_wanted();
    assert_eq!(s.status().state, GatewayState::Stopped);

    s.config().set_gateway_wanted(true).unwrap();
    s.resume_if_wanted();
    wait_state(&s, GatewayState::Running);
    s.shutdown();
}

#[test]
fn resume_does_nothing_under_a_forced_upgrade_or_a_blocked_environment() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    s.config().set_gateway_wanted(true).unwrap();
    s.config().set_upgrade_required(true).unwrap();
    s.resume_if_wanted();
    assert_eq!(s.status().state, GatewayState::Stopped);

    let s2 = service(&f, FakeGate::blocked("先登录 kiro-cli 才能打开"), free_port());
    s2.config().set_upgrade_required(false).unwrap();
    s2.config().set_gateway_wanted(true).unwrap();
    s2.resume_if_wanted();
    assert_eq!(s2.status().state, GatewayState::Stopped);
    assert!(s2.config().get().gateway_wanted, "still wanted, retried next launch");
}

#[test]
fn models_need_a_running_gateway_and_use_the_stored_key() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    assert_eq!(s.models().unwrap_err().code(), "gateway_not_running");
    s.start().unwrap();
    wait_state(&s, GatewayState::Running);
    let ids: Vec<String> = s.models().unwrap().into_iter().map(|m| m.id).collect();
    assert_eq!(ids, vec!["auto", "claude-haiku-4.5"]);
    s.shutdown();
}

#[test]
fn model_json_has_only_the_id_field() {
    let value = serde_json::to_value(GatewayModel { id: "auto".into() }).unwrap();
    assert_eq!(value, serde_json::json!({"id": "auto"}));
}

#[test]
fn port_changes_reach_the_status_while_stopped() {
    let f = fixture();
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    let other = free_port();
    s.apply_port(other);
    assert_eq!(s.status().port, other);
}

#[test]
fn is_running_flag_covers_starting_and_running_only() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    assert!(!s.is_active());
    s.start().unwrap();
    assert!(s.is_active());
    wait_state(&s, GatewayState::Running);
    assert!(s.is_active());
    s.stop().unwrap();
    assert!(!s.is_active());
}

#[test]
fn service_errors_have_the_contract_codes() {
    assert_eq!(ServiceError::UpgradeRequired.code(), "upgrade_required");
    assert_eq!(ServiceError::EnvNotReady("x".into()).code(), "env_not_ready");
    assert_eq!(ServiceError::Internal("x".into()).code(), "internal");
    assert_eq!(ServiceError::EnvNotReady("先安装 kiro-cli 才能打开".into()).message(), "先安装 kiro-cli 才能打开");
}

// ------------------------------------------------- injectable start guard (A-process 3)

fn forced_guard(forced: Arc<std::sync::atomic::AtomicBool>) -> StartGuard {
    Arc::new(move || forced.load(Ordering::SeqCst))
}

#[test]
fn resume_if_wanted_does_not_start_while_the_guard_says_forced() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let gate = FakeGate::ok(&f.db);
    let s = service(&f, gate.clone(), free_port());
    let forced = Arc::new(std::sync::atomic::AtomicBool::new(true));
    s.set_start_guard(forced_guard(forced.clone()));
    // The persisted flag is NOT set: only the in-memory state of the update service says forced.
    assert!(!s.config().upgrade_required());
    s.config().set_gateway_wanted(true).unwrap();

    s.resume_if_wanted();
    assert_eq!(s.status().state, GatewayState::Stopped);
    assert_eq!(gate.calls.load(Ordering::SeqCst), 0, "no environment check either");
    assert!(s.config().get().gateway_wanted, "still wanted for after the upgrade");

    forced.store(false, Ordering::SeqCst);
    s.resume_if_wanted();
    wait_state(&s, GatewayState::Running);
    s.shutdown();
}

#[test]
fn start_rejects_with_upgrade_required_when_the_guard_says_forced() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    s.set_start_guard(forced_guard(Arc::new(std::sync::atomic::AtomicBool::new(true))));
    let err = s.start().unwrap_err();
    assert_eq!(err.code(), "upgrade_required");
    assert_eq!(s.status().state, GatewayState::Stopped);
    assert!(!s.config().get().gateway_wanted);
}

#[test]
fn a_guard_that_says_not_forced_changes_nothing() {
    let f = fixture();
    install_sidecar(&f, "serve");
    let s = service(&f, FakeGate::ok(&f.db), free_port());
    s.set_start_guard(forced_guard(Arc::new(std::sync::atomic::AtomicBool::new(false))));
    s.start().unwrap();
    wait_state(&s, GatewayState::Running);
    s.shutdown();
}
