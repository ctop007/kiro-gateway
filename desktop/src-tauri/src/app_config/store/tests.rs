use super::*;
use crate::app_config::model::PartialModels;
use crate::app_config::paths::AppPaths;
use chrono::{NaiveDate, NaiveDateTime};
use std::fs;
use tempfile::{tempdir, TempDir};

fn now() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_opt(14, 41, 8).unwrap()
}

fn setup() -> (TempDir, AppPaths) {
    let dir = tempdir().unwrap();
    let paths = AppPaths::new(dir.path().join("data"));
    (dir, paths)
}

fn names(paths: &AppPaths) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(paths.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

fn env_lines(paths: &AppPaths) -> Vec<String> {
    fs::read_to_string(paths.kclaude_env_file()).unwrap().lines().map(str::to_string).collect()
}

#[test]
fn first_open_creates_defaults_secret_and_env_file() {
    let (_d, paths) = setup();
    let (store, notes) = ConfigStore::open(paths.clone(), now()).unwrap();
    assert_eq!(store.get(), AppConfig::default());
    assert_eq!(notes, vec![OpenNote::SecretCreated]);
    assert_eq!(names(&paths), vec!["config.json", "kclaude.env", "secret"]);
    let secret = fs::read_to_string(paths.secret_file()).unwrap();
    assert_eq!(secret.len(), 64);
    assert!(env_lines(&paths).contains(&format!("KIRO_GATEWAY_KEY={secret}")));
    assert!(env_lines(&paths).contains(&"KIRO_GATEWAY_PORT=8000".to_string()));
}

#[test]
fn config_json_on_disk_uses_the_contract_shape() {
    let (_d, paths) = setup();
    ConfigStore::open(paths.clone(), now()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(paths.config_file()).unwrap()).unwrap();
    assert_eq!(value["port"], 8000);
    assert_eq!(value["models"]["opus"], "claude-opus-5-5[1m]");
    assert_eq!(value["gatewayWanted"], false);
    assert_eq!(value["upgradeRequired"], false);
    assert!(value.get("secret").is_none(), "the key must never be stored in config.json");
}

#[test]
fn reopening_keeps_config_and_secret_unchanged() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    let secret = store.secret();
    store.update(&ConfigPatch { port: Some(8123), ..Default::default() }, false).unwrap();
    drop(store);

    let (again, notes) = ConfigStore::open(paths.clone(), now()).unwrap();
    assert_eq!(again.get().port, 8123);
    assert_eq!(again.secret(), secret);
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn reopening_an_up_to_date_install_writes_nothing() {
    let (_d, paths) = setup();
    ConfigStore::open(paths.clone(), now()).unwrap();
    let stamp = |p: std::path::PathBuf| fs::metadata(p).unwrap().modified().unwrap();
    let before = (stamp(paths.config_file()), stamp(paths.kclaude_env_file()), stamp(paths.secret_file()));
    std::thread::sleep(std::time::Duration::from_millis(30));
    ConfigStore::open(paths.clone(), now()).unwrap();
    let after = (stamp(paths.config_file()), stamp(paths.kclaude_env_file()), stamp(paths.secret_file()));
    assert_eq!(before, after);
}

#[test]
fn a_missing_or_stale_env_file_is_rewritten_on_open() {
    let (_d, paths) = setup();
    ConfigStore::open(paths.clone(), now()).unwrap();
    fs::remove_file(paths.kclaude_env_file()).unwrap();
    ConfigStore::open(paths.clone(), now()).unwrap();
    assert!(paths.kclaude_env_file().exists());

    fs::write(paths.kclaude_env_file(), "KIRO_GATEWAY_PORT=1\n").unwrap();
    ConfigStore::open(paths.clone(), now()).unwrap();
    assert!(env_lines(&paths).contains(&"KIRO_GATEWAY_PORT=8000".to_string()));
}

#[test]
fn corrupt_config_is_backed_up_and_replaced_by_defaults() {
    let (_d, paths) = setup();
    fs::create_dir_all(paths.root()).unwrap();
    fs::write(paths.config_file(), "{ this is not json").unwrap();
    let (store, notes) = ConfigStore::open(paths.clone(), now()).unwrap();
    assert_eq!(store.get(), AppConfig::default());
    let backup = paths.root().join("config.json.bak-20261022-144108");
    assert_eq!(notes, vec![OpenNote::ConfigCorrupt { backup_kept: true }, OpenNote::SecretCreated]);
    assert_eq!(fs::read_to_string(&backup).unwrap(), "{ this is not json");
    // The new config.json is valid defaults.
    let reloaded: AppConfig = serde_json::from_str(&fs::read_to_string(paths.config_file()).unwrap()).unwrap();
    assert_eq!(reloaded, AppConfig::default());
}

#[test]
fn every_kind_of_corruption_is_handled() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", b"".to_vec()),
        ("truncated", b"{\"port\": 81".to_vec()),
        ("array", b"[1,2,3]".to_vec()),
        ("null", b"null".to_vec()),
        ("wrong type", b"{\"port\": \"eight thousand\"}".to_vec()),
        ("port out of range", b"{\"port\": 80}".to_vec()),
        ("port overflow", b"{\"port\": 99999999999}".to_vec()),
        ("empty model", b"{\"models\": {\"opus\": \"\"}}".to_vec()),
        ("unsafe model", b"{\"models\": {\"opus\": \"a b\"}}".to_vec()),
        ("binary", vec![0xff, 0xfe, 0x00, 0x01]),
    ];
    for (label, bytes) in cases {
        let (_d, paths) = setup();
        fs::create_dir_all(paths.root()).unwrap();
        fs::write(paths.config_file(), &bytes).unwrap();
        let (store, notes) = ConfigStore::open(paths.clone(), now()).unwrap();
        assert_eq!(store.get(), AppConfig::default(), "{label}");
        assert!(notes.contains(&OpenNote::ConfigCorrupt { backup_kept: true }), "{label}: {notes:?}");
        let backups: Vec<_> = names(&paths).into_iter().filter(|n| n.starts_with("config.json.bak-")).collect();
        assert_eq!(backups.len(), 1, "{label}");
        assert_eq!(fs::read(paths.root().join(&backups[0])).unwrap(), bytes, "{label}: backup must be byte-identical");
    }
}

#[test]
fn two_corruptions_in_the_same_second_keep_both_backups() {
    let (_d, paths) = setup();
    fs::create_dir_all(paths.root()).unwrap();
    for text in ["first{", "second{"] {
        fs::write(paths.config_file(), text).unwrap();
        ConfigStore::open(paths.clone(), now()).unwrap();
    }
    let mut contents: Vec<String> = names(&paths)
        .into_iter()
        .filter(|n| n.starts_with("config.json.bak-"))
        .map(|n| fs::read_to_string(paths.root().join(n)).unwrap())
        .collect();
    contents.sort();
    assert_eq!(contents, vec!["first{".to_string(), "second{".to_string()]);
}

#[test]
fn corrupt_config_does_not_regenerate_a_good_secret() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    let secret = store.secret();
    drop(store);
    fs::write(paths.config_file(), "garbage").unwrap();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    assert_eq!(store.secret(), secret);
}

#[test]
fn a_garbled_secret_file_is_regenerated_and_reported() {
    let (_d, paths) = setup();
    fs::create_dir_all(paths.root()).unwrap();
    fs::write(paths.secret_file(), "short").unwrap();
    let (store, notes) = ConfigStore::open(paths.clone(), now()).unwrap();
    assert!(notes.contains(&OpenNote::SecretRegenerated));
    assert!(env_lines(&paths).contains(&format!("KIRO_GATEWAY_KEY={}", store.secret())));
}

#[test]
fn update_changes_only_patched_fields_and_persists_them() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    let outcome = store
        .update(
            &ConfigPatch {
                port: Some(8150),
                models: Some(PartialModels { opus: Some("claude-opus-4.5".into()), ..Default::default() }),
                autostart: Some(true),
            },
            false,
        )
        .unwrap();
    assert_eq!(outcome.config.port, 8150);
    assert_eq!(outcome.config.models.opus, "claude-opus-4.5");
    assert_eq!(outcome.config.models.sonnet, "claude-sonnet-5-5[1m]");
    assert!(outcome.config.autostart);
    assert!(!outcome.restart_required);
    let on_disk: AppConfig = serde_json::from_str(&fs::read_to_string(paths.config_file()).unwrap()).unwrap();
    assert_eq!(on_disk, outcome.config);
}

#[test]
fn update_rewrites_kclaude_env_with_the_new_values() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    store
        .update(
            &ConfigPatch {
                port: Some(8100),
                models: Some(PartialModels { haiku: Some("claude-haiku-4.5".into()), ..Default::default() }),
                autostart: None,
            },
            false,
        )
        .unwrap();
    let lines = env_lines(&paths);
    assert!(lines.contains(&"KIRO_GATEWAY_PORT=8100".to_string()));
    assert!(lines.contains(&"KCLAUDE_HAIKU_MODEL=claude-haiku-4.5".to_string()));
    assert!(lines.contains(&format!("KIRO_GATEWAY_KEY={}", store.secret())));
}

#[test]
fn restart_required_only_when_the_port_changes_while_running() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths, now()).unwrap();
    let port = |p| ConfigPatch { port: Some(p), ..Default::default() };
    assert!(!store.update(&port(8101), false).unwrap().restart_required, "stopped gateway");
    assert!(store.update(&port(8102), true).unwrap().restart_required, "running + port changed");
    assert!(!store.update(&port(8102), true).unwrap().restart_required, "same port");
    let models = ConfigPatch {
        models: Some(PartialModels { opus: Some("x-opus".into()), ..Default::default() }),
        ..Default::default()
    };
    assert!(!store.update(&models, true).unwrap().restart_required, "models need no restart");
}

#[test]
fn invalid_patch_is_rejected_and_nothing_changes() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    let config_before = fs::read(paths.config_file()).unwrap();
    let env_before = fs::read(paths.kclaude_env_file()).unwrap();
    let err = store
        .update(&ConfigPatch { port: Some(80), autostart: Some(true), ..Default::default() }, false)
        .unwrap_err();
    assert_eq!(err.code(), "invalid_argument");
    assert_eq!(store.get(), AppConfig::default());
    assert_eq!(fs::read(paths.config_file()).unwrap(), config_before);
    assert_eq!(fs::read(paths.kclaude_env_file()).unwrap(), env_before);
}

#[test]
fn gateway_wanted_is_persisted_and_does_not_touch_the_env_file_content() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    let env_before = fs::read(paths.kclaude_env_file()).unwrap();
    store.set_gateway_wanted(true).unwrap();
    assert!(store.get().gateway_wanted);
    drop(store);
    let (again, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    assert!(again.get().gateway_wanted);
    assert_eq!(fs::read(paths.kclaude_env_file()).unwrap(), env_before);
}

#[test]
fn set_gateway_wanted_is_idempotent_and_skips_the_write() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    store.set_gateway_wanted(false).unwrap();
    let stamp = fs::metadata(paths.config_file()).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    store.set_gateway_wanted(false).unwrap();
    assert_eq!(fs::metadata(paths.config_file()).unwrap().modified().unwrap(), stamp);
}

#[test]
fn upgrade_required_is_written_to_both_files_and_survives_reopen() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    assert!(!store.upgrade_required());
    store.set_upgrade_required(true).unwrap();
    assert!(store.upgrade_required());
    assert!(env_lines(&paths).contains(&"KIRO_GATEWAY_UPGRADE_REQUIRED=1".to_string()));
    let on_disk: serde_json::Value = serde_json::from_str(&fs::read_to_string(paths.config_file()).unwrap()).unwrap();
    assert_eq!(on_disk["upgradeRequired"], true);
    drop(store);

    let (again, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    assert!(again.upgrade_required());
    again.set_upgrade_required(false).unwrap();
    assert!(env_lines(&paths).contains(&"KIRO_GATEWAY_UPGRADE_REQUIRED=0".to_string()));
}

#[test]
fn a_user_patch_cannot_clear_upgrade_required() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths, now()).unwrap();
    store.set_upgrade_required(true).unwrap();
    let patch: ConfigPatch = serde_json::from_str(r#"{"upgradeRequired": false, "port": 8101}"#).unwrap();
    let outcome = store.update(&patch, false).unwrap();
    assert!(outcome.config.upgrade_required);
    assert_eq!(outcome.config.port, 8101);
}

#[test]
fn sync_autostart_records_the_actual_system_state() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    store.update(&ConfigPatch { autostart: Some(true), ..Default::default() }, false).unwrap();
    assert!(store.get().autostart);
    // The OS refused: the stored value must follow reality.
    let config = store.sync_autostart(false).unwrap();
    assert!(!config.autostart);
    let on_disk: AppConfig = serde_json::from_str(&fs::read_to_string(paths.config_file()).unwrap()).unwrap();
    assert!(!on_disk.autostart);
}

#[cfg(unix)]
#[test]
fn env_file_and_secret_are_0600_and_config_is_not_world_writable() {
    use std::os::unix::fs::PermissionsExt;
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    store.update(&ConfigPatch { port: Some(8101), ..Default::default() }, false).unwrap();
    let mode = |p: std::path::PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(paths.secret_file()), 0o600);
    assert_eq!(mode(paths.kclaude_env_file()), 0o600);
    assert_eq!(mode(paths.config_file()) & 0o022, 0);
}

#[test]
fn no_temporary_files_are_left_after_many_updates() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    for port in 8101..8121 {
        store.update(&ConfigPatch { port: Some(port), ..Default::default() }, false).unwrap();
    }
    assert_eq!(names(&paths), vec!["config.json", "kclaude.env", "secret"]);
}

#[test]
fn write_failure_is_reported_and_memory_keeps_the_old_config() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    // Make config.json a non-empty directory so the atomic rename fails.
    fs::remove_file(paths.config_file()).unwrap();
    fs::create_dir(paths.config_file()).unwrap();
    fs::write(paths.config_file().join("x"), "x").unwrap();
    let err = store.update(&ConfigPatch { port: Some(8200), ..Default::default() }, false).unwrap_err();
    assert_eq!(err.code(), "write_failed");
    assert_eq!(store.get().port, 8000);
    assert!(!err.message().contains('/'));
}

#[test]
fn open_fails_clearly_when_the_data_dir_cannot_be_created() {
    let dir = tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "a file, not a directory").unwrap();
    let err = ConfigStore::open(AppPaths::new(blocker.join("data")), now()).err().unwrap();
    assert_eq!(err.code(), "internal");
}

#[test]
fn concurrent_updates_never_lose_or_tear_the_file() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    let store = std::sync::Arc::new(store);
    let handles: Vec<_> = (0..8u16)
        .map(|i| {
            let store = store.clone();
            std::thread::spawn(move || {
                for n in 0..10u16 {
                    store
                        .update(&ConfigPatch { port: Some(i64::from(9000 + i * 10 + n)), ..Default::default() }, false)
                        .unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let on_disk: AppConfig = serde_json::from_str(&fs::read_to_string(paths.config_file()).unwrap()).unwrap();
    assert_eq!(on_disk, store.get());
    let env_port = format!("KIRO_GATEWAY_PORT={}", store.get().port);
    assert!(env_lines(&paths).contains(&env_port), "env file must match the last committed config");
}

#[test]
fn check_patch_validates_without_changing_anything() {
    let (_d, paths) = setup();
    let (store, _) = ConfigStore::open(paths.clone(), now()).unwrap();
    let before = fs::read(paths.config_file()).unwrap();
    assert!(store.check_patch(&ConfigPatch { port: Some(8100), autostart: Some(true), ..Default::default() }).is_ok());
    assert_eq!(store.check_patch(&ConfigPatch { port: Some(80), ..Default::default() }).unwrap_err().code(), "invalid_argument");
    assert_eq!(store.get(), AppConfig::default());
    assert_eq!(fs::read(paths.config_file()).unwrap(), before);
}
