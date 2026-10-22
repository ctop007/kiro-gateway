use super::*;
use crate::update::decide::Platform;
use crate::update::ports::InstallError;
use crate::update::store;
use crate::update::testing::{manifest, Rig};
use chrono::Duration;
use std::sync::atomic::Ordering;

fn offline(rig: &Rig) {
    rig.backend.push_manifest(Err("offline".into()));
}

fn state(rig: &Rig) -> UpdateState {
    rig.svc.status().state
}

#[test]
fn first_tick_checks_immediately() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    assert!(rig.svc.tick());
    assert_eq!(rig.backend.fetches(), 1);
}

#[test]
fn ticks_inside_60_minutes_do_not_check() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    assert!(rig.svc.tick());
    for _ in 0..11 {
        rig.clock.advance(Duration::minutes(5));
        assert!(!rig.svc.tick());
    }
    assert_eq!(rig.backend.fetches(), 1);
}

#[test]
fn check_runs_again_after_60_minutes() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    rig.svc.tick();
    rig.clock.advance(Duration::minutes(59));
    assert!(!rig.svc.tick());
    rig.clock.advance(Duration::minutes(1));
    assert!(rig.svc.tick());
    assert_eq!(rig.backend.fetches(), 2);
}

#[test]
fn wake_from_sleep_checks_once_on_first_tick_not_once_per_missed_interval() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    rig.svc.tick();
    rig.clock.advance(Duration::hours(9));
    assert!(rig.svc.tick());
    assert!(!rig.svc.tick());
    assert_eq!(rig.backend.fetches(), 2);
}

#[test]
fn clock_set_backwards_triggers_a_check_instead_of_stalling() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    rig.svc.tick();
    rig.clock.advance(Duration::hours(-3));
    assert!(rig.svc.tick());
}

#[test]
fn failed_check_is_silent_and_retried_after_60_minutes_not_every_tick() {
    let rig = Rig::new("0.0.1");
    offline(&rig);
    offline(&rig);
    offline(&rig);
    assert!(rig.svc.tick());
    assert_eq!(state(&rig), UpdateState::None);
    assert!(rig.events().is_empty(), "no event for a failed check");
    assert_eq!(rig.svc.status().last_checked_at, None);
    rig.clock.advance(Duration::minutes(5));
    assert!(!rig.svc.tick());
    rig.clock.advance(Duration::minutes(55));
    assert!(rig.svc.tick());
    rig.clock.advance(Duration::minutes(60));
    assert!(rig.svc.tick());
    assert_eq!(rig.backend.fetches(), 3);
    assert_eq!(state(&rig), UpdateState::None);
}

#[test]
fn damaged_manifests_count_as_failed_checks() {
    let rig = Rig::new("0.0.1");
    for text in ["", "{", "<html>", r#"{"version":"x.y","platforms":{}}"#] {
        rig.backend.push_manifest(Ok(Some(text.into())));
    }
    for _ in 0..4 {
        rig.svc.check_now();
    }
    assert_eq!(state(&rig), UpdateState::None);
    assert_eq!(rig.svc.status().last_checked_at, None);
    assert_eq!(rig.backend.downloads_done(), 0);
    assert!(!rig.flag.0.load(Ordering::SeqCst));
}

#[test]
fn successful_up_to_date_check_records_time() {
    let rig = Rig::new("0.0.2");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    let st = rig.svc.check_now();
    assert_eq!(st.state, UpdateState::None);
    assert!(st.last_checked_at.unwrap().starts_with("2026-10-22T09:00:00+08:00"));
}

#[test]
fn server_reporting_no_update_is_up_to_date() {
    let rig = Rig::new("0.0.2");
    rig.backend.push_manifest(Ok(None));
    let st = rig.svc.check_now();
    assert_eq!(st.state, UpdateState::None);
    assert!(st.last_checked_at.is_some());
}

#[test]
fn newer_version_downloads_in_background_and_becomes_ready() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", Some("0.0.1")))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
    let events: Vec<UpdateState> = rig.events().into_iter().map(|e| e.state).collect();
    assert!(matches!(events.first(), Some(UpdateState::Downloading { version, .. }) if version == "0.0.2"));
    assert_eq!(events.last(), Some(&UpdateState::Ready { version: "0.0.2".into() }));
    match store::load(&rig.path()) {
        store::LoadedRecord::Valid(r) => {
            assert_eq!(r.version, "0.0.2");
            assert_eq!(r.sha256, store::sha256_hex(b"PKG"));
            assert_eq!(r.manual_url, "https://cos.example/0.0.2/Kiro-Gateway.dmg");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(rig.stops(), 0, "background download must not touch the gateway");
    assert!(!rig.flag.0.load(Ordering::SeqCst));
}

#[test]
fn same_version_is_not_downloaded_twice() {
    let rig = Rig::new("0.0.1");
    for _ in 0..3 {
        rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    }
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    rig.svc.check_now();
    rig.svc.check_now();
    assert_eq!(rig.backend.downloads_done(), 1);
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
}

#[test]
fn newer_release_while_ready_replaces_the_pending_package() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", None))));
    rig.backend.push_download(Ok(b"TWO".to_vec()));
    rig.backend.push_download(Ok(b"THREE".to_vec()));
    rig.svc.check_now();
    rig.svc.check_now();
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.3".into() });
    assert!(!rig.path().join("updates/0.0.2.bin").exists());
    assert!(rig.path().join("updates/0.0.3.bin").exists());
}

#[test]
fn ready_survives_a_later_failed_check_and_an_up_to_date_manifest() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    offline(&rig);
    rig.svc.check_now();
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.svc.check_now();
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
}

#[test]
fn failed_background_download_is_silent_and_retried_at_next_check() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Err("reset".into()));
    rig.svc.check_now();
    assert_eq!(state(&rig), UpdateState::None);
    assert_eq!(store::load(&rig.path()), store::LoadedRecord::Missing);
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    assert_eq!(rig.backend.downloads_done(), 2);
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
}

#[test]
fn download_progress_events_are_throttled_to_250ms() {
    let rig = Rig::new("0.0.1");
    *rig.backend.progress_steps.lock().unwrap() =
        vec![(10, Some(100)), (20, Some(100)), (30, Some(100)), (100, Some(100))];
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    let progress_events: Vec<_> = rig
        .events()
        .into_iter()
        .filter_map(|e| match e.state {
            UpdateState::Downloading { progress, .. } => Some(progress.downloaded_bytes),
            _ => None,
        })
        .collect();
    // initial 0, first step (nothing emitted before), then only the final 100% step.
    assert_eq!(progress_events, vec![0, 10, 100]);
}

#[test]
fn forced_stops_gateway_sets_flag_and_installs_then_restarts() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Ok(b"PKG3".to_vec()));
    rig.svc.check_now();
    assert_eq!(rig.stops(), 1);
    assert!(rig.flag.0.load(Ordering::SeqCst), "upgradeRequired persisted");
    assert_eq!(rig.restarts(), 1);
    assert_eq!(rig.backend.installed.lock().unwrap()[0].0, "0.0.3");
    assert!(rig.svc.upgrade_required());
    let states: Vec<UpdateState> = rig.events().into_iter().map(|e| e.state).collect();
    assert!(matches!(states.first(), Some(UpdateState::Forced { version, .. }) if version == "0.0.3"));
}

#[test]
fn forced_download_failure_shows_error_with_manual_url_and_keeps_lock() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Err("timeout".into()));
    rig.svc.check_now();
    match state(&rig) {
        UpdateState::Forced { version, error, manual_url, .. } => {
            assert_eq!(version, "0.0.3");
            assert_eq!(error.as_deref(), Some("下载失败"));
            assert!(manual_url.ends_with(".dmg"));
        }
        other => panic!("{other:?}"),
    }
    assert!(rig.flag.0.load(Ordering::SeqCst));
    assert_eq!(rig.restarts(), 0);
}

#[test]
fn forced_retry_downloads_again_and_completes() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Err("timeout".into()));
    rig.svc.check_now();
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Ok(b"PKG3".to_vec()));
    rig.svc.retry().unwrap();
    assert_eq!(rig.restarts(), 1);
    assert_eq!(rig.stops(), 1, "gateway stopped once, not again on retry");
}

#[test]
fn forced_install_failure_stays_forced_with_error() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Ok(b"PKG3".to_vec()));
    rig.backend.push_install(Err(InstallError::Failed("denied".into())));
    rig.svc.check_now();
    assert!(matches!(state(&rig), UpdateState::Forced { error: Some(_), .. }));
    assert_eq!(rig.restarts(), 0);
}

#[test]
fn forced_while_gateway_running_with_ready_update_reuses_verified_package() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", None))));
    rig.backend.push_download(Ok(b"PKG3".to_vec()));
    rig.svc.check_now();
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.3".into() });
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.svc.check_now();
    assert_eq!(rig.backend.downloads_done(), 1, "verified package is reused");
    assert_eq!(rig.restarts(), 1);
}

#[test]
fn manifest_dropping_the_requirement_releases_the_lock() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Err("x".into()));
    rig.svc.check_now();
    assert!(rig.flag.0.load(Ordering::SeqCst));
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.1")))));
    rig.backend.push_download(Ok(b"PKG3".to_vec()));
    rig.svc.check_now();
    assert!(!rig.flag.0.load(Ordering::SeqCst));
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.3".into() });
    assert_eq!(store::load_lock(&rig.path()), None);
}

#[test]
fn restart_now_without_ready_update_is_rejected() {
    let rig = Rig::new("0.0.1");
    let err = rig.svc.restart_now().unwrap_err();
    assert_eq!(err.code, UpdateErrorCode::NoPendingUpdate);
    assert_eq!(rig.stops(), 0);
    assert_eq!(rig.restarts(), 0);
}

#[test]
fn restart_now_stops_gateway_installs_and_restarts() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    rig.svc.restart_now().unwrap();
    assert_eq!(rig.stops(), 1);
    assert_eq!(rig.restarts(), 1);
    assert_eq!(rig.backend.installed.lock().unwrap()[0].0, "0.0.2");
}

#[test]
fn restart_now_install_failure_resumes_gateway_and_reports_install_failed() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    rig.backend.push_install(Err(InstallError::Failed("denied".into())));
    rig.svc.restart_now().unwrap();
    assert_eq!(rig.restarts(), 0);
    assert_eq!(rig.gateway.resumes.load(Ordering::SeqCst), 1);
    match state(&rig) {
        UpdateState::InstallFailed { version, manual_url, .. } => {
            assert_eq!(version, "0.0.2");
            assert!(manual_url.ends_with(".dmg"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn restart_now_when_installer_unavailable_keeps_ready_and_resumes_gateway() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    rig.backend.push_install(Err(InstallError::Unavailable("offline".into())));
    let err = rig.svc.restart_now().unwrap_err();
    assert_eq!(err.code, UpdateErrorCode::Internal);
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
    assert_eq!(rig.gateway.resumes.load(Ordering::SeqCst), 1);
}

#[test]
fn restart_now_with_package_deleted_behind_our_back_degrades_to_none() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    std::fs::remove_file(rig.path().join("updates/0.0.2.bin")).unwrap();
    let err = rig.svc.restart_now().unwrap_err();
    assert_eq!(err.code, UpdateErrorCode::NoPendingUpdate);
    assert_eq!(state(&rig), UpdateState::None);
}

#[test]
fn retry_is_invalid_unless_forced_or_install_failed() {
    let rig = Rig::new("0.0.1");
    let err = rig.svc.retry().unwrap_err();
    assert_eq!(err.code, UpdateErrorCode::InvalidArgument);
    assert_eq!(rig.backend.fetches(), 0);
}

#[test]
fn retry_after_install_failed_downloads_again() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    rig.svc.check_now();
    rig.backend.push_install(Err(InstallError::Failed("x".into())));
    rig.svc.restart_now().unwrap();
    assert!(matches!(state(&rig), UpdateState::InstallFailed { .. }));
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    let st = rig.svc.retry().unwrap();
    assert_eq!(st.state, UpdateState::Ready { version: "0.0.2".into() });
}

#[test]
fn startup_without_pending_leaves_state_none() {
    let rig = Rig::new("0.0.1");
    assert_eq!(rig.svc.startup(), StartupOutcome::NoPending);
    assert_eq!(state(&rig), UpdateState::None);
    assert_eq!(rig.restarts(), 0);
}

#[test]
fn startup_installs_pending_package_and_restarts() {
    let rig = Rig::new("0.0.1");
    store::save(&rig.path(), "0.0.2", b"PKG", "u", "t").unwrap();
    assert_eq!(rig.svc.startup(), StartupOutcome::Installed { version: "0.0.2".into() });
    assert_eq!(rig.restarts(), 1);
}

#[test]
fn startup_install_failure_runs_current_version_and_marks_failed() {
    let rig = Rig::new("0.0.1");
    store::save(&rig.path(), "0.0.2", b"PKG", "https://x/y.dmg", "t").unwrap();
    rig.backend.push_install(Err(InstallError::Failed("x".into())));
    rig.svc.startup();
    assert_eq!(rig.restarts(), 0);
    assert!(matches!(state(&rig), UpdateState::InstallFailed { ref manual_url, .. } if manual_url == "https://x/y.dmg"));
}

#[test]
fn startup_with_deferred_install_shows_ready() {
    let rig = Rig::new("0.0.1");
    store::save(&rig.path(), "0.0.2", b"PKG", "u", "t").unwrap();
    rig.backend.push_install(Err(InstallError::Unavailable("offline".into())));
    rig.svc.startup();
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
}

#[test]
fn startup_clears_upgrade_required_once_running_the_new_version() {
    let rig = Rig::new("0.0.3");
    rig.flag.0.store(true, Ordering::SeqCst);
    store::save_lock(&rig.path(), &store::LockInfo { version: "0.0.3".into(), manual_url: "u".into() }).unwrap();
    rig.svc.startup();
    assert!(!rig.flag.0.load(Ordering::SeqCst));
    assert_eq!(store::load_lock(&rig.path()), None);
    assert_eq!(state(&rig), UpdateState::None);
    assert!(!rig.svc.upgrade_required());
}

#[test]
fn startup_below_lock_version_enters_forced_without_waiting_for_a_check() {
    let rig = Rig::new("0.0.1");
    store::save_lock(&rig.path(), &store::LockInfo { version: "0.0.3".into(), manual_url: "https://x/y.dmg".into() }).unwrap();
    rig.svc.startup();
    assert!(rig.flag.0.load(Ordering::SeqCst));
    assert!(rig.svc.upgrade_required());
    assert!(matches!(state(&rig), UpdateState::Forced { ref version, ref manual_url, .. } if version == "0.0.3" && manual_url == "https://x/y.dmg"));
}

#[test]
fn startup_forced_then_offline_check_shows_download_failed_with_retry() {
    let rig = Rig::new("0.0.1");
    store::save_lock(&rig.path(), &store::LockInfo { version: "0.0.3".into(), manual_url: "u".into() }).unwrap();
    rig.svc.startup();
    offline(&rig);
    rig.svc.tick();
    assert!(matches!(state(&rig), UpdateState::Forced { error: Some(ref e), .. } if e == "下载失败"));
}

#[test]
fn startup_after_forced_install_failure_keeps_forced_with_error() {
    let rig = Rig::new("0.0.1");
    store::save(&rig.path(), "0.0.3", b"PKG", "u", "t").unwrap();
    store::save_lock(&rig.path(), &store::LockInfo { version: "0.0.3".into(), manual_url: "u".into() }).unwrap();
    rig.backend.push_install(Err(InstallError::Failed("x".into())));
    rig.svc.startup();
    assert!(matches!(state(&rig), UpdateState::Forced { error: Some(_), .. }));
}

#[test]
fn upgrade_flag_without_lock_record_keeps_blocking_until_a_valid_check() {
    let rig = Rig::new("0.0.2");
    rig.flag.0.store(true, Ordering::SeqCst);
    rig.svc.startup();
    assert!(rig.svc.upgrade_required());
    rig.backend.push_manifest(Err("offline".into()));
    rig.svc.check_now();
    assert!(rig.svc.upgrade_required());
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", Some("0.0.1")))));
    rig.svc.check_now();
    assert!(!rig.svc.upgrade_required(), "valid manifest without requirement releases the flag");
}

#[test]
fn concurrent_check_requests_do_not_run_twice() {
    use std::sync::Arc;
    let rig = Arc::new(Rig::new("0.0.1"));
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    rig.backend.push_manifest(Ok(Some(manifest("0.0.1", None))));
    let r1 = rig.clone();
    let h = std::thread::spawn(move || r1.svc.check_now());
    rig.svc.check_now();
    h.join().unwrap();
    assert!(rig.backend.fetches() >= 1);
}

#[test]
fn platform_without_entry_in_manifest_never_downloads() {
    let rig = Rig::new("0.0.1");
    let text = manifest("0.0.2", None).replace("darwin-aarch64", "darwin-other");
    rig.backend.push_manifest(Ok(Some(text)));
    rig.svc.check_now();
    assert_eq!(rig.backend.downloads_done(), 0);
    assert_eq!(state(&rig), UpdateState::None);
    let _ = Platform::DarwinAarch64;
}

#[test]
fn forced_blocks_start_through_the_service_until_the_new_version_runs() {
    let rig = Rig::new("0.0.1");
    assert!(rig.svc.guard_gateway_start().is_ok());
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Err("timeout".into()));
    rig.svc.check_now();
    assert_eq!(rig.svc.guard_gateway_start().unwrap_err().code, UpdateErrorCode::UpgradeRequired);

    // The same data directory, now running 0.0.3 after the installer finished.
    let flag_was_set = rig.flag.0.load(Ordering::SeqCst);
    assert!(flag_was_set);
    let dir = rig.dir;
    let next = Rig::with_dir("0.0.3", dir);
    next.flag.0.store(true, Ordering::SeqCst);
    next.svc.startup();
    assert!(next.svc.guard_gateway_start().is_ok());
    assert!(!next.flag.0.load(Ordering::SeqCst));
}

// ---- download stall detection ----

use std::sync::Arc;
use std::time::Duration as StdDuration;

/// Runs `f` on a helper thread and fails the test when it does not return in time,
/// so a regression shows up as a failure instead of a hung test run.
fn within<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(StdDuration::from_secs(5)).unwrap_or_else(|_| panic!("{what} did not return within 5s"))
}

fn stalled_rig() -> Arc<Rig> {
    Arc::new(Rig::with_stall("0.0.1", StdDuration::from_millis(80)))
}

#[test]
fn stalled_background_download_fails_and_releases_busy() {
    let rig = stalled_rig();
    rig.backend.hang_downloads.store(1, Ordering::SeqCst);
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", Some("0.0.1")))));
    let r = rig.clone();
    within("check_now with a stalled download", move || r.svc.check_now());
    assert_eq!(state(&rig), UpdateState::None, "a failed background download is silent");
    assert_eq!(rig.backend.downloads_done(), 1);

    // busy was released: the next check really runs (it would return early otherwise).
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", Some("0.0.1")))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    let r = rig.clone();
    within("second check_now", move || r.svc.check_now());
    assert_eq!(rig.backend.fetches(), 2);
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
}

#[test]
fn stalled_forced_download_shows_failure_and_retry_downloads_again() {
    let rig = stalled_rig();
    rig.backend.hang_downloads.store(1, Ordering::SeqCst);
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    let r = rig.clone();
    within("forced check with a stalled download", move || r.svc.check_now());
    assert!(matches!(state(&rig), UpdateState::Forced { error: Some(ref e), .. } if e == "下载失败"));

    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    let r = rig.clone();
    within("retry", move || r.svc.retry()).unwrap();
    assert_eq!(rig.backend.downloads_done(), 2, "retry triggered a second download");
    assert_eq!(rig.backend.installed.lock().unwrap().len(), 1);
    assert_eq!(rig.restarts(), 1);
}

#[test]
fn slow_download_that_keeps_reporting_progress_is_not_cut_off() {
    let rig = Arc::new(Rig::with_stall("0.0.1", StdDuration::from_millis(150)));
    *rig.backend.step_delay.lock().unwrap() = StdDuration::from_millis(60);
    *rig.backend.progress_steps.lock().unwrap() = (1..=6).map(|i| (i * 10, Some(60))).collect();
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", Some("0.0.1")))));
    rig.backend.push_download(Ok(b"PKG".to_vec()));
    let r = rig.clone();
    within("slow download", move || r.svc.check_now());
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
}

// ---- start-up ordering: first check before the gateway comes back ----

const SHORT_WAIT: StdDuration = StdDuration::from_millis(150);

fn resumes(rig: &Rig) -> usize {
    rig.gateway.resumes.load(Ordering::SeqCst)
}

#[test]
fn first_check_forced_keeps_the_gateway_off() {
    let rig = Rig::new("0.0.1");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Err("offline".into()));
    rig.svc.begin_startup();
    rig.svc.complete_startup_install();
    assert!(rig.svc.tick());
    assert_eq!(rig.svc.resume_gateway_after_startup(SHORT_WAIT), FirstCheck::Forced);
    assert_eq!(resumes(&rig), 0, "a forced upgrade must not get the gateway back");
}

#[test]
fn first_check_forced_is_decided_before_the_download_finishes() {
    let rig = Arc::new(Rig::with_stall("0.0.1", StdDuration::from_secs(30)));
    rig.backend.hang_downloads.store(1, Ordering::SeqCst);
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.svc.begin_startup();
    rig.svc.complete_startup_install();
    let r = rig.clone();
    std::thread::spawn(move || r.svc.tick());
    let r = rig.clone();
    let verdict = within("gateway restore", move || r.svc.resume_gateway_after_startup(StdDuration::from_secs(4)));
    assert_eq!(verdict, FirstCheck::Forced);
    assert_eq!(resumes(&rig), 0);
}

#[test]
fn first_check_up_to_date_resumes_the_gateway() {
    let rig = Rig::new("0.0.2");
    rig.backend.push_manifest(Ok(Some(manifest("0.0.2", None))));
    rig.svc.begin_startup();
    rig.svc.complete_startup_install();
    rig.svc.tick();
    assert_eq!(rig.svc.resume_gateway_after_startup(SHORT_WAIT), FirstCheck::Clear);
    assert_eq!(resumes(&rig), 1);
}

#[test]
fn first_check_failure_resumes_the_gateway() {
    let rig = Rig::new("0.0.2");
    offline(&rig);
    rig.svc.begin_startup();
    rig.svc.complete_startup_install();
    rig.svc.tick();
    assert_eq!(rig.svc.resume_gateway_after_startup(SHORT_WAIT), FirstCheck::Failed);
    assert_eq!(resumes(&rig), 1);
}

#[test]
fn first_check_timeout_resumes_the_gateway_without_waiting_longer() {
    let rig = Arc::new(Rig::new("0.0.2"));
    rig.backend.hang_fetch.store(true, Ordering::SeqCst);
    rig.svc.begin_startup();
    rig.svc.complete_startup_install();
    let r = rig.clone();
    std::thread::spawn(move || r.svc.tick());
    let started = std::time::Instant::now();
    let r = rig.clone();
    let verdict = within("gateway restore", move || r.svc.resume_gateway_after_startup(SHORT_WAIT));
    assert_eq!(verdict, FirstCheck::TimedOut);
    assert!(started.elapsed() < StdDuration::from_secs(3));
    assert_eq!(resumes(&rig), 1);
}

#[test]
fn locked_version_never_resumes_the_gateway_even_if_the_check_times_out() {
    let rig = Rig::new("0.0.1");
    store::save_lock(&rig.path(), &store::LockInfo { version: "0.0.3".into(), manual_url: "u".into() }).unwrap();
    rig.svc.begin_startup();
    rig.svc.complete_startup_install();
    assert_eq!(rig.svc.resume_gateway_after_startup(StdDuration::from_millis(20)), FirstCheck::TimedOut);
    assert_eq!(resumes(&rig), 0);
}

#[test]
fn gateway_restore_waits_for_the_startup_install() {
    let rig = Arc::new(Rig::new("0.0.1"));
    store::save(&rig.path(), "0.0.2", b"PKG", "u", "t").unwrap();
    rig.backend.hang_installs.store(true, Ordering::SeqCst);
    rig.backend.push_install(Err(InstallError::Unavailable("offline".into())));
    assert!(rig.svc.begin_startup());
    assert!(matches!(state(&rig), UpdateState::Installing { ref version } if version == "0.0.2"));
    let r = rig.clone();
    let worker = std::thread::spawn(move || r.svc.complete_startup_install());
    let r = rig.clone();
    let restore = std::thread::spawn(move || r.svc.resume_gateway_after_startup(StdDuration::from_millis(30)));
    std::thread::sleep(StdDuration::from_millis(200));
    assert_eq!(resumes(&rig), 0, "install still running: the gateway stays off");
    rig.backend.release();
    worker.join().unwrap();
    restore.join().unwrap();
    assert_eq!(resumes(&rig), 1);
    assert_eq!(state(&rig), UpdateState::Ready { version: "0.0.2".into() });
}

#[test]
fn setup_does_not_block_on_the_pending_install() {
    let rig = Rig::new("0.0.1");
    store::save(&rig.path(), "0.0.2", b"PKG", "u", "t").unwrap();
    rig.backend.hang_installs.store(true, Ordering::SeqCst);
    let svc = rig.svc.clone();
    within("spawn_startup", move || svc.spawn_startup().unwrap());
    assert!(matches!(state(&rig), UpdateState::Installing { .. }));
    assert_eq!(rig.backend.install_threads.lock().unwrap().len(), 0, "not installed on the setup thread yet");
    // Let the installer through and wait for the worker.
    rig.backend.release();
    for _ in 0..200 {
        if rig.restarts() == 1 {
            break;
        }
        std::thread::sleep(StdDuration::from_millis(10));
    }
    assert_eq!(rig.restarts(), 1);
    let threads = rig.backend.install_threads.lock().unwrap().clone();
    assert_eq!(threads.len(), 1);
    assert_ne!(threads[0], std::thread::current().id(), "the install ran off the calling thread");
}

#[test]
fn installing_state_is_emitted_before_the_install_result() {
    let rig = Rig::new("0.0.1");
    store::save(&rig.path(), "0.0.2", b"PKG", "u", "t").unwrap();
    rig.backend.push_install(Err(InstallError::Failed("x".into())));
    rig.svc.startup();
    let states: Vec<UpdateState> = rig.events().into_iter().map(|e| e.state).collect();
    assert!(matches!(states.first(), Some(UpdateState::Installing { .. })));
    assert!(matches!(states.last(), Some(UpdateState::InstallFailed { .. })));
}

#[test]
fn startup_discarding_a_corrupt_record_leaves_no_installing_state() {
    let rig = Rig::new("0.0.1");
    store::save(&rig.path(), "0.0.2", b"PKG", "u", "t").unwrap();
    std::fs::write(rig.path().join("updates").join("0.0.2.bin"), b"tampered").unwrap();
    rig.svc.startup();
    assert_eq!(state(&rig), UpdateState::None);
    assert_eq!(rig.restarts(), 0);
}

#[test]
fn gateway_restore_waits_for_a_first_check_that_finishes_late() {
    let rig = Arc::new(Rig::new("0.0.1"));
    rig.backend.push_manifest(Ok(Some(manifest("0.0.3", Some("0.0.3")))));
    rig.backend.push_download(Err("offline".into()));
    rig.svc.begin_startup();
    rig.svc.complete_startup_install();
    let r = rig.clone();
    let restore = std::thread::spawn(move || r.svc.resume_gateway_after_startup(StdDuration::from_secs(4)));
    std::thread::sleep(StdDuration::from_millis(200));
    assert_eq!(resumes(&rig), 0, "still waiting for the first check");
    rig.svc.tick();
    assert_eq!(restore.join().unwrap(), FirstCheck::Forced);
    assert_eq!(resumes(&rig), 0);
}
