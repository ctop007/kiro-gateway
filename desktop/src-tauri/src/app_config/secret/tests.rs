use super::*;
use crate::app_config::paths::AppPaths;
use std::fs;
use tempfile::tempdir;

#[test]
fn generated_secret_is_64_lowercase_hex_chars() {
    let secret = generate_secret().unwrap();
    assert_eq!(secret.len(), 64);
    assert!(secret.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)), "{secret}");
}

#[test]
fn generated_secrets_differ() {
    assert_ne!(generate_secret().unwrap(), generate_secret().unwrap());
}

#[test]
fn accepts_hex_and_hand_set_tokens() {
    assert!(is_valid_secret(&"a".repeat(64)));
    assert!(is_valid_secret("my-hand-set_token.0123456789"));
    assert!(is_valid_secret(&"x".repeat(128)));
}

#[test]
fn rejects_unsafe_or_malformed_secrets() {
    assert!(!is_valid_secret(""));
    assert!(!is_valid_secret("short"));
    assert!(!is_valid_secret(&"x".repeat(129)));
    assert!(!is_valid_secret("has space in it 0123456789"));
    assert!(!is_valid_secret("line\nbreak0123456789ab"));
    assert!(!is_valid_secret("quote'0123456789abcdef"));
    assert!(!is_valid_secret("dollar$0123456789abcdef"));
    assert!(!is_valid_secret("semi;colon0123456789abcd"));
    assert!(!is_valid_secret("unicode-密钥-0123456789abcd"));
}

#[test]
fn first_run_creates_the_file_and_reports_created() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::new(dir.path());
    let (secret, origin) = load_or_create(&paths).unwrap();
    assert_eq!(origin, SecretOrigin::Created);
    assert_eq!(fs::read_to_string(paths.secret_file()).unwrap(), secret);
    assert!(is_valid_secret(&secret));
}

#[cfg(unix)]
#[test]
fn created_file_is_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let paths = AppPaths::new(dir.path());
    load_or_create(&paths).unwrap();
    let mode = fs::metadata(paths.secret_file()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn second_run_returns_the_same_secret_without_rewriting() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::new(dir.path());
    let (first, _) = load_or_create(&paths).unwrap();
    let before = fs::metadata(paths.secret_file()).unwrap().modified().unwrap();
    let (second, origin) = load_or_create(&paths).unwrap();
    assert_eq!(first, second);
    assert_eq!(origin, SecretOrigin::Existing);
    assert_eq!(fs::metadata(paths.secret_file()).unwrap().modified().unwrap(), before);
}

#[test]
fn trailing_newline_from_a_manual_edit_is_tolerated() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::new(dir.path());
    let token = "0123456789abcdef0123456789abcdef";
    fs::write(paths.secret_file(), format!("{token}\n")).unwrap();
    let (secret, origin) = load_or_create(&paths).unwrap();
    assert_eq!(secret, token);
    assert_eq!(origin, SecretOrigin::Existing);
}

#[cfg(unix)]
#[test]
fn loose_permissions_on_an_existing_secret_are_tightened() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let paths = AppPaths::new(dir.path());
    let token = "0123456789abcdef0123456789abcdef";
    fs::write(paths.secret_file(), token).unwrap();
    fs::set_permissions(paths.secret_file(), fs::Permissions::from_mode(0o644)).unwrap();
    let (secret, _) = load_or_create(&paths).unwrap();
    assert_eq!(secret, token);
    assert_eq!(fs::metadata(paths.secret_file()).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn empty_or_garbled_file_is_regenerated() {
    for bad in ["", "   \n", "short", "has space in the middle of it 123456", "密钥"] {
        let dir = tempdir().unwrap();
        let paths = AppPaths::new(dir.path());
        fs::write(paths.secret_file(), bad).unwrap();
        let (secret, origin) = load_or_create(&paths).unwrap();
        assert_eq!(origin, SecretOrigin::Regenerated, "{bad:?}");
        assert!(is_valid_secret(&secret));
        assert_eq!(fs::read_to_string(paths.secret_file()).unwrap(), secret);
    }
}

#[test]
fn non_utf8_file_is_regenerated() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::new(dir.path());
    fs::write(paths.secret_file(), [0xff, 0xfe, 0x00, 0x80]).unwrap();
    let (_, origin) = load_or_create(&paths).unwrap();
    assert_eq!(origin, SecretOrigin::Regenerated);
}

#[test]
fn a_directory_in_place_of_the_file_is_an_error() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::new(dir.path());
    fs::create_dir(paths.secret_file()).unwrap();
    assert!(load_or_create(&paths).is_err());
}
