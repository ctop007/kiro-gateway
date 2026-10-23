//! Test support for the gateway module: free ports, fake sidecars, status recording.
//!
//! The fake sidecar is a small Python script with a shebang. It stands in for the
//! PyInstaller binary: it listens on `SERVER_HOST:SERVER_PORT`, answers `/health` and
//! `/v1/models`, and can be told (by the mode baked into its source) to start slowly,
//! never listen, exit at once, crash later, ignore SIGTERM, or print the log lines of a
//! real failed start. Copying a system binary such as `sleep` does not work on macOS
//! (the copy is killed by code signing), hence scripts.

use super::env::{build_env, EnvInputs};
use crate::app_config::AppPaths;
use super::health::HttpHealth;
use super::manager::{Deps, GatewayManager, ManagerSettings, StartRequest};
use super::port::SystemPorts;
use super::sidecar::Sidecar;
use super::sys::SystemProcessOps;
use super::types::{GatewayState, GatewayStatus};
use chrono::{DateTime, FixedOffset, TimeZone};
use std::ffi::OsString;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Ports handed out recently by this test process, oldest first.
///
/// The OS hands out ephemeral ports round-robin, so a port is rarely given twice in a row,
/// but a sidecar that was just started on a port may not have bound it yet. A port is
/// therefore not handed out again while it is among the last `RECENT_WINDOW` ones.
static RECENT: Mutex<Vec<u16>> = Mutex::new(Vec::new());
const RECENT_WINDOW: usize = 64;

/// A free loopback port chosen by the OS from its ephemeral range.
///
/// Fake sidecars of concurrent tests (and of other test binaries running at the same
/// time) used to share 8100..=8199 and occasionally answered each other's health probes.
/// The ephemeral range never contains 6868 / 8000 (the App's default port and the user's own
/// gateway) nor the 81xx ports used for manual verification.
pub fn free_port() -> u16 {
    let mut recent = RECENT.lock().unwrap_or_else(|p| p.into_inner());
    for _ in 0..200 {
        let port = TcpListener::bind(("127.0.0.1", 0))
            .and_then(|listener| listener.local_addr())
            .map(|addr| addr.port())
            .expect("the OS must hand out an ephemeral loopback port");
        if recent.contains(&port) || (8000..8200).contains(&port) {
            continue;
        }
        recent.push(port);
        if recent.len() > RECENT_WINDOW {
            recent.remove(0);
        }
        return port;
    }
    panic!("the OS kept handing out recently used ports ({} recent)", recent.len());
}

#[test]
fn free_port_never_returns_the_users_gateway_range_or_a_recent_port() {
    let ports: Vec<u16> = (0..40).map(|_| free_port()).collect();
    assert!(ports.iter().all(|p| !(8000..8200).contains(p)), "{ports:?}");
    let mut unique = ports.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ports.len(), "a port was handed out twice: {ports:?}");
}

/// Fixed clock so `since` / `at` are predictable.
pub struct FixedClock(pub Mutex<DateTime<FixedOffset>>);

impl FixedClock {
    pub fn new() -> Self {
        Self(Mutex::new(FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2026, 10, 22, 14, 41, 8).unwrap()))
    }
}

impl super::manager::Clock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        *self.0.lock().unwrap()
    }
}

/// Collects every status the manager reports, in order.
#[derive(Clone, Default)]
pub struct Recorder(pub Arc<Mutex<Vec<GatewayStatus>>>);

impl Recorder {
    pub fn states(&self) -> Vec<GatewayState> {
        self.0.lock().unwrap().iter().map(|s| s.state).collect()
    }
    pub fn all(&self) -> Vec<GatewayStatus> {
        self.0.lock().unwrap().clone()
    }
    pub fn listener(&self) -> Box<dyn Fn(&GatewayStatus) + Send + Sync> {
        let inner = self.0.clone();
        Box::new(move |s| inner.lock().unwrap().push(s.clone()))
    }
}

/// Poll `condition` every 10 ms until it holds or `timeout` passes.
pub fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    condition()
}

/// Timings short enough for tests.
pub fn fast_settings() -> ManagerSettings {
    ManagerSettings {
        start_timeout: Duration::from_secs(10),
        poll_interval: Duration::from_millis(50),
        heartbeat_interval: Duration::from_millis(100),
        heartbeat_failures: 3,
        term_grace: Duration::from_millis(600),
        excerpt_lines: 20,
        pump_join_timeout: Duration::from_secs(2),
    }
}

/// Production adapters with the fixed clock.
pub fn system_deps() -> Deps {
    Deps {
        clock: Arc::new(FixedClock::new()),
        health: Arc::new(HttpHealth::new(Duration::from_millis(800))),
        ops: Arc::new(SystemProcessOps),
        ports: Arc::new(SystemPorts),
    }
}

const FAKE_TEMPLATE: &str = r#"#!/usr/bin/env python3
import json, os, signal, socket, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODE = "__MODE__"
SECRET = os.environ.get("PROXY_API_KEY", "")
DUMP = os.environ.get("KGW_FAKE_DUMP")

def out(text, stream=sys.stdout):
    stream.write(text + "\n")
    stream.flush()

LAUNCHES = os.environ.get("KGW_FAKE_LAUNCHES")
if LAUNCHES:
    with open(LAUNCHES, "a") as f:
        f.write(str(os.getpid()) + "\n")

if DUMP:
    # Atomic, so a reader never sees a half-written file.
    with open(DUMP + ".tmp", "w") as f:
        json.dump({"cwd": os.getcwd(), "env": dict(os.environ), "pid": os.getpid()}, f)
    os.replace(DUMP + ".tmp", DUMP)

REAL_403 = [
    "2026-10-22 23:22:35 | INFO     | kiro.auth:_do_aws_sso_oidc_refresh:809 - Refreshing Kiro token via AWS SSO OIDC...",
    "2026-10-22 23:22:35 | ERROR    | kiro.auth:_do_aws_sso_oidc_refresh:840 - AWS SSO OIDC refresh failed: status=403, body={\"message\":\"forbidden\"}",
    "2026-10-22 23:22:35 | ERROR    | kiro.account_manager:_initialize_account:578 - Failed to initialize account /x/data.sqlite3: Client error '403 Forbidden' for url 'https://oidc.us-east-1.amazonaws.com/token'",
    "2026-10-22 23:22:35 | ERROR    | main:lifespan:499 - Failed to initialize any account. Check your credentials.",
]

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass
    def _send(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def do_GET(self):
        if self.path == "/health":
            self._send(200, {"status": "healthy"})
        elif self.path == "/v1/models":
            if self.headers.get("Authorization") != "Bearer " + SECRET:
                self._send(401, {"detail": "Invalid or missing API Key"})
            else:
                self._send(200, {"object": "list", "data": [{"id": "claude-haiku-4.5"}, {"id": "auto"}]})
        else:
            self._send(404, {})

def serve():
    host = os.environ.get("SERVER_HOST", "127.0.0.1")
    port = int(os.environ["SERVER_PORT"])
    server = ThreadingHTTPServer((host, port), Handler)
    server.serve_forever()

name, _, arg = MODE.partition(":")
if name == "serve":
    out("startup banner")
    serve()
elif name == "slow":
    time.sleep(float(arg))
    out("late banner")
    serve()
elif name == "never":
    out("loading, never listens")
    time.sleep(120)
elif name == "exit":
    out("about to fail", sys.stderr)
    sys.exit(int(arg or 1))
elif name == "crash_after":
    threading.Thread(target=lambda: (time.sleep(float(arg)), out("fatal: boom", sys.stderr), os._exit(1)), daemon=True).start()
    serve()
elif name == "ignore_term":
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    out("ignoring SIGTERM")
    serve()
elif name == "graceful_term":
    # Records that SIGTERM was handled (as opposed to the process being killed), then exits 0.
    def on_term(signum, frame):
        with open(os.environ["KGW_FAKE_TERM_MARK"], "w") as f:
            f.write("handled")
        os._exit(0)
    signal.signal(signal.SIGTERM, on_term)
    out("handling SIGTERM")
    serve()
elif name == "login_fail":
    for line in REAL_403:
        out(line, sys.stderr)
    sys.exit(1)
elif name == "login_fail_hang":
    for line in REAL_403:
        out(line, sys.stderr)
    time.sleep(120)
elif name == "bind_fail":
    out("2026-10-22 22:02:36 | ERROR    | logging:callHandlers:1737 - [Errno 48] error while attempting to bind on address ('127.0.0.1', " + os.environ["SERVER_PORT"] + "): address already in use", sys.stderr)
    sys.exit(1)
elif name == "echo_secret":
    out("using key " + SECRET)
    out("token 9b5ad71b2ce5302211f9c61530b329a4922fc6a4")
    sys.exit(1)
elif name == "many_lines":
    for i in range(int(arg)):
        out("log line %d" % i)
    sys.exit(2)
elif name == "serve_then_quiet":
    serve()
else:
    out("unknown mode " + MODE, sys.stderr)
    sys.exit(99)
"#;

/// Fakes for the timeout tests are plain `sh` scripts: they print within milliseconds even
/// on a machine busy compiling, whereas Python may need over a second to start, which
/// would make a 1.2 s start timeout fire before the fake printed anything.
fn shell_fake(mode: &str) -> Option<String> {
    match mode {
        "never" => Some("#!/bin/sh\necho 'loading, never listens'\nexec sleep 120\n".to_string()),
        "login_fail_hang" => Some(
            "#!/bin/sh\ncat >&2 <<'LOG'\n\
2026-10-22 23:22:35 | INFO     | kiro.auth:_do_aws_sso_oidc_refresh:809 - Refreshing Kiro token via AWS SSO OIDC...\n\
2026-10-22 23:22:35 | ERROR    | kiro.auth:_do_aws_sso_oidc_refresh:840 - AWS SSO OIDC refresh failed: status=403, body={\"message\":\"forbidden\"}\n\
2026-10-22 23:22:35 | ERROR    | kiro.account_manager:_initialize_account:578 - Failed to initialize account /x/data.sqlite3: Client error '403 Forbidden' for url 'https://oidc.us-east-1.amazonaws.com/token'\n\
2026-10-22 23:22:35 | ERROR    | main:lifespan:499 - Failed to initialize any account. Check your credentials.\n\
LOG\nexec sleep 120\n"
                .to_string(),
        ),
        _ => None,
    }
}

/// Write an executable fake sidecar with the given mode into `dir`.
pub fn write_fake_sidecar(dir: &Path, mode: &str) -> Sidecar {
    fs::create_dir_all(dir).unwrap();
    let exe = dir.join("kiro-gateway");
    let content = shell_fake(mode).unwrap_or_else(|| FAKE_TEMPLATE.replace("__MODE__", mode));
    // Rewriting a script that is being executed gives half-written files; leave identical content alone.
    if fs::read_to_string(&exe).map(|existing| existing != content).unwrap_or(true) {
        let temp = dir.join(".kiro-gateway.new");
        fs::write(&temp, &content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::rename(&temp, &exe).unwrap();
    }
    Sidecar { exe, tiktoken_cache: None }
}

/// A start request for `sidecar` on `port`, with an environment built by the real
/// `build_env` from a minimal inherited environment (PATH for `env python3`).
pub fn start_request(sidecar: Sidecar, port: u16, extra_env: &[(&str, &str)], dir: &Path) -> StartRequest {
    let inputs = EnvInputs {
        secret: SECRET.to_string(),
        kiro_cli_db: dir.join("kiro-cli.sqlite3"),
        port,
        request_log_dir: dir.join("logs").join("requests"),
        tiktoken_cache_dir: None,
    };
    let mut inherited: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap_or_default()),
        ("ANTHROPIC_BASE_URL".into(), "https://leaked.invalid".into()),
        ("PROXY_URL".into(), "leaked".into()),
    ];
    for (k, v) in extra_env {
        inherited.push((OsString::from(*k), OsString::from(*v)));
    }
    StartRequest {
        sidecar,
        env: build_env(&inputs, inherited),
        port,
        secret: SECRET.to_string(),
        upgrade_required: false,
    }
}

/// A manager over `data_dir` with a recorder attached.
pub fn manager_with_recorder(data_dir: &Path, port: u16) -> (GatewayManager, Recorder) {
    let manager = GatewayManager::new(AppPaths::new(data_dir), system_deps(), fast_settings(), port);
    let recorder = Recorder::default();
    manager.add_listener(recorder.listener());
    (manager, recorder)
}

/// Like [`manager_with_recorder`] with the settings adjusted by `tweak`.
pub fn manager_with_recorder_settings(
    data_dir: &Path,
    port: u16,
    tweak: impl FnOnce(&mut ManagerSettings),
) -> (GatewayManager, Recorder) {
    let mut settings = fast_settings();
    tweak(&mut settings);
    let manager = GatewayManager::new(AppPaths::new(data_dir), system_deps(), settings, port);
    let recorder = Recorder::default();
    manager.add_listener(recorder.listener());
    (manager, recorder)
}

/// Health probe for timeout tests: blocks until the sidecar's log contains `needle` (up to 30 s),
/// then answers like the wrapped probe. The start timeout is only checked after a probe returns,
/// so it cannot fire before the fake has printed what the test wants to see, however slow the
/// machine is (other lanes compile in parallel here and macOS scans fresh executables).
pub struct GateOnLog {
    pub inner: Arc<dyn super::health::HealthProbe>,
    pub log: PathBuf,
    pub needle: &'static str,
}

impl super::health::HealthProbe for GateOnLog {
    fn is_healthy(&self, port: u16) -> bool {
        wait_until(Duration::from_secs(30), || fs::read_to_string(&self.log).map(|t| t.contains(self.needle)).unwrap_or(false));
        self.inner.is_healthy(port)
    }
}

/// Like [`manager_with_recorder_settings`], with the health probe gated on `needle` appearing in the log.
pub fn manager_gated_on_log(
    data_dir: &Path,
    port: u16,
    needle: &'static str,
    tweak: impl FnOnce(&mut ManagerSettings),
) -> (GatewayManager, Recorder) {
    let mut settings = fast_settings();
    tweak(&mut settings);
    let paths = AppPaths::new(data_dir);
    let mut deps = system_deps();
    deps.health = Arc::new(GateOnLog { inner: deps.health.clone(), log: paths.gateway_log(), needle });
    let manager = GatewayManager::new(paths, deps, settings, port);
    let recorder = Recorder::default();
    manager.add_listener(recorder.listener());
    (manager, recorder)
}

/// Where a started fake sidecar dumped its cwd and environment.
///
/// Waits up to five seconds for the file: under a loaded machine the process may still be
/// starting when the caller asks.
pub fn read_dump(path: &PathBuf) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            Some(value) => return value,
            None if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            None => panic!("the fake sidecar never wrote {}", path.display()),
        }
    }
}

/// How many times the fake sidecar was launched (one line per launch in the file).
pub fn launch_count(path: &Path) -> usize {
    fs::read_to_string(path).map(|t| t.lines().count()).unwrap_or(0)
}

/// Process inspector for tests: real signals and liveness, but the image path of chosen
/// pids is overridden (a script's real image is the interpreter, not the script).
pub struct ScriptAwareOps {
    pub images: Mutex<std::collections::HashMap<u32, PathBuf>>,
}

impl ScriptAwareOps {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { images: Mutex::new(Default::default()) })
    }
    pub fn pretend(&self, pid: u32, image: &Path) {
        self.images.lock().unwrap().insert(pid, image.to_path_buf());
    }
}

impl super::sys::ProcessOps for ScriptAwareOps {
    // (delegates to the real implementation; only image_path can be overridden)
    fn is_alive(&self, pid: u32) -> bool {
        super::sys::ProcessOps::is_alive(&SystemProcessOps, pid)
    }
    fn image_path(&self, pid: u32) -> Option<PathBuf> {
        self.images.lock().unwrap().get(&pid).cloned().or_else(|| super::sys::ProcessOps::image_path(&SystemProcessOps, pid))
    }
    fn terminate(&self, pid: u32) {
        super::sys::ProcessOps::terminate(&SystemProcessOps, pid)
    }
    fn kill(&self, pid: u32) {
        super::sys::ProcessOps::kill(&SystemProcessOps, pid)
    }
}

/// Everything a fake process inspector saw, with scripted answers.
pub struct FakeOps {
    pub alive: Mutex<std::collections::HashSet<u32>>,
    pub images: Mutex<std::collections::HashMap<u32, PathBuf>>,
    pub calls: Mutex<Vec<String>>,
    /// Pids that survive SIGTERM (only `kill` ends them).
    pub stubborn: Mutex<std::collections::HashSet<u32>>,
}

impl FakeOps {
    pub fn new() -> Self {
        Self {
            alive: Default::default(),
            images: Default::default(),
            calls: Default::default(),
            stubborn: Default::default(),
        }
    }
    pub fn with_process(self, pid: u32, image: Option<&Path>) -> Self {
        self.alive.lock().unwrap().insert(pid);
        if let Some(image) = image {
            self.images.lock().unwrap().insert(pid, image.to_path_buf());
        }
        self
    }
    pub fn stubborn(self, pid: u32) -> Self {
        self.stubborn.lock().unwrap().insert(pid);
        self
    }
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl super::sys::ProcessOps for FakeOps {
    fn is_alive(&self, pid: u32) -> bool {
        self.alive.lock().unwrap().contains(&pid)
    }
    fn image_path(&self, pid: u32) -> Option<PathBuf> {
        self.images.lock().unwrap().get(&pid).cloned()
    }
    fn terminate(&self, pid: u32) {
        self.calls.lock().unwrap().push(format!("terminate {pid}"));
        if !self.stubborn.lock().unwrap().contains(&pid) {
            self.alive.lock().unwrap().remove(&pid);
        }
    }
    fn kill(&self, pid: u32) {
        self.calls.lock().unwrap().push(format!("kill {pid}"));
        self.alive.lock().unwrap().remove(&pid);
    }
}

/// Minimal one-shot-per-connection HTTP server on a free 81xx port. `respond` gets the raw
/// request head and returns the raw response bytes (or `None` to accept and stay silent).
pub struct TestServer {
    pub port: u16,
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    pub requests: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
    pub fn start(respond: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static) -> Self {
        use std::io::{Read, Write};
        let port = free_port();
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (stop2, requests2) = (stop.clone(), requests.clone());
        let handle = std::thread::spawn(move || {
            let mut silent = Vec::new();
            while !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let mut buf = vec![0u8; 8192];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let head = String::from_utf8_lossy(&buf[..n]).to_string();
                        requests2.lock().unwrap().push(head.clone());
                        match respond(&head) {
                            Some(bytes) => {
                                let _ = stream.write_all(&bytes);
                            }
                            None => silent.push(stream),
                        }
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self { port, stop, handle: Some(handle), requests }
    }

    pub fn json(status: u16, body: &str) -> Vec<u8> {
        let reason = match status {
            200 => "OK",
            401 => "Unauthorized",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "Status",
        };
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
