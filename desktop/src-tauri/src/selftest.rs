//! Headless acceptance run (debug builds only): `kiro-gateway-desktop --selftest`.
//!
//! Drives the same functions the commands call, with no window: the environment check, the
//! kclaude install into a startup file, the real PyInstaller sidecar, the `cclaude` command
//! (the default command name) in a real zsh, renaming it, stopping, and removal. Everything happens below a throw-away HOME and data
//! directory; the user's real startup files are only `stat`ed before and after to prove they
//! were not touched.
//!
//! Environment (all optional):
//!
//! - `KGW_SELFTEST_PORT`: port to use (default: first free one in 8100..=8199; the App's own
//!   default, 6868, is never used so a gateway the user runs there is left alone).
//! - `KGW_SELFTEST_DB`: kiro-cli database to hand to the sidecar. Default: a fake one with a
//!   far-future expiry, so no token refresh can happen and the user's real login (whose refresh
//!   token a refresh would rotate) is never involved.
//! - `KIRO_GATEWAY_SIDECAR`: sidecar executable (default: the bundle's `Resources/gateway`, or
//!   `src-tauri/resources/gateway` of the source tree).
//!
//! Exit status: 0 when every check passed.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app_config::command_name::DEFAULT_COMMAND_NAME;
use crate::app_config::{AppPaths, ConfigStore};
use crate::env_check::report::{gateway_gate, run_env_check, CheckStatus, GatewayGate};
use crate::gateway::port::{PortOps, SystemPorts};
use crate::gateway::service::{EnvGate, GatewayService, ServiceError};
use crate::gateway::sys::{ProcessOps, SystemProcessOps};
use crate::gateway::GatewayState;
use crate::kclaude::service::{KclaudeService, SystemBackend};
use crate::kclaude::types::KclaudeState;
use crate::kclaude::verify::{run_shell_with_env, RunOutcome};

struct Checks {
    passed: u32,
    failed: u32,
}

impl Checks {
    fn check(&mut self, name: &str, ok: bool, detail: impl FnOnce() -> String) {
        if ok {
            self.passed += 1;
            println!("PASS {name}");
        } else {
            self.failed += 1;
            println!("FAIL {name}: {}", detail());
        }
    }
}

struct FixedGate(PathBuf);

impl EnvGate for FixedGate {
    fn check(&self) -> Result<PathBuf, String> {
        Ok(self.0.clone())
    }
}

/// `(mtime, len)` of a file, or `None` when it does not exist.
fn stat(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    fs::metadata(path).ok().map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len()))
}

fn free_port() -> Option<u16> {
    if let Some(p) = std::env::var("KGW_SELFTEST_PORT").ok().and_then(|v| v.parse().ok()) {
        return Some(p);
    }
    (8100..=8199).find(|p| TcpListener::bind(("127.0.0.1", *p)).is_ok() && SystemPorts.is_free(*p))
}

fn resource_dir() -> PathBuf {
    let bundle = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(|p| p.parent()).map(|p| p.join("Resources")))
        .filter(|dir| dir.join("gateway").is_dir());
    bundle.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources"))
}

/// Same layout `smoke_sidecar.py` uses: an OIDC token that does not expire for a year.
fn write_fake_kiro_cli_db(path: &Path) -> Result<(), String> {
    let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    let expires = (chrono::Utc::now() + chrono::Duration::days(365)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let token = serde_json::json!({
        "access_token": "selftest-fake-access", "refresh_token": "selftest-fake-refresh",
        "expires_at": expires, "region": "us-east-1"
    });
    let registration = serde_json::json!({
        "client_id": "selftest-fake-client", "client_secret": "selftest-fake-secret", "region": "us-east-1"
    });
    conn.execute("CREATE TABLE auth_kv (key TEXT PRIMARY KEY, value TEXT)", []).map_err(|e| e.to_string())?;
    for (key, value) in [("kirocli:odic:token", token), ("kirocli:odic:device-registration", registration)] {
        conn.execute("INSERT INTO auth_kv VALUES (?1, ?2)", rusqlite::params![key, value.to_string()])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn http_get_status(port: u16, path: &str) -> Option<u16> {
    let mut stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").ok()?;
    let mut text = String::new();
    stream.read_to_string(&mut text).ok()?;
    text.split_whitespace().nth(1)?.parse().ok()
}

fn wait_for(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    condition()
}

/// Run `<command> <args>` in a login interactive zsh with `HOME` set to the throw-away one.
fn run_command(home: &Path, name: &str, args: &str) -> Option<(Option<i32>, String, String)> {
    let command = format!("{name} {args}");
    match run_shell_with_env(Path::new("/bin/zsh"), &["-lic", &command], home, Duration::from_secs(30), &[]) {
        RunOutcome::Finished { code, stdout, stderr } => Some((code, stdout, stderr)),
        _ => None,
    }
}

/// Run the default-named command (`cclaude`).
fn run_kclaude(home: &Path, args: &str) -> Option<(Option<i32>, String, String)> {
    run_command(home, DEFAULT_COMMAND_NAME, args)
}

fn pgrep_count(pattern: &str) -> usize {
    std::process::Command::new("pgrep")
        .args(["-f", pattern])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

fn value_of(output: &str, key: &str) -> Option<String> {
    output.lines().find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_string))
}

/// Run every scenario. Returns the process exit code.
pub fn run() -> i32 {
    let mut c = Checks { passed: 0, failed: 0 };
    let real_home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let guarded: Vec<PathBuf> = [".zshrc", ".zprofile", ".zshenv", ".bashrc", ".bash_profile", ".claude/settings.json"]
        .iter()
        .map(|f| real_home.join(f))
        .collect();
    let before: Vec<_> = guarded.iter().map(|p| stat(p)).collect();

    // ---- 1. environment check against the real machine (read-only) -------------------
    let report = run_env_check();
    let status_of = |id: &str| {
        report.items.iter().find(|i| format!("{:?}", i.id).eq_ignore_ascii_case(&id.replace('_', ""))).map(|i| i.status)
    };
    println!(
        "INFO env on this machine: kiro-cli {:?}, logged in {:?}, login valid {:?}, claude {:?}",
        status_of("kiro_cli_installed"),
        status_of("kiro_cli_logged_in"),
        status_of("login_valid"),
        status_of("claude_installed")
    );
    let serialized = serde_json::to_string(&report).unwrap_or_default();
    c.check("env report never contains token material", !serialized.to_lowercase().contains("token\":\""), || {
        "report mentions a token value".into()
    });

    // ---- throw-away world ------------------------------------------------------------
    let root = std::env::temp_dir().join(format!("kgw-selftest-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let home = root.join("home");
    let data = root.join("data");
    let bin = home.join("bin");
    for dir in [&home, &data, &bin] {
        if fs::create_dir_all(dir).is_err() {
            println!("FAIL cannot create {}", dir.display());
            return 2;
        }
    }
    std::env::set_var("HOME", &home);
    std::env::set_var("SHELL", "/bin/zsh");
    std::env::remove_var("ZDOTDIR");

    let original_rc = "# my own zshrc\nexport PATH=\"$HOME/bin:$PATH\"\nalias ll='ls -l'\n";
    let rc_path = home.join(".zshrc");
    let _ = fs::write(&rc_path, original_rc);
    let fake_claude = bin.join("claude");
    let _ = fs::write(
        &fake_claude,
        "#!/bin/sh\necho \"CLAUDE_ARGS=$*\"\necho \"BASE=$ANTHROPIC_BASE_URL\"\necho \"TOKEN=$ANTHROPIC_AUTH_TOKEN\"\n\
         echo \"OPUS=$ANTHROPIC_DEFAULT_OPUS_MODEL\"\necho \"SONNET=$ANTHROPIC_DEFAULT_SONNET_MODEL\"\n\
         echo \"HAIKU=$ANTHROPIC_DEFAULT_HAIKU_MODEL\"\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&fake_claude, fs::Permissions::from_mode(0o755));
    }

    let Some(port) = free_port() else {
        println!("FAIL no free port in 8100-8199");
        return 2;
    };
    let paths = AppPaths::new(&data);
    let opened = ConfigStore::open(paths.clone(), chrono::Local::now().naive_local());
    let Ok((config, _notes)) = opened else {
        println!("FAIL cannot open the configuration");
        return 2;
    };
    let config = Arc::new(config);
    let patch = crate::app_config::ConfigPatch { port: Some(port as i64), ..Default::default() };
    let _ = config.update(&patch, false);
    c.check("configuration written to the throw-away data dir", paths.config_file().is_file() && paths.secret_file().is_file(), || {
        "config.json or secret missing".into()
    });

    // ---- 2. first install without a kiro-cli login (task 9.2) ------------------------
    let blocked_report = run_env_check();
    let login_item = blocked_report.items.iter().find(|i| i.status != CheckStatus::Ok);
    c.check("no kiro-cli login in the fresh HOME: some check fails", login_item.is_some(), || "all checks passed".into());
    c.check(
        "no login: the gate blocks the switch with a reason",
        matches!(gateway_gate(&blocked_report), GatewayGate::Blocked { ref reason } if !reason.is_empty()),
        || "gate allowed the switch".into(),
    );
    let sidecar_dir = resource_dir();
    let blocked_service = crate::gateway::commands::build_service(config.clone(), sidecar_dir.clone(), crate::gateway::commands::env_check_gate());
    let refused = blocked_service.start();
    c.check("no login: gateway start is refused with env_not_ready", matches!(refused, Err(ServiceError::EnvNotReady(_))), || {
        format!("{refused:?}")
    });
    c.check(
        "no login: nothing started and the switch is not remembered",
        blocked_service.status().state == GatewayState::Stopped && !config.get().gateway_wanted,
        || "state or gatewayWanted changed".into(),
    );
    drop(blocked_service);

    // ---- 3. install kclaude into the throw-away .zshrc -------------------------------
    let env = crate::kclaude::commands::make_env(home.clone(), data.clone(), &config.get().command_name);
    let backend = SystemBackend::new(crate::kclaude::commands::search_dirs(&[bin.clone()], &home));
    let kclaude = KclaudeService::new(env, &backend);
    let status = kclaude.install();
    c.check(
        "the default command name is cclaude and the default port is 6868",
        config.get().command_name == "cclaude" && crate::app_config::model::DEFAULT_PORT == 6868,
        || format!("{:?}", config.get().command_name),
    );
    c.check("kclaude install succeeds", matches!(&status, Ok(s) if s.state == KclaudeState::Installed), || format!("{status:?}"));
    if let Ok(s) = &status {
        c.check("kclaude check in a fresh login shell is ok", format!("{:?}", s.verify.status) == "Ok", || format!("{:?}", s.verify.detail));
        c.check("claude found on the given directories", s.claude_installed, || "claude_installed false".into());
    }
    let installed_rc = fs::read_to_string(&rc_path).unwrap_or_default();
    c.check(
        "the startup file keeps the user's lines and gains one marked block",
        installed_rc.starts_with(original_rc) && installed_rc.matches("# >>> kiro-gateway-desktop command >>>").count() == 1,
        || "block missing, duplicated or user lines changed".into(),
    );
    c.check(
        "a backup of the startup file was written before the change",
        fs::read_dir(&home).map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with(".zshrc.bak-kiro-gateway-"))).unwrap_or(false),
        || "no .zshrc.bak-kiro-gateway-* file".into(),
    );

    c.check(
        "scripts are named after the command: cclaude.zsh exists, kclaude.zsh does not",
        data.join("cclaude.zsh").is_file() && !data.join("kclaude.zsh").exists(),
        || "unexpected script file names".into(),
    );
    match run_command(&home, "type", "cclaude") {
        Some((_, out, _)) => c.check("a fresh login shell knows cclaude as a function", out.contains("function"), || out.clone()),
        None => c.check("a fresh login shell ran `type cclaude`", false, || "shell failed".into()),
    }

    // ---- 4. start the real sidecar, /health only ------------------------------------------
    let db = std::env::var_os("KGW_SELFTEST_DB").map(PathBuf::from).unwrap_or_else(|| {
        let fake = root.join("fake-kiro-cli.sqlite3");
        if let Err(e) = write_fake_kiro_cli_db(&fake) {
            println!("FAIL cannot write the fake kiro-cli database: {e}");
        }
        fake
    });
    let service: Arc<GatewayService> = crate::gateway::commands::build_service(config.clone(), sidecar_dir.clone(), Arc::new(FixedGate(db)));
    let started = service.start();
    c.check("gateway start accepted", started.is_ok(), || format!("{started:?}"));
    let running = wait_for(Duration::from_secs(40), || service.status().state == GatewayState::Running);
    c.check("gateway reaches running within 40 s", running, || format!("{:?}", service.status()));
    let pid = service.status().pid;
    c.check("/health answers 200 on the chosen port", http_get_status(port, "/health") == Some(200), || "no 200".into());
    c.check("gatewayWanted remembered", config.get().gateway_wanted, || "false".into());
    let log_text = fs::read_to_string(paths.gateway_log()).unwrap_or_default();
    c.check("gateway.log exists and does not contain the access key", !log_text.is_empty() && !log_text.contains(&config.secret()), || {
        "empty log or key leaked".into()
    });
    let loopback_only = std::process::Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
        .output()
        .map(|o| {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            text.contains("127.0.0.1:") && !text.contains("*:") && !text.contains("0.0.0.0:")
        })
        .unwrap_or(false);
    c.check("the gateway listens on 127.0.0.1 only", loopback_only, || "lsof shows a wider listener".into());

    // ---- 5. kclaude in a fresh zsh: the environment chain --------------------------------
    match run_kclaude(&home, "--version") {
        Some((code, out, err)) => {
            c.check("kclaude exits 0 with the gateway running", code == Some(0), || format!("code {code:?}: {err}"));
            c.check("claude got the arguments", value_of(&out, "CLAUDE_ARGS").as_deref() == Some("--version"), || out.clone());
            c.check(
                "ANTHROPIC_BASE_URL points at the configured port",
                value_of(&out, "BASE").as_deref() == Some(&format!("http://127.0.0.1:{port}")),
                || out.clone(),
            );
            c.check(
                "ANTHROPIC_AUTH_TOKEN is the stored access key",
                value_of(&out, "TOKEN").as_deref() == Some(config.secret().as_str()),
                || "token differs".into(),
            );
            let models = config.get().models;
            c.check(
                "the three model tiers come from the configuration",
                value_of(&out, "OPUS").as_deref() == Some(models.opus.as_str())
                    && value_of(&out, "SONNET").as_deref() == Some(models.sonnet.as_str())
                    && value_of(&out, "HAIKU").as_deref() == Some(models.haiku.as_str()),
                || out.clone(),
            );
        }
        None => c.check("kclaude ran in zsh", false, || "shell failed or timed out".into()),
    }

    // ---- 6. forced upgrade: kclaude refuses, start refuses -------------------------------------
    let _ = config.set_upgrade_required(true);
    match run_kclaude(&home, "--version") {
        Some((code, out, err)) => {
            c.check("forced upgrade: kclaude exits non-zero", code.is_some_and(|c| c != 0), || format!("{code:?}"));
            c.check("forced upgrade: claude is not called", value_of(&out, "CLAUDE_ARGS").is_none(), || out.clone());
            c.check("forced upgrade: the message asks for the upgrade", err.contains("需要升级"), || err.clone());
        }
        None => c.check("kclaude ran in zsh (forced)", false, || "shell failed".into()),
    }
    let _ = service.stop();
    let refused = service.start();
    c.check("forced upgrade: gateway start is refused", matches!(refused, Err(ServiceError::UpgradeRequired)), || format!("{refused:?}"));
    let _ = config.set_upgrade_required(false);

    // ---- 7. stop: nothing is left behind -------------------------------------------------
    c.check("stop returned the stopped state", service.status().state == GatewayState::Stopped, || format!("{:?}", service.status()));
    c.check("the sidecar process is gone", pid.map(|p| !SystemProcessOps.is_alive(p)).unwrap_or(true), || "pid still alive".into());
    c.check("the port is free again", SystemPorts.is_free(port), || "port still taken".into());
    let sidecar_path = sidecar_dir.join("gateway").join("kiro-gateway");
    c.check("no process of the sidecar executable remains", pgrep_count(&sidecar_path.to_string_lossy()) == 0, || {
        "pgrep still finds one".into()
    });
    match run_kclaude(&home, "--version") {
        Some((code, out, err)) => {
            c.check("gateway stopped: kclaude exits non-zero", code.is_some_and(|c| c != 0), || format!("{code:?}"));
            c.check("gateway stopped: claude is not called", value_of(&out, "CLAUDE_ARGS").is_none(), || out.clone());
            c.check("gateway stopped: the message says it is not running", err.contains("未运行"), || err.clone());
        }
        None => c.check("kclaude ran in zsh (stopped)", false, || "shell failed".into()),
    }

    // ---- 7b. rename the command: new name works, old name is gone, one block -------------------
    let renamed = crate::kclaude::rename::apply_update(
        &config,
        &crate::kclaude::commands::make_env(home.clone(), data.clone(), &config.get().command_name),
        &backend,
        &crate::app_config::ConfigPatch { command_name: Some("kclaude-test".into()), ..Default::default() },
        false,
    );
    c.check("renaming the command to kclaude-test succeeds", renamed.is_ok(), || format!("{renamed:?}"));
    let after_rename = fs::read_to_string(&rc_path).unwrap_or_default();
    c.check(
        "after the rename the startup file has one block and points at kclaude-test.zsh",
        after_rename.matches("# >>> kiro-gateway-desktop command >>>").count() == 1
            && after_rename.contains("kclaude-test.zsh")
            && !after_rename.contains("cclaude.zsh"),
        || after_rename.clone(),
    );
    c.check(
        "the old script is deleted and the new one exists",
        !data.join("cclaude.zsh").exists() && data.join("kclaude-test.zsh").is_file(),
        || "script files do not match the new name".into(),
    );
    match run_command(&home, "type", "kclaude-test") {
        Some((_, out, _)) => c.check("the new name is a function in a fresh login shell", out.contains("function"), || out.clone()),
        None => c.check("a fresh login shell ran `type kclaude-test`", false, || "shell failed".into()),
    }
    match run_command(&home, "type", "cclaude") {
        Some((_, out, _)) => c.check("the old name no longer exists", !out.contains("cclaude is a"), || out.clone()),
        None => c.check("a fresh login shell ran `type cclaude`", false, || "shell failed".into()),
    }
    match run_command(&home, "kclaude-test", "--version") {
        Some((code, _out, err)) => {
            c.check("renamed command: gateway stopped gives a non-zero exit", code.is_some_and(|c| c != 0), || format!("{code:?}"));
            c.check("renamed command: the message starts with the new name", err.contains("kclaude-test: "), || err.clone());
        }
        None => c.check("the renamed command ran in zsh", false, || "shell failed".into()),
    }
    let refused = crate::kclaude::rename::apply_update(
        &config,
        &crate::kclaude::commands::make_env(home.clone(), data.clone(), &config.get().command_name),
        &backend,
        &crate::app_config::ConfigPatch { command_name: Some("claude".into()), ..Default::default() },
        false,
    );
    c.check(
        "renaming to claude is refused and the name stays",
        refused.is_err() && config.get().command_name == "kclaude-test",
        || format!("{refused:?}"),
    );

    // ---- 8. remove kclaude: the startup file is back to the original bytes ---------------
    let kclaude = KclaudeService::new(
        crate::kclaude::commands::make_env(home.clone(), data.clone(), &config.get().command_name),
        &backend,
    );
    let removed = kclaude.remove();
    c.check("kclaude remove succeeds", matches!(&removed, Ok(s) if s.state == KclaudeState::NotInstalled), || format!("{removed:?}"));
    c.check(
        "removing leaves the user's startup file byte-for-byte as it was",
        fs::read_to_string(&rc_path).map(|t| t == original_rc).unwrap_or(false),
        || "content differs from the original".into(),
    );

    // ---- 9. the real startup files were not touched ----------------------------------------
    let after: Vec<_> = guarded.iter().map(|p| stat(p)).collect();
    for ((path, b), a) in guarded.iter().zip(&before).zip(&after) {
        c.check(&format!("real {} untouched (mtime and size)", path.strip_prefix(&real_home).unwrap_or(path).display()), b == a, || {
            format!("before {b:?}, after {a:?}")
        });
    }
    let _ = fs::remove_dir_all(&root);

    println!("RESULT passed={} failed={}", c.passed, c.failed);
    i32::from(c.failed > 0)
}
