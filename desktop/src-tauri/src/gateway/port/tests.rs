use super::*;
use crate::gateway::sys::ProcessOps;
use crate::gateway::testing::{free_port, FakeOps};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::tempdir;

// ---------------------------------------------------------------- bind probe

#[test]
fn a_free_port_is_free() {
    assert!(SystemPorts.is_free(free_port()));
}

#[test]
fn a_loopback_listener_makes_the_port_taken_and_dropping_it_frees_it_again() {
    let port = free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    assert!(!SystemPorts.is_free(port));
    drop(listener);
    assert!(SystemPorts.is_free(port), "a port that was just released must be usable at once");
}

#[test]
fn a_wildcard_listener_counts_as_taken_too() {
    // `python main.py` listens on 0.0.0.0 by default; a bind on 127.0.0.1 alone would miss it.
    let port = free_port();
    let _listener = TcpListener::bind(("0.0.0.0", port)).unwrap();
    assert!(!SystemPorts.is_free(port));
}

#[test]
fn probing_does_not_leave_the_port_occupied() {
    let port = free_port();
    for _ in 0..5 {
        assert!(SystemPorts.is_free(port));
    }
    TcpListener::bind(("127.0.0.1", port)).unwrap();
}

#[test]
fn probing_a_taken_port_is_fast() {
    let port = free_port();
    let _listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let started = Instant::now();
    assert!(!SystemPorts.is_free(port));
    assert!(started.elapsed() < Duration::from_secs(2));
}

// ----------------------------------------------------------------- occupant

#[test]
fn the_occupant_of_a_port_is_found_by_pid_and_name() {
    let port = free_port();
    let _listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let occupant = SystemPorts.occupant(port);
    assert_eq!(occupant.pid, Some(std::process::id()));
    assert!(occupant.name.as_deref().is_some_and(|n| !n.is_empty()), "{occupant:?}");
}

#[test]
fn the_occupant_of_a_wildcard_listener_is_found_too() {
    let port = free_port();
    let _listener = TcpListener::bind(("0.0.0.0", port)).unwrap();
    assert_eq!(SystemPorts.occupant(port).pid, Some(std::process::id()));
}

#[test]
fn a_port_nobody_listens_on_has_no_occupant() {
    let occupant = SystemPorts.occupant(free_port());
    assert_eq!(occupant, PortOccupant { name: None, pid: None });
}

#[test]
fn lsof_arguments_are_fixed_and_contain_nothing_from_outside() {
    let args = lsof_args(8123);
    assert_eq!(args, vec!["-nP", "-iTCP:8123", "-sTCP:LISTEN", "-Fpc"]);
}

#[test]
fn lsof_output_for_one_listener_is_parsed() {
    let out = "p84400\ncpython3.13\nf3\n";
    assert_eq!(parse_lsof(out), PortOccupant { name: Some("python3.13".into()), pid: Some(84400) });
}

#[test]
fn lsof_output_listing_ipv4_and_ipv6_of_the_same_process_reports_it_once() {
    let out = "p61088\ncpython3.13\nf9\nf10\n";
    assert_eq!(parse_lsof(out).pid, Some(61088));
}

#[test]
fn lsof_output_with_several_processes_reports_the_first() {
    let out = "p100\ncnode\nf3\np200\ncpython\nf4\n";
    assert_eq!(parse_lsof(out), PortOccupant { name: Some("node".into()), pid: Some(100) });
}

#[test]
fn lsof_process_names_with_spaces_are_kept() {
    let out = "p77\ncGoogle Chrome Helper\nf30\n";
    assert_eq!(parse_lsof(out).name.as_deref(), Some("Google Chrome Helper"));
}

#[test]
fn lsof_output_without_a_name_gives_only_the_pid() {
    assert_eq!(parse_lsof("p55\nf3\n"), PortOccupant { name: None, pid: Some(55) });
}

#[test]
fn lsof_garbage_and_empty_output_give_no_occupant() {
    for out in ["", "\n\n", "lsof: WARNING: can't stat() fuse file system\n", "pabc\ncfoo\n", "p-5\ncfoo\n", "p0\ncfoo\n"] {
        assert_eq!(parse_lsof(out), PortOccupant { name: None, pid: None }, "{out:?}");
    }
}

#[test]
fn netstat_listing_is_parsed_for_the_wanted_port_only() {
    let out = "\
  Proto  Local Address          Foreign Address        State           PID\r\n\
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1104\r\n\
  TCP    127.0.0.1:8000         0.0.0.0:0              LISTENING       4242\r\n\
  TCP    127.0.0.1:8000         127.0.0.1:50122        ESTABLISHED     4242\r\n\
  TCP    [::]:8001              [::]:0                 LISTENING       999\r\n";
    assert_eq!(parse_netstat(out, 8000), Some(4242));
    assert_eq!(parse_netstat(out, 8001), Some(999));
    assert_eq!(parse_netstat(out, 80), None);
    assert_eq!(parse_netstat(out, 800), None, "800 must not match 8000");
    assert_eq!(parse_netstat("", 8000), None);
}

#[test]
fn tasklist_csv_gives_the_image_name() {
    assert_eq!(parse_tasklist("\"python.exe\",\"4242\",\"Console\",\"1\",\"30,000 K\"\r\n").as_deref(), Some("python.exe"));
    assert_eq!(parse_tasklist("INFO: No tasks are running which match the specified criteria.\r\n"), None);
    assert_eq!(parse_tasklist(""), None);
}

// ------------------------------------------------------------------ pid file

fn record(pid: u32, exe: &str) -> PidRecord {
    PidRecord { pid, exe: PathBuf::from(exe) }
}

#[test]
fn pid_file_round_trips_pid_and_executable_path() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("gateway.pid");
    write_pid_file(&file, &record(4242, "/Applications/Kiro Gateway.app/Contents/Resources/gateway/kiro-gateway")).unwrap();
    assert_eq!(
        read_pid_file(&file),
        PidFile::Valid(record(4242, "/Applications/Kiro Gateway.app/Contents/Resources/gateway/kiro-gateway"))
    );
}

#[test]
fn missing_pid_file_is_reported_as_missing() {
    let dir = tempdir().unwrap();
    assert_eq!(read_pid_file(&dir.path().join("gateway.pid")), PidFile::Missing);
}

#[test]
fn damaged_pid_files_are_reported_as_corrupt() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", b"".to_vec()),
        ("text", b"4242".to_vec()),
        ("truncated", b"{\"pid\": 42".to_vec()),
        ("array", b"[]".to_vec()),
        ("missing exe", b"{\"pid\": 42}".to_vec()),
        ("missing pid", b"{\"exe\": \"/x\"}".to_vec()),
        ("string pid", b"{\"pid\": \"42\", \"exe\": \"/x\"}".to_vec()),
        ("negative pid", b"{\"pid\": -4, \"exe\": \"/x\"}".to_vec()),
        ("zero pid", b"{\"pid\": 0, \"exe\": \"/x\"}".to_vec()),
        ("huge pid", b"{\"pid\": 99999999999999, \"exe\": \"/x\"}".to_vec()),
        ("empty exe", b"{\"pid\": 42, \"exe\": \"\"}".to_vec()),
        ("binary", vec![0xff, 0xfe, 0x00]),
    ];
    for (label, bytes) in cases {
        let dir = tempdir().unwrap();
        let file = dir.path().join("gateway.pid");
        fs::write(&file, bytes).unwrap();
        assert_eq!(read_pid_file(&file), PidFile::Corrupt, "{label}");
    }
}

#[test]
fn a_directory_in_place_of_the_pid_file_is_corrupt_not_a_panic() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("gateway.pid");
    fs::create_dir(&file).unwrap();
    assert_eq!(read_pid_file(&file), PidFile::Corrupt);
}

#[test]
fn writing_the_pid_file_replaces_the_old_one_and_leaves_no_litter() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("gateway.pid");
    write_pid_file(&file, &record(1000, "/a")).unwrap();
    write_pid_file(&file, &record(2000, "/b")).unwrap();
    assert_eq!(read_pid_file(&file), PidFile::Valid(record(2000, "/b")));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn removing_a_missing_pid_file_is_fine() {
    let dir = tempdir().unwrap();
    remove_pid_file(&dir.path().join("gateway.pid"));
}

// ------------------------------------------------------------------- reclaim

const SIDECAR: &str = "/Applications/Kiro Gateway.app/Contents/Resources/gateway/kiro-gateway";

fn pid_file(dir: &Path, rec: Option<PidRecord>) -> PathBuf {
    let file = dir.join("gateway.pid");
    if let Some(rec) = rec {
        write_pid_file(&file, &rec).unwrap();
    }
    file
}

/// Scripted port inspector: `listener` is the pid that "listens" on the port, if any.
struct FakePorts {
    listener: Option<u32>,
}

impl PortOps for FakePorts {
    fn is_free(&self, _: u16) -> bool {
        self.listener.is_none()
    }
    fn occupant(&self, _: u16) -> PortOccupant {
        PortOccupant { name: self.listener.map(|_| "kiro-gateway".to_string()), pid: self.listener }
    }
}

const PORT: u16 = 8123;

fn reclaim(file: &Path, ops: &FakeOps) -> Reclaim {
    reclaim_stale(file, Path::new(SIDECAR), PORT, ops, &FakePorts { listener: None }, Duration::from_millis(100))
}

#[test]
fn nothing_to_reclaim_without_a_pid_file() {
    let dir = tempdir().unwrap();
    let ops = FakeOps::new();
    assert_eq!(reclaim(&pid_file(dir.path(), None), &ops), Reclaim::NothingToDo);
    assert!(ops.calls().is_empty());
}

#[test]
fn a_leftover_gateway_of_ours_is_terminated_and_the_record_removed() {
    let dir = tempdir().unwrap();
    let file = pid_file(dir.path(), Some(record(4242, SIDECAR)));
    let ops = FakeOps::new().with_process(4242, Some(Path::new(SIDECAR)));
    assert_eq!(reclaim(&file, &ops), Reclaim::Reclaimed { pid: 4242 });
    assert_eq!(ops.calls(), vec!["terminate 4242"]);
    assert!(!file.exists());
}

#[test]
fn a_leftover_that_ignores_sigterm_is_killed_after_the_grace_period() {
    let dir = tempdir().unwrap();
    let file = pid_file(dir.path(), Some(record(4242, SIDECAR)));
    let ops = FakeOps::new().with_process(4242, Some(Path::new(SIDECAR))).stubborn(4242);
    let started = Instant::now();
    assert_eq!(reclaim(&file, &ops), Reclaim::Reclaimed { pid: 4242 });
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert_eq!(ops.calls(), vec!["terminate 4242", "kill 4242"]);
    assert!(!file.exists());
}

#[test]
fn a_dead_pid_only_removes_the_stale_record() {
    let dir = tempdir().unwrap();
    let file = pid_file(dir.path(), Some(record(4242, SIDECAR)));
    let ops = FakeOps::new();
    assert_eq!(reclaim(&file, &ops), Reclaim::StaleRecord);
    assert!(ops.calls().is_empty());
    assert!(!file.exists());
}

#[test]
fn a_pid_reused_by_another_program_is_never_signalled() {
    let dir = tempdir().unwrap();
    let file = pid_file(dir.path(), Some(record(4242, SIDECAR)));
    let ops = FakeOps::new().with_process(4242, Some(Path::new("/Applications/Safari.app/Contents/MacOS/Safari")));
    assert_eq!(reclaim(&file, &ops), Reclaim::NotOurs { pid: 4242 });
    assert!(ops.calls().is_empty());
    assert!(!file.exists(), "the record no longer describes anything of ours");
}

#[test]
fn an_alive_process_whose_path_cannot_be_read_is_left_alone() {
    let dir = tempdir().unwrap();
    let file = pid_file(dir.path(), Some(record(4242, SIDECAR)));
    let ops = FakeOps::new().with_process(4242, None);
    assert_eq!(reclaim(&file, &ops), Reclaim::NotOurs { pid: 4242 });
    assert!(ops.calls().is_empty());
}

#[test]
fn a_record_written_for_another_executable_is_not_trusted() {
    // Process path equals the record, but the record is not our current sidecar:
    // an old install location or a hand-edited file. Only report, never kill.
    let dir = tempdir().unwrap();
    let other = "/opt/other/kiro-gateway";
    let file = pid_file(dir.path(), Some(record(4242, other)));
    let ops = FakeOps::new().with_process(4242, Some(Path::new(other)));
    assert_eq!(reclaim(&file, &ops), Reclaim::NotOurs { pid: 4242 });
    assert!(ops.calls().is_empty());
}

#[test]
fn a_corrupt_record_is_removed_and_nothing_is_killed() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("gateway.pid");
    fs::write(&file, "garbage").unwrap();
    let ops = FakeOps::new().with_process(4242, Some(Path::new(SIDECAR)));
    assert_eq!(reclaim(&file, &ops), Reclaim::CorruptRecord);
    assert!(ops.calls().is_empty());
    assert!(!file.exists());
}

#[test]
fn the_app_never_terminates_itself_or_init_through_a_forged_record() {
    for pid in [std::process::id(), 1] {
        let dir = tempdir().unwrap();
        let file = pid_file(dir.path(), Some(record(pid, SIDECAR)));
        let ops = FakeOps::new().with_process(pid, Some(Path::new(SIDECAR)));
        assert_eq!(reclaim(&file, &ops), Reclaim::NotOurs { pid });
        assert!(ops.calls().is_empty(), "pid {pid}");
    }
}

#[test]
fn a_process_that_cannot_be_ended_is_reported_as_failed() {
    struct Immortal;
    impl ProcessOps for Immortal {
        fn is_alive(&self, _: u32) -> bool {
            true
        }
        fn image_path(&self, _: u32) -> Option<PathBuf> {
            Some(PathBuf::from(SIDECAR))
        }
        fn terminate(&self, _: u32) {}
        fn kill(&self, _: u32) {}
    }
    let dir = tempdir().unwrap();
    let file = pid_file(dir.path(), Some(record(4242, SIDECAR)));
    let outcome = reclaim_stale(&file, Path::new(SIDECAR), PORT, &Immortal, &FakePorts { listener: None }, Duration::from_millis(50));
    assert_eq!(outcome, Reclaim::Failed { pid: 4242 });
    assert!(file.exists(), "the record stays so the next start tries again");
}

#[test]
fn path_comparison_follows_symlinks() {
    let dir = tempdir().unwrap();
    let real = dir.path().join("real-gateway");
    fs::write(&real, "x").unwrap();
    #[cfg(unix)]
    {
        let link = dir.path().join("link-gateway");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(same_path(&real, &link));
    }
    assert!(same_path(&real, &real));
    assert!(!same_path(&real, &dir.path().join("other")));
    assert!(!same_path(Path::new("/nope/a"), Path::new("/nope/b")));
    assert!(same_path(Path::new("/nope/a"), Path::new("/nope/a")));
}

#[cfg(unix)]
#[test]
fn a_real_leftover_process_is_reclaimed_end_to_end() {
    use crate::gateway::sys::SystemProcessOps;
    use crate::gateway::testing::{wait_until, write_fake_sidecar, ScriptAwareOps};
    use std::process::{Command, Stdio};
    let dir = tempdir().unwrap();
    let sidecar = write_fake_sidecar(&dir.path().join("bin"), "serve");
    let port = free_port();
    let mut child = Command::new(&sidecar.exe)
        .env("SERVER_PORT", port.to_string())
        .env("PROXY_API_KEY", "k")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(wait_until(Duration::from_secs(5), || !SystemPorts.is_free(port)));
    let ops = ScriptAwareOps::new();
    ops.pretend(child.id(), &sidecar.exe);
    let file = pid_file(dir.path(), Some(PidRecord { pid: child.id(), exe: sidecar.exe.clone() }));

    let outcome = reclaim_stale(&file, &sidecar.exe, port, ops.as_ref(), &SystemPorts, Duration::from_secs(2));
    assert_eq!(outcome, Reclaim::Reclaimed { pid: child.id() });
    child.wait().unwrap();
    assert!(wait_until(Duration::from_secs(2), || SystemPorts.is_free(port)), "port must be released");
    assert!(!SystemProcessOps.is_alive(child.id()));
}

#[cfg(unix)]
#[test]
fn a_real_unrelated_process_with_a_reused_pid_survives() {
    use crate::gateway::sys::SystemProcessOps;
    use std::process::{Command, Stdio};
    let dir = tempdir().unwrap();
    let mut bystander = Command::new("/bin/sleep").arg("30").stdout(Stdio::null()).spawn().unwrap();
    let file = pid_file(dir.path(), Some(PidRecord { pid: bystander.id(), exe: PathBuf::from(SIDECAR) }));
    let outcome = reclaim_stale(&file, Path::new(SIDECAR), PORT, &SystemProcessOps, &SystemPorts, Duration::from_millis(200));
    assert_eq!(outcome, Reclaim::NotOurs { pid: bystander.id() });
    assert!(bystander.try_wait().unwrap().is_none(), "the bystander must still be running");
    bystander.kill().unwrap();
    bystander.wait().unwrap();
}

// ------------------------------------------- moved or renamed App (A-process 4)

const OLD_SIDECAR: &str = "/Users/me/Downloads/Kiro Gateway.app/Contents/Resources/gateway/kiro-gateway";

fn reclaim_with_listener(file: &Path, ops: &FakeOps, listener: Option<u32>) -> Reclaim {
    reclaim_stale(file, Path::new(SIDECAR), PORT, ops, &FakePorts { listener }, Duration::from_millis(100))
}

#[test]
fn a_leftover_from_a_moved_app_that_listens_on_our_port_is_reclaimed() {
    let dir = tempdir().unwrap();
    let file = pid_file(dir.path(), Some(record(4242, OLD_SIDECAR)));
    let ops = FakeOps::new().with_process(4242, Some(Path::new(OLD_SIDECAR)));
    assert_eq!(reclaim_with_listener(&file, &ops, Some(4242)), Reclaim::Reclaimed { pid: 4242 });
    assert_eq!(ops.calls(), vec!["terminate 4242"]);
    assert!(!file.exists());
}

#[test]
fn a_leftover_from_a_moved_app_that_does_not_listen_on_our_port_is_left_alone() {
    let dir = tempdir().unwrap();
    for listener in [None, Some(9999)] {
        let file = pid_file(dir.path(), Some(record(4242, OLD_SIDECAR)));
        let ops = FakeOps::new().with_process(4242, Some(Path::new(OLD_SIDECAR)));
        assert_eq!(reclaim_with_listener(&file, &ops, listener), Reclaim::NotOurs { pid: 4242 }, "{listener:?}");
        assert!(ops.calls().is_empty());
    }
}

#[test]
fn an_unrelated_program_is_never_reclaimed_even_when_it_listens_on_our_port() {
    let dir = tempdir().unwrap();
    for unrelated in [
        "/usr/bin/python3",
        "/Applications/Safari.app/Contents/MacOS/Safari",
        "/opt/tools/kiro-gateway",                                  // right name, no resources/gateway parent
        "/Applications/X.app/Contents/Resources/gateway/other-tool", // right directories, wrong name
        "/Applications/X.app/Contents/Resources/bin/kiro-gateway",   // wrong directory
        "/Applications/X.app/Contents/Frameworks/gateway/kiro-gateway", // not resources
    ] {
        let file = pid_file(dir.path(), Some(record(4242, unrelated)));
        let ops = FakeOps::new().with_process(4242, Some(Path::new(unrelated)));
        assert_eq!(reclaim_with_listener(&file, &ops, Some(4242)), Reclaim::NotOurs { pid: 4242 }, "{unrelated}");
        assert!(ops.calls().is_empty(), "{unrelated}");
    }
}

#[test]
fn a_bundle_shaped_record_whose_live_process_is_something_else_is_left_alone() {
    // The pid was reused: the record says sidecar, the process is Safari and happens to listen.
    let dir = tempdir().unwrap();
    let file = pid_file(dir.path(), Some(record(4242, OLD_SIDECAR)));
    let ops = FakeOps::new().with_process(4242, Some(Path::new("/Applications/Safari.app/Contents/MacOS/Safari")));
    assert_eq!(reclaim_with_listener(&file, &ops, Some(4242)), Reclaim::NotOurs { pid: 4242 });
    assert!(ops.calls().is_empty());
    let ops = FakeOps::new().with_process(4242, None);
    let file = pid_file(dir.path(), Some(record(4242, OLD_SIDECAR)));
    assert_eq!(reclaim_with_listener(&file, &ops, Some(4242)), Reclaim::NotOurs { pid: 4242 });
    assert!(ops.calls().is_empty());
}

#[test]
fn the_windows_spelling_of_a_bundled_sidecar_is_recognised() {
    assert!(is_bundled_sidecar_path(Path::new("C:/Program Files/Kiro Gateway/resources/gateway/kiro-gateway.exe")));
    assert!(is_bundled_sidecar_path(Path::new(OLD_SIDECAR)));
    assert!(!is_bundled_sidecar_path(Path::new("kiro-gateway")));
    assert!(!is_bundled_sidecar_path(Path::new("/")));
    assert!(!is_bundled_sidecar_path(Path::new("gateway/kiro-gateway")));
}
