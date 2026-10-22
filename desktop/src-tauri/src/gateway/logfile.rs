//! `gateway.log`: stdout and stderr of the sidecar, rotated by size.
//!
//! At most [`LOG_FILES`] files of [`MAX_LOG_BYTES`] each (`gateway.log`, `.1`, `.2`).
//! Rotation happens between lines, so a line is never split across files.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Size at which the active file is rotated: 5 MB.
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
/// Number of files kept, including the active one.
pub const LOG_FILES: usize = 3;
/// A line longer than this is split, so a runaway output cannot grow the buffer forever.
pub const MAX_LINE_BYTES: usize = 64 * 1024;
/// Written when the App starts the sidecar; `read_run_tail` only looks after the last one.
pub const RUN_MARKER: &str = "===== Kiro Gateway App: gateway process started =====";
/// How much of the end of the file is read to build an excerpt.
const TAIL_WINDOW_BYTES: u64 = 256 * 1024;

/// The path of rotation number `index` (`gateway.log.1`, ...).
pub fn rotated(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

/// A size-rotated, append-only log file.
pub struct RotatingLog {
    path: PathBuf,
    max_bytes: u64,
    files: usize,
    file: File,
    size: u64,
}

fn open_append(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

impl RotatingLog {
    /// Open (or create) the log and continue after its current content.
    ///
    /// Args:
    ///     path: The active file.
    ///     max_bytes: Rotate before a write would pass this size.
    ///     files: Total files kept (1 means "truncate instead of keep history").
    pub fn open(path: &Path, max_bytes: u64, files: usize) -> io::Result<Self> {
        let file = open_append(path)?;
        let size = file.metadata()?.len();
        Ok(Self { path: path.to_path_buf(), max_bytes, files: files.max(1), file, size })
    }

    fn rotate(&mut self) -> io::Result<()> {
        if self.files <= 1 {
            self.file = OpenOptions::new().create(true).write(true).truncate(true).open(&self.path)?;
        } else {
            let _ = fs::remove_file(rotated(&self.path, self.files - 1));
            for index in (1..self.files - 1).rev() {
                let from = rotated(&self.path, index);
                if from.exists() {
                    fs::rename(&from, rotated(&self.path, index + 1))?;
                }
            }
            fs::rename(&self.path, rotated(&self.path, 1))?;
            self.file = open_append(&self.path)?;
        }
        self.size = 0;
        Ok(())
    }

    /// Append `line` and a newline, rotating first when the file would grow past the limit.
    ///
    /// A line larger than the limit is still written whole (into an otherwise empty file).
    /// If the file was deleted from outside, it is created again.
    pub fn write_line(&mut self, line: &[u8]) -> io::Result<()> {
        if !self.path.exists() {
            self.file = open_append(&self.path)?;
            self.size = 0;
        }
        let needed = line.len() as u64 + 1;
        if self.size > 0 && self.size + needed > self.max_bytes {
            self.rotate()?;
        }
        let mut buffer = Vec::with_capacity(line.len() + 1);
        buffer.extend_from_slice(line);
        buffer.push(b'\n');
        self.file.write_all(&buffer)?;
        self.size += needed;
        Ok(())
    }
}

fn write_shared(log: &Mutex<RotatingLog>, line: &[u8]) {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let mut guard = log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // A full disk must not stop the pump: the child would block on a full pipe.
    let _ = guard.write_line(line);
}

/// Copy `reader` into the log line by line until it ends.
///
/// Used for the sidecar's stdout and stderr (one thread each, sharing one log), so lines
/// of the two streams never mix inside a line. Invalid UTF-8 is kept as is. The pump keeps
/// draining even when writing fails.
pub fn pump<R: Read>(mut reader: R, log: &Mutex<RotatingLog>) {
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for &byte in &chunk[..read] {
            if byte == b'\n' {
                write_shared(log, &pending);
                pending.clear();
            } else {
                pending.push(byte);
                if pending.len() >= MAX_LINE_BYTES {
                    write_shared(log, &pending);
                    pending.clear();
                }
            }
        }
    }
    if !pending.is_empty() {
        write_shared(log, &pending);
    }
}

/// The last `max_lines` lines of the current run in `path`, unredacted.
///
/// Only the end of the file is read. Lines before the last [`RUN_MARKER`] belong to an
/// earlier run and are skipped. A first line cut by the read window is dropped.
pub fn read_run_tail(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else { return Vec::new() };
    let Ok(len) = file.metadata().map(|m| m.len()) else { return Vec::new() };
    let start = len.saturating_sub(TAIL_WINDOW_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(TAIL_WINDOW_BYTES).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    if let Some(index) = lines.iter().rposition(|l| *l == RUN_MARKER) {
        lines.drain(..=index);
    }
    let skip = lines.len().saturating_sub(max_lines);
    lines.into_iter().skip(skip).map(str::to_string).collect()
}

#[cfg(test)]
mod tests;
