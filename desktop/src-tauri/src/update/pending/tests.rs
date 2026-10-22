use super::*;
use crate::update::testing::FakeBackend;
use crate::update::ports::InstallError;
use crate::update::store;
use std::fs;

fn setup(version: &str, bytes: &[u8]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    store::save(d.path(), version, bytes, "https://x/Kiro.dmg", "t").unwrap();
    d
}

#[test]
fn valid_newer_package_is_installed_and_cleaned_up() {
    let d = setup("0.0.2", b"PKG");
    let backend = FakeBackend::default();
    let out = process_pending(d.path(), "0.0.1", &backend);
    assert_eq!(out, StartupOutcome::Installed { version: "0.0.2".into() });
    assert_eq!(backend.installed.lock().unwrap().as_slice(), &[("0.0.2".to_string(), b"PKG".to_vec())]);
    assert_eq!(store::load(d.path()), store::LoadedRecord::Missing);
    assert!(!d.path().join("updates/0.0.2.bin").exists());
}

#[test]
fn nothing_pending_does_nothing_and_never_calls_installer() {
    let d = tempfile::tempdir().unwrap();
    let backend = FakeBackend::default();
    assert_eq!(process_pending(d.path(), "0.0.1", &backend), StartupOutcome::NoPending);
    assert!(backend.installed.lock().unwrap().is_empty());
}

#[test]
fn stray_package_without_record_is_removed() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("updates")).unwrap();
    fs::write(d.path().join("updates/0.0.2.bin"), b"x").unwrap();
    assert_eq!(process_pending(d.path(), "0.0.1", &FakeBackend::default()), StartupOutcome::NoPending);
    assert!(!d.path().join("updates/0.0.2.bin").exists());
}

#[test]
fn missing_package_file_is_discarded_without_install() {
    let d = setup("0.0.2", b"PKG");
    fs::remove_file(d.path().join("updates/0.0.2.bin")).unwrap();
    let backend = FakeBackend::default();
    assert_eq!(
        process_pending(d.path(), "0.0.1", &backend),
        StartupOutcome::Discarded(DiscardReason::FileMissing)
    );
    assert!(backend.installed.lock().unwrap().is_empty());
    assert_eq!(store::load(d.path()), store::LoadedRecord::Missing);
}

#[test]
fn corrupt_record_is_discarded_and_files_removed() {
    let d = setup("0.0.2", b"PKG");
    fs::write(d.path().join("pending-update.json"), "{ not json").unwrap();
    let backend = FakeBackend::default();
    assert_eq!(
        process_pending(d.path(), "0.0.1", &backend),
        StartupOutcome::Discarded(DiscardReason::CorruptRecord)
    );
    assert!(backend.installed.lock().unwrap().is_empty());
    assert!(!d.path().join("pending-update.json").exists());
    assert!(!d.path().join("updates/0.0.2.bin").exists());
}

#[test]
fn tampered_package_fails_sha256_and_is_not_installed() {
    let d = setup("0.0.2", b"PKG");
    fs::write(d.path().join("updates/0.0.2.bin"), b"PKG-TAMPERED").unwrap();
    let backend = FakeBackend::default();
    assert_eq!(
        process_pending(d.path(), "0.0.1", &backend),
        StartupOutcome::Discarded(DiscardReason::HashMismatch)
    );
    assert!(backend.installed.lock().unwrap().is_empty());
    assert!(!d.path().join("updates/0.0.2.bin").exists());
}

#[test]
fn truncated_package_fails_sha256() {
    let d = setup("0.0.2", b"PACKAGE-CONTENT");
    fs::write(d.path().join("updates/0.0.2.bin"), b"PACKAGE").unwrap();
    assert_eq!(
        process_pending(d.path(), "0.0.1", &FakeBackend::default()),
        StartupOutcome::Discarded(DiscardReason::HashMismatch)
    );
}

#[test]
fn version_equal_to_current_is_discarded_as_not_newer() {
    let d = setup("0.0.2", b"PKG");
    let backend = FakeBackend::default();
    assert_eq!(
        process_pending(d.path(), "0.0.2", &backend),
        StartupOutcome::Discarded(DiscardReason::NotNewer)
    );
    assert!(backend.installed.lock().unwrap().is_empty());
    assert_eq!(store::load(d.path()), store::LoadedRecord::Missing);
}

#[test]
fn version_older_than_current_is_discarded_as_not_newer() {
    let d = setup("0.0.2", b"PKG");
    assert_eq!(
        process_pending(d.path(), "0.1.0", &FakeBackend::default()),
        StartupOutcome::Discarded(DiscardReason::NotNewer)
    );
}

#[test]
fn unparseable_current_version_never_installs() {
    let d = setup("0.0.2", b"PKG");
    let backend = FakeBackend::default();
    assert_eq!(
        process_pending(d.path(), "dev", &backend),
        StartupOutcome::Discarded(DiscardReason::NotNewer)
    );
    assert!(backend.installed.lock().unwrap().is_empty());
}

#[test]
fn installer_failure_removes_package_and_reports_manual_url() {
    let d = setup("0.0.2", b"PKG");
    let backend = FakeBackend::default();
    backend.push_install(Err(InstallError::Failed("disk full".into())));
    match process_pending(d.path(), "0.0.1", &backend) {
        StartupOutcome::InstallFailed { version, error, manual_url } => {
            assert_eq!(version, "0.0.2");
            assert_eq!(error, INSTALL_FAILED_MESSAGE);
            assert!(!error.contains("disk full"), "raw installer detail must not reach the UI");
            assert_eq!(manual_url, "https://x/Kiro.dmg");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(store::load(d.path()), store::LoadedRecord::Missing);
}

#[test]
fn installer_unavailable_keeps_package_for_next_attempt() {
    let d = setup("0.0.2", b"PKG");
    let backend = FakeBackend::default();
    backend.push_install(Err(InstallError::Unavailable("offline".into())));
    assert_eq!(
        process_pending(d.path(), "0.0.1", &backend),
        StartupOutcome::Deferred { version: "0.0.2".into() }
    );
    assert!(d.path().join("updates/0.0.2.bin").exists());
    assert!(matches!(store::load(d.path()), store::LoadedRecord::Valid(_)));
}

#[test]
fn record_version_cannot_escape_updates_directory() {
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("pending-update.json"),
        format!(
            r#"{{"version":"../../x","sha256":"{}","manualUrl":"u","savedAt":"t"}}"#,
            store::sha256_hex(b"x")
        ),
    )
    .unwrap();
    let backend = FakeBackend::default();
    assert_eq!(
        process_pending(d.path(), "0.0.1", &backend),
        StartupOutcome::Discarded(DiscardReason::CorruptRecord)
    );
    assert!(backend.installed.lock().unwrap().is_empty());
}
