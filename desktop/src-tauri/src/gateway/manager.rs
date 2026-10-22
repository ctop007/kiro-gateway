//! The gateway state machine and process supervisor (tasks 4.3).
//!
//! ```text
//! stopped --start--> starting --/health ok--> running
//!    ^                  |  \                     |  \
//!    |                  |   exit / 30 s timeout  |   exit / 3 failed heartbeats
//!    |                  v                        v
//!    +---- stop ---- start_failed              crashed
//! ```
//!
//! Threads per run: a *wait* thread owns the child and notices the exit at once, a
//! *supervisor* thread polls `/health` every 500 ms until ready (30 s limit) and then
//! every 10 s as a heartbeat (3 consecutive failures = hung), and two *pump* threads copy
//! stdout / stderr into the rotated `gateway.log`.
//!
//! Every transition happens under one mutex and is guarded by a run *generation*, so a
//! late thread of an older run can never overwrite the state of a newer one. Whoever ends
//! a run on purpose (stop, timeout, hang) first sets the run's `stopping` flag, which keeps
//! the wait thread from reporting the exit as a crash.
//!
//! Listeners are called while the state lock is held, so that events keep the order of the
//! transitions. A listener must not call back into the manager.

use super::classify::{classify, window, CLASSIFY_WINDOW};
use super::health::{fetch_models, HealthProbe, ModelsError};
use super::logfile::{pump, read_run_tail, RotatingLog, LOG_FILES, MAX_LOG_BYTES, RUN_MARKER};
use super::port::{reclaim_stale, remove_pid_file, write_pid_file, PidRecord, PortOps};
use super::redact::redact_tail;
use super::sidecar::{prepare_runtime_dir, Sidecar};
use super::sys::ProcessOps;
use super::types::{FailureKind, GatewayFailure, GatewayState, GatewayStatus, PortOccupant};
use crate::app_config::AppPaths;
use chrono::{DateTime, FixedOffset, SecondsFormat};
use std::ffi::OsString;
use std::io;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

/// How long to wait for a killed process to be reaped.
const KILL_WAIT: Duration = Duration::from_secs(5);
/// Message shown when the contract's forced-upgrade rule rejects a start.
const UPGRADE_MESSAGE: &str = "需要先升级到新版本才能使用";

/// Wall clock, injectable so `since` / `at` are predictable in tests.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<FixedOffset>;
}

/// The real local clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<FixedOffset> {
        chrono::Local::now().fixed_offset()
    }
}

/// Everything the manager does to the outside world.
#[derive(Clone)]
pub struct Deps {
    pub clock: Arc<dyn Clock>,
    pub health: Arc<dyn HealthProbe>,
    pub ops: Arc<dyn ProcessOps>,
    pub ports: Arc<dyn PortOps>,
}

/// Timings and sizes; the production values are in [`ManagerSettings::default`].
#[derive(Debug, Clone)]
pub struct ManagerSettings {
    /// `/health` must pass within this time after launch (30 s).
    pub start_timeout: Duration,
    /// Probe interval while starting (500 ms).
    pub poll_interval: Duration,
    /// Probe interval while running (10 s).
    pub heartbeat_interval: Duration,
    /// Consecutive failed heartbeats that declare the gateway hung (3).
    pub heartbeat_failures: u32,
    /// SIGTERM to SIGKILL grace period (5 s).
    pub term_grace: Duration,
    /// Lines in `GatewayFailure.logExcerpt` (20).
    pub excerpt_lines: usize,
    /// How long to wait for the output pumps after the process exited.
    pub pump_join_timeout: Duration,
}

impl Default for ManagerSettings {
    fn default() -> Self {
        Self {
            start_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_millis(500),
            heartbeat_interval: Duration::from_secs(10),
            heartbeat_failures: 3,
            term_grace: Duration::from_secs(5),
            excerpt_lines: 20,
            pump_join_timeout: Duration::from_secs(2),
        }
    }
}

/// What `start` needs, prepared by the caller (`commands.rs`).
pub struct StartRequest {
    pub sidecar: Sidecar,
    /// Complete environment of the sidecar, from `env::build_env`.
    pub env: Vec<(OsString, OsString)>,
    pub port: u16,
    /// The access key, used only to redact the log excerpt.
    pub secret: String,
    /// The forced-upgrade flag; `true` rejects the start.
    pub upgrade_required: bool,
}

/// A start request that was rejected outright (not a failed start, which is a state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    UpgradeRequired,
    /// Reserved for failures that must reject the request itself. Spawn problems are
    /// reported as a `start_failed` state instead, so nothing builds this today.
    #[allow(dead_code)]
    Internal(String),
}

impl StartError {
    /// The `code` of the IPC error shape.
    #[cfg(test)]
    pub fn code(&self) -> &'static str {
        match self {
            StartError::UpgradeRequired => "upgrade_required",
            StartError::Internal(_) => "internal",
        }
    }

    /// One Chinese sentence, safe to show.
    pub fn message(&self) -> String {
        match self {
            StartError::UpgradeRequired => UPGRADE_MESSAGE.to_string(),
            StartError::Internal(m) => m.clone(),
        }
    }
}

type Listener = Box<dyn Fn(&GatewayStatus) + Send + Sync>;

// ---------------------------------------------------------------- run state

/// How a process ended.
#[derive(Debug, Clone, Copy, Default)]
struct ExitInfo {
    code: Option<i32>,
    signal: Option<i32>,
}

impl ExitInfo {
    fn from_status(status: io::Result<ExitStatus>) -> Self {
        let Ok(status) = status else { return Self::default() };
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            status.signal()
        };
        #[cfg(not(unix))]
        let signal = None;
        Self { code: status.code(), signal }
    }

    fn describe(&self) -> String {
        match (self.code, self.signal) {
            (Some(code), _) => format!("退出码 {code}"),
            (None, Some(signal)) => format!("被信号 {signal} 结束"),
            _ => "原因未知".to_string(),
        }
    }
}

/// Set once, after the process was reaped and its output drained.
struct ExitSignal {
    info: Mutex<Option<ExitInfo>>,
    changed: Condvar,
}

impl ExitSignal {
    fn new() -> Self {
        Self { info: Mutex::new(None), changed: Condvar::new() }
    }

    fn set(&self, info: ExitInfo) {
        *self.info.lock().unwrap_or_else(|p| p.into_inner()) = Some(info);
        self.changed.notify_all();
    }

    fn get(&self) -> Option<ExitInfo> {
        *self.info.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn is_set(&self) -> bool {
        self.get().is_some()
    }

    /// Wait up to `limit`; true when the process has exited.
    fn wait_timeout(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut guard = self.info.lock().unwrap_or_else(|p| p.into_inner());
        while guard.is_none() {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            guard = self.changed.wait_timeout(guard, deadline - now).unwrap_or_else(|p| p.into_inner()).0;
        }
        true
    }
}

/// One launched process.
struct Run {
    generation: u64,
    pid: u32,
    port: u16,
    secret: String,
    exit: ExitSignal,
    /// Set by whoever ends the run on purpose, before signalling the process.
    stopping: AtomicBool,
}

struct Inner {
    status: GatewayStatus,
    configured_port: u16,
    run: Option<Arc<Run>>,
    last_emitted: Option<GatewayStatus>,
    next_generation: u64,
}

struct Shared {
    paths: AppPaths,
    deps: Deps,
    settings: ManagerSettings,
    inner: Mutex<Inner>,
    /// Serialises `start`, `stop` and `report_start_problem`.
    op_lock: Mutex<()>,
    listeners: Mutex<Vec<Listener>>,
}

/// Owner of at most one sidecar process and of the status the UI shows.
pub struct GatewayManager {
    shared: Arc<Shared>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means a holder panicked between complete transitions.
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

impl Shared {
    fn timestamp(&self) -> String {
        self.deps.clock.now().to_rfc3339_opts(SecondsFormat::Secs, false)
    }

    /// Install `status` and tell the listeners, unless it equals what was last announced.
    fn publish(&self, inner: &mut Inner, status: GatewayStatus) {
        inner.status = status.clone();
        if inner.last_emitted.as_ref() == Some(&status) {
            return;
        }
        inner.last_emitted = Some(status.clone());
        for listener in lock(&self.listeners).iter() {
            listener(&status);
        }
    }

    fn stopped_status(&self, inner: &Inner) -> GatewayStatus {
        GatewayStatus::stopped(inner.configured_port)
    }

    fn failed_status(&self, state: GatewayState, port: u16, failure: GatewayFailure) -> GatewayStatus {
        GatewayStatus { state, port, pid: None, since: Some(failure.at.clone()), failure: Some(failure) }
    }

    fn is_current(&self, inner: &Inner, run: &Run) -> bool {
        inner.run.as_ref().is_some_and(|r| r.generation == run.generation)
    }

    /// Move the current run to a failed state. No-op if `run` is no longer the current run
    /// or its state is not `from`.
    fn finish_run(&self, run: &Run, from: GatewayState, state: GatewayState, failure: GatewayFailure) {
        let mut inner = lock(&self.inner);
        if !self.is_current(&inner, run) || inner.status.state != from {
            return;
        }
        inner.run = None;
        remove_pid_file(&self.paths.pid_file());
        let status = self.failed_status(state, run.port, failure);
        self.publish(&mut inner, status);
    }

    /// The failure record for `run`, reading the log of this run.
    ///
    /// `default` is used when the log has nothing recognisable. `allow_port` lets a bind
    /// error in the log decide (start phase only).
    fn failure_for(&self, run: &Run, allow_port: bool, default: (FailureKind, String)) -> GatewayFailure {
        let lines = read_run_tail(&self.paths.gateway_log(), CLASSIFY_WINDOW);
        let excerpt = redact_tail(&lines, self.settings.excerpt_lines, &run.secret);
        let (kind, message, occupant) = match classify(window(&lines, CLASSIFY_WINDOW)) {
            Some(c) if c.kind == FailureKind::LoginExpired => (c.kind, c.message, None),
            Some(c) if c.kind == FailureKind::PortInUse && allow_port => {
                let port = c.port.unwrap_or(run.port);
                (c.kind, format!("{port} 端口被别的程序占用"), Some(self.deps.ports.occupant(port)))
            }
            _ => (default.0, default.1, None),
        };
        GatewayFailure { kind, message, occupant, log_excerpt: excerpt, at: self.timestamp() }
    }

    /// Ask the process to exit; kill it after the grace period. Returns once it is gone
    /// (or `KILL_WAIT` after the kill).
    fn end_process(&self, run: &Run, polite: bool) {
        if run.exit.is_set() {
            return;
        }
        if polite {
            self.deps.ops.terminate(run.pid);
            if run.exit.wait_timeout(self.settings.term_grace) {
                return;
            }
        }
        if !run.exit.is_set() {
            self.deps.ops.kill(run.pid);
        }
        run.exit.wait_timeout(KILL_WAIT);
    }
}

fn spawn_failure_message(error: &io::Error) -> String {
    let reason = match error.kind() {
        io::ErrorKind::NotFound => "找不到网关程序",
        io::ErrorKind::PermissionDenied => "没有执行权限",
        _ => "系统拒绝启动",
    };
    format!("没能启动网关进程：{reason}")
}

// ----------------------------------------------------------------- threads

/// Owns the child: waits for it, drains the output, then announces the exit.
fn wait_thread(shared: Arc<Shared>, run: Arc<Run>, mut child: Child, pumps: Vec<mpsc::Receiver<()>>) {
    let info = ExitInfo::from_status(child.wait());
    // Let the pumps copy what the process wrote last; they end when the pipes close.
    let deadline = Instant::now() + shared.settings.pump_join_timeout;
    for done in pumps {
        let _ = done.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    }
    run.exit.set(info);

    if run.stopping.load(Ordering::SeqCst) {
        return; // a deliberate end: whoever set the flag does the transition
    }
    let state = lock(&shared.inner).status.state;
    match state {
        GatewayState::Starting => {
            let default = (FailureKind::Other, format!("网关进程在就绪前退出了（{}）", info.describe()));
            let failure = shared.failure_for(&run, true, default);
            shared.finish_run(&run, GatewayState::Starting, GatewayState::StartFailed, failure);
        }
        GatewayState::Running => {
            let default = (FailureKind::Other, format!("网关进程意外退出（{}）", info.describe()));
            let failure = shared.failure_for(&run, false, default);
            shared.finish_run(&run, GatewayState::Running, GatewayState::Crashed, failure);
        }
        _ => {}
    }
}

/// Move `Starting` to `Running`; false when the run is over or no longer starting.
fn mark_running(shared: &Shared, run: &Run) -> bool {
    let mut inner = lock(&shared.inner);
    if !shared.is_current(&inner, run) || inner.status.state != GatewayState::Starting {
        return false;
    }
    let mut status = inner.status.clone();
    status.state = GatewayState::Running;
    status.since = Some(shared.timestamp());
    shared.publish(&mut inner, status);
    true
}

/// Polls `/health` until ready, then keeps probing as a heartbeat.
fn supervisor_thread(shared: Arc<Shared>, run: Arc<Run>) {
    let settings = shared.settings.clone();
    let began = Instant::now();

    loop {
        if run.exit.is_set() || run.stopping.load(Ordering::SeqCst) {
            return;
        }
        if shared.deps.health.is_healthy(run.port) {
            if !mark_running(&shared, &run) {
                return;
            }
            break;
        }
        if began.elapsed() >= settings.start_timeout {
            start_timed_out(&shared, &run);
            return;
        }
        if run.exit.wait_timeout(settings.poll_interval) {
            return;
        }
    }

    let mut failures = 0u32;
    loop {
        if run.exit.wait_timeout(settings.heartbeat_interval) || run.stopping.load(Ordering::SeqCst) {
            return;
        }
        if shared.deps.health.is_healthy(run.port) {
            failures = 0;
            continue;
        }
        failures += 1;
        if failures >= settings.heartbeat_failures {
            gateway_hung(&shared, &run);
            return;
        }
    }
}

fn start_timed_out(shared: &Shared, run: &Run) {
    if run.stopping.swap(true, Ordering::SeqCst) {
        return;
    }
    shared.end_process(run, true);
    let secs = shared.settings.start_timeout.as_secs();
    let default = (FailureKind::Timeout, format!("{secs} 秒内网关没有就绪，已结束这次启动"));
    let failure = shared.failure_for(run, true, default);
    shared.finish_run(run, GatewayState::Starting, GatewayState::StartFailed, failure);
}

fn gateway_hung(shared: &Shared, run: &Run) {
    if run.stopping.swap(true, Ordering::SeqCst) {
        return;
    }
    // A hung process may not react to SIGTERM (a stopped one never does): end it at once.
    shared.end_process(run, false);
    let default = (FailureKind::Other, "网关进程没有响应，已结束".to_string());
    let failure = shared.failure_for(run, false, default);
    shared.finish_run(run, GatewayState::Running, GatewayState::Crashed, failure);
}

// --------------------------------------------------------------- public API

impl GatewayManager {
    /// Args:
    ///     paths: App data layout (log, pid file, runtime directory).
    ///     deps: Clock, health probe, process and port adapters.
    ///     settings: Timings.
    ///     port: The configured port, shown while stopped.
    pub fn new(paths: AppPaths, deps: Deps, settings: ManagerSettings, port: u16) -> Self {
        let inner = Inner {
            status: GatewayStatus::stopped(port),
            configured_port: port,
            run: None,
            last_emitted: None,
            next_generation: 1,
        };
        Self {
            shared: Arc::new(Shared {
                paths,
                deps,
                settings,
                inner: Mutex::new(inner),
                op_lock: Mutex::new(()),
                listeners: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Register a status listener (called on every distinct status, in order, with the
    /// state lock held: do not call back into the manager from it).
    pub fn add_listener(&self, listener: Listener) {
        lock(&self.shared.listeners).push(listener);
    }

    /// The current status.
    pub fn status(&self) -> GatewayStatus {
        lock(&self.shared.inner).status.clone()
    }

    /// Tell the manager which port is configured. Shown while stopped; a live run keeps its port.
    pub fn set_port(&self, port: u16) {
        let mut inner = lock(&self.shared.inner);
        inner.configured_port = port;
        if inner.status.state == GatewayState::Stopped && inner.status.port != port {
            let status = GatewayStatus::stopped(port);
            self.shared.publish(&mut inner, status);
        }
    }

    /// Show a start failure that was found before a process could even be prepared
    /// (for example the sidecar executable is missing).
    pub fn report_start_problem(&self, port: u16, message: String) {
        let _op = lock(&self.shared.op_lock);
        if self.status().is_active() {
            return;
        }
        let failure = GatewayFailure {
            kind: FailureKind::Other,
            message,
            occupant: None,
            log_excerpt: Vec::new(),
            at: self.shared.timestamp(),
        };
        self.fail_before_launch(port, failure);
    }

    fn fail_before_launch(&self, port: u16, failure: GatewayFailure) -> GatewayStatus {
        let mut inner = lock(&self.shared.inner);
        let status = self.shared.failed_status(GatewayState::StartFailed, port, failure);
        self.shared.publish(&mut inner, status.clone());
        status
    }

    /// Start the gateway.
    ///
    /// Idempotent: while `starting` or `running` the current status comes back and no second
    /// process is launched. A leftover sidecar of this App from an earlier run is ended first;
    /// any other program on the port is reported, never touched.
    ///
    /// Returns:
    ///     The status after the launch (`starting`), or `start_failed` for problems that
    ///     are states (port taken, cannot spawn); `Err` only when the request itself is
    ///     rejected (forced upgrade).
    pub fn start(&self, request: StartRequest) -> Result<GatewayStatus, StartError> {
        if request.upgrade_required {
            return Err(StartError::UpgradeRequired);
        }
        let shared = &self.shared;
        let _op = lock(&shared.op_lock);
        if self.status().is_active() {
            return Ok(self.status());
        }

        reclaim_stale(
            &shared.paths.pid_file(),
            &request.sidecar.exe,
            request.port,
            shared.deps.ops.as_ref(),
            shared.deps.ports.as_ref(),
            shared.settings.term_grace,
        );

        if !shared.deps.ports.is_free(request.port) {
            let occupant: PortOccupant = shared.deps.ports.occupant(request.port);
            let failure = GatewayFailure {
                kind: FailureKind::PortInUse,
                message: format!("{} 端口被别的程序占用", request.port),
                occupant: Some(occupant),
                log_excerpt: Vec::new(),
                at: shared.timestamp(),
            };
            return Ok(self.fail_before_launch(request.port, failure));
        }

        let launch = match self.launch(&request) {
            Ok(launch) => launch,
            Err(message) => {
                let failure = GatewayFailure {
                    kind: FailureKind::Other,
                    message,
                    occupant: None,
                    log_excerpt: Vec::new(),
                    at: shared.timestamp(),
                };
                return Ok(self.fail_before_launch(request.port, failure));
            }
        };
        let Launched { child, pumps, pid } = launch;

        let run = {
            let mut inner = lock(&shared.inner);
            let generation = inner.next_generation;
            inner.next_generation += 1;
            let run = Arc::new(Run {
                generation,
                pid,
                port: request.port,
                secret: request.secret.clone(),
                exit: ExitSignal::new(),
                stopping: AtomicBool::new(false),
            });
            inner.run = Some(run.clone());
            let status = GatewayStatus {
                state: GatewayState::Starting,
                port: request.port,
                pid: Some(pid),
                since: Some(shared.timestamp()),
                failure: None,
            };
            shared.publish(&mut inner, status);
            run
        };

        let (s1, r1) = (shared.clone(), run.clone());
        thread::spawn(move || wait_thread(s1, r1, child, pumps));
        let (s2, r2) = (shared.clone(), run);
        thread::spawn(move || supervisor_thread(s2, r2));

        Ok(self.status())
    }

    /// Spawn the process and its output pumps; the pid file is written before returning.
    fn launch(&self, request: &StartRequest) -> Result<Launched, String> {
        let shared = &self.shared;
        let runtime = prepare_runtime_dir(&shared.paths.runtime_dir())
            .map_err(|_| "没能准备网关的工作目录，请检查磁盘空间和权限".to_string())?;

        // Output goes to gateway.log; if the log cannot be opened the gateway still starts.
        let log = RotatingLog::open(&shared.paths.gateway_log(), MAX_LOG_BYTES, LOG_FILES).ok().map(|mut log| {
            let _ = log.write_line(RUN_MARKER.as_bytes());
            Arc::new(Mutex::new(log))
        });
        let stdio = || if log.is_some() { Stdio::piped() } else { Stdio::null() };

        let mut command = Command::new(&request.sidecar.exe);
        command
            .env_clear()
            .envs(request.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&runtime)
            .stdin(Stdio::null())
            .stdout(stdio())
            .stderr(stdio());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Own process group: a kill reaches everything the gateway started.
            command.process_group(0);
            // The App blocks the termination signals; the sidecar must still receive SIGTERM.
            super::sys::restore_default_signals(&mut command);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let mut child = command.spawn().map_err(|e| spawn_failure_message(&e))?;
        #[cfg(windows)]
        super::sys::bind_to_job(&child);

        let pid = child.id();
        let _ = write_pid_file(&shared.paths.pid_file(), &PidRecord { pid, exe: request.sidecar.exe.clone() });

        let mut pumps = Vec::new();
        if let Some(log) = &log {
            if let Some(stdout) = child.stdout.take() {
                pumps.push(spawn_pump(stdout, log.clone()));
            }
            if let Some(stderr) = child.stderr.take() {
                pumps.push(spawn_pump(stderr, log.clone()));
            }
        }
        Ok(Launched { child, pumps, pid })
    }

    /// Stop the gateway and return to `stopped` (also clears a failure).
    ///
    /// SIGTERM first, SIGKILL after the grace period. Safe to call in any state and from
    /// several threads; returns after the process is gone.
    pub fn stop(&self) -> GatewayStatus {
        let shared = &self.shared;
        let _op = lock(&shared.op_lock);
        let run = lock(&shared.inner).run.clone();
        if let Some(run) = &run {
            run.stopping.store(true, Ordering::SeqCst);
            shared.end_process(run, true);
        }
        let mut inner = lock(&shared.inner);
        if run.as_ref().is_some_and(|r| shared.is_current(&inner, r)) {
            inner.run = None;
            remove_pid_file(&shared.paths.pid_file());
        }
        let status = shared.stopped_status(&inner);
        if inner.status != status {
            shared.publish(&mut inner, status);
        }
        inner.status.clone()
    }

    /// Ask the running gateway for its model ids (`gateway_models`).
    pub fn models(&self, secret: &str, timeout: Duration) -> Result<Vec<String>, ModelsError> {
        let status = self.status();
        if status.state != GatewayState::Running {
            return Err(ModelsError::NotRunning);
        }
        fetch_models(status.port, secret, timeout)
    }
}

impl Drop for GatewayManager {
    fn drop(&mut self) {
        // Never leave a sidecar behind when the manager goes away.
        self.stop();
    }
}

struct Launched {
    child: Child,
    pumps: Vec<mpsc::Receiver<()>>,
    pid: u32,
}

fn spawn_pump<R: io::Read + Send + 'static>(reader: R, log: Arc<Mutex<RotatingLog>>) -> mpsc::Receiver<()> {
    let (done, receiver) = mpsc::channel();
    thread::spawn(move || {
        pump(reader, &log);
        let _ = done.send(());
    });
    receiver
}

// The tests drive shell and python fake sidecars and send POSIX signals.
#[cfg(all(test, unix))]
mod tests;
