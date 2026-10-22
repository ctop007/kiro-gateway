//! The access key shared by the gateway and the kclaude scripts.
//!
//! 32 random bytes from the OS CSPRNG, stored as 64 lowercase hex characters in the
//! `secret` file (0600). It is the gateway's `PROXY_API_KEY`, travels to the sidecar
//! through its environment and to the shell scripts through `kclaude.env`, and never
//! crosses IPC.

use super::atomic::{write_atomic, FileMode};
use super::paths::AppPaths;
use std::fs;
use std::io;

/// Number of random bytes behind a generated key.
const SECRET_BYTES: usize = 32;
/// Shortest key accepted from a hand-edited file.
const MIN_LEN: usize = 16;
/// Longest key accepted from a hand-edited file.
const MAX_LEN: usize = 128;

/// How [`load_or_create`] obtained the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretOrigin {
    /// Read from an existing, valid file.
    Existing,
    /// No file yet: a new key was generated (first run).
    Created,
    /// The file existed but was empty or malformed: a new key replaced it.
    Regenerated,
}

/// Generate a fresh key: 32 random bytes as 64 lowercase hex characters.
///
/// Returns:
///     The key, or an error when the OS random source is unavailable.
pub fn generate_secret() -> io::Result<String> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(format!("random source unavailable: {e}")))?;
    let mut text = String::with_capacity(SECRET_BYTES * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    Ok(text)
}

/// Whether `text` can be used as the key.
///
/// The key ends up unquoted in `kclaude.env` and in an HTTP header, so only a
/// conservative character set is allowed: ASCII letters, digits, `-`, `_`, `.`.
pub fn is_valid_secret(text: &str) -> bool {
    (MIN_LEN..=MAX_LEN).contains(&text.len())
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Read the key from the `secret` file, creating or replacing it when needed.
///
/// A trailing newline (from a manual edit) is ignored. A file with loose permissions
/// is tightened to 0600. An empty or malformed file is replaced with a new key, which
/// is reported through [`SecretOrigin::Regenerated`] so the caller can tell the user.
///
/// Args:
///     paths: App data layout.
///
/// Returns:
///     The key and how it was obtained, or the I/O error (including "the path is a
///     directory", which is not repaired silently).
pub fn load_or_create(paths: &AppPaths) -> io::Result<(String, SecretOrigin)> {
    let file = paths.secret_file();
    match fs::read(&file) {
        Ok(bytes) => {
            let trimmed = std::str::from_utf8(&bytes).ok().map(str::trim);
            if let Some(text) = trimmed.filter(|t| is_valid_secret(t)) {
                tighten_permissions(&file)?;
                return Ok((text.to_string(), SecretOrigin::Existing));
            }
            let secret = generate_secret()?;
            write_atomic(&file, secret.as_bytes(), FileMode::Private)?;
            Ok((secret, SecretOrigin::Regenerated))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let secret = generate_secret()?;
            write_atomic(&file, secret.as_bytes(), FileMode::Private)?;
            Ok((secret, SecretOrigin::Created))
        }
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn tighten_permissions(file: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(file)?.permissions().mode() & 0o777;
    if mode != 0o600 {
        fs::set_permissions(file, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn tighten_permissions(_file: &std::path::Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
