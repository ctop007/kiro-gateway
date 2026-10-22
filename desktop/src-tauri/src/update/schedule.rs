//! Update scheduler and state machine (tasks 8.3, 8.4).
//!
//! Pure logic with injected collaborators ([`Ports`]). Blocking: the Tauri
//! glue runs [`UpdateService::tick`] and friends on a blocking thread.
//!
//! - Check at start-up, then every [`CHECK_INTERVAL_MINUTES`] minutes. The
//!   caller ticks every [`TICK_INTERVAL_SECS`] seconds and the service compares
//!   wall-clock times, so a machine that slept through several intervals
//!   checks on its first tick after waking.
//! - Failures (offline, bad manifest, failed download) are silent: the state
//!   stays as it was and the next check tries again.
//! - The same version is never downloaded twice while it is `ready` or
//!   `downloading`.

use chrono::{DateTime, Duration, FixedOffset};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration as StdDuration;

use super::decide::{assess, Decision, Platform, UpdateOffer};
use super::pending::{process_pending, StartupOutcome, INSTALL_FAILED_MESSAGE};
use super::ports::{
    AppControl, Clock, EventSink, GatewayControl, PackageInstaller, UpdateBackend, UpgradeFlag,
};
use super::store::{self, LoadedRecord};
use super::types::{
    DownloadProgress, UpdateError, UpdateErrorCode, UpdateState, UpdateStatus,
};

/// Minutes between two manifest checks.
pub const CHECK_INTERVAL_MINUTES: i64 = 60;
/// Seconds between scheduler ticks (the caller's timer period).
pub const TICK_INTERVAL_SECS: u64 = 300;
/// Minimum milliseconds between two progress events.
pub const PROGRESS_EMIT_INTERVAL_MS: i64 = 250;

/// A download that reports no progress for this long has stalled and fails.
pub const DOWNLOAD_STALL_TIMEOUT: StdDuration = StdDuration::from_secs(60);
/// How long the gateway restore waits for the first manifest check at start-up.
pub const FIRST_CHECK_WAIT: StdDuration = StdDuration::from_secs(10);
/// Longest the gateway restore holds back for the start-up install of a pending update.
pub const INSTALL_WAIT: StdDuration = StdDuration::from_secs(120);

/// Shown on the forced-upgrade overlay when the download failed.
pub const DOWNLOAD_FAILED_MESSAGE: &str = "下载失败";

/// Collaborators of the service.
pub struct Ports {
    pub clock: Arc<dyn Clock>,
    pub backend: Arc<dyn UpdateBackend>,
    pub installer: Arc<dyn PackageInstaller>,
    pub flag: Arc<dyn UpgradeFlag>,
    pub gateway: Arc<dyn GatewayControl>,
    pub app: Arc<dyn AppControl>,
    pub sink: Arc<dyn EventSink>,
}

/// Message from a download worker to whoever waits for it.
pub(crate) enum DownloadEvent {
    /// `(downloaded_bytes, total_bytes)` after a chunk.
    Progress(u64, Option<u64>),
    /// The download ended, with the verified package or the failure.
    Finished(Result<Vec<u8>, String>),
}

/// Waits for a download worker, failing it once no event arrives for `stall`.
///
/// Every progress event restarts the clock, so a slow but alive connection is never
/// cut off, while a connection that goes silent (including before the first byte)
/// fails after `stall` instead of blocking its caller forever.
///
/// Args:
///     events: Receiving end of the worker's channel.
///     stall: Longest allowed silence.
///     on_progress: Called for every progress event, on the calling thread.
///
/// Returns:
///     The package bytes, or a failure text (stalled, worker gone, or the worker's own error).
pub(crate) fn await_download(
    events: &Receiver<DownloadEvent>,
    stall: StdDuration,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, String> {
    loop {
        match events.recv_timeout(stall) {
            Ok(DownloadEvent::Progress(downloaded, total)) => on_progress(downloaded, total),
            Ok(DownloadEvent::Finished(result)) => return result,
            Err(RecvTimeoutError::Timeout) => return Err("download stalled".to_string()),
            Err(RecvTimeoutError::Disconnected) => return Err("download worker ended unexpectedly".to_string()),
        }
    }
}

/// How the first manifest check after start-up ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstCheck {
    /// The manifest demands an upgrade: the gateway must stay off.
    Forced,
    /// A valid manifest that demands nothing.
    Clear,
    /// The check failed (offline, bad manifest).
    Failed,
    /// No answer within the wait: carry on as if nothing is demanded.
    TimedOut,
}

/// Start-up bookkeeping the gateway restore waits on.
struct Gate {
    /// An install of a pending update is still running.
    install_pending: bool,
    first_check: Option<FirstCheck>,
}

struct Inner {
    state: UpdateState,
    last_checked_at: Option<String>,
    last_attempt: Option<DateTime<FixedOffset>>,
    last_progress_emit: Option<DateTime<FixedOffset>>,
}

/// The update service: owns the state and drives check / download / install.
pub struct UpdateService {
    dir: PathBuf,
    current: String,
    platform: Platform,
    ports: Ports,
    inner: Mutex<Inner>,
    busy: AtomicBool,
    stall_timeout: StdDuration,
    gate: Mutex<Gate>,
    gate_changed: Condvar,
}

struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl UpdateService {
    /// Creates the service.
    ///
    /// Args:
    ///     dir: App data directory (holds `updates/`, `pending-update.json`).
    ///     current: Running version (`x.y.z`).
    ///     platform: Platform key used to pick the manifest entry.
    ///     ports: Injected collaborators.
    pub fn new(dir: PathBuf, current: String, platform: Platform, ports: Ports) -> Self {
        Self {
            dir,
            current,
            platform,
            ports,
            inner: Mutex::new(Inner {
                state: UpdateState::None,
                last_checked_at: None,
                last_attempt: None,
                last_progress_emit: None,
            }),
            busy: AtomicBool::new(false),
            stall_timeout: DOWNLOAD_STALL_TIMEOUT,
            gate: Mutex::new(Gate { install_pending: false, first_check: None }),
            gate_changed: Condvar::new(),
        }
    }

    /// Overrides the download stall threshold ([`DOWNLOAD_STALL_TIMEOUT`]); tests use a short one.
    #[cfg(test)]
    pub fn with_stall_timeout(mut self, stall: StdDuration) -> Self {
        self.stall_timeout = stall;
        self
    }

    /// Current status as returned by `update_status`.
    pub fn status(&self) -> UpdateStatus {
        let g = self.inner.lock().unwrap();
        self.status_of(&g)
    }

    /// Whether a forced upgrade currently blocks the gateway (state or persisted flag).
    #[cfg(test)]
    pub fn upgrade_required(&self) -> bool {
        matches!(self.inner.lock().unwrap().state, UpdateState::Forced { .. })
            || self.ports.flag.is_set()
    }

    /// Refuses a gateway start while a forced upgrade is pending. Called by
    /// `gateway_start` and by the tray "start gateway" item.
    ///
    /// Raises:
    ///     `upgrade_required` while the state is `forced` or the persisted flag is set.
    pub fn guard_gateway_start(&self) -> Result<(), UpdateError> {
        let state = self.inner.lock().unwrap().state.clone();
        super::forced::guard_gateway_start(&state, self.ports.flag.as_ref())
    }

    fn status_of(&self, g: &Inner) -> UpdateStatus {
        UpdateStatus {
            state: g.state.clone(),
            current_version: self.current.clone(),
            last_checked_at: g.last_checked_at.clone(),
        }
    }

    fn set_state(&self, state: UpdateState) {
        let status = {
            let mut g = self.inner.lock().unwrap();
            if g.state == state {
                return;
            }
            g.state = state;
            self.status_of(&g)
        };
        self.ports.sink.emit(&status);
    }

    fn try_busy(&self) -> Option<BusyGuard<'_>> {
        self.busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| BusyGuard(&self.busy))
    }

    /// Whether a check is due at the injected wall-clock time.
    fn due(&self) -> bool {
        let g = self.inner.lock().unwrap();
        match g.last_attempt {
            None => true,
            Some(last) => {
                let now = self.ports.clock.now();
                now < last || now - last >= Duration::minutes(CHECK_INTERVAL_MINUTES)
            }
        }
    }

    /// Scheduler entry point, called every [`TICK_INTERVAL_SECS`].
    ///
    /// Returns:
    ///     True when a check ran. The first tick after start-up always checks;
    ///     later ones check once 60 wall-clock minutes have passed since the
    ///     previous attempt (so waking from sleep catches up immediately).
    pub fn tick(&self) -> bool {
        self.wait_install_done();
        if !self.due() {
            return false;
        }
        self.check_now();
        true
    }

    /// Checks the manifest now and acts on it. Never fails: problems leave the
    /// state unchanged and are retried at the next check.
    pub fn check_now(&self) -> UpdateStatus {
        if self.gate.lock().unwrap().install_pending {
            // The start-up install owns the state until it ends.
            return self.status();
        }
        let Some(_guard) = self.try_busy() else {
            return self.status();
        };
        let now = self.ports.clock.now();
        self.inner.lock().unwrap().last_attempt = Some(now);
        let decision = match self.ports.backend.fetch_manifest() {
            Err(_) => return self.check_failed(),
            Ok(None) => Decision::None,
            Ok(Some(text)) => match assess(&self.current, &text, self.platform) {
                Err(_) => return self.check_failed(),
                Ok(d) => d,
            },
        };
        self.inner.lock().unwrap().last_checked_at = Some(now.to_rfc3339());
        // Recorded before any download starts: the gateway restore only needs the verdict.
        self.record_first_check(if matches!(decision, Decision::Forced(_)) {
            FirstCheck::Forced
        } else {
            FirstCheck::Clear
        });
        match decision {
            Decision::None => self.on_nothing_newer(),
            Decision::Background(offer) => {
                self.release_lock_if_set();
                self.run_background(offer);
            }
            Decision::Forced(offer) => self.run_forced(offer),
        }
        self.status()
    }

    /// A failed check is silent, except on the forced-upgrade overlay that was
    /// restored from the lock at start-up and has nothing else to show: there
    /// the user needs the "download failed" + retry state.
    fn check_failed(&self) -> UpdateStatus {
        self.record_first_check(FirstCheck::Failed);
        let state = self.inner.lock().unwrap().state.clone();
        if let UpdateState::Forced { version, progress: None, error: None, manual_url } = state {
            self.set_state(Self::forced_state(&version, None, Some(DOWNLOAD_FAILED_MESSAGE), &manual_url));
        }
        self.status()
    }

    /// A valid manifest with nothing newer: keep a downloaded update, drop a
    /// stale forced state (the lock no longer applies).
    fn on_nothing_newer(&self) {
        self.release_lock_if_set();
        let state = self.inner.lock().unwrap().state.clone();
        match state {
            UpdateState::Downloading { .. } | UpdateState::Forced { .. } => {
                self.set_state(UpdateState::None)
            }
            _ => {}
        }
    }

    /// Clears `upgradeRequired` when the manifest no longer demands an upgrade
    /// and the running version satisfies the recorded lock.
    fn release_lock_if_set(&self) {
        let still_below_lock = store::load_lock(&self.dir)
            .and_then(|l| super::decide::parse_version(&l.version))
            .zip(super::decide::parse_version(&self.current))
            .is_some_and(|(lock, cur)| cur < lock);
        let forced_state = matches!(self.inner.lock().unwrap().state, UpdateState::Forced { .. });
        if forced_state || (self.ports.flag.is_set() && !still_below_lock) {
            let _ = self.ports.flag.set(false);
            let _ = store::clear_lock(&self.dir);
            if forced_state {
                self.set_state(UpdateState::None);
            }
        }
    }

    fn already_have(&self, version: &str) -> bool {
        match &self.inner.lock().unwrap().state {
            UpdateState::Ready { version: v } | UpdateState::Downloading { version: v, .. } => {
                v == version
            }
            _ => false,
        }
    }

    fn run_background(&self, offer: UpdateOffer) {
        if self.already_have(&offer.version) {
            return;
        }
        self.set_state(UpdateState::Downloading {
            version: offer.version.clone(),
            progress: DownloadProgress::start(),
        });
        let version = offer.version.clone();
        let progress = |downloaded: u64, total: Option<u64>| {
            self.on_progress(&version, downloaded, total, false);
        };
        match self.download_and_save(&offer, progress) {
            Ok(()) => self.set_state(UpdateState::Ready { version: offer.version }),
            Err(_) => self.set_state(UpdateState::None),
        }
    }

    fn download_and_save(
        &self,
        offer: &UpdateOffer,
        progress: impl FnMut(u64, Option<u64>),
    ) -> Result<(), String> {
        let bytes = self.download_with_stall_guard(offer, progress)?;
        let saved_at = self.ports.clock.now().to_rfc3339();
        store::save(&self.dir, &offer.version, &bytes, &offer.manual_url, &saved_at)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Runs the backend download on a worker thread and waits for it with stall
    /// detection: no progress for `stall_timeout` cancels the worker and fails the
    /// download, so the caller (and with it the busy flag) is never stuck on a dead
    /// connection.
    fn download_with_stall_guard(
        &self,
        offer: &UpdateOffer,
        progress: impl FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let backend = self.ports.backend.clone();
        let offer = offer.clone();
        let worker_cancel = cancel.clone();
        std::thread::Builder::new()
            .name("update-download".into())
            .spawn(move || {
                let progress_tx = tx.clone();
                let result = backend.download(&offer, &worker_cancel, &mut |downloaded, total| {
                    let _ = progress_tx.send(DownloadEvent::Progress(downloaded, total));
                });
                let _ = tx.send(DownloadEvent::Finished(result));
            })
            .map_err(|e| format!("cannot start download worker: {e}"))?;
        let result = await_download(&rx, self.stall_timeout, progress);
        if result.is_err() {
            cancel.store(true, Ordering::SeqCst);
        }
        result
    }

    /// Records progress; emits at most every [`PROGRESS_EMIT_INTERVAL_MS`].
    fn on_progress(&self, version: &str, downloaded: u64, total: Option<u64>, forced: bool) {
        let now = self.ports.clock.now();
        let status = {
            let mut g = self.inner.lock().unwrap();
            let progress = DownloadProgress { downloaded_bytes: downloaded, total_bytes: total };
            match &mut g.state {
                UpdateState::Downloading { version: v, progress: p } if v == version && !forced => {
                    *p = progress;
                }
                UpdateState::Forced { version: v, progress: p, error, .. } if v == version && forced => {
                    *p = Some(progress);
                    *error = None;
                }
                _ => return,
            }
            let finished = total.is_some_and(|t| downloaded >= t);
            let throttled = g.last_progress_emit.is_some_and(|last| {
                now >= last && now - last < Duration::milliseconds(PROGRESS_EMIT_INTERVAL_MS)
            });
            if throttled && !finished {
                return;
            }
            g.last_progress_emit = Some(now);
            self.status_of(&g)
        };
        self.ports.sink.emit(&status);
    }

    fn forced_state(
        version: &str,
        progress: Option<DownloadProgress>,
        error: Option<&str>,
        manual_url: &str,
    ) -> UpdateState {
        UpdateState::Forced {
            version: version.to_string(),
            progress,
            error: error.map(str::to_string),
            manual_url: manual_url.to_string(),
        }
    }

    fn run_forced(&self, offer: UpdateOffer) {
        let was_forced = matches!(self.inner.lock().unwrap().state, UpdateState::Forced { .. });
        let _ = self.ports.flag.set(true);
        let _ = store::save_lock(
            &self.dir,
            &store::LockInfo { version: offer.version.clone(), manual_url: offer.manual_url.clone() },
        );
        if !was_forced {
            self.ports.gateway.stop();
        }
        self.set_state(Self::forced_state(&offer.version, Some(DownloadProgress::start()), None, &offer.manual_url));

        let reusable = match store::load(&self.dir) {
            store::LoadedRecord::Valid(r) if r.version == offer.version => {
                store::read_verified(&self.dir, &r).is_ok()
            }
            _ => false,
        };
        if !reusable {
            let version = offer.version.clone();
            let progress = |downloaded: u64, total: Option<u64>| {
                self.on_progress(&version, downloaded, total, true);
            };
            if self.download_and_save(&offer, progress).is_err() {
                self.set_state(Self::forced_state(&offer.version, None, Some(DOWNLOAD_FAILED_MESSAGE), &offer.manual_url));
                return;
            }
        }
        match process_pending(&self.dir, &self.current, self.ports.installer.as_ref()) {
            StartupOutcome::Installed { .. } => self.ports.app.restart(),
            _ => self.set_state(Self::forced_state(&offer.version, None, Some(INSTALL_FAILED_MESSAGE), &offer.manual_url)),
        }
    }

    /// `update_retry`: re-runs the check, which downloads and installs again.
    ///
    /// Raises:
    ///     `invalid_argument` unless the state is `forced` or `install_failed`.
    pub fn retry(&self) -> Result<UpdateStatus, UpdateError> {
        let retryable = matches!(
            self.inner.lock().unwrap().state,
            UpdateState::Forced { .. } | UpdateState::InstallFailed { .. }
        );
        if !retryable {
            return Err(UpdateError::new(UpdateErrorCode::InvalidArgument, "当前没有需要重试的更新"));
        }
        Ok(self.check_now())
    }

    /// `update_restart_now`: stops the gateway, installs the downloaded update
    /// and restarts the app.
    ///
    /// Raises:
    ///     `no_pending_update` when nothing verified is waiting;
    ///     `internal` when the installer cannot run right now (the gateway is
    ///     resumed and the update stays pending).
    pub fn restart_now(&self) -> Result<(), UpdateError> {
        let ready = matches!(self.inner.lock().unwrap().state, UpdateState::Ready { .. });
        if !ready {
            return Err(UpdateError::new(UpdateErrorCode::NoPendingUpdate, "没有已下载的更新"));
        }
        self.ports.gateway.stop();
        match process_pending(&self.dir, &self.current, self.ports.installer.as_ref()) {
            StartupOutcome::Installed { .. } => {
                self.ports.app.restart();
                Ok(())
            }
            StartupOutcome::InstallFailed { version, error, manual_url } => {
                self.set_state(UpdateState::InstallFailed { version, error, manual_url });
                self.ports.gateway.resume_if_wanted();
                Ok(())
            }
            StartupOutcome::Deferred { .. } => {
                self.ports.gateway.resume_if_wanted();
                Err(UpdateError::new(UpdateErrorCode::Internal, "暂时无法安装更新，请稍后再试"))
            }
            StartupOutcome::NoPending | StartupOutcome::Discarded(_) => {
                self.set_state(UpdateState::None);
                self.ports.gateway.resume_if_wanted();
                Err(UpdateError::new(UpdateErrorCode::NoPendingUpdate, "没有已下载的更新"))
            }
        }
    }

    /// Blocks while the start-up install of a pending update runs.
    fn wait_install_done(&self) {
        let guard = self.gate.lock().unwrap();
        let _ = self.gate_changed.wait_timeout_while(guard, INSTALL_WAIT, |g| g.install_pending).unwrap();
    }

    fn record_first_check(&self, outcome: FirstCheck) {
        let mut g = self.gate.lock().unwrap();
        if g.first_check.is_none() {
            g.first_check = Some(outcome);
            self.gate_changed.notify_all();
        }
    }

    /// Start-up, phase 1: cheap and offline, safe on the setup thread. Looks at the
    /// pending-update record without installing anything.
    ///
    /// Returns:
    ///     True when a downloaded update will be installed by [`Self::complete_startup_install`]:
    ///     the state is then `installing`, and the gateway restore and the manifest checks hold
    ///     back until that call ends. Otherwise the forced-upgrade lock is reconciled at once, so
    ///     a locked version shows its overlay before the first manifest check.
    pub fn begin_startup(&self) -> bool {
        if let LoadedRecord::Valid(record) = store::load(&self.dir) {
            let newer = super::decide::parse_version(&record.version)
                .zip(super::decide::parse_version(&self.current))
                .is_some_and(|(pending, current)| pending > current);
            if newer {
                self.gate.lock().unwrap().install_pending = true;
                self.set_state(UpdateState::Installing { version: record.version });
                return true;
            }
        }
        self.reconcile_lock(None);
        false
    }

    /// Start-up, phase 2: installs the pending update (or discards a bad one), then
    /// reconciles the forced-upgrade lock with the running version. May block (the
    /// installer can need the network for its update handle): run it on a worker thread.
    ///
    /// Returns:
    ///     The pending-update outcome. On `Installed` the app has been asked to restart. A
    ///     forced lock whose version the running binary reached is released
    ///     (`upgradeRequired` cleared); one still above it puts the service into the `forced`
    ///     state.
    pub fn complete_startup_install(&self) -> StartupOutcome {
        let outcome = process_pending(&self.dir, &self.current, self.ports.installer.as_ref());
        let mut install_error = None;
        match &outcome {
            StartupOutcome::Installed { .. } => {
                self.ports.app.restart();
                return outcome;
            }
            StartupOutcome::InstallFailed { version, error, manual_url } => {
                install_error = Some(error.clone());
                self.set_state(UpdateState::InstallFailed {
                    version: version.clone(),
                    error: error.clone(),
                    manual_url: manual_url.clone(),
                });
            }
            StartupOutcome::Deferred { version } => {
                self.set_state(UpdateState::Ready { version: version.clone() });
            }
            StartupOutcome::NoPending | StartupOutcome::Discarded(_) => {
                if matches!(self.inner.lock().unwrap().state, UpdateState::Installing { .. }) {
                    self.set_state(UpdateState::None);
                }
            }
        }
        self.reconcile_lock(install_error.as_deref());
        {
            let mut g = self.gate.lock().unwrap();
            g.install_pending = false;
            self.gate_changed.notify_all();
        }
        outcome
    }

    /// Start-up entry for the setup thread: runs phase 1 here and phase 2 on a worker thread,
    /// so a pending install (which may need the network) never blocks the main thread.
    ///
    /// Returns:
    ///     An error when the worker thread cannot be created; the state then falls back to
    ///     what phase 2 would have left for a missing record.
    pub fn spawn_startup(self: &Arc<Self>) -> std::io::Result<()> {
        self.begin_startup();
        let service = self.clone();
        let spawned = std::thread::Builder::new()
            .name("update-startup".into())
            .spawn(move || {
                service.complete_startup_install();
            });
        if let Err(error) = spawned {
            let mut g = self.gate.lock().unwrap();
            g.install_pending = false;
            self.gate_changed.notify_all();
            drop(g);
            if matches!(self.inner.lock().unwrap().state, UpdateState::Installing { .. }) {
                self.set_state(UpdateState::None);
            }
            return Err(error);
        }
        Ok(())
    }

    /// Both start-up phases in a row (tests).
    #[cfg(test)]
    pub fn startup(&self) -> StartupOutcome {
        self.begin_startup();
        self.complete_startup_install()
    }

    /// Restores the gateway after start-up, once it is known whether an upgrade is mandatory.
    ///
    /// Waits for the start-up install to end, then for the first manifest check to finish
    /// (success, failure) for at most `first_check_wait`. A forced verdict, or a forced
    /// state / persisted flag, keeps the gateway off; anything else, including a check that
    /// never answered, resumes it as before. A check that turns forced later stops the
    /// gateway itself (see `run_forced`).
    ///
    /// Returns:
    ///     How the first check ended, or `TimedOut`.
    pub fn resume_gateway_after_startup(&self, first_check_wait: StdDuration) -> FirstCheck {
        let verdict = {
            self.wait_install_done();
            let guard = self.gate.lock().unwrap();
            let (guard, _) = self
                .gate_changed
                .wait_timeout_while(guard, first_check_wait, |g| g.first_check.is_none())
                .unwrap();
            guard.first_check.unwrap_or(FirstCheck::TimedOut)
        };
        if verdict != FirstCheck::Forced && self.guard_gateway_start().is_ok() {
            self.ports.gateway.resume_if_wanted();
        }
        verdict
    }

    fn reconcile_lock(&self, install_error: Option<&str>) {
        let cur = super::decide::parse_version(&self.current);
        match store::load_lock(&self.dir) {
            Some(lock) => {
                let target = super::decide::parse_version(&lock.version);
                if cur.zip(target).is_some_and(|(c, t)| c >= t) {
                    let _ = self.ports.flag.set(false);
                    let _ = store::clear_lock(&self.dir);
                } else {
                    let _ = self.ports.flag.set(true);
                    self.set_state(Self::forced_state(&lock.version, None, install_error, &lock.manual_url));
                }
            }
            None => {
                // Flag without a lock record: keep blocking until the first
                // successful manifest check decides (see `release_lock_if_set`).
            }
        }
    }
}

#[cfg(test)]
mod tests;
