//! Fakes shared by the update module tests.

use chrono::{DateTime, Duration, FixedOffset, TimeZone};
use serde_json::json;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::Duration as StdDuration;

use super::decide::{Platform, UpdateOffer};
use super::ports::*;
use super::schedule::{Ports, UpdateService};
use super::types::UpdateStatus;

pub struct FakeClock(pub Mutex<DateTime<FixedOffset>>);

impl FakeClock {
    pub fn new() -> Arc<Self> {
        let tz = FixedOffset::east_opt(8 * 3600).unwrap();
        Arc::new(Self(Mutex::new(tz.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap())))
    }
    pub fn advance(&self, d: Duration) {
        let mut g = self.0.lock().unwrap();
        *g += d;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<FixedOffset> {
        *self.0.lock().unwrap()
    }
}

#[derive(Default)]
pub struct FakeBackend {
    pub manifests: Mutex<VecDeque<Result<Option<String>, String>>>,
    pub downloads: Mutex<VecDeque<Result<Vec<u8>, String>>>,
    pub installs: Mutex<VecDeque<Result<(), InstallError>>>,
    pub fetch_count: AtomicUsize,
    pub download_count: AtomicUsize,
    pub installed: Mutex<Vec<(String, Vec<u8>)>>,
    pub progress_steps: Mutex<Vec<(u64, Option<u64>)>>,
    /// Number of upcoming downloads that block without ever reporting progress (a stalled
    /// connection). A hung download ends with an error once [`FakeBackend::release`] is called
    /// (the rig does so on drop) and never consumes a queued result.
    pub hang_downloads: AtomicUsize,
    /// Hung downloads that ended because the caller cancelled them.
    pub cancelled: AtomicUsize,
    /// Set by `release` to let hung calls return.
    pub released: AtomicBool,
    /// While true, `install` blocks.
    pub hang_installs: AtomicBool,
    /// While true, `fetch_manifest` blocks (a first check that never answers).
    pub hang_fetch: AtomicBool,
    /// Pause before each progress step, to simulate a slow but alive connection.
    pub step_delay: Mutex<StdDuration>,
    /// Threads that called `install`.
    pub install_threads: Mutex<Vec<ThreadId>>,
}

fn block_while(flag: &AtomicBool) {
    while flag.load(Ordering::SeqCst) {
        std::thread::sleep(StdDuration::from_millis(5));
    }
}

fn take_one(counter: &AtomicUsize) -> bool {
    // Single consumer per test: load-then-decrement is enough.
    if counter.load(Ordering::SeqCst) == 0 {
        return false;
    }
    counter.fetch_sub(1, Ordering::SeqCst);
    true
}

impl FakeBackend {
    /// Lets every hung call return.
    pub fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.hang_fetch.store(false, Ordering::SeqCst);
        self.hang_installs.store(false, Ordering::SeqCst);
    }
    pub fn push_manifest(&self, m: Result<Option<String>, String>) {
        self.manifests.lock().unwrap().push_back(m);
    }
    pub fn push_download(&self, d: Result<Vec<u8>, String>) {
        self.downloads.lock().unwrap().push_back(d);
    }
    pub fn push_install(&self, r: Result<(), InstallError>) {
        self.installs.lock().unwrap().push_back(r);
    }
    pub fn fetches(&self) -> usize {
        self.fetch_count.load(Ordering::SeqCst)
    }
    pub fn downloads_done(&self) -> usize {
        self.download_count.load(Ordering::SeqCst)
    }
}

impl UpdateBackend for FakeBackend {
    fn fetch_manifest(&self) -> Result<Option<String>, String> {
        self.fetch_count.fetch_add(1, Ordering::SeqCst);
        block_while(&self.hang_fetch);
        self.manifests.lock().unwrap().pop_front().unwrap_or_else(|| Err("offline".into()))
    }

    fn download(
        &self,
        _offer: &UpdateOffer,
        cancel: &AtomicBool,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>, String> {
        self.download_count.fetch_add(1, Ordering::SeqCst);
        if take_one(&self.hang_downloads) {
            while !self.released.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(StdDuration::from_millis(5));
            }
            self.cancelled.fetch_add(usize::from(cancel.load(Ordering::SeqCst)), Ordering::SeqCst);
            return Err("released".into());
        }
        let delay = *self.step_delay.lock().unwrap();
        let steps = self.progress_steps.lock().unwrap().clone();
        for step in steps {
            std::thread::sleep(delay);
            on_progress(step.0, step.1);
        }
        self.downloads.lock().unwrap().pop_front().unwrap_or_else(|| Err("offline".into()))
    }
}

impl PackageInstaller for FakeBackend {
    fn install(&self, version: &str, bytes: &[u8]) -> Result<(), InstallError> {
        self.install_threads.lock().unwrap().push(std::thread::current().id());
        block_while(&self.hang_installs);
        let r = self.installs.lock().unwrap().pop_front().unwrap_or(Ok(()));
        if r.is_ok() {
            self.installed.lock().unwrap().push((version.to_string(), bytes.to_vec()));
        }
        r
    }
}

#[derive(Default)]
pub struct FakeFlag(pub AtomicBool);

impl UpgradeFlag for FakeFlag {
    fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn set(&self, value: bool) -> Result<(), String> {
        self.0.store(value, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeGateway {
    pub stops: AtomicUsize,
    pub resumes: AtomicUsize,
}

impl GatewayControl for FakeGateway {
    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
    fn resume_if_wanted(&self) {
        self.resumes.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub struct FakeApp {
    pub restarts: AtomicUsize,
}

impl AppControl for FakeApp {
    fn restart(&self) {
        self.restarts.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub struct FakeSink {
    pub events: Mutex<Vec<UpdateStatus>>,
}

impl EventSink for FakeSink {
    fn emit(&self, status: &UpdateStatus) {
        self.events.lock().unwrap().push(status.clone());
    }
}

/// Lets hung fake calls return when the rig goes away, so no thread outlives its test.
pub struct ReleaseOnDrop(Arc<FakeBackend>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

pub struct Rig {
    pub svc: Arc<UpdateService>,
    pub clock: Arc<FakeClock>,
    pub backend: Arc<FakeBackend>,
    pub flag: Arc<FakeFlag>,
    pub gateway: Arc<FakeGateway>,
    pub app: Arc<FakeApp>,
    pub sink: Arc<FakeSink>,
    pub dir: tempfile::TempDir,
    _release: ReleaseOnDrop,
}

impl Rig {
    pub fn new(current: &str) -> Self {
        Self::with_dir(current, tempfile::tempdir().unwrap())
    }

    pub fn with_dir(current: &str, dir: tempfile::TempDir) -> Self {
        Self::build(current, dir, None)
    }

    /// A rig whose downloads count as stalled after `stall` without progress.
    pub fn with_stall(current: &str, stall: StdDuration) -> Self {
        Self::build(current, tempfile::tempdir().unwrap(), Some(stall))
    }

    fn build(current: &str, dir: tempfile::TempDir, stall: Option<StdDuration>) -> Self {
        let clock = FakeClock::new();
        let backend = Arc::new(FakeBackend::default());
        let flag = Arc::new(FakeFlag::default());
        let gateway = Arc::new(FakeGateway::default());
        let app = Arc::new(FakeApp::default());
        let sink = Arc::new(FakeSink::default());
        let mut svc = UpdateService::new(
            dir.path().to_path_buf(),
            current.to_string(),
            Platform::DarwinAarch64,
            Ports {
                clock: clock.clone(),
                backend: backend.clone(),
                installer: backend.clone(),
                flag: flag.clone(),
                gateway: gateway.clone(),
                app: app.clone(),
                sink: sink.clone(),
            },
        );
        if let Some(stall) = stall {
            svc = svc.with_stall_timeout(stall);
        }
        let svc = Arc::new(svc);
        let _release = ReleaseOnDrop(backend.clone());
        Rig { svc, clock, backend, flag, gateway, app, sink, dir, _release }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    pub fn events(&self) -> Vec<UpdateStatus> {
        self.sink.events.lock().unwrap().clone()
    }

    pub fn restarts(&self) -> usize {
        self.app.restarts.load(Ordering::SeqCst)
    }

    pub fn stops(&self) -> usize {
        self.gateway.stops.load(Ordering::SeqCst)
    }
}

/// Manifest text for tests; `min` is `minRequiredVersion`.
pub fn manifest(version: &str, min: Option<&str>) -> String {
    let mut m = json!({
        "version": version,
        "notes": "notes",
        "pub_date": "2026-11-01T10:00:00+08:00",
        "platforms": {
            "darwin-aarch64": { "url": format!("https://cos.example/{version}/app.tar.gz"), "signature": "SIG" },
            "windows-x86_64": { "url": format!("https://cos.example/{version}/setup.exe"), "signature": "SIG-W" }
        },
        "manualDownload": {
            "darwin": format!("https://cos.example/{version}/Kiro-Gateway.dmg"),
            "windows": format!("https://cos.example/{version}/setup.exe")
        }
    });
    if let Some(min) = min {
        m["minRequiredVersion"] = json!(min);
    }
    m.to_string()
}
