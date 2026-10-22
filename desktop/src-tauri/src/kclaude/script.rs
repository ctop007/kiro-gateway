//! Script templates (`kclaude.zsh` / `.bash` / `.ps1`) and their installation into the
//! App data directory.
//!
//! The templates contain one placeholder, `@@KCLAUDE_DATA_DIR@@`, replaced by the quoted
//! absolute path of the data directory. Paths with spaces ("Application Support") are
//! therefore safe in every shell.

use std::fs;
use std::io;
use std::path::Path;

use super::fsutil::write_atomic;

const PLACEHOLDER: &str = "@@KCLAUDE_DATA_DIR@@";
const ZSH_TEMPLATE: &str = include_str!("templates/kclaude.zsh");
const BASH_TEMPLATE: &str = include_str!("templates/kclaude.bash");
const PS1_TEMPLATE: &str = include_str!("templates/kclaude.ps1");
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Which template to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Zsh,
    Bash,
    PowerShell,
}

impl ScriptKind {
    /// File name inside the App data directory.
    pub fn file_name(self) -> &'static str {
        match self {
            ScriptKind::Zsh => "kclaude.zsh",
            ScriptKind::Bash => "kclaude.bash",
            ScriptKind::PowerShell => "kclaude.ps1",
        }
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

/// Render the script for `kind` with `data_dir` baked in.
///
/// Args:
///     kind: Which template.
///     data_dir: Absolute App data directory that holds `kclaude.env`.
///
/// Returns:
///     Script text (no BOM; see [`render_bytes`]).
pub fn render(kind: ScriptKind, data_dir: &Path) -> String {
    let dir = data_dir.to_string_lossy();
    let quoted = match kind {
        ScriptKind::Zsh | ScriptKind::Bash => sh_quote(&dir),
        ScriptKind::PowerShell => ps_quote(&dir),
    };
    kclaude_template_with(kind.template(), &quoted)
}

fn kclaude_template_with(template: &str, quoted_dir: &str) -> String {
    template.replace(PLACEHOLDER, quoted_dir)
}

/// Bytes to put on disk: PowerShell scripts get a UTF-8 BOM so Windows PowerShell 5
/// decodes the Chinese messages correctly.
pub fn render_bytes(kind: ScriptKind, data_dir: &Path) -> Vec<u8> {
    let text = render(kind, data_dir);
    match kind {
        ScriptKind::PowerShell => [UTF8_BOM, text.as_bytes()].concat(),
        _ => text.into_bytes(),
    }
}

/// True when the script on disk is byte-identical to what would be written now.
pub fn is_current(kind: ScriptKind, data_dir: &Path) -> bool {
    fs::read(data_dir.join(kind.file_name()))
        .map(|existing| existing == render_bytes(kind, data_dir))
        .unwrap_or(false)
}

/// Write the script into `data_dir` unless it is already current.
///
/// Returns:
///     `Ok(true)` when the file was written, `Ok(false)` when nothing changed.
pub fn install(kind: ScriptKind, data_dir: &Path) -> io::Result<bool> {
    if is_current(kind, data_dir) {
        return Ok(false);
    }
    write_atomic(&data_dir.join(kind.file_name()), &render_bytes(kind, data_dir))?;
    Ok(true)
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

    #[test]
    fn render_bakes_in_quoted_path_and_leaves_no_placeholder() {
        let dir = Path::new("/Users/me/Library/Application Support/dev.kirogateway.desktop");
        for kind in [ScriptKind::Zsh, ScriptKind::Bash, ScriptKind::PowerShell] {
            let text = render(kind, dir);
            assert!(!text.contains(PLACEHOLDER), "{kind:?}");
            assert!(text.contains("Application Support"), "{kind:?}");
        }
        assert!(render(ScriptKind::Zsh, dir).contains("='/Users/me/Library/Application Support/"));
    }

    #[test]
    fn powershell_script_gets_a_bom_and_shell_scripts_do_not() {
        let dir = Path::new("/d");
        assert!(render_bytes(ScriptKind::PowerShell, dir).starts_with(UTF8_BOM));
        assert!(!render_bytes(ScriptKind::Zsh, dir).starts_with(UTF8_BOM));
    }

    #[test]
    fn templates_carry_the_required_messages() {
        for kind in [ScriptKind::Zsh, ScriptKind::Bash, ScriptKind::PowerShell] {
            let text = render(kind, Path::new("/d"));
            for msg in [
                "找不到 Kiro Gateway 配置，请打开 Kiro Gateway App",
                "Kiro Gateway 需要升级，请打开 App 完成升级",
                "KiroGateway 未运行，请在 Kiro Gateway App 里打开开关",
            ] {
                assert!(text.contains(msg), "{kind:?} missing {msg}");
            }
        }
    }

    #[test]
    fn install_writes_once_then_is_a_no_op_and_repairs_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Application Support").join("app");
        assert!(!is_current(ScriptKind::Zsh, &dir));
        assert!(install(ScriptKind::Zsh, &dir).unwrap());
        assert!(is_current(ScriptKind::Zsh, &dir));
        assert!(!install(ScriptKind::Zsh, &dir).unwrap());

        fs::write(dir.join("kclaude.zsh"), "tampered").unwrap();
        assert!(!is_current(ScriptKind::Zsh, &dir));
        assert!(install(ScriptKind::Zsh, &dir).unwrap());
        assert!(is_current(ScriptKind::Zsh, &dir));
    }

    #[test]
    fn script_for_a_moved_data_dir_is_not_current() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        install(ScriptKind::Bash, &a).unwrap();
        // Same bytes under another path would embed another directory.
        assert_ne!(
            render_bytes(ScriptKind::Bash, &a),
            render_bytes(ScriptKind::Bash, &tmp.path().join("b"))
        );
    }
}
