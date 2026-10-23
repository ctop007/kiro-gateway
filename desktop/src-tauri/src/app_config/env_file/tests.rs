use super::*;
use crate::app_config::model::AppConfig;
use std::process::Command;
use tempfile::tempdir;

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn renders_the_seven_lines_in_a_fixed_order() {
    let text = render(&AppConfig::default(), KEY);
    let expected = format!(
        "KIRO_GATEWAY_PORT=6868\n\
         KIRO_GATEWAY_KEY={KEY}\n\
         KCLAUDE_OPUS_MODEL=claude-opus-5-5[1m]\n\
         KCLAUDE_SONNET_MODEL=claude-sonnet-5-5[1m]\n\
         KCLAUDE_HAIKU_MODEL=claude-haiku-4-5\n\
         KIRO_GATEWAY_UPGRADE_REQUIRED=0\n\
         KIRO_GATEWAY_COMMAND=cclaude\n"
    );
    assert_eq!(text, expected);
}

#[test]
fn command_name_is_rendered_and_read_back_by_real_shells() {
    let mut config = AppConfig::default();
    config.command_name = "kclaude-test".to_string();
    let text = render(&config, KEY);
    assert!(text.contains("KIRO_GATEWAY_COMMAND=kclaude-test\n"));
    let dir = tempdir().unwrap();
    let file = dir.path().join("kclaude.env");
    std::fs::write(&file, &text).unwrap();
    for shell in ["/bin/sh", "/bin/bash", "/bin/zsh"] {
        if !std::path::Path::new(shell).exists() {
            continue;
        }
        let out = Command::new(shell)
            .arg("-c")
            .arg(r#". "$1"; printf '%s' "$KIRO_GATEWAY_COMMAND""#)
            .arg("sh")
            .arg(&file)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "kclaude-test", "{shell}");
    }
}

#[test]
fn upgrade_required_renders_as_one() {
    let mut config = AppConfig::default();
    config.upgrade_required = true;
    assert!(render(&config, KEY).contains("KIRO_GATEWAY_UPGRADE_REQUIRED=1\n"));
}

#[test]
fn custom_port_and_models_are_rendered() {
    let mut config = AppConfig::default();
    config.port = 8123;
    config.models.opus = "claude-opus-4.5".to_string();
    let text = render(&config, KEY);
    assert!(text.contains("KIRO_GATEWAY_PORT=8123\n"));
    assert!(text.contains("KCLAUDE_OPUS_MODEL=claude-opus-4.5\n"));
}

#[test]
fn gateway_wanted_and_autostart_do_not_leak_into_the_file() {
    let mut config = AppConfig::default();
    config.gateway_wanted = true;
    config.autostart = true;
    let text = render(&config, KEY);
    assert!(!text.to_lowercase().contains("wanted"));
    assert!(!text.to_lowercase().contains("autostart"));
}

/// Sources the rendered file with a real shell and prints the variables back, so the
/// "no quoting needed" claim is checked against sh, bash and zsh, not assumed.
fn source_with(shell: &str, text: &str) -> Option<Vec<String>> {
    let dir = tempdir().unwrap();
    let file = dir.path().join("kclaude.env");
    std::fs::write(&file, text).unwrap();
    let script = r#". "$1"; printf '%s\n' "$KIRO_GATEWAY_PORT" "$KIRO_GATEWAY_KEY" "$KCLAUDE_OPUS_MODEL" "$KCLAUDE_SONNET_MODEL" "$KCLAUDE_HAIKU_MODEL" "$KIRO_GATEWAY_UPGRADE_REQUIRED" "$KIRO_GATEWAY_COMMAND""#;
    let output = Command::new(shell).arg("-c").arg(script).arg("sh").arg(&file).output().ok()?;
    assert!(output.status.success(), "{shell}: {}", String::from_utf8_lossy(&output.stderr));
    Some(String::from_utf8_lossy(&output.stdout).lines().map(str::to_string).collect())
}

#[test]
fn real_shells_read_back_exactly_the_configured_values() {
    let mut config = AppConfig::default();
    config.port = 8101;
    config.models.haiku = "us.anthropic.claude-haiku-4-5:0".to_string();
    let text = render(&config, KEY);
    let want = vec![
        "8101".to_string(),
        KEY.to_string(),
        "claude-opus-5-5[1m]".to_string(),
        "claude-sonnet-5-5[1m]".to_string(),
        "us.anthropic.claude-haiku-4-5:0".to_string(),
        "0".to_string(),
        "cclaude".to_string(),
    ];
    let mut tried = 0;
    for shell in ["/bin/sh", "/bin/bash", "/bin/zsh"] {
        if std::path::Path::new(shell).exists() {
            assert_eq!(source_with(shell, &text).unwrap(), want, "{shell}");
            tried += 1;
        }
    }
    assert!(tried >= 1, "no POSIX shell found to test with");
}
