use super::*;
use serde_json::json;

#[test]
fn defaults_match_the_contract_and_kclaude_zsh() {
    let c = AppConfig::default();
    assert_eq!(c.port, 6868);
    assert_eq!(c.command_name, "cclaude");
    assert_eq!(c.models.opus, "claude-opus-5-5[1m]");
    assert_eq!(c.models.sonnet, "claude-sonnet-5-5[1m]");
    assert_eq!(c.models.haiku, "claude-haiku-4-5");
    assert!(!c.autostart);
    assert!(!c.gateway_wanted);
    assert!(!c.upgrade_required);
}

#[test]
fn json_shape_is_camel_case_with_every_key_present() {
    let value = serde_json::to_value(AppConfig::default()).unwrap();
    assert_eq!(
        value,
        json!({
            "port": 6868,
            "commandName": "cclaude",
            "models": {
                "opus": "claude-opus-5-5[1m]",
                "sonnet": "claude-sonnet-5-5[1m]",
                "haiku": "claude-haiku-4-5"
            },
            "autostart": false,
            "gatewayWanted": false,
            "upgradeRequired": false
        })
    );
}

#[test]
fn model_tier_is_snake_case_on_the_wire() {
    assert_eq!(serde_json::to_value(ModelTier::Opus).unwrap(), json!("opus"));
    assert_eq!(serde_json::from_value::<ModelTier>(json!("haiku")).unwrap(), ModelTier::Haiku);
}

#[test]
fn missing_fields_fall_back_to_defaults_and_unknown_fields_are_ignored() {
    let c: AppConfig = serde_json::from_value(json!({"port": 8123, "somethingNew": true})).unwrap();
    assert_eq!(c.port, 8123);
    assert_eq!(c.models, Models::default());
    let c: AppConfig = serde_json::from_value(json!({"models": {"opus": "x-opus"}})).unwrap();
    assert_eq!(c.models.opus, "x-opus");
    assert_eq!(c.models.sonnet, Models::default().sonnet);
}

#[test]
fn patch_deserializes_partial_input() {
    let p: ConfigPatch = serde_json::from_value(json!({"models": {"haiku": "h"}})).unwrap();
    assert_eq!(p.port, None);
    assert_eq!(p.autostart, None);
    assert_eq!(p.models.unwrap().haiku.as_deref(), Some("h"));
    let empty: ConfigPatch = serde_json::from_value(json!({})).unwrap();
    assert_eq!(empty, ConfigPatch::default());
}

#[test]
fn patch_port_out_of_u16_range_deserializes_so_it_can_be_rejected_with_a_message() {
    let p: ConfigPatch = serde_json::from_value(json!({"port": 70000})).unwrap();
    assert_eq!(p.port, Some(70000));
}

#[test]
fn validate_port_accepts_the_inclusive_bounds() {
    assert_eq!(validate_port(1024).unwrap(), 1024);
    assert_eq!(validate_port(65535).unwrap(), 65535);
    assert_eq!(validate_port(6868).unwrap(), 6868);
    assert_eq!(validate_port(8000).unwrap(), 8000);
}

#[test]
fn validate_port_rejects_everything_outside_the_range() {
    for bad in [-1, 0, 1, 80, 1023, 65536, 70000, i64::MAX, i64::MIN] {
        let err = validate_port(bad).unwrap_err();
        assert_eq!(err.code(), "invalid_argument", "{bad}");
        assert!(err.message().contains("1024"), "{}", err.message());
    }
}

#[test]
fn validate_model_trims_and_accepts_real_model_ids() {
    for ok in [
        "claude-opus-5-5[1m]",
        "claude-haiku-4-5",
        "claude-sonnet-4.5",
        "us.anthropic.claude-haiku-4-5:0",
        "  claude-haiku-4-5  ",
        "auto",
    ] {
        assert_eq!(validate_model(ok).unwrap(), ok.trim());
    }
}

#[test]
fn validate_model_rejects_empty_and_shell_unsafe_names() {
    for bad in [
        "",
        "   ",
        "a b",
        "a\nb",
        "a'b",
        "a\"b",
        "a$b",
        "a;b",
        "a`b",
        "a&b",
        "a|b",
        "a\\b",
        "a#b",
        "模型",
        &"x".repeat(129),
    ] {
        let err = validate_model(bad).unwrap_err();
        assert_eq!(err.code(), "invalid_argument", "{bad:?}");
    }
}

#[test]
fn apply_patch_changes_only_the_fields_present() {
    let base = AppConfig::default();
    let patch = ConfigPatch {
        port: Some(8100),
        models: Some(PartialModels { haiku: Some("claude-haiku-4.5".into()), ..Default::default() }),
        autostart: None,
        command_name: None,
    };
    let next = apply_patch(&base, &patch).unwrap();
    assert_eq!(next.port, 8100);
    assert_eq!(next.models.haiku, "claude-haiku-4.5");
    assert_eq!(next.models.opus, base.models.opus);
    assert_eq!(next.models.sonnet, base.models.sonnet);
    assert_eq!(next.autostart, base.autostart);
    assert_eq!(next.gateway_wanted, base.gateway_wanted);
    assert_eq!(next.upgrade_required, base.upgrade_required);
}

#[test]
fn apply_patch_empty_is_identity() {
    let base = AppConfig { port: 9001, gateway_wanted: true, ..AppConfig::default() };
    assert_eq!(apply_patch(&base, &ConfigPatch::default()).unwrap(), base);
}

#[test]
fn apply_patch_is_all_or_nothing() {
    let base = AppConfig::default();
    let patch = ConfigPatch {
        port: Some(8100),
        models: Some(PartialModels { opus: Some("".into()), ..Default::default() }),
        autostart: Some(true),
        command_name: Some("kclaude-test".into()),
    };
    assert!(apply_patch(&base, &patch).is_err());
}

#[test]
fn apply_patch_never_touches_gateway_wanted_or_upgrade_required() {
    // The patch type has no such fields; this documents that upgradeRequired is read-only for the UI.
    let value = serde_json::to_value(ConfigPatch::default()).unwrap();
    assert!(value.get("upgradeRequired").is_none());
    assert!(value.get("gatewayWanted").is_none());
    let sneaky: ConfigPatch = serde_json::from_value(json!({"upgradeRequired": false, "gatewayWanted": true})).unwrap();
    let base = AppConfig { upgrade_required: true, ..AppConfig::default() };
    let next = apply_patch(&base, &sneaky).unwrap();
    assert!(next.upgrade_required);
    assert!(!next.gateway_wanted);
}

#[test]
fn validate_whole_config_catches_hand_edited_files() {
    assert!(AppConfig::default().validate().is_ok());
    let mut c = AppConfig::default();
    c.port = 80;
    assert!(c.validate().is_err());
    let mut c = AppConfig::default();
    c.models.sonnet = "has space".into();
    assert!(c.validate().is_err());
}

#[test]
fn error_codes_follow_the_contract_table() {
    assert_eq!(ConfigError::InvalidArgument("x".into()).code(), "invalid_argument");
    assert_eq!(ConfigError::WriteFailed("x".into()).code(), "write_failed");
    assert_eq!(ConfigError::Internal("x".into()).code(), "internal");
}

#[test]
fn error_messages_are_plain_chinese_without_paths() {
    for e in [
        ConfigError::write_failed(&std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/Users/x/config.json denied")),
        ConfigError::write_failed(&std::io::Error::new(std::io::ErrorKind::Other, "/Users/x/config.json")),
        ConfigError::write_failed(&std::io::Error::from_raw_os_error(28)),
    ] {
        assert!(!e.message().contains('/'), "{}", e.message());
        assert!(!e.message().contains("Users"), "{}", e.message());
        assert!(!e.message().is_empty());
    }
}

// ---- commandName and the 6868 default (task 10.1) ------------------------------------------

#[test]
fn old_config_without_command_name_gets_the_default_and_is_valid() {
    let c: AppConfig = serde_json::from_value(json!({
        "port": 8000,
        "models": {"opus": "claude-opus-5-5[1m]", "sonnet": "claude-sonnet-5-5[1m]", "haiku": "claude-haiku-4-5"},
        "autostart": true, "gatewayWanted": true, "upgradeRequired": false
    }))
    .unwrap();
    assert_eq!(c.command_name, "cclaude");
    assert!(c.validate().is_ok());
}

#[test]
fn an_existing_port_is_kept_only_the_default_changed() {
    let c: AppConfig = serde_json::from_value(json!({"port": 8000})).unwrap();
    assert_eq!(c.port, 8000);
    let c: AppConfig = serde_json::from_value(json!({})).unwrap();
    assert_eq!(c.port, 6868);
}

#[test]
fn hand_edited_bad_command_names_make_the_config_invalid() {
    for bad in ["", "claude", "ls", "9abc", "has space", "a;b"] {
        let c = AppConfig { command_name: bad.to_string(), ..AppConfig::default() };
        let err = c.validate().unwrap_err();
        assert_eq!(err.code(), "invalid_argument", "{bad:?}");
    }
}

#[test]
fn patch_command_name_is_validated_with_the_documented_messages() {
    let base = AppConfig::default();
    let cases = [
        ("claude", "这个名字是系统命令，换一个"),
        ("LS", "这个名字是系统命令，换一个"),
        ("git", "这个名字是系统命令，换一个"),
        ("9abc", "命令名只能用字母、数字、- 和 _，以字母开头，2 到 32 个字符"),
        ("a b", "命令名只能用字母、数字、- 和 _，以字母开头，2 到 32 个字符"),
        ("", "命令名只能用字母、数字、- 和 _，以字母开头，2 到 32 个字符"),
        ("x", "命令名只能用字母、数字、- 和 _，以字母开头，2 到 32 个字符"),
    ];
    for (name, message) in cases {
        let patch = ConfigPatch { command_name: Some(name.to_string()), ..Default::default() };
        let err = apply_patch(&base, &patch).unwrap_err();
        assert_eq!(err.code(), "invalid_argument", "{name:?}");
        assert_eq!(err.message(), message, "{name:?}");
    }
}

#[test]
fn patch_command_name_accepts_a_good_name_and_changes_nothing_else() {
    let base = AppConfig::default();
    let patch = ConfigPatch { command_name: Some("kclaude-test".to_string()), ..Default::default() };
    let next = apply_patch(&base, &patch).unwrap();
    assert_eq!(next.command_name, "kclaude-test");
    assert_eq!(AppConfig { command_name: base.command_name.clone(), ..next }, base);
}

#[test]
fn a_bad_command_name_makes_the_whole_patch_fail() {
    let patch = ConfigPatch { port: Some(9000), command_name: Some("claude".into()), ..Default::default() };
    assert!(apply_patch(&AppConfig::default(), &patch).is_err());
}

#[test]
fn patch_deserializes_command_name_from_camel_case() {
    let p: ConfigPatch = serde_json::from_value(json!({"commandName": "abc"})).unwrap();
    assert_eq!(p.command_name.as_deref(), Some("abc"));
}
