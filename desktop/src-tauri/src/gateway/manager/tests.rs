//! Tasks 4.3 scenarios, run against fake sidecars (see `testing.rs`).

use super::*;
use crate::app_config::AppPaths;
use crate::gateway::health::HttpHealth;
use crate::gateway::logfile::RUN_MARKER;
use crate::gateway::port::{read_pid_file, write_pid_file, PidFile, PidRecord, PortOps, SystemPorts};
use crate::gateway::sys::{ProcessOps, SystemProcessOps};
use crate::gateway::testing::*;
use crate::gateway::types::{FailureKind, GatewayState};
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

const LONG: Duration = Duration::from_secs(15);

struct Fixture {
    _dir: TempDir,
    data: std::path::PathBuf,
    bin: std::path::PathBuf,
    port: u16,
}

fn fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let data = dir.path().join("data");
    let bin = dir.path().join("bin");
    Fixture { _dir: dir, data, bin, port: free_port() }
}

impl Fixture {
    fn request(&self, mode: &str) -> StartRequest {
        let sidecar = write_fake_sidecar(&self.bin, mode);
        start_request(sidecar, self.port, &[], &self.data)
    }
    fn request_with(&self, mode: &str, extra: &[(&str, &str)]) -> StartRequest {
        let sidecar = write_fake_sidecar(&self.bin, mode);
        start_request(sidecar, self.port, extra, &self.data)
    }
    fn paths(&self) -> AppPaths {
        AppPaths::new(&self.data)
    }
}

fn wait_state(m: &GatewayManager, state: GatewayState) -> GatewayStatus {
    assert!(wait_until(LONG, || m.status().state == state), "wanted {state:?}, stuck at {:?}", m.status());
    m.status()
}

fn alive(pid: u32) -> bool {
    SystemProcessOps.is_alive(pid)
}

// ------------------------------------------------------------- start success

#[test]
fn start_goes_stopped_starting_running_and_health_answers() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    assert_eq!(m.status().state, GatewayState::Stopped);
    assert_eq!(m.status().port, f.port);

    let first = m.start(f.request("serve")).unwrap();
    assert_eq!(first.state, GatewayState::Starting);
    assert!(first.pid.is_some());
    assert_eq!(first.failure, None);

    let running = wait_state(&m, GatewayState::Running);
    assert_eq!(running.pid, first.pid);
    assert_eq!(running.port, f.port);
    assert!(running.since.is_some());
    assert_eq!(rec.states(), vec![GatewayState::Starting, GatewayState::Running]);
    assert!(HttpHealth::new(Duration::from_secs(1)).is_healthy(f.port));
    m.stop();
}

#[test]
fn a_slow_start_stays_starting_until_health_passes() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("slow:1.0")).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(m.status().state, GatewayState::Starting, "not healthy yet");
    wait_state(&m, GatewayState::Running);
    assert_eq!(rec.states(), vec![GatewayState::Starting, GatewayState::Running]);
    m.stop();
}

#[test]
fn the_sidecar_gets_the_cleaned_environment_and_the_runtime_dir_as_cwd() {
    let f = fixture();
    let dump = f.data.join("dump.json");
    fs::create_dir_all(&f.data).unwrap();
    // A stray .env in runtime/ must be gone before the gateway's load_dotenv() could read it.
    fs::create_dir_all(f.paths().runtime_dir()).unwrap();
    fs::write(f.paths().runtime_dir().join(".env"), "PROXY_API_KEY=evil\n").unwrap();

    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request_with("serve", &[("KGW_FAKE_DUMP", dump.to_str().unwrap())])).unwrap();
    wait_state(&m, GatewayState::Running);

    let seen = read_dump(&dump);
    let env = seen["env"].as_object().unwrap();
    assert_eq!(env["PROXY_API_KEY"], SECRET);
    assert_eq!(env["SERVER_HOST"], "127.0.0.1");
    assert_eq!(env["SERVER_PORT"], f.port.to_string());
    assert!(!env.contains_key("ANTHROPIC_BASE_URL"), "inherited ANTHROPIC_* must not reach the sidecar");
    assert!(!env.contains_key("PROXY_URL"));
    let cwd = std::path::PathBuf::from(seen["cwd"].as_str().unwrap()).canonicalize().unwrap();
    assert_eq!(cwd, f.paths().runtime_dir().canonicalize().unwrap());
    assert!(!f.paths().runtime_dir().join(".env").exists());
    m.stop();
}

#[test]
fn the_pid_file_records_pid_and_executable_while_running_and_is_removed_after_stop() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    let req = f.request("serve");
    let exe = req.sidecar.exe.clone();
    let pid = m.start(req).unwrap().pid.unwrap();
    assert_eq!(read_pid_file(&f.paths().pid_file()), PidFile::Valid(PidRecord { pid, exe }));
    wait_state(&m, GatewayState::Running);
    m.stop();
    assert_eq!(read_pid_file(&f.paths().pid_file()), PidFile::Missing);
}

#[test]
fn output_goes_to_gateway_log_with_a_run_marker() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("serve")).unwrap();
    wait_state(&m, GatewayState::Running);
    m.stop();
    let log = fs::read_to_string(f.paths().gateway_log()).unwrap();
    assert!(log.contains(RUN_MARKER));
    assert!(log.contains("startup banner"));
}

// ------------------------------------------------------------- duplicate start

#[test]
fn starting_twice_does_not_launch_a_second_process() {
    let f = fixture();
    let launches = f.data.join("launches.txt");
    fs::create_dir_all(&f.data).unwrap();
    let extra = [("KGW_FAKE_LAUNCHES", launches.to_str().unwrap())];
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    let a = m.start(f.request_with("serve", &extra)).unwrap();
    let b = m.start(f.request_with("serve", &extra)).unwrap();
    assert_eq!(a.pid, b.pid);
    wait_state(&m, GatewayState::Running);
    let c = m.start(f.request_with("serve", &extra)).unwrap();
    assert_eq!(c.state, GatewayState::Running, "start on a running gateway returns the running status");
    assert_eq!(c.pid, a.pid);
    assert_eq!(launch_count(&launches), 1);
    m.stop();
}

#[test]
fn concurrent_starts_launch_exactly_one_process() {
    let f = fixture();
    let launches = f.data.join("launches.txt");
    fs::create_dir_all(&f.data).unwrap();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    let m = Arc::new(m);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let m = m.clone();
            let req = f.request_with("serve", &[("KGW_FAKE_LAUNCHES", launches.to_str().unwrap())]);
            std::thread::spawn(move || m.start(req).unwrap())
        })
        .collect();
    let pids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap().pid).collect();
    assert!(pids.iter().all(|p| *p == pids[0] && p.is_some()), "{pids:?}");
    wait_state(&m, GatewayState::Running);
    assert_eq!(launch_count(&launches), 1);
    assert_eq!(rec.states(), vec![GatewayState::Starting, GatewayState::Running]);
    m.stop();
}

// -------------------------------------------------------------- start failures

#[test]
fn a_start_that_never_becomes_healthy_fails_with_timeout_and_the_process_is_ended() {
    let f = fixture();
    // The health probe is gated on the fake's first line, so the timeout cannot fire before
    // the fake has printed, however loaded the machine is.
    let (m, rec) = manager_gated_on_log(&f.data, f.port, "loading, never listens", |s| s.start_timeout = Duration::from_millis(1500));
    let started = Instant::now();
    let pid = m.start(f.request("never")).unwrap().pid.unwrap();
    let st = wait_state(&m, GatewayState::StartFailed);
    assert!(started.elapsed() >= Duration::from_millis(1500));
    assert!(started.elapsed() < Duration::from_secs(40), "{:?}", started.elapsed());
    let failure = st.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Timeout);
    assert!(failure.message.contains("没有就绪"), "{}", failure.message);
    assert_eq!(failure.occupant, None);
    assert!(failure.log_excerpt.iter().any(|l| l.contains("loading, never listens")));
    assert!(wait_until(Duration::from_secs(3), || !alive(pid)), "the hung process must be ended");
    assert_eq!(st.pid, None);
    assert_eq!(read_pid_file(&f.paths().pid_file()), PidFile::Missing);
    assert_eq!(rec.states(), vec![GatewayState::Starting, GatewayState::StartFailed]);
}

#[test]
fn a_process_that_exits_at_once_is_a_start_failure_with_its_exit_code_and_output() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("exit:3")).unwrap();
    let st = wait_state(&m, GatewayState::StartFailed);
    let failure = st.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Other);
    assert!(failure.message.contains('3'), "{}", failure.message);
    assert!(failure.log_excerpt.iter().any(|l| l.contains("about to fail")), "{:?}", failure.log_excerpt);
    assert_eq!(st.pid, None);
}

#[test]
fn a_log_with_a_refresh_403_means_login_expired() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("login_fail")).unwrap();
    let st = wait_state(&m, GatewayState::StartFailed);
    let failure = st.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::LoginExpired);
    assert_eq!(failure.message, "kiro-cli 的登录已失效");
    assert!(failure.log_excerpt.iter().any(|l| l.contains("refresh failed: status=403")));
}

#[test]
fn login_expired_wins_over_timeout_when_the_process_hangs_after_the_error() {
    let f = fixture();
    let (m, _rec) = manager_gated_on_log(&f.data, f.port, "Failed to initialize any account", |s| s.start_timeout = Duration::from_millis(1500));
    m.start(f.request("login_fail_hang")).unwrap();
    let st = wait_state(&m, GatewayState::StartFailed);
    let failure = st.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::LoginExpired, "excerpt: {:?}\nlog file: {:?}", failure.log_excerpt, fs::read_to_string(f.paths().gateway_log()));
}

#[test]
fn a_bind_error_in_the_log_means_port_in_use_with_the_occupant_looked_up() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    // Another program takes the port only after our pre-check passed: simulate by
    // grabbing it from the fake's first line of output via a listener opened right now,
    // while the pre-check already ran. The fake itself never binds.
    m.start(f.request("bind_fail")).unwrap();
    let holder = TcpListener::bind(("127.0.0.1", f.port));
    let st = wait_state(&m, GatewayState::StartFailed);
    let failure = st.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::PortInUse);
    assert_eq!(failure.message, format!("{} 端口被别的程序占用", f.port));
    assert!(failure.occupant.is_some());
    drop(holder);
}

#[test]
fn the_failure_excerpt_is_redacted_and_the_secret_never_appears() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("echo_secret")).unwrap();
    let st = wait_state(&m, GatewayState::StartFailed);
    let text = st.failure.unwrap().log_excerpt.join("\n");
    assert!(!text.contains(SECRET), "{text}");
    assert!(!text.contains("9b5ad71b2ce5302211f9c61530b329a4922fc6a4"), "{text}");
    assert!(text.contains("using key ***"), "{text}");
    let json = serde_json::to_string(&m.status()).unwrap();
    assert!(!json.contains(SECRET));
}

#[test]
fn the_excerpt_keeps_only_the_last_twenty_lines() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("many_lines:60")).unwrap();
    let excerpt = wait_state(&m, GatewayState::StartFailed).failure.unwrap().log_excerpt;
    assert_eq!(excerpt.len(), 20);
    assert_eq!(excerpt.first().unwrap(), "log line 40");
    assert_eq!(excerpt.last().unwrap(), "log line 59");
}

#[test]
fn the_excerpt_of_a_second_run_does_not_contain_the_first_runs_output() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("exit:1")).unwrap();
    wait_state(&m, GatewayState::StartFailed);
    m.start(f.request("many_lines:3")).unwrap();
    assert!(wait_until(LONG, || {
        let s = m.status();
        s.state == GatewayState::StartFailed && s.failure.as_ref().is_some_and(|x| x.log_excerpt.iter().any(|l| l == "log line 2"))
    }));
    let excerpt = m.status().failure.unwrap().log_excerpt;
    assert!(!excerpt.iter().any(|l| l.contains("about to fail")), "{excerpt:?}");
}

#[test]
fn an_executable_that_cannot_be_started_is_a_start_failure_not_a_command_error() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    let mut req = f.request("serve");
    fs::remove_file(&req.sidecar.exe).unwrap();
    req.sidecar.exe = f.bin.join("missing-gateway");
    let st = m.start(req).unwrap();
    assert_eq!(st.state, GatewayState::StartFailed);
    let failure = st.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Other);
    assert!(!failure.message.contains('/'), "{}", failure.message);
    assert_eq!(rec.states(), vec![GatewayState::StartFailed]);
    assert_eq!(read_pid_file(&f.paths().pid_file()), PidFile::Missing);
}

#[test]
fn a_problem_found_before_building_the_request_can_be_reported_as_a_start_failure() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    m.report_start_problem(f.port, "没找到内置的网关程序，请重新安装 Kiro Gateway".to_string());
    let st = m.status();
    assert_eq!(st.state, GatewayState::StartFailed);
    assert_eq!(st.failure.as_ref().unwrap().kind, FailureKind::Other);
    assert_eq!(st.failure.unwrap().message, "没找到内置的网关程序，请重新安装 Kiro Gateway");
    assert_eq!(rec.states(), vec![GatewayState::StartFailed]);
}

#[test]
fn a_start_failure_can_be_retried_and_clears_the_failure() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("exit:1")).unwrap();
    wait_state(&m, GatewayState::StartFailed);
    m.start(f.request("serve")).unwrap();
    let running = wait_state(&m, GatewayState::Running);
    assert_eq!(running.failure, None);
    m.stop();
}

// ------------------------------------------------------------ forced upgrade

#[test]
fn upgrade_required_rejects_start_without_launching_anything() {
    let f = fixture();
    let launches = f.data.join("launches.txt");
    fs::create_dir_all(&f.data).unwrap();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    let mut req = f.request_with("serve", &[("KGW_FAKE_LAUNCHES", launches.to_str().unwrap())]);
    req.upgrade_required = true;
    let err = m.start(req).unwrap_err();
    assert_eq!(err, StartError::UpgradeRequired);
    assert_eq!(err.code(), "upgrade_required");
    assert_eq!(err.message(), "需要先升级到新版本才能使用");
    assert_eq!(m.status().state, GatewayState::Stopped);
    assert_eq!(launch_count(&launches), 0);
    assert!(rec.all().is_empty());
}

// ------------------------------------------------------------------ the port

#[test]
fn a_port_held_by_someone_else_is_reported_with_name_and_pid_and_left_alone() {
    let f = fixture();
    let launches = f.data.join("launches.txt");
    fs::create_dir_all(&f.data).unwrap();
    let holder = TcpListener::bind(("127.0.0.1", f.port)).unwrap();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    let st = m.start(f.request_with("serve", &[("KGW_FAKE_LAUNCHES", launches.to_str().unwrap())])).unwrap();
    assert_eq!(st.state, GatewayState::StartFailed);
    let failure = st.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::PortInUse);
    assert_eq!(failure.message, format!("{} 端口被别的程序占用", f.port));
    let occupant = failure.occupant.unwrap();
    assert_eq!(occupant.pid, Some(std::process::id()));
    assert!(occupant.name.is_some());
    assert_eq!(failure.log_excerpt, Vec::<String>::new());
    assert_eq!(launch_count(&launches), 0, "nothing may be launched when the port is taken");
    assert_eq!(rec.states(), vec![GatewayState::StartFailed]);
    // The holder is untouched and still listening.
    assert!(!SystemPorts.is_free(f.port));
    drop(holder);
    // Freed port: a retry works.
    m.start(f.request("serve")).unwrap();
    wait_state(&m, GatewayState::Running);
    m.stop();
}

#[test]
fn a_leftover_sidecar_of_ours_is_reclaimed_so_the_start_succeeds() {
    let f = fixture();
    let req = f.request("serve");
    let exe = req.sidecar.exe.clone();
    let mut leftover = std::process::Command::new(&exe)
        .env("SERVER_PORT", f.port.to_string())
        .env("PROXY_API_KEY", SECRET)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    assert!(wait_until(LONG, || !SystemPorts.is_free(f.port)));
    fs::create_dir_all(&f.data).unwrap();
    write_pid_file(&f.paths().pid_file(), &PidRecord { pid: leftover.id(), exe: exe.clone() }).unwrap();

    let ops = ScriptAwareOps::new();
    ops.pretend(leftover.id(), &exe);
    let mut deps = system_deps();
    deps.ops = ops;
    let m = GatewayManager::new(f.paths(), deps, fast_settings(), f.port);
    m.start(req).unwrap();
    wait_state(&m, GatewayState::Running);
    leftover.wait().unwrap();
    m.stop();
}

#[test]
fn a_bystander_on_the_port_with_a_recorded_pid_of_another_program_is_never_killed() {
    let f = fixture();
    let bystander_script = f.bin.join("bystander");
    let sidecar = write_fake_sidecar(&f.bin, "serve");
    fs::copy(&sidecar.exe, &bystander_script).unwrap();
    let mut bystander = std::process::Command::new(&bystander_script)
        .env("SERVER_PORT", f.port.to_string())
        .env("PROXY_API_KEY", "x")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    assert!(wait_until(LONG, || !SystemPorts.is_free(f.port)));
    fs::create_dir_all(&f.data).unwrap();
    // The pid file claims the bystander is our sidecar, but its image is another program.
    write_pid_file(&f.paths().pid_file(), &PidRecord { pid: bystander.id(), exe: sidecar.exe.clone() }).unwrap();

    let ops = ScriptAwareOps::new();
    ops.pretend(bystander.id(), Path::new("/Applications/Safari.app/Contents/MacOS/Safari"));
    let mut deps = system_deps();
    deps.ops = ops;
    let m = GatewayManager::new(f.paths(), deps, fast_settings(), f.port);
    let st = m.start(start_request(sidecar, f.port, &[], &f.data)).unwrap();
    assert_eq!(st.state, GatewayState::StartFailed);
    assert_eq!(st.failure.unwrap().kind, FailureKind::PortInUse);
    assert!(bystander.try_wait().unwrap().is_none(), "the bystander must still be running");
    bystander.kill().unwrap();
    bystander.wait().unwrap();
}

// ------------------------------------------------------------------- crashes

#[test]
fn an_external_kill_is_noticed_within_five_seconds_and_becomes_crashed() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    let pid = m.start(f.request("serve")).unwrap().pid.unwrap();
    wait_state(&m, GatewayState::Running);

    let killed_at = Instant::now();
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    let st = wait_state(&m, GatewayState::Crashed);
    assert!(killed_at.elapsed() < Duration::from_secs(5), "{:?}", killed_at.elapsed());
    assert!(killed_at.elapsed() < Duration::from_secs(1), "the wait thread should notice at once, took {:?}", killed_at.elapsed());
    let failure = st.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Other);
    assert!(failure.message.contains("意外退出") || failure.message.contains("结束"), "{}", failure.message);
    assert!(failure.log_excerpt.iter().any(|l| l.contains("startup banner")));
    assert_eq!(st.pid, None);
    assert_eq!(rec.states(), vec![GatewayState::Starting, GatewayState::Running, GatewayState::Crashed]);
    assert_eq!(read_pid_file(&f.paths().pid_file()), PidFile::Missing);
    assert!(SystemPorts.is_free(f.port));
}

#[test]
fn a_process_that_dies_by_itself_after_running_is_crashed_with_its_last_output() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("crash_after:1.0")).unwrap();
    let st = wait_state(&m, GatewayState::Crashed);
    let failure = st.failure.unwrap();
    assert!(failure.log_excerpt.iter().any(|l| l.contains("fatal: boom")), "{:?}", failure.log_excerpt);
    assert!(failure.message.contains('1'), "exit code in the message: {}", failure.message);
}

#[test]
fn a_crash_can_be_followed_by_a_normal_restart() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    let pid = m.start(f.request("serve")).unwrap().pid.unwrap();
    wait_state(&m, GatewayState::Running);
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    wait_state(&m, GatewayState::Crashed);
    m.start(f.request("serve")).unwrap();
    let st = wait_state(&m, GatewayState::Running);
    assert_ne!(st.pid, Some(pid));
    assert_eq!(st.failure, None);
    m.stop();
    assert_eq!(
        rec.states(),
        vec![GatewayState::Starting, GatewayState::Running, GatewayState::Crashed, GatewayState::Starting, GatewayState::Running, GatewayState::Stopped]
    );
}

#[test]
fn three_failed_heartbeats_in_a_row_declare_a_hung_gateway_crashed() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    let pid = m.start(f.request("serve")).unwrap().pid.unwrap();
    wait_state(&m, GatewayState::Running);
    // SIGSTOP: the process stays alive and keeps the socket, but never answers.
    unsafe {
        libc::kill(pid as i32, libc::SIGSTOP);
    }
    let stopped_at = Instant::now();
    let st = wait_state(&m, GatewayState::Crashed);
    assert!(stopped_at.elapsed() >= Duration::from_millis(700), "needs several failed probes, took {:?}", stopped_at.elapsed());
    let failure = st.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Other);
    assert!(failure.message.contains("没有响应"), "{}", failure.message);
    assert!(wait_until(Duration::from_secs(5), || !alive(pid)), "the hung process must be ended (SIGKILL works on stopped processes)");
}

#[test]
fn one_or_two_failed_heartbeats_do_not_declare_the_gateway_dead() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder_settings(&f.data, f.port, |s| {
        s.heartbeat_interval = Duration::from_millis(150);
        s.heartbeat_failures = 3;
    });
    let pid = m.start(f.request("serve")).unwrap().pid.unwrap();
    wait_state(&m, GatewayState::Running);
    unsafe {
        libc::kill(pid as i32, libc::SIGSTOP);
    }
    // One probe takes ~800 ms to fail; resume well before the third failure completes.
    std::thread::sleep(Duration::from_millis(1300));
    unsafe {
        libc::kill(pid as i32, libc::SIGCONT);
    }
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(m.status().state, GatewayState::Running, "failures must be consecutive to count");
    m.stop();
}

// ---------------------------------------------------------------------- stop

#[test]
fn stop_ends_the_process_frees_the_port_and_reports_stopped() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    let pid = m.start(f.request("serve")).unwrap().pid.unwrap();
    wait_state(&m, GatewayState::Running);
    let st = m.stop();
    assert_eq!(st.state, GatewayState::Stopped);
    assert_eq!(st.pid, None);
    assert_eq!(st.failure, None);
    assert!(!alive(pid));
    assert!(SystemPorts.is_free(f.port));
    assert_eq!(read_pid_file(&f.paths().pid_file()), PidFile::Missing);
    assert_eq!(rec.states(), vec![GatewayState::Starting, GatewayState::Running, GatewayState::Stopped]);
}

#[test]
fn a_deliberate_stop_is_not_reported_as_a_crash() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("serve")).unwrap();
    wait_state(&m, GatewayState::Running);
    m.stop();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!rec.states().contains(&GatewayState::Crashed));
    assert_eq!(m.status().state, GatewayState::Stopped);
}

#[test]
fn stop_sends_sigkill_after_the_grace_period_when_sigterm_is_ignored() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    let pid = m.start(f.request("ignore_term")).unwrap().pid.unwrap();
    wait_state(&m, GatewayState::Running);
    let began = Instant::now();
    let st = m.stop();
    let took = began.elapsed();
    assert_eq!(st.state, GatewayState::Stopped);
    assert!(took >= Duration::from_millis(550), "SIGKILL must wait for the grace period, took {took:?}");
    assert!(took < Duration::from_secs(5), "took {took:?}");
    assert!(!alive(pid));
}

#[test]
fn a_cooperative_process_is_stopped_well_before_the_grace_period() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder_settings(&f.data, f.port, |s| s.term_grace = Duration::from_secs(5));
    m.start(f.request("serve")).unwrap();
    wait_state(&m, GatewayState::Running);
    let began = Instant::now();
    m.stop();
    assert!(began.elapsed() < Duration::from_secs(2), "took {:?}", began.elapsed());
}

#[test]
fn stop_while_starting_cancels_the_start() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    let pid = m.start(f.request("never")).unwrap().pid.unwrap();
    let st = m.stop();
    assert_eq!(st.state, GatewayState::Stopped);
    assert!(!alive(pid));
    assert_eq!(rec.states(), vec![GatewayState::Starting, GatewayState::Stopped]);
}

#[test]
fn stop_on_a_stopped_gateway_is_a_quiet_no_op() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    assert_eq!(m.stop().state, GatewayState::Stopped);
    assert_eq!(m.stop().state, GatewayState::Stopped);
    assert!(rec.all().is_empty());
}

#[test]
fn stop_clears_a_failure_back_to_stopped() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("exit:1")).unwrap();
    wait_state(&m, GatewayState::StartFailed);
    let st = m.stop();
    assert_eq!(st.state, GatewayState::Stopped);
    assert_eq!(st.failure, None);
    assert_eq!(rec.states(), vec![GatewayState::Starting, GatewayState::StartFailed, GatewayState::Stopped]);

    let (m2, _rec2) = manager_with_recorder(&f.data, f.port);
    let pid = m2.start(f.request("serve")).unwrap().pid.unwrap();
    wait_state(&m2, GatewayState::Running);
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    wait_state(&m2, GatewayState::Crashed);
    assert_eq!(m2.stop().state, GatewayState::Stopped);
    assert_eq!(m2.status().failure, None);
}

#[test]
fn stop_can_be_called_from_several_threads_at_once() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    let pid = m.start(f.request("serve")).unwrap().pid.unwrap();
    wait_state(&m, GatewayState::Running);
    let m = Arc::new(m);
    let handles: Vec<_> = (0..4).map(|_| { let m = m.clone(); std::thread::spawn(move || m.stop().state) }).collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), GatewayState::Stopped);
    }
    assert!(!alive(pid));
}

// -------------------------------------------------------------------- events

#[test]
fn consecutive_identical_statuses_are_never_emitted() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("serve")).unwrap();
    wait_state(&m, GatewayState::Running);
    m.set_port(f.port);
    m.stop();
    m.stop();
    let all = rec.all();
    for pair in all.windows(2) {
        assert_ne!(pair[0], pair[1]);
    }
}

#[test]
fn changing_the_configured_port_while_stopped_updates_status_and_emits_once() {
    let f = fixture();
    let (m, rec) = manager_with_recorder(&f.data, f.port);
    let other = free_port();
    m.set_port(other);
    assert_eq!(m.status().port, other);
    m.set_port(other);
    assert_eq!(rec.all().len(), 1);
    assert_eq!(rec.all()[0].port, other);
    assert_eq!(rec.all()[0].state, GatewayState::Stopped);
}

#[test]
fn changing_the_configured_port_while_running_does_not_touch_the_live_status() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("serve")).unwrap();
    wait_state(&m, GatewayState::Running);
    m.set_port(f.port + 1);
    assert_eq!(m.status().port, f.port, "the running gateway keeps its port until restarted");
    m.stop();
    assert_eq!(m.status().port, f.port + 1, "after the stop the configured port is shown");
}

#[test]
fn since_and_failure_time_come_from_the_injected_clock() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("exit:1")).unwrap();
    let st = wait_state(&m, GatewayState::StartFailed);
    assert_eq!(st.since.as_deref(), Some("2026-10-22T14:41:08+08:00"));
    assert_eq!(st.failure.unwrap().at, "2026-10-22T14:41:08+08:00");
}

#[test]
fn status_json_follows_the_contract_shape() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    let value = serde_json::to_value(m.status()).unwrap();
    assert_eq!(value["state"], "stopped");
    assert_eq!(value["port"], f.port);
    assert!(value["pid"].is_null());
    assert!(value["since"].is_null());
    assert!(value["failure"].is_null());
}

// -------------------------------------------------------------------- models

#[test]
fn models_come_from_the_running_gateway_with_the_key() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    m.start(f.request("serve")).unwrap();
    wait_state(&m, GatewayState::Running);
    assert_eq!(m.models(SECRET, Duration::from_secs(3)).unwrap(), vec!["auto", "claude-haiku-4.5"]);
    assert_eq!(m.models("wrong-key-wrong-key", Duration::from_secs(3)).unwrap_err().code(), "gateway_unreachable");
    m.stop();
}

#[test]
fn models_while_not_running_is_gateway_not_running() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    let err = m.models(SECRET, Duration::from_secs(1)).unwrap_err();
    assert_eq!(err.code(), "gateway_not_running");
    m.start(f.request("never")).unwrap();
    assert_eq!(m.models(SECRET, Duration::from_secs(1)).unwrap_err().code(), "gateway_not_running", "starting is not running");
    m.stop();
}

// ------------------------------------------------------------------ cleanup

#[test]
fn the_group_of_a_stopped_gateway_leaves_no_processes_behind() {
    let f = fixture();
    let (m, _rec) = manager_with_recorder(&f.data, f.port);
    let pid = m.start(f.request("serve")).unwrap().pid.unwrap();
    wait_state(&m, GatewayState::Running);
    m.stop();
    // `ps` by process group: nothing with that pgid remains.
    let out = std::process::Command::new("/bin/ps").args(["-o", "pid=", "-g", &pid.to_string()]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty(), "{}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn error_messages_of_start_errors_are_plain_chinese() {
    assert_eq!(StartError::Internal("没能启动网关进程".into()).code(), "internal");
    assert_eq!(StartError::Internal("没能启动网关进程".into()).message(), "没能启动网关进程");
    assert!(StartError::UpgradeRequired.message().contains("升级"));
}

#[test]
fn production_settings_match_the_design() {
    let s = ManagerSettings::default();
    assert_eq!(s.start_timeout, Duration::from_secs(30), "tasks 4.3: 30 s start timeout");
    assert_eq!(s.poll_interval, Duration::from_millis(500), "tasks 4.3: poll /health every 500 ms");
    assert_eq!(s.heartbeat_interval, Duration::from_secs(10), "design 8 / tasks 4.3: 10 s heartbeat");
    assert_eq!(s.heartbeat_failures, 3, "design risk 2: 3 consecutive failures");
    assert_eq!(s.term_grace, Duration::from_secs(5), "design 8: SIGTERM, 5 s, SIGKILL");
    assert_eq!(s.excerpt_lines, 20, "contract: last 20 lines");
}

// ------------------------------------------------- inherited signal mask (A-process 1)

/// The App blocks SIGTERM in its main thread (`signals::install`); a mask is inherited
/// across fork and exec, so the sidecar must get it reset or it can never see SIGTERM.
#[test]
fn a_sidecar_started_from_a_thread_that_blocks_sigterm_still_shuts_down_gracefully() {
    let f = fixture();
    let mark = f.data.join("term.mark");
    fs::create_dir_all(&f.data).unwrap();
    let request = f.request_with("graceful_term", &[("KGW_FAKE_TERM_MARK", mark.to_str().unwrap())]);
    let (m, _rec) = manager_with_recorder_settings(&f.data, f.port, |s| s.term_grace = Duration::from_secs(5));

    let took = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                // Same as signals::install does for the main thread.
                unsafe {
                    let mut set: libc::sigset_t = std::mem::zeroed();
                    libc::sigemptyset(&mut set);
                    libc::sigaddset(&mut set, libc::SIGTERM);
                    libc::sigaddset(&mut set, libc::SIGINT);
                    libc::sigaddset(&mut set, libc::SIGHUP);
                    assert_eq!(libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()), 0);
                }
                m.start(request).unwrap();
                wait_state(&m, GatewayState::Running);
                let began = Instant::now();
                m.stop();
                began.elapsed()
            })
            .join()
            .unwrap()
    });

    assert!(took < Duration::from_secs(2), "stop waited for SIGKILL, took {took:?}");
    assert_eq!(fs::read_to_string(&mark).unwrap_or_default(), "handled", "the sidecar never saw SIGTERM");
}
