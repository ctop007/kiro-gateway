//! Behavioural tests for the kclaude scripts: real `zsh -c` / `bash -c` in a temporary
//! HOME, a fake `claude` that prints what it received, and `python3 -m http.server` on a
//! port in 8100-8199 standing in for the gateway's `/health`.

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::script::{install, ScriptKind};

const SHELLS: [(&str, ScriptKind); 2] = [("zsh", ScriptKind::Zsh), ("bash", ScriptKind::Bash)];
const MSG_NO_CONFIG: &str = "找不到 Kiro Gateway 配置，请打开 Kiro Gateway App";
const MSG_UPGRADE: &str = "Kiro Gateway 需要升级，请打开 App 完成升级";
const MSG_NOT_RUNNING: &str = "KiroGateway 未运行，请在 Kiro Gateway App 里打开开关";

static NEXT_PORT: Mutex<u16> = Mutex::new(8100);

/// Hand out a port in 8100-8199 that nothing is listening on. Never hands out a port twice
/// within this test process.
fn free_port() -> u16 {
    let mut next = NEXT_PORT.lock().unwrap();
    while *next <= 8199 {
        let port = *next;
        *next += 1;
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port left in 8100-8199");
}

/// A `/health` stand-in. Only the child started here is ever killed.
struct HealthServer {
    child: Child,
    port: u16,
    _dir: tempfile::TempDir,
}

impl HealthServer {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("health"), r#"{"status":"healthy"}"#).unwrap();
        let port = free_port();
        let child = Command::new("python3")
            .args(["-m", "http.server", &port.to_string(), "--bind", "127.0.0.1", "--directory"])
            .arg(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("python3 is required for the kclaude script tests");
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "health server did not start");
            std::thread::sleep(Duration::from_millis(50));
        }
        Self { child, port, _dir: dir }
    }
}

impl Drop for HealthServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct RunResult {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Temporary HOME with an App data directory whose path contains a space, plus a fake
/// `claude` in front of PATH.
struct Sandbox {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    data_dir: PathBuf,
    bin_dir: PathBuf,
    marker: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let data_dir = home.join("Library/Application Support/dev.kirogateway.desktop");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        let marker = tmp.path().join("claude-called");
        let fake = bin_dir.join("claude");
        fs::write(
            &fake,
            format!(
                "#!/bin/sh\ntouch '{}'\necho \"CLAUDE_ARGS=$*\"\n\
                 echo \"BASE_URL=${{ANTHROPIC_BASE_URL-unset}}\"\n\
                 echo \"TOKEN=${{ANTHROPIC_AUTH_TOKEN-unset}}\"\n\
                 echo \"OPUS=${{ANTHROPIC_DEFAULT_OPUS_MODEL-unset}}\"\n\
                 echo \"SONNET=${{ANTHROPIC_DEFAULT_SONNET_MODEL-unset}}\"\n\
                 echo \"HAIKU=${{ANTHROPIC_DEFAULT_HAIKU_MODEL-unset}}\"\n\
                 echo \"NO_PROXY_UPPER=${{NO_PROXY-unset}}\"\n\
                 echo \"NO_PROXY_LOWER=${{no_proxy-unset}}\"\n\
                 case \"$1\" in --exit=*) exit \"${{1#--exit=}}\" ;; esac\n",
                marker.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { _tmp: tmp, home, data_dir, bin_dir, marker }
    }

    fn write_env(&self, contents: &str) {
        fs::write(self.data_dir.join("kclaude.env"), contents).unwrap();
    }

    fn env_for(port: u16) -> String {
        format!(
            "KIRO_GATEWAY_PORT={port}\nKIRO_GATEWAY_KEY='sk-test-key-123'\n\
             KCLAUDE_OPUS_MODEL='claude-opus-5-5[1m]'\nKCLAUDE_SONNET_MODEL='claude-sonnet-5-5[1m]'\n\
             KCLAUDE_HAIKU_MODEL='claude-haiku-4-5'\nKIRO_GATEWAY_UPGRADE_REQUIRED=0\n"
        )
    }

    fn claude_called(&self) -> bool {
        self.marker.exists()
    }

    /// Install the script for `kind`, then run `body` in `shell -c` after sourcing it.
    fn run(&self, shell: &str, kind: ScriptKind, extra_env: &[(&str, &str)], body: &str) -> RunResult {
        install(kind, &self.data_dir).unwrap();
        let script = self.data_dir.join(kind.file_name());
        let program = format!("source '{}'\n{}", script.display(), body);
        let path = format!("{}:/usr/bin:/bin", self.bin_dir.display());
        let mut cmd = Command::new(format!("/bin/{shell}"));
        cmd.arg("-c")
            .arg(program)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", path);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        RunResult {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }
}

fn line_value<'a>(out: &'a str, key: &str) -> Option<&'a str> {
    out.lines().find_map(|l| l.strip_prefix(&format!("{key}=")))
}

#[test]
fn missing_env_file_prompts_to_open_the_app_and_never_calls_claude() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let r = sb.run(shell, kind, &[], "kclaude hello");
        assert_ne!(r.code, 0, "{shell}");
        assert!(r.stderr.contains(MSG_NO_CONFIG), "{shell}: {}", r.stderr);
        assert!(!sb.claude_called(), "{shell}");
        assert!(r.stdout.is_empty(), "{shell}: {}", r.stdout);
    }
}

#[test]
fn env_file_without_port_or_key_is_treated_as_missing_config() {
    for (shell, kind) in SHELLS {
        for contents in ["", "KIRO_GATEWAY_PORT=8000\n", "KIRO_GATEWAY_KEY=abc\n", "KIRO_GATEWAY_PORT=abc\nKIRO_GATEWAY_KEY=k\n"] {
            let sb = Sandbox::new();
            sb.write_env(contents);
            let r = sb.run(shell, kind, &[], "kclaude");
            assert_ne!(r.code, 0, "{shell} {contents:?}");
            assert!(r.stderr.contains(MSG_NO_CONFIG), "{shell} {contents:?}: {}", r.stderr);
            assert!(!sb.claude_called(), "{shell} {contents:?}");
        }
    }
}

#[test]
fn gateway_not_running_exits_non_zero_without_calling_claude_or_starting_a_gateway() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let port = free_port();
        sb.write_env(&Sandbox::env_for(port));
        let r = sb.run(shell, kind, &[], "kclaude hello");
        assert_ne!(r.code, 0, "{shell}");
        assert!(r.stderr.contains(MSG_NOT_RUNNING), "{shell}: {}", r.stderr);
        assert!(!sb.claude_called(), "{shell}");
        // The script must not have launched anything that now listens on the port.
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "{shell}");
    }
}

#[test]
fn non_gateway_service_without_health_endpoint_counts_as_not_running() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let dir = tempfile::tempdir().unwrap(); // no `health` file: the server answers 404
        let port = free_port();
        let mut child = Command::new("python3")
            .args(["-m", "http.server", &port.to_string(), "--bind", "127.0.0.1", "--directory"])
            .arg(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        }
        sb.write_env(&Sandbox::env_for(port));
        let r = sb.run(shell, kind, &[], "kclaude");
        let _ = child.kill();
        let _ = child.wait();
        assert_ne!(r.code, 0, "{shell}");
        assert!(r.stderr.contains(MSG_NOT_RUNNING), "{shell}: {}", r.stderr);
        assert!(!sb.claude_called(), "{shell}");
    }
}

#[test]
fn running_gateway_sets_the_variables_and_passes_arguments_through() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&Sandbox::env_for(gw.port));
        let r = sb.run(shell, kind, &[], "kclaude --resume 'two words'");
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        assert_eq!(line_value(&r.stdout, "CLAUDE_ARGS"), Some("--resume two words"), "{shell}");
        let expected_url = format!("http://127.0.0.1:{}", gw.port);
        assert_eq!(line_value(&r.stdout, "BASE_URL"), Some(expected_url.as_str()), "{shell}");
        assert_eq!(line_value(&r.stdout, "TOKEN"), Some("sk-test-key-123"), "{shell}");
        assert_eq!(line_value(&r.stdout, "OPUS"), Some("claude-opus-5-5[1m]"), "{shell}");
        assert_eq!(line_value(&r.stdout, "SONNET"), Some("claude-sonnet-5-5[1m]"), "{shell}");
        assert_eq!(line_value(&r.stdout, "HAIKU"), Some("claude-haiku-4-5"), "{shell}");
        assert!(sb.claude_called(), "{shell}");
    }
}

#[test]
fn unquoted_env_values_and_double_quotes_are_accepted() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&format!(
            "KIRO_GATEWAY_PORT={}\nKIRO_GATEWAY_KEY=\"abc\"\nKCLAUDE_OPUS_MODEL=claude-opus-5-5[1m]\n",
            gw.port
        ));
        let r = sb.run(shell, kind, &[], "kclaude");
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        assert_eq!(line_value(&r.stdout, "TOKEN"), Some("abc"), "{shell}");
        assert_eq!(line_value(&r.stdout, "OPUS"), Some("claude-opus-5-5[1m]"), "{shell}");
    }
}

#[test]
fn missing_model_fields_fall_back_to_the_documented_defaults() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&format!("KIRO_GATEWAY_PORT={}\nKIRO_GATEWAY_KEY=k\n", gw.port));
        let r = sb.run(shell, kind, &[], "kclaude");
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        assert_eq!(line_value(&r.stdout, "OPUS"), Some("claude-opus-5-5[1m]"), "{shell}");
        assert_eq!(line_value(&r.stdout, "SONNET"), Some("claude-sonnet-5-5[1m]"), "{shell}");
        assert_eq!(line_value(&r.stdout, "HAIKU"), Some("claude-haiku-4-5"), "{shell}");
    }
}

#[test]
fn changing_the_port_takes_effect_in_an_already_open_shell() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let first = HealthServer::start();
        let second = HealthServer::start();
        let env_path = sb.data_dir.join("kclaude.env");
        sb.write_env(&Sandbox::env_for(first.port));
        let body = format!(
            "kclaude\nprintf '%s' '{}' > '{}'\necho ---\nkclaude\n",
            Sandbox::env_for(second.port).replace('\'', "'\\''"),
            env_path.display()
        );
        let r = sb.run(shell, kind, &[], &body);
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        let (before, after) = r.stdout.split_once("---\n").unwrap();
        assert_eq!(line_value(before, "BASE_URL"), Some(format!("http://127.0.0.1:{}", first.port).as_str()), "{shell}");
        assert_eq!(line_value(after, "BASE_URL"), Some(format!("http://127.0.0.1:{}", second.port).as_str()), "{shell}");
    }
}

#[test]
fn stale_port_after_the_gateway_moved_reports_not_running() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        let dead = free_port();
        let env_path = sb.data_dir.join("kclaude.env");
        sb.write_env(&Sandbox::env_for(gw.port));
        let body = format!(
            "printf '%s' '{}' > '{}'\nkclaude\n",
            Sandbox::env_for(dead).replace('\'', "'\\''"),
            env_path.display()
        );
        let r = sb.run(shell, kind, &[], &body);
        assert_ne!(r.code, 0, "{shell}");
        assert!(r.stderr.contains(MSG_NOT_RUNNING), "{shell}: {}", r.stderr);
        assert!(!sb.claude_called(), "{shell}");
    }
}

#[test]
fn upgrade_required_blocks_even_when_the_gateway_answers() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&Sandbox::env_for(gw.port).replace("UPGRADE_REQUIRED=0", "UPGRADE_REQUIRED=1"));
        let r = sb.run(shell, kind, &[], "kclaude");
        assert_ne!(r.code, 0, "{shell}");
        assert!(r.stderr.contains(MSG_UPGRADE), "{shell}: {}", r.stderr);
        assert!(!r.stderr.contains(MSG_NOT_RUNNING), "{shell}");
        assert!(!sb.claude_called(), "{shell}");
    }
}

#[test]
fn upgrade_required_wins_over_missing_gateway() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        sb.write_env(&Sandbox::env_for(free_port()).replace("UPGRADE_REQUIRED=0", "UPGRADE_REQUIRED=1"));
        let r = sb.run(shell, kind, &[], "kclaude");
        assert_ne!(r.code, 0, "{shell}");
        assert!(r.stderr.contains(MSG_UPGRADE), "{shell}: {}", r.stderr);
    }
}

#[test]
fn inherited_variables_cannot_override_the_env_file() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&Sandbox::env_for(gw.port));
        let r = sb.run(
            shell,
            kind,
            &[("KIRO_GATEWAY_PORT", "1"), ("KIRO_GATEWAY_UPGRADE_REQUIRED", "1"), ("KCLAUDE_HAIKU_MODEL", "evil")],
            "kclaude",
        );
        // UPGRADE_REQUIRED=0 in the file is authoritative; inherited values are discarded.
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        assert_eq!(line_value(&r.stdout, "HAIKU"), Some("claude-haiku-4-5"), "{shell}");
    }
}

#[test]
fn kclaude_does_not_leak_variables_into_the_calling_shell_or_plain_claude() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&Sandbox::env_for(gw.port));
        let body = "kclaude >/dev/null\necho \"AFTER_URL=${ANTHROPIC_BASE_URL-unset}\"\n\
                    echo \"AFTER_PORT=${KIRO_GATEWAY_PORT-unset}\"\nclaude plain\n";
        let r = sb.run(shell, kind, &[], body);
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        assert_eq!(line_value(&r.stdout, "AFTER_URL"), Some("unset"), "{shell}");
        assert_eq!(line_value(&r.stdout, "AFTER_PORT"), Some("unset"), "{shell}");
        // The plain `claude` run after kclaude sees no gateway variables.
        assert_eq!(line_value(&r.stdout, "BASE_URL"), Some("unset"), "{shell}");
        assert_eq!(line_value(&r.stdout, "TOKEN"), Some("unset"), "{shell}");
    }
}

#[test]
fn claude_exit_status_is_propagated() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&Sandbox::env_for(gw.port));
        let r = sb.run(shell, kind, &[], "kclaude --exit=7");
        assert_eq!(r.code, 7, "{shell}");
    }
}

#[test]
fn missing_claude_binary_gives_an_actionable_message() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&Sandbox::env_for(gw.port));
        fs::remove_file(sb.bin_dir.join("claude")).unwrap();
        let r = sb.run(shell, kind, &[], "kclaude");
        assert_ne!(r.code, 0, "{shell}");
        assert!(r.stderr.contains("请先安装 Claude Code"), "{shell}: {}", r.stderr);
    }
}

#[test]
fn data_dir_with_quotes_and_dollar_signs_is_quoted_correctly() {
    for (shell, kind) in SHELLS {
        let tmp = tempfile::tempdir().unwrap();
        let odd = tmp.path().join("it's $HOME `x` dir");
        fs::create_dir_all(&odd).unwrap();
        fs::write(odd.join("kclaude.env"), "KIRO_GATEWAY_PORT=1\n").unwrap();
        install(kind, &odd).unwrap();
        let program = format!("source '{}'\nkclaude", odd.join(kind.file_name()).to_string_lossy().replace('\'', "'\\''"));
        let out = Command::new(format!("/bin/{shell}"))
            .arg("-c")
            .arg(program)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        // The env file was found (so the path survived quoting) and judged incomplete.
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(MSG_NO_CONFIG), "{shell}: {stderr}");
        assert!(!stderr.contains("not found") && !stderr.contains("No such file"), "{shell}: {stderr}");
    }
}

#[test]
fn zsh_script_survives_zsh_option_changes_in_the_user_shell() {
    let sb = Sandbox::new();
    let gw = HealthServer::start();
    sb.write_env(&Sandbox::env_for(gw.port));
    // sh-style emulation, a hostile IFS and nomatch/nounset on: kclaude must still work.
    let r = sb.run(
        "zsh",
        ScriptKind::Zsh,
        &[],
        "setopt nounset ksharrays shwordsplit\nIFS=x\nkclaude",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(line_value(&r.stdout, "HAIKU"), Some("claude-haiku-4-5"));
}


const PROXY_ENV: [(&str, &str); 4] = [
    ("http_proxy", "http://127.0.0.1:9"),
    ("https_proxy", "http://127.0.0.1:9"),
    ("HTTP_PROXY", "http://127.0.0.1:9"),
    ("HTTPS_PROXY", "http://127.0.0.1:9"),
];

fn no_proxy_has_loopback(value: &str) -> bool {
    let entries: Vec<&str> = value.split(',').map(str::trim).collect();
    entries.contains(&"127.0.0.1") && entries.contains(&"localhost")
}

#[test]
fn health_probe_ignores_user_http_proxy_variables() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&Sandbox::env_for(gw.port));
        // Nothing listens on the proxy port 9: a probe that honours the proxy would fail.
        let r = sb.run(shell, kind, &PROXY_ENV, "kclaude hello");
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        assert!(!r.stderr.contains(MSG_NOT_RUNNING), "{shell}: {}", r.stderr);
        assert!(sb.claude_called(), "{shell}");
    }
}

#[test]
fn claude_gets_loopback_in_no_proxy_without_dropping_the_user_value() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&Sandbox::env_for(gw.port));
        let mut env = PROXY_ENV.to_vec();
        env.push(("NO_PROXY", "corp.example.com"));
        env.push(("no_proxy", "intranet.local"));
        let r = sb.run(shell, kind, &env, "kclaude");
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        for key in ["NO_PROXY_UPPER", "NO_PROXY_LOWER"] {
            let v = line_value(&r.stdout, key).unwrap();
            assert!(no_proxy_has_loopback(v), "{shell} {key}: {v}");
        }
        assert!(line_value(&r.stdout, "NO_PROXY_UPPER").unwrap().contains("corp.example.com"), "{shell}");
        assert!(line_value(&r.stdout, "NO_PROXY_LOWER").unwrap().contains("intranet.local"), "{shell}");
    }
}

#[test]
fn no_proxy_is_set_when_the_user_had_none_and_does_not_leak_into_the_shell() {
    for (shell, kind) in SHELLS {
        let sb = Sandbox::new();
        let gw = HealthServer::start();
        sb.write_env(&Sandbox::env_for(gw.port));
        let body = "kclaude\necho \"AFTER=${NO_PROXY-unset}\"\n";
        let r = sb.run(shell, kind, &PROXY_ENV, body);
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        assert!(no_proxy_has_loopback(line_value(&r.stdout, "NO_PROXY_UPPER").unwrap()), "{shell}");
        assert!(no_proxy_has_loopback(line_value(&r.stdout, "NO_PROXY_LOWER").unwrap()), "{shell}");
        assert_eq!(line_value(&r.stdout, "AFTER"), Some("unset"), "{shell}");
    }
}

#[test]
fn powershell_script_probes_without_a_proxy_and_extends_no_proxy() {
    let ps1 = include_str!("templates/kclaude.ps1");
    assert!(ps1.contains("HttpClient"), "health probe must use a proxy-free HttpClient");
    assert!(ps1.contains("UseProxy"), "health probe must disable the proxy");
    assert!(!ps1.contains("Invoke-RestMethod -"), "Invoke-RestMethod honours the system proxy on PS5");
    assert!(ps1.contains("'NO_PROXY'") && ps1.contains("'no_proxy'"));
    assert!(ps1.contains("127.0.0.1,localhost"));
}
