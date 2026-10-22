//! Tests for startup-file block management (`cargo test kclaude::rc`).

use std::fs;
use std::path::Path;

use super::*;

const SCRIPT: &str = "/Users/me/Library/Application Support/dev.kirogateway.desktop/kclaude.zsh";
const STAMP: &str = "20261022-101500";

fn block() -> String {
    block_text(Dialect::Posix, Path::new(SCRIPT))
}

fn install(bytes: &[u8]) -> Option<Vec<u8>> {
    plan_install(bytes, Dialect::Posix, &block()).unwrap()
}

fn state(bytes: &[u8]) -> RcState {
    analyze(bytes, Dialect::Posix).state(&block())
}

fn s(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

// --- block text ------------------------------------------------------------------------

#[test]
fn block_has_markers_and_exactly_one_source_line_with_the_quoted_path() {
    let text = block();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], BEGIN_MARKER);
    assert_eq!(lines[2], END_MARKER);
    assert!(lines[1].contains("source '/Users/me/Library/Application Support/"), "{}", lines[1]);
    assert!(text.ends_with('\n'));
}

#[test]
fn block_survives_a_path_with_a_single_quote() {
    let text = block_text(Dialect::Posix, Path::new("/Users/o'neil/kclaude.zsh"));
    assert!(text.contains(r"'/Users/o'\''neil/kclaude.zsh'"), "{text}");
}

#[test]
fn powershell_block_uses_literal_path_test_and_dot_source() {
    let text = block_text(Dialect::PowerShell, Path::new(r"C:\Users\me\AppData\Roaming\x y\kclaude.ps1"));
    assert!(text.contains(r"if (Test-Path -LiteralPath 'C:\Users\me\AppData\Roaming\x y\kclaude.ps1') { . 'C:\Users\me\AppData\Roaming\x y\kclaude.ps1' }"), "{text}");
}

// --- not installed / install -------------------------------------------------------------

#[test]
fn missing_file_is_not_installed_and_install_creates_exactly_the_block() {
    assert_eq!(state(b""), RcState::NotInstalled);
    assert_eq!(s(&install(b"").unwrap()), block());
}

#[test]
fn existing_content_is_kept_and_block_is_appended() {
    let original = b"export PATH=$HOME/bin:$PATH\nalias ll='ls -l'\n";
    let out = install(original).unwrap();
    assert!(out.starts_with(original));
    assert_eq!(s(&out[original.len()..]), block());
}

#[test]
fn file_without_trailing_newline_gets_a_separating_newline_and_stays_invertible() {
    let original = b"export A=1";
    let out = install(original).unwrap();
    assert!(s(&out).starts_with("export A=1\n# >>> kiro-gateway-desktop kclaude >>>\n"));
    assert_eq!(plan_remove(&out, Dialect::Posix).unwrap().unwrap(), original);
}

#[test]
fn crlf_files_get_crlf_block_lines() {
    let original = b"export A=1\r\nexport B=2\r\n";
    let out = install(original).unwrap();
    let tail = &out[original.len()..];
    assert!(s(tail).contains("\r\n") && !s(tail).replace("\r\n", "").contains('\n'), "{:?}", s(tail));
    assert_eq!(state(&out), RcState::Installed);
    assert_eq!(plan_remove(&out, Dialect::Posix).unwrap().unwrap(), original);
}

#[test]
fn non_utf8_bytes_elsewhere_in_the_file_are_preserved() {
    let mut original = b"# caf\xe9 latin-1 comment\nexport A=1\n".to_vec();
    let out = install(&original).unwrap();
    assert!(out.starts_with(&original));
    original.extend_from_slice(block().as_bytes());
    assert_eq!(out, original);
    assert_eq!(plan_remove(&out, Dialect::Posix).unwrap().unwrap(), b"# caf\xe9 latin-1 comment\nexport A=1\n");
}

// --- installed latest: no write ------------------------------------------------------------

#[test]
fn already_installed_latest_plans_no_write() {
    let installed = install(b"export A=1\n").unwrap();
    assert_eq!(state(&installed), RcState::Installed);
    assert!(install(&installed).is_none());
}

#[test]
fn installed_block_in_the_middle_of_a_file_is_still_installed() {
    let text = format!("export A=1\n{}export B=2\n", block());
    assert_eq!(state(text.as_bytes()), RcState::Installed);
    assert!(install(text.as_bytes()).is_none());
}

// --- outdated ------------------------------------------------------------------------------

#[test]
fn block_pointing_at_another_path_is_outdated_and_is_rewritten_in_place() {
    let old = block_text(Dialect::Posix, Path::new("/old/dir/kclaude.zsh"));
    let text = format!("before\n{old}after\n");
    assert_eq!(state(text.as_bytes()), RcState::Outdated);
    let out = install(text.as_bytes()).unwrap();
    assert_eq!(s(&out), format!("before\n{}after\n", block()));
    assert_eq!(state(&out), RcState::Installed);
}

#[test]
fn block_with_an_edited_body_is_outdated() {
    let text = format!("{BEGIN_MARKER}\nsource /something/else\n{END_MARKER}\n");
    assert_eq!(state(text.as_bytes()), RcState::Outdated);
    assert_eq!(s(&install(text.as_bytes()).unwrap()), block());
}

#[test]
fn outdated_block_at_end_of_file_without_newline_stays_without_newline() {
    let old = block_text(Dialect::Posix, Path::new("/old/kclaude.zsh"));
    let text = format!("a\n{}", old.trim_end_matches('\n'));
    let out = install(text.as_bytes()).unwrap();
    assert_eq!(s(&out), format!("a\n{}", block().trim_end_matches('\n')));
}

#[test]
fn duplicate_blocks_collapse_into_one() {
    let text = format!("{}mid\n{}", block(), block());
    assert_eq!(state(text.as_bytes()), RcState::Outdated);
    let out = install(text.as_bytes()).unwrap();
    assert_eq!(s(&out), format!("{}mid\n", block()));
}

// --- damaged markers -----------------------------------------------------------------------

#[test]
fn begin_without_end_is_damaged_and_nothing_is_planned() {
    let text = format!("a\n{BEGIN_MARKER}\nsource x\n");
    assert_eq!(state(text.as_bytes()), RcState::Damaged);
    assert_eq!(plan_install(text.as_bytes(), Dialect::Posix, &block()), Err(Refusal::Damaged));
    assert_eq!(plan_remove(text.as_bytes(), Dialect::Posix), Err(Refusal::Damaged));
    assert_eq!(plan_adopt(text.as_bytes(), Dialect::Posix, &block()), Err(Refusal::Damaged));
}

#[test]
fn end_without_begin_and_nested_begins_are_damaged() {
    for text in [
        format!("{END_MARKER}\n"),
        format!("{BEGIN_MARKER}\n{BEGIN_MARKER}\n{END_MARKER}\n"),
    ] {
        assert_eq!(state(text.as_bytes()), RcState::Damaged, "{text}");
    }
}

// --- foreign detection ---------------------------------------------------------------------

#[test]
fn foreign_source_of_the_repo_script_is_detected_and_not_written() {
    let text = b"export A=1\nsource \"$HOME/kiro-gateway/scripts/kclaude.zsh\"\n";
    assert_eq!(state(text), RcState::Foreign);
    assert_eq!(plan_install(text, Dialect::Posix, &block()), Err(Refusal::Foreign));
}

#[test]
fn foreign_forms_are_all_detected() {
    let cases = [
        "source ~/kiro-gateway/scripts/kclaude.zsh",
        ". ~/kiro-gateway/scripts/kclaude.zsh",
        "[ -f ~/x/kclaude.zsh ] && source ~/x/kclaude.zsh",
        "if [ -r ~/x ]; then source ~/x/kclaude.zsh; fi",
        "alias kclaude='claude --foo'",
        "alias kclaude=\"claude\"",
        "alias -g kclaude=claude",
        "kclaude() { claude \"$@\"; }",
        "kclaude () {\n  claude \"$@\"\n}",
        "function kclaude { claude; }",
        "function kclaude() {\n  claude\n}",
        "  source /x/kclaude.bash",
    ];
    for case in cases {
        let text = format!("export A=1\n{case}\nexport B=2\n");
        assert_eq!(state(text.as_bytes()), RcState::Foreign, "{case}");
    }
}

#[test]
fn comments_and_unrelated_mentions_are_not_foreign() {
    let cases = [
        "# source ~/kiro-gateway/scripts/kclaude.zsh",
        "# alias kclaude=claude",
        "# disabled by Kiro Gateway App: source ~/kclaude.zsh",
        "echo kclaude is great",
        "alias claude-k='claude'",
        "source ~/.nvm/nvm.sh",
        "export KCLAUDE_NOTE=1",
        "kclaude_helper() { :; }",
        "function kclaude_helper { :; }",
        "alias kclaudex=claude",
    ];
    for case in cases {
        let text = format!("{case}\n");
        assert_eq!(state(text.as_bytes()), RcState::NotInstalled, "{case}");
    }
}

#[test]
fn a_kclaude_mention_inside_our_own_block_is_not_foreign() {
    let text = block();
    assert_eq!(state(text.as_bytes()), RcState::Installed);
}

#[test]
fn foreign_beats_an_existing_marker_block() {
    let text = format!("source ~/x/kclaude.zsh\n{}", block());
    assert_eq!(state(text.as_bytes()), RcState::Foreign);
    assert_eq!(plan_install(text.as_bytes(), Dialect::Posix, &block()), Err(Refusal::Foreign));
}

#[test]
fn foreign_function_with_braces_in_strings_and_comments_is_delimited() {
    let text = "kclaude() {\n  echo \"}\" # {\n  claude\n}\nexport AFTER=1\n";
    let a = analyze(text.as_bytes(), Dialect::Posix);
    assert_eq!(a.foreign_line_numbers(), vec![1]);
    let out = plan_adopt(text.as_bytes(), Dialect::Posix, &block()).unwrap().unwrap();
    let out = s(&out);
    assert!(out.contains("# disabled by Kiro Gateway App: kclaude() {\n"), "{out}");
    assert!(out.contains("# disabled by Kiro Gateway App: }\nexport AFTER=1\n"), "{out}");
}

#[test]
fn unbalanced_foreign_function_cannot_be_adopted() {
    let text = "kclaude() {\n  claude\n";
    assert_eq!(state(text.as_bytes()), RcState::Foreign);
    assert_eq!(plan_adopt(text.as_bytes(), Dialect::Posix, &block()), Err(Refusal::UnparsableForeign));
}

// --- adopt ---------------------------------------------------------------------------------

#[test]
fn adopt_comments_out_the_old_line_then_appends_the_block() {
    let text = "export A=1\nsource \"$HOME/kiro-gateway/scripts/kclaude.zsh\"\nexport B=2\n";
    let out = s(&plan_adopt(text.as_bytes(), Dialect::Posix, &block()).unwrap().unwrap());
    assert_eq!(
        out,
        format!(
            "export A=1\n# disabled by Kiro Gateway App: source \"$HOME/kiro-gateway/scripts/kclaude.zsh\"\nexport B=2\n{}",
            block()
        )
    );
    assert_eq!(state(out.as_bytes()), RcState::Installed);
}

#[test]
fn adopt_comments_out_every_line_of_a_multiline_function_and_alias() {
    let text = "alias kclaude=claude\nkclaude() {\n  claude\n}\n";
    let out = s(&plan_adopt(text.as_bytes(), Dialect::Posix, &block()).unwrap().unwrap());
    assert!(out.starts_with(
        "# disabled by Kiro Gateway App: alias kclaude=claude\n# disabled by Kiro Gateway App: kclaude() {\n  # disabled by Kiro Gateway App: claude\n# disabled by Kiro Gateway App: }\n"
    ), "{out}");
    assert_eq!(state(out.as_bytes()), RcState::Installed);
}

#[test]
fn adopt_keeps_indentation_and_uses_a_null_command_for_indented_lines() {
    let text = "if true; then\n  source ~/x/kclaude.zsh\nfi\n";
    let out = s(&plan_adopt(text.as_bytes(), Dialect::Posix, &block()).unwrap().unwrap());
    assert!(out.contains("\n  : # disabled by Kiro Gateway App: source ~/x/kclaude.zsh\nfi\n"), "{out}");
}

#[test]
fn adopt_preserves_a_missing_trailing_newline_and_crlf() {
    let text = b"export A=1\r\nsource ~/x/kclaude.zsh";
    let out = plan_adopt(text, Dialect::Posix, &block()).unwrap().unwrap();
    let out = s(&out);
    assert!(out.starts_with("export A=1\r\n# disabled by Kiro Gateway App: source ~/x/kclaude.zsh\r\n# >>>"), "{out:?}");
}

#[test]
fn adopt_on_an_installed_file_without_foreign_is_a_no_op() {
    let text = install(b"export A=1\n").unwrap();
    assert!(plan_adopt(&text, Dialect::Posix, &block()).unwrap().is_none());
}

#[test]
fn adopt_when_block_already_present_only_comments_the_foreign_line() {
    let text = format!("source ~/x/kclaude.zsh\n{}", block());
    let out = s(&plan_adopt(text.as_bytes(), Dialect::Posix, &block()).unwrap().unwrap());
    assert_eq!(out, format!("# disabled by Kiro Gateway App: source ~/x/kclaude.zsh\n{}", block()));
}

#[test]
fn adopt_then_remove_leaves_only_the_commented_lines() {
    let text = "a\nsource ~/x/kclaude.zsh\nb\n";
    let adopted = plan_adopt(text.as_bytes(), Dialect::Posix, &block()).unwrap().unwrap();
    let removed = s(&plan_remove(&adopted, Dialect::Posix).unwrap().unwrap());
    assert_eq!(removed, "a\n# disabled by Kiro Gateway App: source ~/x/kclaude.zsh\nb\n");
}

// --- remove --------------------------------------------------------------------------------

#[test]
fn remove_is_the_exact_inverse_of_install_for_many_shapes() {
    let originals: Vec<&[u8]> = vec![
        b"",
        b"\n",
        b"x",
        b"x\n",
        b"x\n\n",
        b"\n\nx\n\n\n",
        b"export A=1\n\n# comment\nalias ll='ls -l'\n",
        b"a\r\nb\r\n",
        b"a\r\nb",
        b"no newline at end",
        b"\xff\xfe binary-ish\n",
        b"trailing spaces   \n   \n",
    ];
    for original in originals {
        match install(original) {
            Some(installed) => {
                let removed = plan_remove(&installed, Dialect::Posix).unwrap().unwrap();
                assert_eq!(removed, original, "original={:?}", String::from_utf8_lossy(original));
            }
            None => panic!("expected an install plan"),
        }
    }
}

#[test]
fn remove_deletes_a_block_in_the_middle_and_keeps_the_rest_verbatim() {
    let text = format!("top  \n\n{}\n\tbottom\n", block());
    let removed = plan_remove(text.as_bytes(), Dialect::Posix).unwrap().unwrap();
    assert_eq!(s(&removed), "top  \n\n\n\tbottom\n");
}

#[test]
fn remove_without_a_block_plans_nothing() {
    assert_eq!(plan_remove(b"export A=1\n", Dialect::Posix), Ok(None));
    assert_eq!(plan_remove(b"", Dialect::Posix), Ok(None));
}

#[test]
fn remove_deletes_every_duplicate_block() {
    let text = format!("a\n{}b\n{}c\n", block(), block());
    let removed = plan_remove(text.as_bytes(), Dialect::Posix).unwrap().unwrap();
    assert_eq!(s(&removed), "a\nb\nc\n");
}

#[test]
fn remove_works_even_when_a_foreign_kclaude_is_present() {
    let text = format!("source ~/x/kclaude.zsh\n{}", block());
    let removed = plan_remove(text.as_bytes(), Dialect::Posix).unwrap().unwrap();
    assert_eq!(s(&removed), "source ~/x/kclaude.zsh\n");
}

// --- powershell dialect --------------------------------------------------------------------

#[test]
fn powershell_foreign_forms_are_detected() {
    let ps_block = block_text(Dialect::PowerShell, Path::new(r"C:\k\kclaude.ps1"));
    let cases = [
        ". C:\\tools\\kclaude.ps1",
        "function kclaude { claude @args }",
        "function kclaude {\n  claude @args\n}",
        "Function Kclaude() { claude }",
        "Set-Alias kclaude claude",
        "Set-Alias -Name kclaude -Value claude",
        "New-Alias kclaude claude",
        "sal kclaude claude",
    ];
    for case in cases {
        let text = format!("{case}\n");
        assert_eq!(analyze(text.as_bytes(), Dialect::PowerShell).state(&ps_block), RcState::Foreign, "{case}");
    }
    for ok in ["# function kclaude {}", "Write-Host kclaude", "function kclaude-helper { }"] {
        let text = format!("{ok}\n");
        assert_eq!(analyze(text.as_bytes(), Dialect::PowerShell).state(&ps_block), RcState::NotInstalled, "{ok}");
    }
}

#[test]
fn powershell_install_and_remove_round_trip() {
    let b = block_text(Dialect::PowerShell, Path::new(r"C:\k\kclaude.ps1"));
    let original = b"Set-Alias ll Get-ChildItem\r\n";
    let out = plan_install(original, Dialect::PowerShell, &b).unwrap().unwrap();
    assert_eq!(analyze(&out, Dialect::PowerShell).state(&b), RcState::Installed);
    assert_eq!(plan_remove(&out, Dialect::PowerShell).unwrap().unwrap(), original);
}

// --- shell and file selection --------------------------------------------------------------

#[test]
fn shell_env_maps_to_zsh_or_bash_with_zsh_as_the_fallback() {
    assert_eq!(shell_from_env(Some("/bin/zsh")), ShellKind::Zsh);
    assert_eq!(shell_from_env(Some("/opt/homebrew/bin/bash")), ShellKind::Bash);
    assert_eq!(shell_from_env(Some("/usr/bin/bash")), ShellKind::Bash);
    assert_eq!(shell_from_env(Some("/usr/local/bin/fish")), ShellKind::Zsh);
    assert_eq!(shell_from_env(Some("")), ShellKind::Zsh);
    assert_eq!(shell_from_env(None), ShellKind::Zsh);
}

#[test]
fn zsh_always_uses_zshrc() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(choose_rc_file(ShellKind::Zsh, home.path(), true), home.path().join(".zshrc"));
}

#[test]
fn bash_on_macos_without_any_profile_uses_bash_profile() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(choose_rc_file(ShellKind::Bash, home.path(), true), home.path().join(".bash_profile"));
}

#[test]
fn bash_on_linux_without_any_profile_uses_bashrc() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(choose_rc_file(ShellKind::Bash, home.path(), false), home.path().join(".bashrc"));
}

#[test]
fn bash_profile_that_sources_bashrc_means_bashrc() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join(".bash_profile"), "[ -f ~/.bashrc ] && . ~/.bashrc\n").unwrap();
    assert_eq!(choose_rc_file(ShellKind::Bash, home.path(), true), home.path().join(".bashrc"));
}

#[test]
fn bash_profile_that_does_not_source_bashrc_is_used_itself() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join(".bash_profile"), "export A=1\n").unwrap();
    fs::write(home.path().join(".bashrc"), "export B=1\n").unwrap();
    assert_eq!(choose_rc_file(ShellKind::Bash, home.path(), true), home.path().join(".bash_profile"));
}

#[test]
fn bash_without_bash_profile_but_with_profile_uses_bashrc() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join(".profile"), "export A=1\n").unwrap();
    assert_eq!(choose_rc_file(ShellKind::Bash, home.path(), true), home.path().join(".bashrc"));
}

// --- apply: backup and write ---------------------------------------------------------------

fn backups(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.contains(".bak-kiro-gateway-"))
        .collect();
    v.sort();
    v
}

#[test]
fn apply_backs_up_the_original_bytes_before_writing() {
    let home = tempfile::tempdir().unwrap();
    let rc = home.path().join(".zshrc");
    let original = b"export A=1\n\xff\n";
    fs::write(&rc, original).unwrap();

    let new = install(original).unwrap();
    apply(&rc, &new, STAMP).unwrap();

    assert_eq!(fs::read(&rc).unwrap(), new);
    assert_eq!(backups(home.path()), vec![".zshrc.bak-kiro-gateway-20261022-101500".to_string()]);
    assert_eq!(fs::read(home.path().join(".zshrc.bak-kiro-gateway-20261022-101500")).unwrap(), original);
}

#[test]
fn apply_to_a_missing_file_creates_it_without_a_backup() {
    let home = tempfile::tempdir().unwrap();
    let rc = home.path().join(".zshrc");
    apply(&rc, install(b"").unwrap().as_slice(), STAMP).unwrap();
    assert_eq!(s(&fs::read(&rc).unwrap()), block());
    assert!(backups(home.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn apply_does_not_write_when_the_backup_cannot_be_created() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let rc = dir.path().join(".bashrc");
    fs::write(&rc, b"keep me\n").unwrap();
    // A read-only directory: the original can be read but no backup file can be added.
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let can_write_anyway = fs::write(dir.path().join("probe"), b"").is_ok(); // running as root
    let result = apply(&rc, b"changed\n", STAMP);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    if can_write_anyway {
        return;
    }
    let err = result.unwrap_err();
    assert_eq!(err.code, ErrorCode::BackupFailed);
    assert!(!err.message.contains(dir.path().to_str().unwrap()), "message must not leak paths");
    assert_eq!(fs::read(&rc).unwrap(), b"keep me\n");
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn apply_with_an_unreadable_original_reports_backup_failed_and_leaves_it_alone() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let rc = home.path().join(".zshrc");
        fs::write(&rc, b"secret rc\n").unwrap();
        fs::set_permissions(&rc, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::read(&rc).is_ok(); // running as root can still read it
        let result = apply(&rc, b"changed\n", STAMP);
        fs::set_permissions(&rc, fs::Permissions::from_mode(0o644)).unwrap();
        if !readable {
            assert_eq!(result.unwrap_err().code, ErrorCode::BackupFailed);
            assert_eq!(fs::read(&rc).unwrap(), b"secret rc\n");
            assert!(backups(home.path()).is_empty());
        }
    }
}

#[test]
fn two_applies_in_the_same_second_keep_both_backups() {
    let home = tempfile::tempdir().unwrap();
    let rc = home.path().join(".zshrc");
    fs::write(&rc, b"v1\n").unwrap();
    apply(&rc, b"v2\n", STAMP).unwrap();
    apply(&rc, b"v3\n", STAMP).unwrap();
    let names = backups(home.path());
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(fs::read(home.path().join(&names[0])).unwrap(), b"v1\n");
    assert_eq!(fs::read(home.path().join(&names[1])).unwrap(), b"v2\n");
}

#[test]
fn read_rc_treats_a_missing_file_as_empty_and_other_errors_as_errors() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(read_rc(&home.path().join("nope")).unwrap(), Vec::<u8>::new());
    assert!(read_rc(home.path()).is_err(), "a directory is not a readable startup file");
}

#[test]
fn full_cycle_on_disk_install_adopt_remove_keeps_unrelated_bytes() {
    let home = tempfile::tempdir().unwrap();
    let rc = home.path().join(".zshrc");
    let original = b"# my zshrc\nexport EDITOR=vim\nsource \"$HOME/kiro-gateway/scripts/kclaude.zsh\"\nprompt pure\n";
    fs::write(&rc, original).unwrap();

    // install is refused while the foreign line exists
    assert_eq!(plan_install(&read_rc(&rc).unwrap(), Dialect::Posix, &block()), Err(Refusal::Foreign));
    assert_eq!(fs::read(&rc).unwrap(), original);

    // adopt, then verify the file is installed
    let adopted = plan_adopt(&read_rc(&rc).unwrap(), Dialect::Posix, &block()).unwrap().unwrap();
    apply(&rc, &adopted, STAMP).unwrap();
    assert_eq!(state(&fs::read(&rc).unwrap()), RcState::Installed);
    assert_eq!(fs::read(home.path().join(".zshrc.bak-kiro-gateway-20261022-101500")).unwrap(), original);

    // second install: no write
    assert!(install(&fs::read(&rc).unwrap()).is_none());

    // remove: only the block disappears
    let removed = plan_remove(&fs::read(&rc).unwrap(), Dialect::Posix).unwrap().unwrap();
    assert_eq!(
        s(&removed),
        "# my zshrc\nexport EDITOR=vim\n# disabled by Kiro Gateway App: source \"$HOME/kiro-gateway/scripts/kclaude.zsh\"\nprompt pure\n"
    );
}
