//! On-disk store of the downloaded-but-not-installed update.
//!
//! Layout inside the app data directory:
//! - `updates/<version>.bin`: the verified package bytes
//! - `pending-update.json`: `{ version, sha256, manualUrl, savedAt }`
//! - `upgrade-lock.json`: `{ version, manualUrl }`, present while a forced upgrade is pending
//!
//! The file name is derived from the version, never read from the record, so a
//! tampered record cannot point outside `updates/`. Writes are atomic
//! (temporary file + rename).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::decide::parse_version;

const RECORD_FILE: &str = "pending-update.json";
const UPDATES_DIR: &str = "updates";
const LOCK_FILE: &str = "upgrade-lock.json";

/// Contents of `pending-update.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRecord {
    /// Version of the downloaded package (`x.y.z`).
    pub version: String,
    /// Lower-case hex SHA-256 of the package file.
    pub sha256: String,
    /// Direct installer link shown when installing fails.
    pub manual_url: String,
    /// When the package was saved, ISO 8601.
    pub saved_at: String,
}

/// Result of reading `pending-update.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadedRecord {
    /// No record file.
    Missing,
    /// The file exists but is not a valid record (bad JSON, missing fields,
    /// unparseable version, malformed digest).
    Corrupt,
    /// A well-formed record.
    Valid(PendingRecord),
}

/// Why a recorded package cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageProblem {
    /// The package file does not exist or cannot be read.
    FileMissing,
    /// The file content does not match the recorded digest.
    HashMismatch,
}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn record_path(dir: &Path) -> PathBuf {
    dir.join(RECORD_FILE)
}

fn updates_dir(dir: &Path) -> PathBuf {
    dir.join(UPDATES_DIR)
}

/// Path of the package file for `version`; `None` when the version is not `x.y.z`.
pub fn package_path(dir: &Path, version: &str) -> Option<PathBuf> {
    parse_version(version)?;
    Some(updates_dir(dir).join(format!("{version}.bin")))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = parent.join(tmp_name);
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Saves a verified package and its record, replacing any older pending update.
///
/// Args:
///     dir: App data directory.
///     version: Package version (`x.y.z`).
///     bytes: Verified package bytes.
///     manual_url: Direct installer link for the failure UI.
///     saved_at: Timestamp to record.
///
/// Returns:
///     The written record. The package file is written first, so a crash in
///     between leaves no record pointing at a missing file.
pub fn save(
    dir: &Path,
    version: &str,
    bytes: &[u8],
    manual_url: &str,
    saved_at: &str,
) -> io::Result<PendingRecord> {
    let path = package_path(dir, version)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid version"))?;
    clear(dir)?;
    write_atomic(&path, bytes)?;
    let record = PendingRecord {
        version: version.to_string(),
        sha256: sha256_hex(bytes),
        manual_url: manual_url.to_string(),
        saved_at: saved_at.to_string(),
    };
    let json = serde_json::to_vec_pretty(&record)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let Err(e) = write_atomic(&record_path(dir), &json) {
        let _ = fs::remove_file(&path);
        return Err(e);
    }
    Ok(record)
}

/// Reads `pending-update.json`.
pub fn load(dir: &Path) -> LoadedRecord {
    let text = match fs::read_to_string(record_path(dir)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return LoadedRecord::Missing,
        Err(_) => return LoadedRecord::Corrupt,
    };
    let Ok(record) = serde_json::from_str::<PendingRecord>(&text) else {
        return LoadedRecord::Corrupt;
    };
    let digest_ok = record.sha256.len() == 64
        && record.sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if parse_version(&record.version).is_none() || !digest_ok {
        return LoadedRecord::Corrupt;
    }
    LoadedRecord::Valid(record)
}

/// Reads the package of `record` and checks it against the recorded digest.
pub fn read_verified(dir: &Path, record: &PendingRecord) -> Result<Vec<u8>, PackageProblem> {
    let path = package_path(dir, &record.version).ok_or(PackageProblem::FileMissing)?;
    let bytes = fs::read(path).map_err(|_| PackageProblem::FileMissing)?;
    if sha256_hex(&bytes) == record.sha256 {
        Ok(bytes)
    } else {
        Err(PackageProblem::HashMismatch)
    }
}

/// Removes the record and every package file. Missing files are fine.
pub fn clear(dir: &Path) -> io::Result<()> {
    remove_if_exists(&record_path(dir))?;
    remove_packages(dir)
}

/// Removes only the package files, keeping the record.
pub fn remove_packages(dir: &Path) -> io::Result<()> {
    let updates = updates_dir(dir);
    let entries = match fs::read_dir(&updates) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            remove_if_exists(&path)?;
        }
    }
    Ok(())
}

/// Contents of `upgrade-lock.json`: the version a forced upgrade must reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockInfo {
    /// Version that ends the lock (the running version must be at least this).
    pub version: String,
    /// Direct installer link shown on the forced-upgrade overlay.
    pub manual_url: String,
}

/// Writes the forced-upgrade lock.
pub fn save_lock(dir: &Path, lock: &LockInfo) -> io::Result<()> {
    if parse_version(&lock.version).is_none() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid version"));
    }
    let json = serde_json::to_vec_pretty(lock)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_atomic(&dir.join(LOCK_FILE), &json)
}

/// Reads the forced-upgrade lock; `None` when absent or invalid.
pub fn load_lock(dir: &Path) -> Option<LockInfo> {
    let text = fs::read_to_string(dir.join(LOCK_FILE)).ok()?;
    let lock: LockInfo = serde_json::from_str(&text).ok()?;
    parse_version(&lock.version)?;
    Some(lock)
}

/// Removes the forced-upgrade lock file.
pub fn clear_lock(dir: &Path) -> io::Result<()> {
    remove_if_exists(&dir.join(LOCK_FILE))
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn save_then_load_and_verify_round_trips() {
        let d = dir();
        let rec = save(d.path(), "0.0.2", b"package", "https://x/a.dmg", "t").unwrap();
        assert_eq!(load(d.path()), LoadedRecord::Valid(rec.clone()));
        assert_eq!(read_verified(d.path(), &rec).unwrap(), b"package");
        assert!(d.path().join("updates/0.0.2.bin").is_file());
    }

    #[test]
    fn load_reports_missing_and_corrupt() {
        let d = dir();
        assert_eq!(load(d.path()), LoadedRecord::Missing);
        for bad in ["", "not json", "{}", "[]", r#"{"version":"0.0.2"}"#] {
            fs::write(d.path().join(RECORD_FILE), bad).unwrap();
            assert_eq!(load(d.path()), LoadedRecord::Corrupt, "content {bad:?}");
        }
    }

    #[test]
    fn load_rejects_bad_version_and_bad_digest() {
        let d = dir();
        let digest = sha256_hex(b"x");
        let cases = [
            ("../../etc/passwd", digest.as_str()),
            ("1.2", digest.as_str()),
            ("0.0.2", "DEADBEEF"),
            ("0.0.2", &digest.to_uppercase()),
        ];
        for (version, sha) in cases {
            let text = format!(
                r#"{{"version":"{version}","sha256":"{sha}","manualUrl":"u","savedAt":"t"}}"#
            );
            fs::write(d.path().join(RECORD_FILE), text).unwrap();
            assert_eq!(load(d.path()), LoadedRecord::Corrupt, "{version} / {sha}");
        }
    }

    #[test]
    fn read_verified_detects_missing_file_and_modified_content() {
        let d = dir();
        let rec = save(d.path(), "0.0.2", b"package", "u", "t").unwrap();
        let file = d.path().join("updates/0.0.2.bin");
        fs::write(&file, b"tampered").unwrap();
        assert_eq!(read_verified(d.path(), &rec), Err(PackageProblem::HashMismatch));
        fs::remove_file(&file).unwrap();
        assert_eq!(read_verified(d.path(), &rec), Err(PackageProblem::FileMissing));
    }

    #[test]
    fn save_replaces_older_pending_update() {
        let d = dir();
        save(d.path(), "0.0.2", b"old", "u", "t").unwrap();
        save(d.path(), "0.0.3", b"new", "u", "t").unwrap();
        assert!(!d.path().join("updates/0.0.2.bin").exists());
        assert!(d.path().join("updates/0.0.3.bin").exists());
        match load(d.path()) {
            LoadedRecord::Valid(r) => assert_eq!(r.version, "0.0.3"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn save_rejects_invalid_version_without_touching_disk() {
        let d = dir();
        assert!(save(d.path(), "../evil", b"x", "u", "t").is_err());
        assert!(!d.path().join("pending-update.json").exists());
    }

    #[test]
    fn clear_is_idempotent_and_leaves_no_files() {
        let d = dir();
        clear(d.path()).unwrap();
        save(d.path(), "0.0.2", b"x", "u", "t").unwrap();
        clear(d.path()).unwrap();
        clear(d.path()).unwrap();
        assert_eq!(load(d.path()), LoadedRecord::Missing);
        assert!(!d.path().join("updates/0.0.2.bin").exists());
    }

    #[test]
    fn lock_round_trips_and_rejects_garbage() {
        let d = dir();
        assert_eq!(load_lock(d.path()), None);
        let lock = LockInfo { version: "0.0.3".into(), manual_url: "https://x".into() };
        save_lock(d.path(), &lock).unwrap();
        assert_eq!(load_lock(d.path()), Some(lock));
        fs::write(d.path().join(LOCK_FILE), "nope").unwrap();
        assert_eq!(load_lock(d.path()), None);
        fs::write(d.path().join(LOCK_FILE), r#"{"version":"x","manualUrl":"u"}"#).unwrap();
        assert_eq!(load_lock(d.path()), None);
        assert!(save_lock(d.path(), &LockInfo { version: "bad".into(), manual_url: "u".into() }).is_err());
        clear_lock(d.path()).unwrap();
        clear_lock(d.path()).unwrap();
    }

    #[test]
    fn package_path_ignores_record_contents_and_validates_version() {
        let d = dir();
        assert!(package_path(d.path(), "0.0.2").unwrap().ends_with("updates/0.0.2.bin"));
        assert!(package_path(d.path(), "../0.0.2").is_none());
        assert!(package_path(d.path(), "").is_none());
    }
}
