//! Injection points of the update module. The Tauri glue implements them with
//! the real updater plugin, gateway manager and `app_config`; tests use fakes.

use chrono::{DateTime, FixedOffset};
use std::sync::atomic::AtomicBool;

use super::decide::UpdateOffer;
use super::types::UpdateStatus;

/// Wall clock. Using the wall clock (not a monotonic one) makes a sleeping
/// laptop catch up right after it wakes.
pub trait Clock: Send + Sync {
    /// Current local time with its UTC offset.
    fn now(&self) -> DateTime<FixedOffset>;
}

/// Real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<FixedOffset> {
        chrono::Local::now().fixed_offset()
    }
}

/// Why installing a downloaded package did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// Could not even start (for example offline, so the updater cannot resolve
    /// the update handle). The package stays on disk for the next attempt.
    Unavailable(String),
    /// The installer ran and failed (disk full, permissions). The package is dropped.
    Failed(String),
}

/// Installs an already downloaded and signature-verified package.
pub trait PackageInstaller: Send + Sync {
    /// Args:
    ///     version: Version the package belongs to.
    ///     bytes: Raw package (`.app.tar.gz` / NSIS `setup.exe`).
    fn install(&self, version: &str, bytes: &[u8]) -> Result<(), InstallError>;
}

/// Source of the manifest and the packages.
pub trait UpdateBackend: Send + Sync {
    /// Fetches the manifest.
    ///
    /// Returns:
    ///     `Ok(Some(text))` raw manifest JSON, `Ok(None)` when the server says
    ///     nothing newer than the running version exists, `Err` on any failure
    ///     (offline, timeout, unusable response, missing platform entry).
    fn fetch_manifest(&self) -> Result<Option<String>, String>;

    /// Downloads the package of `offer` and verifies its minisign signature.
    /// `on_progress(downloaded_bytes, total_bytes)` is called per chunk.
    ///
    /// `cancel` is set by the caller when it gave up waiting (no progress for too long);
    /// the implementation must then stop as soon as it can and return an error, so a hung
    /// connection does not keep a thread and a socket forever.
    fn download(
        &self,
        offer: &UpdateOffer,
        cancel: &AtomicBool,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>, String>;
}

/// The persisted `upgradeRequired` flag in `config.json`, owned by `app_config`.
pub trait UpgradeFlag: Send + Sync {
    /// Whether the flag is currently set.
    fn is_set(&self) -> bool;
    /// Persists the flag.
    fn set(&self, value: bool) -> Result<(), String>;
}

/// Gateway control as far as the update module needs it.
pub trait GatewayControl: Send + Sync {
    /// Stops the gateway (no-op when stopped). Does not touch `gatewayWanted`.
    fn stop(&self);
    /// Starts the gateway again if `gatewayWanted` is true (after a failed install).
    fn resume_if_wanted(&self);
}

/// Application lifecycle.
pub trait AppControl: Send + Sync {
    /// Restarts the application; does not return in production.
    fn restart(&self);
}

/// Receiver of `update-status` events.
pub trait EventSink: Send + Sync {
    /// Called on every state transition and, throttled, for download progress.
    fn emit(&self, status: &UpdateStatus);
}
