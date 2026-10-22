//! Guards on the hand-written project config: identity, window geometry, CSP,
//! bundle layout, capability surface and version consistency.

use serde_json::Value;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(rel: &str) -> Value {
    let path = manifest_dir().join(rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

fn tauri_conf() -> Value {
    read_json("tauri.conf.json")
}

#[test]
fn identity_matches_product_constants() {
    let conf = tauri_conf();
    assert_eq!(conf["productName"], "Kiro Gateway");
    assert_eq!(conf["identifier"], "dev.kirogateway.desktop");
}

#[test]
fn version_is_consistent_across_three_files() {
    let conf_version = tauri_conf()["version"].as_str().unwrap().to_string();
    let pkg_version = read_json("../package.json")["version"].as_str().unwrap().to_string();
    let cargo_version = env!("CARGO_PKG_VERSION");
    assert_eq!(conf_version, "0.0.1");
    assert_eq!(pkg_version, conf_version, "package.json vs tauri.conf.json");
    assert_eq!(cargo_version, conf_version, "Cargo.toml vs tauri.conf.json");
}

#[test]
fn single_main_window_with_required_geometry() {
    let conf = tauri_conf();
    let windows = conf["app"]["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 1, "exactly one window");
    let w = &windows[0];
    assert_eq!(w["label"], crate::window::MAIN_WINDOW_LABEL);
    assert_eq!(w["width"], 880);
    assert_eq!(w["height"], 600);
    assert_eq!(w["minWidth"], 760);
    assert_eq!(w["minHeight"], 560);
    assert_eq!(w["resizable"], true);
}

#[test]
fn csp_allows_no_remote_or_localhost_connections() {
    let csp = tauri_conf()["app"]["security"]["csp"].as_str().unwrap().to_string();
    let connect = csp
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("connect-src"))
        .expect("connect-src directive present");
    let sources: Vec<&str> = connect.split_whitespace().skip(1).collect();
    // Exactly the IPC channels: no https:, http:, localhost or 127.0.0.1.
    // Local gateway calls (e.g. /v1/models) go through Rust commands.
    assert_eq!(sources, vec!["ipc:", "http://ipc.localhost"]);
    assert!(!csp.contains("unsafe-eval"));
}

#[test]
fn bundle_targets_and_gateway_resource_dir() {
    let conf = tauri_conf();
    let targets: Vec<&str> = conf["bundle"]["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(targets, vec!["dmg", "nsis"]);
    assert_eq!(conf["bundle"]["resources"]["resources/gateway/"], "gateway/");
    assert!(
        manifest_dir().join("resources/gateway/.gitkeep").is_file(),
        "resources/gateway/.gitkeep keeps the empty dir buildable"
    );
}

#[test]
fn update_manifest_url_is_declared_once() {
    // The full URL lives only in tauri.conf.json; here we check its shape.
    let conf = tauri_conf();
    let endpoints = conf["plugins"]["updater"]["endpoints"].as_array().unwrap();
    assert_eq!(endpoints.len(), 1, "exactly one manifest endpoint");
    let url = endpoints[0].as_str().unwrap();
    assert!(url.starts_with("https://"), "manifest must be fetched over https");
    assert!(url.contains(".cos.") && url.contains(".myqcloud.com/"), "manifest is on Tencent COS");
    assert!(url.ends_with("/kiro-gateway/desktop/latest.json"));
    assert!(!url.contains("{{"), "static manifest: no updater URL template variables");
}

#[test]
fn capabilities_do_not_expose_fs_or_shell() {
    let caps = read_json("capabilities/default.json");
    let perms = caps["permissions"].as_array().unwrap();
    for p in perms {
        let id = p
            .as_str()
            .map(str::to_string)
            .or_else(|| p["identifier"].as_str().map(str::to_string))
            .expect("permission has an identifier");
        assert!(
            !(id.starts_with("fs:") || id.starts_with("shell:") || id.starts_with("http:")),
            "forbidden capability {id}"
        );
    }
}

#[test]
fn opener_is_limited_to_https_urls() {
    let caps = read_json("capabilities/default.json");
    let opener = caps["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["identifier"] == "opener:allow-open-url")
        .expect("opener permission");
    let urls: Vec<&str> = opener["allow"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["url"].as_str().unwrap())
        .collect();
    assert_eq!(urls, vec!["https://*"]);
}
