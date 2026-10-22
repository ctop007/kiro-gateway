use super::*;
use crate::gateway::testing::wait_until;
use std::process::{Command, Stdio};
use std::time::Duration;

fn spawn_sleep() -> std::process::Child {
    let mut cmd = Command::new(if cfg!(windows) { "ping" } else { "/bin/sleep" });
    if cfg!(windows) {
        cmd.args(["-n", "30", "127.0.0.1"]);
    } else {
        cmd.arg("30");
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap()
}

#[test]
fn own_process_is_alive_and_a_surely_dead_pid_is_not() {
    assert!(SystemProcessOps.is_alive(std::process::id()));
    let mut child = spawn_sleep();
    let pid = child.id();
    assert!(SystemProcessOps.is_alive(pid));
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!SystemProcessOps.is_alive(pid), "a reaped child must not count as alive");
}

#[cfg(unix)]
#[test]
fn a_killed_but_not_yet_reaped_child_is_not_alive() {
    // The exited child stays a zombie until wait(); it must not count as running, or a
    // "wait until it is gone" loop would spin until its timeout.
    let mut child = spawn_sleep();
    let pid = child.id();
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    assert!(wait_until(Duration::from_secs(2), || !SystemProcessOps.is_alive(pid)), "zombie counted as alive");
    child.wait().unwrap();
}

#[test]
fn pid_zero_and_overflowing_pids_are_never_alive() {
    assert!(!SystemProcessOps.is_alive(0));
    assert!(!SystemProcessOps.is_alive(u32::MAX));
    assert_eq!(SystemProcessOps.image_path(0), None);
    assert_eq!(SystemProcessOps.image_path(u32::MAX), None);
}

#[test]
fn image_path_of_own_process_is_the_test_executable() {
    let mine = SystemProcessOps.image_path(std::process::id()).expect("image path of self");
    let expected = std::env::current_exe().unwrap();
    assert_eq!(mine.canonicalize().unwrap(), expected.canonicalize().unwrap());
}

#[test]
fn image_path_of_a_spawned_program_is_that_program() {
    let mut child = spawn_sleep();
    let path = SystemProcessOps.image_path(child.id()).expect("image path of child");
    let name = path.file_name().unwrap().to_string_lossy().to_lowercase();
    assert!(name.starts_with(if cfg!(windows) { "ping" } else { "sleep" }), "{name}");
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn image_path_of_a_dead_process_is_none() {
    let mut child = spawn_sleep();
    let pid = child.id();
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(SystemProcessOps.image_path(pid), None);
}

#[cfg(unix)]
#[test]
fn terminate_ends_a_normal_process_with_sigterm() {
    use std::os::unix::process::ExitStatusExt;
    let mut child = spawn_sleep();
    SystemProcessOps.terminate(child.id());
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM));
}

#[cfg(unix)]
#[test]
fn kill_ends_a_process_that_ignores_sigterm() {
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", "trap '' TERM; while :; do sleep 1; done"]).process_group(0);
    let mut child = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let pid = child.id();
    std::thread::sleep(Duration::from_millis(200));
    SystemProcessOps.terminate(pid);
    std::thread::sleep(Duration::from_millis(300));
    assert!(child.try_wait().unwrap().is_none(), "SIGTERM must have been ignored");
    SystemProcessOps.kill(pid);
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
}

#[cfg(unix)]
#[test]
fn kill_takes_the_whole_process_group_with_it() {
    use std::os::unix::process::CommandExt;
    // The shell starts a background sleeper in the same group and prints its pid.
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", "sleep 60 & echo $!; wait"]).process_group(0).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(child.stdout.take().unwrap()), &mut line).unwrap();
    let sleeper: u32 = line.trim().parse().unwrap();
    assert!(SystemProcessOps.is_alive(sleeper));
    SystemProcessOps.kill(child.id());
    child.wait().unwrap();
    assert!(wait_until(Duration::from_secs(2), || !SystemProcessOps.is_alive(sleeper)), "grandchild left behind");
}

#[test]
fn signalling_a_dead_or_invalid_pid_is_harmless() {
    let mut child = spawn_sleep();
    let pid = child.id();
    child.kill().unwrap();
    child.wait().unwrap();
    SystemProcessOps.terminate(pid);
    SystemProcessOps.kill(pid);
    SystemProcessOps.terminate(0);
    SystemProcessOps.kill(0);
    SystemProcessOps.terminate(u32::MAX);
    SystemProcessOps.kill(u32::MAX);
}

#[test]
fn never_signals_pid_zero_one_or_itself() {
    // pid 0 would be "my own process group", pid 1 is init / launchd; both are refused.
    assert!(!is_signalable(0));
    assert!(!is_signalable(1));
    assert!(!is_signalable(std::process::id()));
    assert!(!is_signalable(u32::MAX));
    assert!(is_signalable(std::process::id().wrapping_add(7)));
}
