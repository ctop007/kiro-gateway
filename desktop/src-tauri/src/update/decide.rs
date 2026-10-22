//! Pure decision logic: given the running version and the manifest text, what
//! should the app do?
//!
//! Manifest format (design decision 11): Tauri static-JSON plus
//! `minRequiredVersion` and an optional `manualDownload` map.
//!
//! Principle: better to skip an upgrade than to upgrade by mistake. Anything
//! that cannot be parsed or is inconsistent yields `Decision::None`.

use serde::Deserialize;
use std::collections::HashMap;

#[cfg(test)]
use super::types::{DownloadProgress, UpdateState};

/// Platforms the manifest can carry packages for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    DarwinAarch64,
    DarwinX86_64,
    WindowsX86_64,
}

impl Platform {
    /// Manifest key, identical to the Tauri updater target string.
    pub fn key(self) -> &'static str {
        match self {
            Platform::DarwinAarch64 => "darwin-aarch64",
            Platform::DarwinX86_64 => "darwin-x86_64",
            Platform::WindowsX86_64 => "windows-x86_64",
        }
    }

    /// Parses a manifest key; `None` for unsupported platforms.
    #[cfg(test)]
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "darwin-aarch64" => Some(Platform::DarwinAarch64),
            "darwin-x86_64" => Some(Platform::DarwinX86_64),
            "windows-x86_64" => Some(Platform::WindowsX86_64),
            _ => None,
        }
    }

    /// Platform of the running binary, `None` on unsupported targets.
    pub fn current() -> Option<Self> {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some(Platform::DarwinAarch64)
        } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            Some(Platform::DarwinX86_64)
        } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Some(Platform::WindowsX86_64)
        } else {
            None
        }
    }

    fn is_darwin(self) -> bool {
        matches!(self, Platform::DarwinAarch64 | Platform::DarwinX86_64)
    }
}

#[derive(Debug, Deserialize)]
struct PlatformEntry {
    url: String,
    signature: String,
}

#[derive(Debug, Deserialize, Default)]
struct ManualDownload {
    darwin: Option<String>,
    windows: Option<String>,
}

/// Parsed manifest (`latest.json`). Fields not listed here are ignored.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    version: String,
    min_required_version: Option<String>,
    notes: Option<String>,
    pub_date: Option<String>,
    platforms: HashMap<String, serde_json::Value>,
    manual_download: Option<ManualDownload>,
}

/// What the manifest offers for this platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOffer {
    /// Version announced by the manifest.
    pub version: String,
    /// Package URL for this platform (updater artifact).
    pub url: String,
    /// minisign signature text of the package.
    pub signature: String,
    /// Release notes, one entry per line.
    pub notes: Option<String>,
    /// Publish date as written in the manifest.
    pub pub_date: Option<String>,
    /// Direct installer link (dmg / setup.exe) for manual installation.
    pub manual_url: String,
}

/// Result of comparing the running version with the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to do (up to date, or the manifest is unusable).
    None,
    /// A newer version exists; download silently.
    Background(UpdateOffer),
    /// The running version is below `minRequiredVersion`; upgrade is mandatory.
    Forced(UpdateOffer),
}

/// Parses a plain `x.y.z` version (digits only, no leading zeros, no suffix).
///
/// Args:
///     text: Version text.
///
/// Returns:
///     The three numeric segments, or `None` for anything else.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let mut next = || -> Option<u64> {
        let seg = parts.next()?;
        let valid = !seg.is_empty()
            && seg.bytes().all(|b| b.is_ascii_digit())
            && (seg.len() == 1 || !seg.starts_with('0'));
        if valid {
            seg.parse().ok()
        } else {
            None
        }
    };
    let triple = (next()?, next()?, next()?);
    if parts.next().is_some() {
        return None;
    }
    Some(triple)
}

fn offer_for(manifest: &Manifest, platform: Platform) -> Option<UpdateOffer> {
    let entry: PlatformEntry =
        serde_json::from_value(manifest.platforms.get(platform.key())?.clone()).ok()?;
    if entry.url.trim().is_empty() || entry.signature.trim().is_empty() {
        return None;
    }
    let manual = manifest.manual_download.as_ref().and_then(|m| {
        if platform.is_darwin() { m.darwin.clone() } else { m.windows.clone() }
    });
    let manual_url = manual.filter(|u| !u.trim().is_empty()).unwrap_or_else(|| entry.url.clone());
    Some(UpdateOffer {
        version: manifest.version.clone(),
        url: entry.url,
        signature: entry.signature,
        notes: manifest.notes.clone(),
        pub_date: manifest.pub_date.clone(),
        manual_url,
    })
}

/// Why a manifest could not be used. All of them count as a failed check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestProblem {
    /// Not JSON, or required fields missing / of the wrong type.
    Malformed,
    /// A version string is not a plain `x.y.z`.
    BadVersion,
    /// `minRequiredVersion` is newer than the published `version`.
    Inconsistent,
    /// No usable entry (url + signature) for this platform.
    PlatformMissing,
}

/// Compares the running version with the manifest and reports problems.
///
/// Args:
///     current: Running version (`x.y.z`).
///     manifest_text: Raw `latest.json` body.
///     platform: Platform of the running binary.
///
/// Returns:
///     `Ok(Decision::None)` when the manifest is valid and nothing newer is
///     published. `Ok(Forced)` when `current < minRequiredVersion`,
///     `Ok(Background)` for any other newer version (a missing
///     `minRequiredVersion` never forces). `Err` when the manifest is unusable.
pub fn assess(
    current: &str,
    manifest_text: &str,
    platform: Platform,
) -> Result<Decision, ManifestProblem> {
    let manifest = serde_json::from_str::<Manifest>(manifest_text)
        .map_err(|_| ManifestProblem::Malformed)?;
    let cur = parse_version(current).ok_or(ManifestProblem::BadVersion)?;
    let new = parse_version(&manifest.version).ok_or(ManifestProblem::BadVersion)?;
    let min = match manifest.min_required_version.as_deref() {
        None => None,
        Some(text) => Some(parse_version(text).ok_or(ManifestProblem::BadVersion)?),
    };
    if min.is_some_and(|m| m > new) {
        return Err(ManifestProblem::Inconsistent);
    }
    if new <= cur {
        return Ok(Decision::None);
    }
    let offer = offer_for(&manifest, platform).ok_or(ManifestProblem::PlatformMissing)?;
    if min.is_some_and(|m| cur < m) {
        Ok(Decision::Forced(offer))
    } else {
        Ok(Decision::Background(offer))
    }
}

/// Like [`assess`], but every problem collapses to `Decision::None`
/// (better to skip an upgrade than to upgrade by mistake).
#[cfg(test)]
pub fn decide_offer(current: &str, manifest_text: &str, platform: Platform) -> Decision {
    assess(current, manifest_text, platform).unwrap_or(Decision::None)
}

/// Same decision expressed as the IPC [`UpdateState`] (`downloading` / `forced`
/// carry the initial, empty progress).
#[cfg(test)]
pub fn decide_update(current: &str, manifest_text: &str, platform: Platform) -> UpdateState {
    match decide_offer(current, manifest_text, platform) {
        Decision::None => UpdateState::None,
        Decision::Background(o) => {
            UpdateState::Downloading { version: o.version, progress: DownloadProgress::start() }
        }
        Decision::Forced(o) => UpdateState::Forced {
            version: o.version,
            progress: None,
            error: None,
            manual_url: o.manual_url,
        },
    }
}

#[cfg(test)]
mod tests;
