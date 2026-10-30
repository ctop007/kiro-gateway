use super::*;
use std::ffi::OsString;
use std::path::PathBuf;

fn inputs() -> EnvInputs {
    EnvInputs {
        secret: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        kiro_cli_db: PathBuf::from("/home/u/.local/share/kiro-cli/data.sqlite3"),
        port: 8000,
        request_log_dir: PathBuf::from("/data/logs/requests"),
        tiktoken_cache_dir: Some(PathBuf::from("/res/gateway/tiktoken_cache")),
    }
}

fn os(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))).collect()
}

fn get<'a>(env: &'a [(OsString, OsString)], key: &str) -> Option<&'a str> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| v.to_str().unwrap())
}

fn keys(env: &[(OsString, OsString)]) -> Vec<String> {
    env.iter().map(|(k, _)| k.to_string_lossy().to_string()).collect()
}

#[test]
fn sets_every_variable_of_design_decision_2() {
    let env = build_env(&inputs(), os(&[]));
    assert_eq!(get(&env, "PROXY_API_KEY"), Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
    assert_eq!(get(&env, "KIRO_CLI_DB_FILE"), Some("/home/u/.local/share/kiro-cli/data.sqlite3"));
    assert_eq!(get(&env, "SERVER_HOST"), Some("127.0.0.1"));
    assert_eq!(get(&env, "SERVER_PORT"), Some("8000"));
    assert_eq!(get(&env, "FAKE_REASONING"), Some("false"));
    assert_eq!(get(&env, "REQUEST_LOG_DIR"), Some("/data/logs/requests"));
    assert_eq!(get(&env, "TIKTOKEN_CACHE_DIR"), Some("/res/gateway/tiktoken_cache"));
    assert_eq!(get(&env, "LOG_LEVEL"), Some("INFO"));
}

#[test]
fn also_sets_python_io_variables_the_smoke_test_found_necessary() {
    let env = build_env(&inputs(), os(&[]));
    assert_eq!(get(&env, "PYTHONUTF8"), Some("1"));
    assert_eq!(get(&env, "PYTHONIOENCODING"), Some("utf-8"));
    assert_eq!(get(&env, "PYTHONUNBUFFERED"), Some("1"));
}

#[test]
fn the_listening_address_is_loopback_only_whatever_the_environment_says() {
    let env = build_env(&inputs(), os(&[("SERVER_HOST", "0.0.0.0")]));
    assert_eq!(get(&env, "SERVER_HOST"), Some("127.0.0.1"));
}

#[test]
fn inherited_anthropic_variables_are_removed() {
    let env = build_env(
        &inputs(),
        os(&[
            ("ANTHROPIC_BASE_URL", "https://example.invalid"),
            ("ANTHROPIC_AUTH_TOKEN", "t"),
            ("ANTHROPIC_API_KEY", "k"),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL", "m"),
            ("PATH", "/usr/bin"),
        ]),
    );
    assert!(keys(&env).iter().all(|k| !k.starts_with("ANTHROPIC_")), "{:?}", keys(&env));
    assert_eq!(get(&env, "PATH"), Some("/usr/bin"));
}

#[test]
fn inherited_proxy_prefixed_variables_are_removed_and_the_key_is_ours() {
    let env = build_env(
        &inputs(),
        os(&[("PROXY_API_KEY", "leaked-key"), ("PROXY_FOO", "x"), ("PROXY_URL", "y")]),
    );
    assert_eq!(get(&env, "PROXY_API_KEY"), Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
    assert_eq!(get(&env, "PROXY_FOO"), None);
    assert_eq!(get(&env, "PROXY_URL"), None);
    assert_eq!(keys(&env).iter().filter(|k| k.as_str() == "PROXY_API_KEY").count(), 1);
}

#[test]
fn removal_is_case_insensitive_for_windows_style_names() {
    let env = build_env(&inputs(), os(&[("anthropic_base_url", "x"), ("Proxy_Foo", "y")]));
    assert!(keys(&env).iter().all(|k| !k.eq_ignore_ascii_case("anthropic_base_url")));
    assert!(keys(&env).iter().all(|k| !k.eq_ignore_ascii_case("proxy_foo")));
}

#[test]
fn variables_that_would_pick_another_credential_source_are_removed() {
    let env = build_env(
        &inputs(),
        os(&[
            ("REFRESH_TOKEN", "r"),
            ("KIRO_CREDS_FILE", "/x.json"),
            ("ACCOUNT_SYSTEM", "true"),
            ("ACCOUNTS_CONFIG_FILE", "/c.json"),
            ("ACCOUNTS_STATE_FILE", "/s.json"),
        ]),
    );
    for name in ["REFRESH_TOKEN", "KIRO_CREDS_FILE", "ACCOUNT_SYSTEM", "ACCOUNTS_CONFIG_FILE", "ACCOUNTS_STATE_FILE"] {
        assert_eq!(get(&env, name), None, "{name}");
    }
}

#[test]
fn explicit_values_override_inherited_ones() {
    let env = build_env(
        &inputs(),
        os(&[("SERVER_PORT", "1"), ("LOG_LEVEL", "DEBUG"), ("FAKE_REASONING", "true"), ("REQUEST_LOG_DIR", "/elsewhere"), ("KIRO_CLI_DB_FILE", "/other.db")]),
    );
    assert_eq!(get(&env, "SERVER_PORT"), Some("8000"));
    assert_eq!(get(&env, "LOG_LEVEL"), Some("INFO"));
    assert_eq!(get(&env, "FAKE_REASONING"), Some("false"));
    assert_eq!(get(&env, "REQUEST_LOG_DIR"), Some("/data/logs/requests"));
    assert_eq!(get(&env, "KIRO_CLI_DB_FILE"), Some("/home/u/.local/share/kiro-cli/data.sqlite3"));
}

#[test]
fn unrelated_inherited_variables_such_as_path_home_and_system_proxy_pass_through() {
    let env = build_env(
        &inputs(),
        os(&[("PATH", "/usr/bin:/bin"), ("HOME", "/Users/me"), ("HTTPS_PROXY", "http://127.0.0.1:7890"), ("LANG", "en_US.UTF-8")]),
    );
    assert_eq!(get(&env, "HOME"), Some("/Users/me"));
    assert_eq!(get(&env, "HTTPS_PROXY"), Some("http://127.0.0.1:7890"));
    assert_eq!(get(&env, "LANG"), Some("en_US.UTF-8"));
}

#[test]
fn missing_tiktoken_cache_leaves_the_variable_unset_even_if_inherited() {
    let mut i = inputs();
    i.tiktoken_cache_dir = None;
    let env = build_env(&i, os(&[("TIKTOKEN_CACHE_DIR", "/stale")]));
    assert_eq!(get(&env, "TIKTOKEN_CACHE_DIR"), None);
}

#[test]
fn port_is_rendered_as_plain_decimal() {
    let mut i = inputs();
    i.port = 65535;
    assert_eq!(get(&build_env(&i, os(&[])), "SERVER_PORT"), Some("65535"));
}

#[test]
fn no_duplicate_keys_in_the_result() {
    let env = build_env(&inputs(), os(&[("SERVER_PORT", "1"), ("PATH", "/bin"), ("PATH", "/usr/bin")]));
    let mut seen = std::collections::HashSet::new();
    for k in keys(&env) {
        assert!(seen.insert(k.to_ascii_uppercase()), "duplicate {k}");
    }
}

#[test]
fn the_secret_appears_only_as_proxy_api_key() {
    let env = build_env(&inputs(), os(&[]));
    let secret = inputs().secret;
    let holders: Vec<_> = env.iter().filter(|(_, v)| v.to_string_lossy().contains(&secret)).map(|(k, _)| k.to_string_lossy().to_string()).collect();
    assert_eq!(holders, vec!["PROXY_API_KEY".to_string()]);
}

#[test]
fn process_environment_snapshot_can_be_fed_straight_in() {
    let env = build_env(&inputs(), std::env::vars_os());
    assert_eq!(get(&env, "SERVER_PORT"), Some("8000"));
}

// ------------------------------------------------- allow-list (A-process 2)

/// Every environment variable name the gateway's Python code reads, scraped from the source
/// so a variable added upstream later is covered without touching this test.
fn gateway_variable_names() -> std::collections::BTreeSet<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let mut names = std::collections::BTreeSet::new();
    let mut files = vec![root.join("main.py")];
    let mut dirs = vec![root.join("kiro")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "py") {
                files.push(path);
            }
        }
    }
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for marker in ["getenv(\"", "getenv('", "environ.get(\"", "environ.get('", "environ[\"", "environ['", "_get_raw_env_value(\""] {
            for part in text.split(marker).skip(1) {
                let name: String = part.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if name.len() > 2 && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
                    names.insert(name);
                }
            }
        }
    }
    names
}

#[test]
fn the_scan_of_the_gateway_source_finds_the_known_settings() {
    let names = gateway_variable_names();
    for expected in ["DEBUG_MODE", "VPN_PROXY_URL", "SERVER_PORT", "KIRO_API_REGION", "STREAM_KEEPALIVE_INTERVAL"] {
        assert!(names.contains(expected), "scan missed {expected}: {names:?}");
    }
    assert!(names.len() > 30, "{names:?}");
}

#[test]
fn no_gateway_setting_inherited_from_the_app_reaches_the_sidecar_except_those_we_set() {
    let set_by_us: std::collections::BTreeSet<String> =
        explicit(&inputs()).iter().map(|(k, _)| k.to_string()).collect();
    let inherited: Vec<(OsString, OsString)> = gateway_variable_names()
        .into_iter()
        .filter(|n| n != "PATH" && !n.ends_with("_PROXY"))
        .map(|n| (OsString::from(&n), OsString::from("leaked")))
        .collect();
    assert!(inherited.len() > 30);
    let env = build_env(&inputs(), inherited);
    for (key, value) in &env {
        let key = key.to_string_lossy().to_string();
        if !set_by_us.contains(&key) {
            assert_ne!(value.to_string_lossy(), "leaked", "{key} leaked from the App's environment");
        }
    }
    let leaked: Vec<_> = keys(&env).into_iter().filter(|k| !set_by_us.contains(k)).collect();
    assert!(leaked.is_empty(), "inherited gateway settings leaked: {leaked:?}");
}

#[test]
fn debug_mode_exported_by_the_user_does_not_switch_on_conversation_logging() {
    let names = ["DEBUG_MODE", "DEBUG_DIR", "VPN_PROXY_URL", "EXPOSE_CREDITS_USED", "HIDDEN_MODELS", "MODEL_ALIASES", "TIKTOKEN_CACHE_DIR_X", "TIKTOKEN_FOO"];
    let inherited: Vec<(&str, &str)> = names.iter().map(|n| (*n, "x")).collect();
    let env = build_env(&inputs(), os(&inherited));
    for name in names {
        assert_eq!(get(&env, name), None, "{name}");
    }
}

#[test]
fn system_basics_locale_certificates_and_proxies_are_kept_in_both_cases() {
    let pairs = [
        ("PATH", "/usr/bin"), ("HOME", "/h"), ("USER", "u"), ("LOGNAME", "u"), ("TMPDIR", "/t"), ("LANG", "C"),
        ("LC_ALL", "C"), ("LC_CTYPE", "UTF-8"), ("SHELL", "/bin/zsh"), ("SYSTEMROOT", "C:\\Windows"), ("WINDIR", "C:\\Windows"),
        ("APPDATA", "a"), ("LOCALAPPDATA", "l"), ("USERPROFILE", "p"), ("TEMP", "t"), ("TMP", "t"), ("COMSPEC", "c"),
        ("PATHEXT", ".EXE"), ("SSL_CERT_FILE", "/c.pem"), ("SSL_CERT_DIR", "/certs"),
        ("HTTP_PROXY", "http://p:1"), ("HTTPS_PROXY", "http://p:1"), ("NO_PROXY", "localhost"),
        ("http_proxy", "http://p:1"), ("https_proxy", "http://p:1"), ("no_proxy", "localhost"),
    ];
    let env = build_env(&inputs(), os(&pairs));
    for (name, value) in pairs {
        assert_eq!(get(&env, name), Some(value), "{name}");
    }
}

#[test]
fn unknown_inherited_variables_are_dropped_by_default() {
    let env = build_env(&inputs(), os(&[("SOME_TOOL_TOKEN", "x"), ("AWS_PROFILE", "p"), ("PYTHONPATH", "/evil"), ("OPENAI_API_KEY", "k")]));
    for name in ["SOME_TOOL_TOKEN", "AWS_PROFILE", "PYTHONPATH", "OPENAI_API_KEY"] {
        assert_eq!(get(&env, name), None, "{name}");
    }
}

#[test]
fn desktop_controls_are_explicitly_enabled_by_the_app() {
    let env = build_env(&inputs(), os(&[("DESKTOP_API_ENABLED", "false")]));
    let values: Vec<_> = env.iter().filter(|(key, _)| key == "DESKTOP_API_ENABLED").collect();
    assert_eq!(values.len(), 1);
    assert_eq!(get(&env, "DESKTOP_API_ENABLED"), Some("true"));
}
