//! Post-install check: does a fresh login shell know `kclaude`?
//!
//! macOS: `$SHELL -lic 'type kclaude'` with a 5 second timeout, in the user's real HOME
//! (tests inject a temporary one and a fake shell). The shell and its children are killed
//! when the timeout expires. Output is only classified; it is never copied into the
//! result, so nothing from the user's startup files can reach the UI.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::ps_profile::PolicyHint;
use super::types::{KclaudeVerify, VerifyStatus};

/// Time a fresh login shell gets before the check is abandoned.
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(5);
/// Arguments of the check: login, interactive, run one command.
pub const LOGIN_TYPE_ARGS: [&str; 2] = ["-lic", "type kclaude"];

const MSG_NOT_FOUND: &str = "新开的终端里找不到 kclaude";
const MAX_CAPTURE: usize = 256 * 1024;

/// What happened when the shell ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Finished { code: Option<i32>, stdout: String, stderr: String },
    TimedOut,
    SpawnFailed,
}

/// Run `shell args...` with `HOME=home`, killing it (and its process group) after `timeout`.
pub fn run_shell(shell: &Path, args: &[&str], home: &Path, timeout: Duration) -> RunOutcome {
    run_shell_with_env(shell, args, home, timeout, &[])
}

/// Like [`run_shell`] with environment changes applied after the inherited ones
/// (`Some` sets, `None` removes).
///
/// Args:
///     shell: Shell executable.
///     args: Arguments, e.g. [`LOGIN_TYPE_ARGS`].
///     home: Value of `HOME` for the child.
///     timeout: Hard limit.
///     env_changes: Variables to set or remove.
pub fn run_shell_with_env(
    shell: &Path,
    args: &[&str],
    home: &Path,
    timeout: Duration,
    env_changes: &[(&str, Option<&str>)],
) -> RunOutcome {
    let mut cmd = Command::new(shell);
    cmd.args(args)
        .env("HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env_changes {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group so a hung startup file's children can be killed together.
        cmd.process_group(0);
    }
    let Ok(mut child) = cmd.spawn() else {
        return RunOutcome::SpawnFailed;
    };
    let out_reader = drain(child.stdout.take());
    let err_reader = drain(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => break None,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    match status {
        Some(status) => {
            let stdout = out_reader.join().unwrap_or_default();
            let stderr = err_reader.join().unwrap_or_default();
            RunOutcome::Finished { code: status.code(), stdout, stderr }
        }
        None => {
            kill_tree(&mut child);
            let _ = child.wait();
            // Reader threads end when the pipes close; background children were killed with
            // the group, so this does not block.
            let _ = out_reader.join();
            let _ = err_reader.join();
            RunOutcome::TimedOut
        }
    }
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut kept = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut buf = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                kept.extend_from_slice(&buf[..n]);
                // Keep only the tail: the `type` answer is the last thing the shell prints,
                // and reading on (instead of stopping) keeps the child from blocking on a
                // full pipe.
                if kept.len() > 2 * MAX_CAPTURE {
                    kept.drain(..kept.len() - MAX_CAPTURE);
                }
            }
        }
        if kept.len() > MAX_CAPTURE {
            kept.drain(..kept.len() - MAX_CAPTURE);
        }
        String::from_utf8_lossy(&kept).into_owned()
    })
}

#[cfg(unix)]
fn kill_tree(child: &mut std::process::Child) {
    // SAFETY: killpg on the group created for this child; the pid is our own live child.
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

fn fail(detail: &str) -> KclaudeVerify {
    KclaudeVerify { status: VerifyStatus::Fail, detail: detail.to_string(), fix: None }
}

/// Classify the output of `type kclaude` from zsh or bash.
///
/// Only a shell *function* counts: an alias or an executable with the same name would not
/// carry the App's environment handling.
pub fn verify_posix(outcome: &RunOutcome) -> KclaudeVerify {
    let stdout = match outcome {
        RunOutcome::SpawnFailed => return fail("没能启动终端做检查，请手动新开一个终端试试"),
        RunOutcome::TimedOut => {
            return fail("新开的终端 5 秒内没有响应，检查启动文件里有没有会卡住的命令")
        }
        RunOutcome::Finished { stdout, .. } => stdout,
    };
    let Some(line) = stdout.lines().map(str::trim).find(|l| l.starts_with("kclaude is ")) else {
        return fail(MSG_NOT_FOUND);
    };
    let rest = &line["kclaude is ".len()..];
    if rest.starts_with("a function") || rest.starts_with("a shell function") {
        KclaudeVerify { status: VerifyStatus::Ok, detail: "新开的终端里可以使用 kclaude".to_string(), fix: None }
    } else if rest.starts_with("an alias") || rest.starts_with("aliased to") {
        fail("新开的终端里 kclaude 被别名盖住了，检查启动文件里后面有没有 alias kclaude")
    } else {
        fail("新开的终端里 kclaude 是另一个同名命令，不是 App 装的那个")
    }
}

/// Classify the output of a PowerShell `(Get-Command kclaude).CommandType` check.
///
/// Args:
///     outcome: Result of running PowerShell.
///     policy_hint: Execution-policy problem found beforehand, if any; used to explain a
///         failure and to offer the copyable fix.
pub fn verify_powershell(outcome: &RunOutcome, policy_hint: Option<PolicyHint>) -> KclaudeVerify {
    if let RunOutcome::Finished { stdout, .. } = outcome {
        if stdout.lines().any(|l| l.trim().eq_ignore_ascii_case("function")) {
            return KclaudeVerify {
                status: VerifyStatus::Ok,
                detail: "新开的终端里可以使用 kclaude".to_string(),
                fix: None,
            };
        }
    }
    if let Some(hint) = policy_hint {
        return KclaudeVerify { status: hint.status, detail: hint.detail, fix: hint.fix };
    }
    match outcome {
        RunOutcome::SpawnFailed => fail("没能启动 PowerShell 做检查，请手动新开一个终端试试"),
        RunOutcome::TimedOut => fail("新开的 PowerShell 5 秒内没有响应，检查 profile 里有没有会卡住的命令"),
        RunOutcome::Finished { .. } => fail(MSG_NOT_FOUND),
    }
}

#[cfg(test)]
mod tests;
