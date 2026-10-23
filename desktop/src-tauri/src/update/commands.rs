//! Tauri glue: the real updater backend, the four
//! `update_*` commands, the `update-status` event and the scheduler thread.
//!
//! All decisions live in [`super::schedule::UpdateService`]; this file only
//! adapts `tauri-plugin-updater` 2.x:
//!
//! - `Updater::check()` reads the manifest. Its `Update::raw_json` carries the
//!   whole manifest (including `minRequiredVersion`), which is what
//!   [`super::decide::assess`] works on. `check()` returns `None` when the
//!   manifest is not newer than the running version.
//! - `Update::download(on_chunk, on_finish)` returns the package as
//!   `Vec<u8>` after verifying its minisign signature against
//!   `plugins.updater.pubkey`.
//! - `Update::install(bytes)` installs those bytes. It only needs the update
//!   handle for the target and the path of the running app, so any handle
//!   works; for the start-up install a handle is obtained with a comparator
//!   that accepts every version.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use super::decide::{Platform, UpdateOffer};
use super::ports::{
    AppControl, EventSink, GatewayControl, InstallError, PackageInstaller, SystemClock,
    UpdateBackend, UpgradeFlag,
};
use super::schedule::{Ports, UpdateService, TICK_INTERVAL_SECS};
use super::types::{UpdateError, UpdateErrorCode, UpdateStatus};

/// Event name of the contract (`update-status`).
pub const UPDATE_STATUS_EVENT: &str = "update-status";

const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound for one whole package download (see `TauriBackend::download`).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How often a running download checks whether its caller gave up.
const CANCEL_POLL: Duration = Duration::from_millis(250);

/// Debug builds only: replaces the manifest URL (end-to-end tests against a local server).
pub const ENDPOINT_OVERRIDE_VAR: &str = "KIRO_GATEWAY_UPDATE_URL";

/// Decides whether a manifest URL override is honoured.
///
/// Only debug builds read it, and only for loopback hosts: a release build can never be
/// pointed at another server through the environment, and a debug build cannot be tricked
/// into fetching packages from a remote one.
///
/// Args:
///     value: Value of [`ENDPOINT_OVERRIDE_VAR`], if set.
///     debug_build: Whether this is a debug build (`cfg!(debug_assertions)`).
///
/// Returns:
///     The URL to use instead of the configured endpoint, or `None`.
pub fn endpoint_override(value: Option<&str>, debug_build: bool) -> Option<tauri::Url> {
    if !debug_build {
        return None;
    }
    let url = tauri::Url::parse(value?.trim()).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let loopback = match url.host()? {
        url::Host::Domain(name) => name == "localhost",
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
    };
    (loopback && matches!(url.scheme(), "http" | "https")).then_some(url)
}

/// Backend over `tauri-plugin-updater`.
pub struct TauriBackend<R: Runtime> {
    app: AppHandle<R>,
    handle: Mutex<Option<Update>>,
}

impl<R: Runtime> TauriBackend<R> {
    /// Creates a backend for `app`.
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app, handle: Mutex::new(None) }
    }

    fn check_with(&self, accept_any_version: bool) -> Result<Option<Update>, String> {
        let mut builder = self.app.updater_builder().timeout(NETWORK_TIMEOUT);
        let override_value = std::env::var(ENDPOINT_OVERRIDE_VAR).ok();
        if let Some(url) = endpoint_override(override_value.as_deref(), cfg!(debug_assertions)) {
            builder = builder.endpoints(vec![url]).map_err(|e| e.to_string())?;
        }
        if accept_any_version {
            builder = builder.version_comparator(|_, _| true);
        }
        let updater = builder.build().map_err(|e| e.to_string())?;
        tauri::async_runtime::block_on(updater.check()).map_err(|e| e.to_string())
    }
}

impl<R: Runtime> UpdateBackend for TauriBackend<R> {
    fn fetch_manifest(&self) -> Result<Option<String>, String> {
        match self.check_with(false)? {
            None => {
                *self.handle.lock().unwrap() = None;
                Ok(None)
            }
            Some(update) => {
                let text = update.raw_json.to_string();
                *self.handle.lock().unwrap() = Some(update);
                Ok(Some(text))
            }
        }
    }

    fn download(
        &self,
        offer: &UpdateOffer,
        cancel: &AtomicBool,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>, String> {
        let mut update = self
            .handle
            .lock()
            .unwrap()
            .clone()
            .filter(|u| u.version == offer.version)
            .ok_or_else(|| "no update handle for this version".to_string())?;
        // `check()` leaves `Update::timeout` unset, so the builder's 30 s only covered the
        // manifest request. This is a backstop for the whole transfer; a connection that
        // merely goes quiet is caught much sooner by the caller's stall detection.
        update.timeout = Some(DOWNLOAD_TIMEOUT);

        enum Msg {
            Chunk(u64, Option<u64>),
            Done(Result<Vec<u8>, String>),
        }
        let (tx, rx) = mpsc::channel();
        let chunk_tx = tx.clone();
        let task = tauri::async_runtime::spawn(async move {
            let mut downloaded: u64 = 0;
            let result = update
                .download(
                    |chunk, total| {
                        downloaded += chunk as u64;
                        let _ = chunk_tx.send(Msg::Chunk(downloaded, total));
                    },
                    || {},
                )
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Done(result));
        });
        loop {
            match rx.recv_timeout(CANCEL_POLL) {
                Ok(Msg::Chunk(downloaded, total)) => on_progress(downloaded, total),
                Ok(Msg::Done(result)) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if cancel.load(Ordering::SeqCst) {
                        // Dropping the future closes the socket.
                        task.abort();
                        return Err("download cancelled".to_string());
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("download task ended unexpectedly".to_string())
                }
            }
        }
    }
}

impl<R: Runtime> PackageInstaller for TauriBackend<R> {
    fn install(&self, version: &str, bytes: &[u8]) -> Result<(), InstallError> {
        let held = self.handle.lock().unwrap().clone();
        let update = match held {
            Some(u) => u,
            None => self
                .check_with(true)
                .map_err(InstallError::Unavailable)?
                .ok_or_else(|| InstallError::Unavailable("no update handle".into()))?,
        };
        eprintln!("[update] installing {version}");
        update.install(bytes).map_err(|e| InstallError::Failed(e.to_string()))
    }
}

struct TauriEvents<R: Runtime>(AppHandle<R>);

impl<R: Runtime> EventSink for TauriEvents<R> {
    fn emit(&self, status: &UpdateStatus) {
        // One line per transition (no URLs, no package data): the only trace of the update
        // flow when the window is hidden.
        let state = serde_json::to_value(status).ok().and_then(|v| v.get("state").and_then(|s| s.as_str().map(str::to_string)));
        eprintln!("[update] state: {}", state.unwrap_or_default());
        let _ = self.0.emit(UPDATE_STATUS_EVENT, status);
    }
}

struct TauriApp<R: Runtime>(AppHandle<R>);

impl<R: Runtime> AppControl for TauriApp<R> {
    fn restart(&self) {
        self.0.restart();
    }
}

/// Registers the update service as managed state and starts installing a pending update on a
/// worker thread (may restart the app). Does not start the scheduler: call [`spawn_scheduler`]
/// once the rest of start-up has run.
///
/// Args:
///     app: The app handle.
///     gateway: Gateway control provided by the gateway module.
///     flag: `upgradeRequired` accessor provided by `app_config`.
///
/// Returns:
///     The shared service (also available as `State<Arc<UpdateService>>`).
pub fn setup<R: Runtime>(
    app: &AppHandle<R>,
    gateway: Arc<dyn GatewayControl>,
    flag: Arc<dyn UpgradeFlag>,
) -> Result<Arc<UpdateService>, UpdateError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| UpdateError::new(UpdateErrorCode::Internal, "找不到应用数据目录"))?;
    let platform = Platform::current()
        .ok_or_else(|| UpdateError::new(UpdateErrorCode::Internal, "当前平台不支持应用内升级"))?;
    let backend = Arc::new(TauriBackend::new(app.clone()));
    let service = Arc::new(UpdateService::new(
        dir,
        app.package_info().version.to_string(),
        platform,
        Ports {
            clock: Arc::new(SystemClock),
            backend: backend.clone(),
            installer: backend,
            flag,
            gateway,
            app: Arc::new(TauriApp(app.clone())),
            sink: Arc::new(TauriEvents(app.clone())),
        },
    ));
    app.manage(service.clone());
    // Reads the pending record only; the install itself (which may touch the network)
    // runs on a worker thread, see `UpdateService::spawn_startup`.
    service
        .spawn_startup()
        .map_err(|_| UpdateError::new(UpdateErrorCode::Internal, "无法启动更新检查"))?;
    Ok(service)
}

/// Starts the scheduler thread: one tick every [`TICK_INTERVAL_SECS`], the first one at once.
///
/// Returns:
///     An error when the thread cannot be created.
pub fn spawn_scheduler(service: Arc<UpdateService>) -> Result<(), UpdateError> {
    std::thread::Builder::new()
        .name("update-scheduler".into())
        .spawn(move || loop {
            service.tick();
            std::thread::sleep(Duration::from_secs(TICK_INTERVAL_SECS));
        })
        .map(|_| ())
        .map_err(|_| UpdateError::new(UpdateErrorCode::Internal, "无法启动更新检查"))
}

/// `update_status`
#[tauri::command]
pub fn update_status(service: State<'_, Arc<UpdateService>>) -> UpdateStatus {
    service.status()
}

/// `update_check_now`: always resolves; failures show up as `none`.
#[tauri::command]
pub async fn update_check_now(service: State<'_, Arc<UpdateService>>) -> Result<UpdateStatus, UpdateError> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.check_now())
        .await
        .map_err(|_| UpdateError::new(UpdateErrorCode::Internal, "检查更新时出错"))
}

/// `update_restart_now`
#[tauri::command]
pub async fn update_restart_now(service: State<'_, Arc<UpdateService>>) -> Result<(), UpdateError> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.restart_now())
        .await
        .map_err(|_| UpdateError::new(UpdateErrorCode::Internal, "重启时出错"))?
}

/// `update_retry`
#[tauri::command]
pub async fn update_retry(service: State<'_, Arc<UpdateService>>) -> Result<UpdateStatus, UpdateError> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.retry())
        .await
        .map_err(|_| UpdateError::new(UpdateErrorCode::Internal, "重试时出错"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_builds_ignore_the_override_whatever_it_says() {
        for value in ["http://127.0.0.1:8123/latest.json", "http://localhost:1/x", "https://evil.example/x"] {
            assert_eq!(endpoint_override(Some(value), false), None, "{value}");
        }
    }

    #[test]
    fn debug_builds_accept_loopback_hosts() {
        for value in [
            "http://127.0.0.1:8123/latest.json",
            "http://localhost:8123/latest.json",
            "https://127.0.0.1:8123/latest.json",
            "http://[::1]:8123/latest.json",
            "  http://127.0.0.1:8123/latest.json \n",
        ] {
            assert!(endpoint_override(Some(value), true).is_some(), "{value}");
        }
    }

    #[test]
    fn debug_builds_refuse_remote_hosts_and_junk() {
        for value in [
            "https://github.com/heyuan110/kiro-gateway/releases/latest/download/latest.json",
            "http://192.168.1.5:8123/latest.json",
            "http://0.0.0.0:8123/latest.json",
            "http://127.0.0.1.evil.example/latest.json",
            "http://localhost.evil.example/latest.json",
            "http://user@evil.example@127.0.0.1/latest.json",
            "file:///tmp/latest.json",
            "ftp://127.0.0.1/latest.json",
            "127.0.0.1:8123/latest.json",
            "",
            "   ",
        ] {
            assert_eq!(endpoint_override(Some(value), true), None, "{value}");
        }
        assert_eq!(endpoint_override(None, true), None);
    }
}
