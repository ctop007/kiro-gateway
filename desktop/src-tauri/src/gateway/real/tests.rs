//! Runs the real PyInstaller sidecar (`resources/gateway/kiro-gateway`) through the manager.
//!
//! Skipped (with a message) when the sidecar has not been built on this machine
//! (`make sidecar`). Uses a throwaway HOME and a fake kiro-cli database whose token does not
//! expire, so the gateway never goes to the network, and a port in 8100..=8199.

use crate::app_config::AppPaths;
use crate::gateway::classify::classify;
use crate::gateway::env::{build_env, EnvInputs};
use crate::gateway::logfile::read_run_tail;
use crate::gateway::manager::StartRequest;
use crate::gateway::port::{PortOps, SystemPorts};
use crate::gateway::sidecar::{self, Sidecar};
use crate::gateway::testing::*;
use crate::gateway::types::{FailureKind, GatewayState};
use std::ffi::OsString;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::tempdir;

fn built_sidecar() -> Option<Sidecar> {
    let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    sidecar::resolve(&resources, None, std::env::consts::ARCH).ok()
}

fn skip(test: &str) {
    eprintln!("SKIPPED {test}: no built sidecar (run `make sidecar` in desktop/)");
}

/// A kiro-cli database like the sidecar smoke test uses: OIDC token valid for a year.
fn write_fake_db(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let script = r#"
import json, sqlite3, sys
from datetime import datetime, timedelta, timezone
exp = (datetime.now(timezone.utc) + timedelta(days=365)).strftime("%Y-%m-%dT%H:%M:%SZ")
c = sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE auth_kv (key TEXT PRIMARY KEY, value TEXT)")
c.execute("INSERT INTO auth_kv VALUES (?, ?)", ("kirocli:odic:token", json.dumps({"access_token": "fake-access", "refresh_token": "fake-refresh", "expires_at": exp, "region": "us-east-1"})))
c.execute("INSERT INTO auth_kv VALUES (?, ?)", ("kirocli:odic:device-registration", json.dumps({"client_id": "fake-id", "client_secret": "fake-secret", "region": "us-east-1"})))
c.commit()
"#;
    let status = Command::new("python3").args(["-I", "-c", script]).arg(path).status().unwrap();
    assert!(status.success());
}

fn real_env(sidecar: &Sidecar, db: &Path, home: &Path, port: u16, data: &Path) -> Vec<(OsString, OsString)> {
    let inputs = EnvInputs {
        secret: SECRET.to_string(),
        kiro_cli_db: db.to_path_buf(),
        port,
        request_log_dir: AppPaths::new(data).request_log_dir(),
        tiktoken_cache_dir: sidecar.tiktoken_cache.clone(),
    };
    let inherited: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("HOME".into(), home.as_os_str().to_os_string()),
        // Whatever the developer shell had must not matter:
        ("ANTHROPIC_BASE_URL".into(), "https://leaked.invalid".into()),
        ("PROXY_API_KEY".into(), "leaked-key".into()),
        ("HTTPS_PROXY".into(), "http://127.0.0.1:9".into()),
        ("NO_PROXY".into(), "127.0.0.1,localhost".into()),
    ];
    build_env(&inputs, inherited)
}

#[test]
fn the_real_sidecar_starts_serves_models_with_our_key_and_stops_cleanly() {
    let Some(sidecar) = built_sidecar() else { return skip("real start/stop") };
    let dir = tempdir().unwrap();
    let data = dir.path().join("data");
    let db = dir.path().join("kiro-cli").join("data.sqlite3");
    write_fake_db(&db);
    let port = free_port();
    let (m, rec) = manager_with_recorder_settings(&data, port, |s| s.start_timeout = Duration::from_secs(60));

    let env = real_env(&sidecar, &db, dir.path(), port, &data);
    let started = std::time::Instant::now();
    let pid = m
        .start(StartRequest { sidecar, env, port, secret: SECRET.to_string(), upgrade_required: false })
        .unwrap()
        .pid
        .unwrap();
    assert!(wait_until(Duration::from_secs(60), || m.status().state == GatewayState::Running), "{:?}", m.status());
    eprintln!("real sidecar ready in {:?}", started.elapsed());

    // /health and /v1/models with our key (the fake token never needs a refresh).
    let models = m.models(SECRET, Duration::from_secs(5)).unwrap();
    assert!(!models.is_empty(), "the gateway lists its static models");
    assert!(m.models("wrong-key-wrong-key-1234", Duration::from_secs(5)).is_err());

    // Only on loopback.
    let listing = Command::new("/usr/sbin/lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&listing.stdout).to_string();
    assert!(text.contains("127.0.0.1:") && !text.contains("*:") && !text.contains("0.0.0.0"), "{text}");

    // The log has a real banner and the secret never appears in it.
    let log = fs::read_to_string(AppPaths::new(&data).gateway_log()).unwrap();
    assert!(log.contains("Kiro Gateway"), "{}", &log[..log.len().min(400)]);
    assert!(!log.contains(SECRET));

    let status = m.stop();
    assert_eq!(status.state, GatewayState::Stopped);
    assert!(wait_until(Duration::from_secs(5), || {
        Command::new("/bin/ps").args(["-p", &pid.to_string()]).output().map(|o| !String::from_utf8_lossy(&o.stdout).contains(&pid.to_string())).unwrap_or(true)
    }));
    assert!(wait_until(Duration::from_secs(5), || SystemPorts.is_free(port)), "the port must be released after stop");
    assert_eq!(rec.states(), vec![GatewayState::Starting, GatewayState::Running, GatewayState::Stopped]);
}

#[test]
fn the_real_sidecar_killed_from_outside_is_reported_crashed() {
    let Some(sidecar) = built_sidecar() else { return skip("real kill") };
    let dir = tempdir().unwrap();
    let data = dir.path().join("data");
    let db = dir.path().join("kiro-cli").join("data.sqlite3");
    write_fake_db(&db);
    let port = free_port();
    let (m, _rec) = manager_with_recorder_settings(&data, port, |s| s.start_timeout = Duration::from_secs(60));
    let env = real_env(&sidecar, &db, dir.path(), port, &data);
    let pid = m
        .start(StartRequest { sidecar, env, port, secret: SECRET.to_string(), upgrade_required: false })
        .unwrap()
        .pid
        .unwrap();
    assert!(wait_until(Duration::from_secs(60), || m.status().state == GatewayState::Running));

    let at = std::time::Instant::now();
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    assert!(wait_until(Duration::from_secs(5), || m.status().state == GatewayState::Crashed), "{:?}", m.status());
    assert!(at.elapsed() < Duration::from_secs(5));
    let failure = m.status().failure.unwrap();
    assert!(!failure.log_excerpt.is_empty(), "the real banner and startup lines are in the excerpt");
    assert!(failure.log_excerpt.iter().all(|l| !l.contains(SECRET) && !l.contains('\u{1b}')));
}

/// The real uvicorn bind error, classified. This is the check that the text patterns match
/// what the shipped gateway actually prints, not just what the fixtures say.
#[test]
fn the_real_sidecar_reports_a_taken_port_in_a_way_the_classifier_understands() {
    let Some(sidecar) = built_sidecar() else { return skip("real bind error") };
    let dir = tempdir().unwrap();
    let data = dir.path().join("data");
    let db = dir.path().join("kiro-cli").join("data.sqlite3");
    write_fake_db(&db);
    let port = free_port();
    let _holder = TcpListener::bind(("127.0.0.1", port)).unwrap();

    // Launch directly (the manager would refuse earlier, which is the point of its pre-check).
    let env = real_env(&sidecar, &db, dir.path(), port, &data);
    let paths = AppPaths::new(&data);
    let runtime = sidecar::prepare_runtime_dir(&paths.runtime_dir()).unwrap();
    let output = Command::new(&sidecar.exe)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(&runtime)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success(), "the gateway must exit when it cannot bind");
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout).lines().map(str::to_string).collect();
    lines.extend(String::from_utf8_lossy(&output.stderr).lines().map(str::to_string));
    let found = classify(&lines).unwrap_or_else(|| panic!("not classified; output tail: {:?}", &lines[lines.len().saturating_sub(8)..]));
    assert_eq!(found.kind, FailureKind::PortInUse);
    assert_eq!(found.port, Some(port));
}

#[test]
fn read_run_tail_of_a_real_log_shows_the_current_run_only() {
    let Some(sidecar) = built_sidecar() else { return skip("real log tail") };
    let dir = tempdir().unwrap();
    let data = dir.path().join("data");
    let db = dir.path().join("kiro-cli").join("data.sqlite3");
    write_fake_db(&db);
    let port = free_port();
    let (m, _rec) = manager_with_recorder_settings(&data, port, |s| s.start_timeout = Duration::from_secs(60));
    for _ in 0..2 {
        let env = real_env(&sidecar, &db, dir.path(), port, &data);
        m.start(StartRequest { sidecar: sidecar.clone(), env, port, secret: SECRET.to_string(), upgrade_required: false }).unwrap();
        assert!(wait_until(Duration::from_secs(60), || m.status().state == GatewayState::Running));
        m.stop();
    }
    let tail = read_run_tail(&AppPaths::new(&data).gateway_log(), 1000);
    let starts = tail.iter().filter(|l| l.contains("Starting Uvicorn server")).count();
    assert_eq!(starts, 1, "only the last run is in the tail");
}
