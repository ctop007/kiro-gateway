//! Start-up handling of a downloaded update (tasks 8.5).
//!
//! Called before the main window is shown. The package is installed only when
//! the record is well-formed, names a version newer than the running one, and
//! the package file still matches its recorded SHA-256. Anything else is
//! discarded so a bad file can never be installed or loop forever.

use std::path::Path;

use super::decide::parse_version;
use super::ports::{InstallError, PackageInstaller};
use super::store::{self, LoadedRecord, PackageProblem};

/// Why a pending update was thrown away without installing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardReason {
    /// `pending-update.json` is not a valid record.
    CorruptRecord,
    /// The package file is gone.
    FileMissing,
    /// The package file does not match the recorded SHA-256.
    HashMismatch,
    /// The recorded version is not newer than the running one (already applied
    /// or superseded).
    NotNewer,
}

/// What start-up did with the pending update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupOutcome {
    /// There was nothing pending.
    NoPending,
    /// The pending update was dropped; the app starts normally.
    Discarded(DiscardReason),
    /// Installed; the caller restarts the app (Windows exits by itself).
    Installed { version: String },
    /// The installer ran and failed; the package was removed and the app runs
    /// the current version.
    InstallFailed { version: String, error: String, manual_url: String },
    /// The installer could not run now (for example offline); the package is
    /// kept and the update shows as ready.
    Deferred { version: String },
}

/// Message stored in `InstallFailed` when the installer gave no detail.
pub const INSTALL_FAILED_MESSAGE: &str = "更新安装失败";

/// Installs the pending update, if any.
///
/// Args:
///     dir: App data directory.
///     current_version: Running version (`x.y.z`).
///     installer: Installs the verified package bytes.
///
/// Returns:
///     The outcome. Files are cleaned up as described on [`StartupOutcome`].
pub fn process_pending(
    dir: &Path,
    current_version: &str,
    installer: &dyn PackageInstaller,
) -> StartupOutcome {
    let record = match store::load(dir) {
        LoadedRecord::Missing => {
            // A stray package without a record can never be installed.
            let _ = store::remove_packages(dir);
            return StartupOutcome::NoPending;
        }
        LoadedRecord::Corrupt => return discard(dir, DiscardReason::CorruptRecord),
        LoadedRecord::Valid(r) => r,
    };
    let newer = match (parse_version(&record.version), parse_version(current_version)) {
        (Some(pending), Some(current)) => pending > current,
        _ => false,
    };
    if !newer {
        return discard(dir, DiscardReason::NotNewer);
    }
    let bytes = match store::read_verified(dir, &record) {
        Ok(b) => b,
        Err(PackageProblem::FileMissing) => return discard(dir, DiscardReason::FileMissing),
        Err(PackageProblem::HashMismatch) => return discard(dir, DiscardReason::HashMismatch),
    };
    match installer.install(&record.version, &bytes) {
        Ok(()) => {
            let _ = store::clear(dir);
            StartupOutcome::Installed { version: record.version }
        }
        Err(InstallError::Unavailable(_)) => StartupOutcome::Deferred { version: record.version },
        Err(InstallError::Failed(_)) => {
            let _ = store::clear(dir);
            StartupOutcome::InstallFailed {
                version: record.version,
                error: INSTALL_FAILED_MESSAGE.to_string(),
                manual_url: record.manual_url,
            }
        }
    }
}

fn discard(dir: &Path, reason: DiscardReason) -> StartupOutcome {
    let _ = store::clear(dir);
    StartupOutcome::Discarded(reason)
}

#[cfg(test)]
mod tests;
