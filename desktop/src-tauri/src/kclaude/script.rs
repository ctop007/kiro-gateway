//! Script templates (`<command>.zsh` / `.bash` / `.ps1`) and their installation into the
//! App data directory.
//!
//! The templates contain two placeholders: `@@KCLAUDE_DATA_DIR@@`, replaced by the quoted
//! absolute path of the data directory (paths with spaces such as "Application Support"
//! are therefore safe in every shell), and `@@KCLAUDE_COMMAND@@`, replaced by the command
//! name. The name is the shell function name and the prefix of every error message; it is
//! unquoted, which is safe because `app_config::command_name` only lets
//! `[A-Za-z][A-Za-z0-9_-]{1,31}` through (and [`render`] re-checks that).

use std::fs;
use std::io;
use std::path::Path;

use super::fsutil::write_atomic;
use crate::app_config::command_name;

const PLACEHOLDER: &str = "@@KCLAUDE_DATA_DIR@@";
const COMMAND_PLACEHOLDER: &str = "@@KCLAUDE_COMMAND@@";
const ZSH_TEMPLATE: &str = include_str!("templates/command.zsh");
const BASH_TEMPLATE: &str = include_str!("templates/command.bash");
const PS1_TEMPLATE: &str = include_str!("templates/command.ps1");
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Which template to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Zsh,
    Bash,
    PowerShell,
}

impl ScriptKind {
    /// Every kind, for cleaning up scripts of an old command name.
    pub const ALL: [ScriptKind; 3] = [ScriptKind::Zsh, ScriptKind::Bash, ScriptKind::PowerShell];

    /// File name inside the App data directory: `<command>.zsh` / `.bash` / `.ps1`.
    pub fn file_name(self, command: &str) -> String {
        let extension = match self {
            ScriptKind::Zsh => "zsh",
            ScriptKind::Bash => "bash",
            ScriptKind::PowerShell => "ps1",
        };
        format!("{command}.{extension}")
    }

    fn template(self) -> &'static str {
        match self {
            ScriptKind::Zsh => ZSH_TEMPLATE,
            ScriptKind::Bash => BASH_TEMPLATE,
            ScriptKind::PowerShell => PS1_TEMPLATE,
        }
    }
}

/// Quote `value` as one POSIX shell word: `'...'` with embedded `'` written as `'\''`.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Quote `value` as a PowerShell single-quoted string literal (`'` doubled).
///
/// Curly quotes are doubled as well: PowerShell treats them as quote characters.
pub fn ps_quote(value: &str) -> String {
    let mut out = String::from("'");
    for ch in value.chars() {
        out.push(ch);
        if matches!(ch, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Render a PowerShell expression that evaluates to `value` and survives being saved in a
/// profile whose encoding is unknown.
///
/// ASCII values become a quoted literal. Anything else (a user name in Chinese, say) is
/// built from UTF-16 code units, because Windows PowerShell 5 reads a BOM-less file in
/// the ANSI code page and would otherwise see different characters.
pub fn ps_string_expr(value: &str) -> String {
    if value.is_ascii() {
        return ps_quote(value);
    }
    let units: Vec<String> = value.encode_utf16().map(|u| u.to_string()).collect();
    format!("(-join [char[]]({}))", units.join(","))
}

/// Render the script for `kind` with `command` and `data_dir` baked in.
///
/// Args:
///     kind: Which template.
///     command: Command name (the shell function name).
///     data_dir: Absolute App data directory that holds `kclaude.env`.
///
/// Returns:
///     Script text (no BOM; see [`render_bytes`]). A `command` that does not match the
///     allowed shape is replaced by the default name instead of being spliced into the
///     script, so a hand-edited name can never inject shell syntax.
pub fn render(kind: ScriptKind, command: &str, data_dir: &Path) -> String {
    let dir = data_dir.to_string_lossy();
    let quoted = match kind {
        ScriptKind::Zsh | ScriptKind::Bash => sh_quote(&dir),
        ScriptKind::PowerShell => ps_quote(&dir),
    };
    let command = if command_name::is_valid_format(command) { command } else { command_name::DEFAULT_COMMAND_NAME };
    kind.template().replace(COMMAND_PLACEHOLDER, command).replace(PLACEHOLDER, &quoted)
}

/// Bytes to put on disk: PowerShell scripts get a UTF-8 BOM so Windows PowerShell 5
/// decodes the Chinese messages correctly.
pub fn render_bytes(kind: ScriptKind, command: &str, data_dir: &Path) -> Vec<u8> {
    let text = render(kind, command, data_dir);
    match kind {
        ScriptKind::PowerShell => [UTF8_BOM, text.as_bytes()].concat(),
        _ => text.into_bytes(),
    }
}

/// True when the script on disk is byte-identical to what would be written now.
pub fn is_current(kind: ScriptKind, command: &str, data_dir: &Path) -> bool {
    fs::read(data_dir.join(kind.file_name(command)))
        .map(|existing| existing == render_bytes(kind, command, data_dir))
        .unwrap_or(false)
}

/// Write the script into `data_dir` unless it is already current.
///
/// Returns:
///     `Ok(true)` when the file was written, `Ok(false)` when nothing changed.
pub fn install(kind: ScriptKind, command: &str, data_dir: &Path) -> io::Result<bool> {
    if is_current(kind, command, data_dir) {
        return Ok(false);
    }
    write_atomic(&data_dir.join(kind.file_name(command)), &render_bytes(kind, command, data_dir))?;
    Ok(true)
}

/// Delete the scripts App wrote for `command` (all three kinds). Missing files are fine.
///
/// Only names that pass the command-name format check are touched, so this can never be
/// pointed at a file outside the App's own `<command>.zsh|bash|ps1` set.
///
/// Returns:
///     How many files were deleted.
pub fn remove_all(command: &str, data_dir: &Path) -> usize {
    if !command_name::is_valid_format(command) {
        return 0;
    }
    ScriptKind::ALL
        .iter()
        .filter(|kind| fs::remove_file(data_dir.join(kind.file_name(command))).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_quote_handles_spaces_and_single_quotes() {
        assert_eq!(sh_quote("/a b/c"), "'/a b/c'");
        assert_eq!(sh_quote("/it's"), "'/it'\\''s'");
        assert_eq!(sh_quote("/$HOME/`x`"), "'/$HOME/`x`'");
    }

    #[test]
    fn ps_quote_doubles_single_and_curly_quotes() {
        assert_eq!(ps_quote(r"C:\Users\O'Neil\x"), r"'C:\Users\O''Neil\x'");
        assert_eq!(ps_quote("a\u{2019}b"), "'a\u{2019}\u{2019}b'");
    }

    #[test]
    fn ps_string_expr_is_literal_for_ascii_and_char_codes_otherwise() {
        assert_eq!(ps_string_expr(r"C:\a b"), r"'C:\a b'");
        let expr = ps_string_expr("C:\\张");
        assert!(expr.is_ascii(), "{expr}");
        assert_eq!(expr, "(-join [char[]](67,58,92,24352))");
    }

    const CMD: &str = "cclaude";

    #[test]
    fn render_bakes_in_quoted_path_and_leaves_no_placeholder() {
        let dir = Path::new("/Users/me/Library/Application Support/dev.kirogateway.desktop");
        for kind in ScriptKind::ALL {
            let text = render(kind, CMD, dir);
            assert!(!text.contains(PLACEHOLDER), "{kind:?}");
            assert!(!text.contains(COMMAND_PLACEHOLDER), "{kind:?}");
            assert!(text.contains("Application Support"), "{kind:?}");
        }
        assert!(render(ScriptKind::Zsh, CMD, dir).contains("='/Users/me/Library/Application Support/"));
    }

    #[test]
    fn file_names_follow_the_command_name() {
        assert_eq!(ScriptKind::Zsh.file_name("cclaude"), "cclaude.zsh");
        assert_eq!(ScriptKind::Bash.file_name("kclaude-test"), "kclaude-test.bash");
        assert_eq!(ScriptKind::PowerShell.file_name("my_cc"), "my_cc.ps1");
    }

    #[test]
    fn function_name_and_error_prefix_use_the_command_name() {
        let dir = Path::new("/d");
        let zsh = render(ScriptKind::Zsh, "kclaude-test", dir);
        assert!(zsh.contains("\nkclaude-test() {\n"), "{zsh}");
        assert!(zsh.contains("echo \"kclaude-test: KiroGateway 未运行，请在 Kiro Gateway App 里打开开关\" >&2"));
        assert!(!zsh.contains("\nkclaude() {"), "the old function name must not remain");
        let bash = render(ScriptKind::Bash, "kclaude-test", dir);
        assert!(bash.contains("\nkclaude-test() {\n"));
        let ps = render(ScriptKind::PowerShell, "kclaude-test", dir);
        assert!(ps.contains("function kclaude-test {"));
        assert!(ps.contains("WriteLine(\"kclaude-test: $Message\")"));
        assert!(!ps.contains("function kclaude {"));
    }

    #[test]
    fn a_malformed_command_name_never_reaches_the_script_text() {
        let dir = Path::new("/d");
        for bad in ["x; rm -rf ~", "a b", "", "a$(id)", "$(id)", "a\nb", "a'b"] {
            for kind in ScriptKind::ALL {
                let text = render(kind, bad, dir);
                assert!(!text.contains("rm -rf") && !text.contains("$(id)") && !text.contains("a b"), "{bad:?}");
                assert!(text.contains("cclaude"), "falls back to the default name for {bad:?}");
            }
        }
    }

    #[test]
    fn powershell_script_gets_a_bom_and_shell_scripts_do_not() {
        let dir = Path::new("/d");
        assert!(render_bytes(ScriptKind::PowerShell, CMD, dir).starts_with(UTF8_BOM));
        assert!(!render_bytes(ScriptKind::Zsh, CMD, dir).starts_with(UTF8_BOM));
    }

    #[test]
    fn templates_carry_the_required_messages_with_the_command_prefix() {
        for kind in ScriptKind::ALL {
            let text = render(kind, "cclaude", Path::new("/d"));
            for msg in [
                "cclaude: 找不到 Kiro Gateway 配置，请打开 Kiro Gateway App",
                "cclaude: Kiro Gateway 需要升级，请打开 App 完成升级",
                "cclaude: KiroGateway 未运行，请在 Kiro Gateway App 里打开开关",
                "cclaude: 找不到 claude 命令，请先安装 Claude Code",
            ] {
                let wanted = if kind == ScriptKind::PowerShell { msg.replace("cclaude: ", "") } else { msg.to_string() };
                assert!(text.contains(&wanted), "{kind:?} missing {wanted}");
            }
            assert!(!text.contains("kclaude:"), "{kind:?}: no hard-coded old prefix");
        }
    }

    #[test]
    fn install_writes_once_then_is_a_no_op_and_repairs_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Application Support").join("app");
        assert!(!is_current(ScriptKind::Zsh, CMD, &dir));
        assert!(install(ScriptKind::Zsh, CMD, &dir).unwrap());
        assert!(is_current(ScriptKind::Zsh, CMD, &dir));
        assert!(!install(ScriptKind::Zsh, CMD, &dir).unwrap());

        fs::write(dir.join("cclaude.zsh"), "tampered").unwrap();
        assert!(!is_current(ScriptKind::Zsh, CMD, &dir));
        assert!(install(ScriptKind::Zsh, CMD, &dir).unwrap());
        assert!(is_current(ScriptKind::Zsh, CMD, &dir));
    }

    #[test]
    fn a_script_for_another_command_name_is_not_current() {
        let tmp = tempfile::tempdir().unwrap();
        install(ScriptKind::Zsh, "cclaude", tmp.path()).unwrap();
        assert!(!is_current(ScriptKind::Zsh, "kclaude-test", tmp.path()));
        assert!(!tmp.path().join("kclaude-test.zsh").exists());
    }

    #[test]
    fn script_for_a_moved_data_dir_is_not_current() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        install(ScriptKind::Bash, CMD, &a).unwrap();
        // Same bytes under another path would embed another directory.
        assert_ne!(
            render_bytes(ScriptKind::Bash, CMD, &a),
            render_bytes(ScriptKind::Bash, CMD, &tmp.path().join("b"))
        );
    }

    #[test]
    fn remove_all_deletes_only_the_three_scripts_of_that_command() {
        let tmp = tempfile::tempdir().unwrap();
        for kind in ScriptKind::ALL {
            install(kind, "old-name", tmp.path()).unwrap();
            install(kind, "keep-me", tmp.path()).unwrap();
        }
        fs::write(tmp.path().join("kclaude.env"), "KIRO_GATEWAY_PORT=1\n").unwrap();
        assert_eq!(remove_all("old-name", tmp.path()), 3);
        assert_eq!(remove_all("old-name", tmp.path()), 0, "second call finds nothing");
        for kind in ScriptKind::ALL {
            assert!(!tmp.path().join(kind.file_name("old-name")).exists());
            assert!(tmp.path().join(kind.file_name("keep-me")).exists());
        }
        assert!(tmp.path().join("kclaude.env").exists(), "kclaude.env keeps its name and stays");
    }

    #[test]
    fn remove_all_refuses_names_that_are_not_plain_command_names() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("secret"), "k").unwrap();
        fs::write(tmp.path().join("evil.zsh"), "x").unwrap();
        for bad in ["../data/evil", "", "a/b", "evil.zsh/.."] {
            assert_eq!(remove_all(bad, tmp.path()), 0, "{bad:?}");
        }
        assert!(tmp.path().join("evil.zsh").exists());
        assert!(tmp.path().join("secret").exists());
    }
}
