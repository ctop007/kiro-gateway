use super::*;
use crate::update::types::{DownloadProgress, UpdateState};
use serde_json::json;

const P: Platform = Platform::DarwinAarch64;

fn manifest(version: &str, min: Option<&str>) -> String {
    let mut m = json!({
        "version": version,
        "notes": "line one\nline two",
        "pub_date": "2026-11-01T10:00:00+08:00",
        "platforms": {
            "darwin-aarch64": { "url": "https://cos.example/0.0.2/Kiro-Gateway.app.tar.gz", "signature": "SIG-A" },
            "darwin-x86_64": { "url": "https://cos.example/0.0.2/Kiro-Gateway-x64.app.tar.gz", "signature": "SIG-X" },
            "windows-x86_64": { "url": "https://cos.example/0.0.2/Kiro-Gateway_0.0.2_x64-setup.exe", "signature": "SIG-W" }
        },
        "manualDownload": {
            "darwin": "https://cos.example/0.0.2/Kiro-Gateway_0.0.2_universal.dmg",
            "windows": "https://cos.example/0.0.2/Kiro-Gateway_0.0.2_x64-setup.exe"
        }
    });
    if let Some(min) = min {
        m["minRequiredVersion"] = json!(min);
    }
    m.to_string()
}

fn with(text: &str, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut v: serde_json::Value = serde_json::from_str(text).unwrap();
    edit(&mut v);
    v.to_string()
}

#[test]
fn newer_version_below_min_floor_is_background() {
    let d = decide_offer("0.0.1", &manifest("0.0.2", Some("0.0.1")), P);
    match d {
        Decision::Background(o) => {
            assert_eq!(o.version, "0.0.2");
            assert_eq!(o.url, "https://cos.example/0.0.2/Kiro-Gateway.app.tar.gz");
            assert_eq!(o.signature, "SIG-A");
            assert_eq!(o.notes.as_deref(), Some("line one\nline two"));
            assert_eq!(o.manual_url, "https://cos.example/0.0.2/Kiro-Gateway_0.0.2_universal.dmg");
        }
        other => panic!("expected background, got {other:?}"),
    }
}

#[test]
fn equal_version_is_none() {
    assert_eq!(decide_offer("0.0.2", &manifest("0.0.2", Some("0.0.1")), P), Decision::None);
}

#[test]
fn current_newer_than_manifest_is_none() {
    assert_eq!(decide_offer("0.1.0", &manifest("0.0.2", None), P), Decision::None);
}

#[test]
fn current_below_min_required_is_forced() {
    let d = decide_offer("0.0.1", &manifest("0.0.3", Some("0.0.3")), P);
    assert!(matches!(d, Decision::Forced(ref o) if o.version == "0.0.3"));
}

#[test]
fn current_exactly_min_required_is_background_not_forced() {
    let d = decide_offer("0.0.2", &manifest("0.0.3", Some("0.0.2")), P);
    assert!(matches!(d, Decision::Background(_)), "got {d:?}");
}

#[test]
fn current_one_below_min_required_is_forced() {
    let d = decide_offer("0.0.1", &manifest("0.0.3", Some("0.0.2")), P);
    assert!(matches!(d, Decision::Forced(_)), "got {d:?}");
}

#[test]
fn missing_min_required_means_no_forcing() {
    let d = decide_offer("0.0.1", &manifest("0.0.2", None), P);
    assert!(matches!(d, Decision::Background(_)), "got {d:?}");
}

#[test]
fn unparseable_min_required_is_none() {
    assert_eq!(decide_offer("0.0.1", &manifest("0.0.2", Some("soon")), P), Decision::None);
}

#[test]
fn min_required_above_published_version_is_inconsistent_and_none() {
    assert_eq!(decide_offer("0.0.1", &manifest("0.0.2", Some("0.0.9")), P), Decision::None);
}

#[test]
fn forced_requires_a_newer_published_version() {
    // min says "must upgrade" but the manifest version is not newer: nothing to install.
    assert_eq!(decide_offer("0.0.1", &manifest("0.0.1", Some("0.0.2")), P), Decision::None);
}

#[test]
fn missing_platform_entry_is_none() {
    let text = with(&manifest("0.0.2", None), |v| {
        v["platforms"].as_object_mut().unwrap().remove("darwin-aarch64");
    });
    assert_eq!(decide_offer("0.0.1", &text, P), Decision::None);
    assert!(matches!(decide_offer("0.0.1", &text, Platform::WindowsX86_64), Decision::Background(_)));
}

#[test]
fn platform_entry_with_empty_url_or_signature_is_none() {
    let no_sig = with(&manifest("0.0.2", None), |v| v["platforms"]["darwin-aarch64"]["signature"] = json!(""));
    let no_url = with(&manifest("0.0.2", None), |v| v["platforms"]["darwin-aarch64"]["url"] = json!(""));
    let wrong_type = with(&manifest("0.0.2", None), |v| v["platforms"]["darwin-aarch64"]["url"] = json!(7));
    assert_eq!(decide_offer("0.0.1", &no_sig, P), Decision::None);
    assert_eq!(decide_offer("0.0.1", &no_url, P), Decision::None);
    assert_eq!(decide_offer("0.0.1", &wrong_type, P), Decision::None);
}

#[test]
fn malformed_version_strings_are_none() {
    for bad in ["", "1", "1.2", "1.2.3.4", "v1.2.3", "1.2.x", "1.2.3-beta", "1..3", "-1.0.0", "01.0.0", " 1.0.0", "1.0.0 ", "99999999999999999999.0.0"] {
        assert_eq!(decide_offer("0.0.1", &manifest(bad, None), P), Decision::None, "manifest version {bad:?}");
        assert_eq!(decide_offer(bad, &manifest("0.0.2", None), P), Decision::None, "current version {bad:?}");
    }
}

#[test]
fn manifest_that_is_not_json_is_none() {
    for text in ["", "not json", "<html>404</html>", "[]", "null", "\"0.0.2\"", "{"] {
        assert_eq!(decide_offer("0.0.1", text, P), Decision::None, "text {text:?}");
    }
}

#[test]
fn manifest_missing_version_or_platforms_is_none() {
    let no_version = with(&manifest("0.0.2", None), |v| {
        v.as_object_mut().unwrap().remove("version");
    });
    let no_platforms = with(&manifest("0.0.2", None), |v| {
        v.as_object_mut().unwrap().remove("platforms");
    });
    let version_number = with(&manifest("0.0.2", None), |v| v["version"] = json!(2));
    assert_eq!(decide_offer("0.0.1", &no_version, P), Decision::None);
    assert_eq!(decide_offer("0.0.1", &no_platforms, P), Decision::None);
    assert_eq!(decide_offer("0.0.1", &version_number, P), Decision::None);
}

#[test]
fn versions_compare_numerically_not_lexically() {
    assert!(matches!(decide_offer("0.0.9", &manifest("0.0.10", None), P), Decision::Background(_)));
    assert!(matches!(decide_offer("0.9.0", &manifest("0.10.0", None), P), Decision::Background(_)));
    assert!(matches!(decide_offer("9.0.0", &manifest("10.0.0", None), P), Decision::Background(_)));
    assert_eq!(decide_offer("0.0.10", &manifest("0.0.9", None), P), Decision::None);
}

#[test]
fn manual_url_is_chosen_per_platform() {
    let win = decide_offer("0.0.1", &manifest("0.0.2", None), Platform::WindowsX86_64);
    let x64 = decide_offer("0.0.1", &manifest("0.0.2", None), Platform::DarwinX86_64);
    match (win, x64) {
        (Decision::Background(w), Decision::Background(x)) => {
            assert!(w.manual_url.ends_with("x64-setup.exe"));
            assert!(x.manual_url.ends_with("universal.dmg"));
            assert_eq!(x.signature, "SIG-X");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn missing_manual_download_falls_back_to_platform_package_url() {
    let text = with(&manifest("0.0.2", None), |v| {
        v.as_object_mut().unwrap().remove("manualDownload");
    });
    match decide_offer("0.0.1", &text, P) {
        Decision::Background(o) => assert_eq!(o.manual_url, o.url),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn notes_and_pub_date_are_optional() {
    let text = with(&manifest("0.0.2", None), |v| {
        let o = v.as_object_mut().unwrap();
        o.remove("notes");
        o.remove("pub_date");
    });
    match decide_offer("0.0.1", &text, P) {
        Decision::Background(o) => {
            assert_eq!(o.notes, None);
            assert_eq!(o.pub_date, None);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn decide_update_returns_contract_states() {
    assert_eq!(decide_update("0.0.2", &manifest("0.0.2", None), P), UpdateState::None);
    assert_eq!(
        decide_update("0.0.1", &manifest("0.0.2", None), P),
        UpdateState::Downloading { version: "0.0.2".into(), progress: DownloadProgress::start() }
    );
    match decide_update("0.0.1", &manifest("0.0.3", Some("0.0.3")), P) {
        UpdateState::Forced { version, progress, error, manual_url } => {
            assert_eq!(version, "0.0.3");
            assert_eq!(progress, None);
            assert_eq!(error, None);
            assert!(manual_url.ends_with(".dmg"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn platform_keys_round_trip() {
    for (p, key) in [
        (Platform::DarwinAarch64, "darwin-aarch64"),
        (Platform::DarwinX86_64, "darwin-x86_64"),
        (Platform::WindowsX86_64, "windows-x86_64"),
    ] {
        assert_eq!(p.key(), key);
        assert_eq!(Platform::from_key(key), Some(p));
    }
    assert_eq!(Platform::from_key("linux-x86_64"), None);
}

#[test]
fn parse_version_accepts_only_plain_x_y_z() {
    assert_eq!(parse_version("0.0.1"), Some((0, 0, 1)));
    assert_eq!(parse_version("10.20.30"), Some((10, 20, 30)));
    assert_eq!(parse_version("1.2"), None);
    assert_eq!(parse_version("1.2.3+build"), None);
}
