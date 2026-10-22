//! Tests for the fresh-login-shell check (`cargo test kclaude::verify`).
//!
//! Fake shells are small executable scripts in a temp dir; the "real shell" tests run
//! /bin/zsh and /bin/bash against a temporary HOME that contains a rc file with the block.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;
use crate::kclaude::rc::{block_text, Dialect};
use crate::kclaude::script::{install, ScriptKind};
use crate::kclaude::types::{FixAction, VerifyStatus};

fn fake_shell(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("fakeshell");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn run(shell: &Path, home: &Path, timeout: Duration) -> KclaudeVerify {
    verify_posix(&run_shell(shell, &LOGIN_TYPE_ARGS, home, timeout))
}

// --- interpretation of shell output ------------------------------------------------------

fn done(stdout: &str, code: i32) -> RunOutcome {
    RunOutcome::Finished { code: Some(code), stdout: stdout.to_string(), stderr: String::new() }
}

#[test]
fn zsh_function_output_is_ok() {
    let v = verify_posix(&done("kclaude is a shell function from /x/kclaude.zsh\n", 0));
    assert_eq!(v.status, VerifyStatus::Ok);
    assert!(v.fix.is_none());
}

#[test]
fn bash_function_output_is_ok() {
    let v = verify_posix(&done("kclaude is a function\nkclaude () \n{ \n  :\n}\n", 0));
    assert_eq!(v.status, VerifyStatus::Ok);
}

#[test]
fn startup_noise_before_the_type_line_is_ignored() {
    let v = verify_posix(&done("Last login: today\n[oh-my-zsh] update?\nkclaude is a shell function from /x\n", 0));
    assert_eq!(v.status, VerifyStatus::Ok);
}

#[test]
fn not_found_variants_fail_with_the_new_terminal_message() {
    for (out, code) in [("kclaude not found\n", 1), ("", 1), ("bash: type: kclaude: not found\n", 1), ("hello\n", 0)] {
        let v = verify_posix(&done(out, code));
        assert_eq!(v.status, VerifyStatus::Fail, "{out:?}");
        assert_eq!(v.detail, "新开的终端里找不到 kclaude", "{out:?}");
    }
}

#[test]
fn an_alias_shadowing_the_function_is_reported_as_such() {
    let v = verify_posix(&done("kclaude is an alias for claude\n", 0));
    assert_eq!(v.status, VerifyStatus::Fail);
    assert!(v.detail.contains("别名"), "{}", v.detail);
}

#[test]
fn an_executable_named_kclaude_is_not_our_function() {
    let v = verify_posix(&done("kclaude is /usr/local/bin/kclaude\n", 0));
    assert_eq!(v.status, VerifyStatus::Fail);
    assert!(!v.detail.contains("/usr/local"), "details must not echo paths: {}", v.detail);
}

#[test]
fn timeout_and_spawn_failure_have_their_own_messages() {
    let t = verify_posix(&RunOutcome::TimedOut);
    assert_eq!(t.status, VerifyStatus::Fail);
    assert!(t.detail.contains("5 秒"), "{}", t.detail);
    let s = verify_posix(&RunOutcome::SpawnFailed);
    assert_eq!(s.status, VerifyStatus::Fail);
    assert!(s.detail.contains("没能启动"), "{}", s.detail);
}

#[test]
fn detail_never_contains_shell_output_beyond_the_fixed_messages() {
    let v = verify_posix(&done("kclaude is an alias for TOKEN=sk-abcdef0123456789abcdef0123456789\n", 0));
    assert!(!v.detail.contains("sk-"), "{}", v.detail);
}

// --- running a process: success and failure with a fake SHELL ------------------------------

#[cfg(unix)]
#[test]
fn fake_shell_that_prints_a_function_line_verifies_ok_and_receives_lic_type_kclaude() {
    let dir = tempfile::tempdir().unwrap();
    let args_file = dir.path().join("args");
    let shell = fake_shell(
        dir.path(),
        &format!("echo \"$@\" > '{}'\necho \"HOME=$HOME\" >> '{}'\necho 'kclaude is a shell function from /x'", args_file.display(), args_file.display()),
    );
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let v = run(&shell, &home, Duration::from_secs(5));
    assert_eq!(v.status, VerifyStatus::Ok, "{v:?}");
    let recorded = fs::read_to_string(&args_file).unwrap();
    assert!(recorded.starts_with("-lic type kclaude\n"), "{recorded}");
    assert!(recorded.contains(&format!("HOME={}", home.display())), "{recorded}");
}

#[cfg(unix)]
#[test]
fn fake_shell_that_cannot_find_kclaude_verifies_fail() {
    let dir = tempfile::tempdir().unwrap();
    let shell = fake_shell(dir.path(), "echo 'kclaude not found'; exit 1");
    let v = run(&shell, dir.path(), Duration::from_secs(5));
    assert_eq!(v.status, VerifyStatus::Fail);
    assert_eq!(v.detail, "新开的终端里找不到 kclaude");
}

#[cfg(unix)]
#[test]
fn hanging_shell_is_killed_at_the_timeout_including_its_children() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    // The first line lets the test warm the file up (first exec of a new file can be slow
    // under load) without starting the hang.
    let shell = fake_shell(
        dir.path(),
        &format!("[ \"$1\" = --warm ] && exit 0\nsleep 30 &\necho $! > '{}'\nwait", pid_file.display()),
    );
    let warm = std::process::Command::new(&shell).arg("--warm").status().unwrap();
    assert!(warm.success());

    let started = Instant::now();
    let v = run(&shell, dir.path(), Duration::from_secs(3));
    assert!(started.elapsed() < Duration::from_secs(8), "took {:?}", started.elapsed());
    assert_eq!(v.status, VerifyStatus::Fail);
    assert!(v.detail.contains("5 秒"));
    // The background sleep must be gone too (poll briefly: kill is asynchronous).
    let pid: i32 = fs::read_to_string(&pid_file).expect("the shell started before the timeout").trim().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let gone = loop {
        let alive = std::process::Command::new("kill").args(["-0", &pid.to_string()]).output().map(|o| o.status.success()).unwrap_or(false);
        if !alive || Instant::now() > deadline {
            break !alive;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(gone, "background child {pid} survived the timeout");
}

#[test]
fn missing_shell_binary_is_a_spawn_failure_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let v = run(&dir.path().join("no-such-shell"), dir.path(), Duration::from_secs(1));
    assert_eq!(v.status, VerifyStatus::Fail);
    assert!(v.detail.contains("没能启动"), "{}", v.detail);
}

#[cfg(unix)]
#[test]
fn shell_that_floods_stdout_does_not_deadlock() {
    let dir = tempfile::tempdir().unwrap();
    let shell = fake_shell(dir.path(), "yes 'noise noise noise noise' | head -c 3000000\necho 'kclaude is a shell function from /x'");
    let v = run(&shell, dir.path(), Duration::from_secs(10));
    assert_eq!(v.status, VerifyStatus::Ok, "{v:?}");
}

#[cfg(unix)]
#[test]
fn env_changes_set_and_remove_variables_for_the_child() {
    let dir = tempfile::tempdir().unwrap();
    let shell = fake_shell(dir.path(), "echo \"ZDOTDIR=${ZDOTDIR-unset}\"; echo 'kclaude is a shell function'");
    let outcome = run_shell_with_env(&shell, &LOGIN_TYPE_ARGS, dir.path(), Duration::from_secs(5), &[("ZDOTDIR", Some("/elsewhere"))]);
    match outcome {
        RunOutcome::Finished { stdout, .. } => assert!(stdout.contains("ZDOTDIR=/elsewhere"), "{stdout}"),
        other => panic!("{other:?}"),
    }
}

// --- real shells with a temporary HOME -------------------------------------------------------

fn real_shell_case(shell_path: &str, kind: ScriptKind, rc_name: &str, with_block: bool) -> KclaudeVerify {
    if !Path::new(shell_path).exists() {
        panic!("{shell_path} is required on this platform");
    }
    let home = tempfile::tempdir().unwrap();
    let data_dir = home.path().join("Library/Application Support/dev.kirogateway.desktop");
    install(kind, &data_dir).unwrap();
    let rc = home.path().join(rc_name);
    if with_block {
        let block = block_text(Dialect::Posix, &data_dir.join(kind.file_name()));
        fs::write(&rc, format!("export FOO=1\n{block}")).unwrap();
    } else {
        fs::write(&rc, "export FOO=1\n").unwrap();
    }
    // A login shell also reads .bash_profile / .zprofile; keep them from sourcing the real ones.
    if rc_name == ".bashrc" {
        fs::write(home.path().join(".bash_profile"), "[ -f ~/.bashrc ] && . ~/.bashrc\n").unwrap();
    }
    let outcome = run_shell_with_env(
        Path::new(shell_path),
        &LOGIN_TYPE_ARGS,
        home.path(),
        Duration::from_secs(5),
        &[("ZDOTDIR", None)],
    );
    verify_posix(&outcome)
}

#[cfg(unix)]
#[test]
fn real_zsh_in_temp_home_finds_kclaude_after_install() {
    let v = real_shell_case("/bin/zsh", ScriptKind::Zsh, ".zshrc", true);
    assert_eq!(v.status, VerifyStatus::Ok, "{v:?}");
}

#[cfg(unix)]
#[test]
fn real_zsh_in_temp_home_without_the_block_fails() {
    let v = real_shell_case("/bin/zsh", ScriptKind::Zsh, ".zshrc", false);
    assert_eq!(v.status, VerifyStatus::Fail, "{v:?}");
    assert_eq!(v.detail, "新开的终端里找不到 kclaude");
}

#[cfg(unix)]
#[test]
fn real_bash_in_temp_home_finds_kclaude_through_bashrc() {
    let v = real_shell_case("/bin/bash", ScriptKind::Bash, ".bashrc", true);
    assert_eq!(v.status, VerifyStatus::Ok, "{v:?}");
}

#[cfg(unix)]
#[test]
fn real_bash_in_temp_home_without_the_block_fails() {
    let v = real_shell_case("/bin/bash", ScriptKind::Bash, ".bashrc", false);
    assert_eq!(v.status, VerifyStatus::Fail, "{v:?}");
}

#[cfg(unix)]
#[test]
fn real_zsh_where_a_later_line_shadows_kclaude_with_an_alias_fails() {
    let home = tempfile::tempdir().unwrap();
    let data_dir = home.path().join("Application Support/app");
    install(ScriptKind::Zsh, &data_dir).unwrap();
    let block = block_text(Dialect::Posix, &data_dir.join("kclaude.zsh"));
    fs::write(home.path().join(".zshrc"), format!("{block}alias kclaude='echo shadow'\n")).unwrap();
    let outcome = run_shell_with_env(Path::new("/bin/zsh"), &LOGIN_TYPE_ARGS, home.path(), Duration::from_secs(5), &[("ZDOTDIR", None)]);
    let v = verify_posix(&outcome);
    assert_eq!(v.status, VerifyStatus::Fail, "{v:?}");
    assert!(v.detail.contains("别名"));
}

// --- PowerShell interpretation (pure) ---------------------------------------------------------

#[test]
fn powershell_function_output_is_ok_and_missing_command_fails() {
    let ok = verify_powershell(&done("Function\r\n", 0), None);
    assert_eq!(ok.status, VerifyStatus::Ok);
    let missing = verify_powershell(&done("", 1), None);
    assert_eq!(missing.status, VerifyStatus::Fail);
    assert_eq!(missing.detail, "新开的终端里找不到 kclaude");
}

#[test]
fn powershell_blocked_by_policy_reports_the_policy_with_a_copyable_fix() {
    let hint = crate::kclaude::ps_profile::execution_policy_hint(&Default::default());
    let v = verify_powershell(&done("", 1), hint);
    assert_eq!(v.status, VerifyStatus::Fail);
    assert!(v.detail.contains("执行策略"));
    assert!(matches!(v.fix, Some(FixAction::CopyCommand { .. })));
}

#[test]
fn powershell_policy_hint_wins_over_a_function_that_somehow_loaded() {
    // If the policy blocks profiles but the check still saw the function (profile loaded by
    // other means), the check result is what counts.
    let hint = crate::kclaude::ps_profile::execution_policy_hint(&Default::default());
    let v = verify_powershell(&done("Function\n", 0), hint);
    assert_eq!(v.status, VerifyStatus::Ok);
}
