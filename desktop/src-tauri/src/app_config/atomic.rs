//! Atomic file writes: temporary file in the same directory, then rename.
//!
//! A reader (the kclaude scripts sourcing `kclaude.env`, a second App start reading
//! `config.json`) sees either the old file or the new one, never half of it.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Permissions of the file being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    /// Whatever the umask gives (0644 in practice).
    Default,
    /// Owner read/write only (0600), set when the temporary file is created so the
    /// content is never readable by others, not even for a moment. No-op on Windows,
    /// where the per-user profile directory is already private.
    Private,
}

/// Distinguishes temporary files of concurrent writers inside one process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_path_for(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".to_string());
    let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".{name}.tmp-{}-{unique}", std::process::id()))
}

fn create_temp(path: &Path, mode: FileMode) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if mode == FileMode::Private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = mode;
    options.open(path)
}

/// Write `content` to `target` atomically.
///
/// Missing parent directories are created. The temporary file is removed again if any
/// step fails, so a failed write leaves no litter and the previous file untouched.
///
/// Args:
///     target: Final path of the file.
///     content: Complete new content.
///     mode: Permissions of the resulting file.
///
/// Returns:
///     The I/O error of the first failing step (create, write, sync, rename).
pub fn write_atomic(target: &Path, content: &[u8], mode: FileMode) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let temp = temp_path_for(target);
    let result = (|| {
        let mut file = create_temp(&temp, mode)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests;
