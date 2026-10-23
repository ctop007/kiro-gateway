//! Port checks, the `gateway.pid` file and reclaiming a leftover gateway (tasks 4.4).
//!
//! Rules (design decision 8, `desktop/AGENTS.md`):
//! - A port is free only if nothing answers on it and a bind on `127.0.0.1` works.
//! - `gateway.pid` records the pid and the executable path of the sidecar the App started.
//! - On the next start, a process is ended only if it is alive, its *actual* executable is
//!   the recorded one, and the recorded one is this App's sidecar. Anything else holding
//!   the port is only reported (name and pid), never signalled.
//! - One relaxation: when the App was moved or renamed, the leftover runs from the old
//!   location. It still counts as ours if both its recorded and its actual path look like a
//!   bundled sidecar (`.../resources/gateway/kiro-gateway`) and it is the process that
//!   listens on the port this App is configured for.

use super::sys::{is_signalable, ProcessOps};
use super::types::PortOccupant;
use crate::app_config::atomic::{write_atomic, FileMode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long a connect attempt may take when probing whether something listens.
const CONNECT_PROBE: Duration = Duration::from_millis(300);
/// Upper bound for the external lookup of the port holder.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);
/// Poll interval while waiting for a process to disappear.
const EXIT_POLL: Duration = Duration::from_millis(20);

/// Port questions, behind a trait so the manager can be tested without real sockets.
pub trait PortOps: Send + Sync {
    /// Whether the sidecar could listen on `127.0.0.1:port` right now.
    fn is_free(&self, port: u16) -> bool;
    /// Who listens on `port`. Both fields are `None` when it cannot be found out.
    fn occupant(&self, port: u16) -> PortOccupant;
}

/// Real sockets and the platform's lookup tool.
pub struct SystemPorts;

impl PortOps for SystemPorts {
    fn is_free(&self, port: u16) -> bool {
        // A connection succeeding means something accepts on loopback. This also catches a
        // listener on 0.0.0.0, which a plain bind on 127.0.0.1 can miss on macOS.
        let target = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        if TcpStream::connect_timeout(&target, CONNECT_PROBE).is_ok() {
            return false;
        }
        // The bind is what the sidecar will do. The listener is dropped at once; no
        // connection was accepted, so it leaves nothing behind.
        TcpListener::bind(target).is_ok()
    }

    fn occupant(&self, port: u16) -> PortOccupant {
        lookup_occupant(port)
    }
}

const NO_OCCUPANT: PortOccupant = PortOccupant { name: None, pid: None };

/// Fixed arguments of the lookup: only the port number is variable, and it is a `u16`.
pub fn lsof_args(port: u16) -> Vec<String> {
    vec!["-nP".to_string(), format!("-iTCP:{port}"), "-sTCP:LISTEN".to_string(), "-Fpc".to_string()]
}

/// Parse `lsof -Fpc` output: `p<pid>` starts a process, `c<command>` names it. The first
/// valid process wins; one process listening on IPv4 and IPv6 appears with several `f` lines
/// but a single `p` line.
pub fn parse_lsof(output: &str) -> PortOccupant {
    let mut occupant = PortOccupant { name: None, pid: None };
    let mut skipping = false;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            if occupant.pid.is_some() {
                break;
            }
            match rest.parse::<u32>() {
                Ok(pid) if pid > 0 => {
                    occupant.pid = Some(pid);
                    skipping = false;
                }
                _ => skipping = true,
            }
        } else if let Some(rest) = line.strip_prefix('c') {
            if !skipping && occupant.pid.is_some() && occupant.name.is_none() && !rest.is_empty() {
                occupant.name = Some(rest.to_string());
            }
        }
    }
    occupant
}

/// Pid of the process LISTENING on `port` in `netstat -ano -p TCP` output (Windows).
#[cfg(any(windows, test))]
pub fn parse_netstat(output: &str, port: u16) -> Option<u32> {
    let suffix = format!(":{port}");
    output.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 || !fields[0].eq_ignore_ascii_case("TCP") || !fields[3].eq_ignore_ascii_case("LISTENING") {
            return None;
        }
        if !fields[1].ends_with(&suffix) {
            return None;
        }
        fields[4].parse::<u32>().ok().filter(|pid| *pid > 0)
    })
}

/// Image name in `tasklist /FO CSV /NH` output (Windows): the first quoted field.
#[cfg(any(windows, test))]
pub fn parse_tasklist(output: &str) -> Option<String> {
    let line = output.lines().next()?.trim();
    let rest = line.strip_prefix('"')?;
    let name = rest.split('"').next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Run `command` with `args`, give it at most `limit`, return its stdout when it succeeds.
fn run_with_timeout(command: &str, args: &[String], limit: Duration) -> Option<String> {
    let mut cmd = Command::new(command);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(unix)]
    super::sys::restore_default_signals(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(cmd.output());
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(output)) => Some(String::from_utf8_lossy(&output.stdout).to_string()),
        _ => None,
    }
}

#[cfg(unix)]
fn lookup_occupant(port: u16) -> PortOccupant {
    // GUI apps start with a short PATH; use the system location first.
    let tool = if Path::new("/usr/sbin/lsof").exists() { "/usr/sbin/lsof" } else { "lsof" };
    match run_with_timeout(tool, &lsof_args(port), LOOKUP_TIMEOUT) {
        Some(output) => parse_lsof(&output),
        None => NO_OCCUPANT,
    }
}

#[cfg(windows)]
fn lookup_occupant(port: u16) -> PortOccupant {
    let netstat = run_with_timeout("netstat", &["-ano".to_string(), "-p".to_string(), "TCP".to_string()], LOOKUP_TIMEOUT);
    let Some(pid) = netstat.and_then(|out| parse_netstat(&out, port)) else { return NO_OCCUPANT };
    let args = vec!["/FI".to_string(), format!("PID eq {pid}"), "/FO".to_string(), "CSV".to_string(), "/NH".to_string()];
    let name = run_with_timeout("tasklist", &args, LOOKUP_TIMEOUT).and_then(|out| parse_tasklist(&out));
    PortOccupant { name, pid: Some(pid) }
}

// ------------------------------------------------------------------ pid file

/// Content of `gateway.pid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PidRecord {
    pub pid: u32,
    /// Executable path of the sidecar that was started.
    pub exe: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct PidRecordWire {
    pid: u32,
    exe: String,
}

/// Result of reading `gateway.pid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidFile {
    Missing,
    /// Present but not a usable record (bad JSON, missing field, pid 0, empty path).
    Corrupt,
    Valid(PidRecord),
}

/// Read `gateway.pid`.
pub fn read_pid_file(path: &Path) -> PidFile {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return PidFile::Missing,
        Err(_) => return PidFile::Corrupt,
    };
    match serde_json::from_slice::<PidRecordWire>(&bytes) {
        Ok(wire) if wire.pid > 0 && !wire.exe.is_empty() => PidFile::Valid(PidRecord { pid: wire.pid, exe: PathBuf::from(wire.exe) }),
        _ => PidFile::Corrupt,
    }
}

/// Write `gateway.pid` atomically.
pub fn write_pid_file(path: &Path, record: &PidRecord) -> io::Result<()> {
    let wire = PidRecordWire { pid: record.pid, exe: record.exe.to_string_lossy().to_string() };
    let mut text = serde_json::to_string(&wire).expect("pid record serializes");
    text.push('\n');
    write_atomic(path, text.as_bytes(), FileMode::Default)
}

/// Delete `gateway.pid`; a missing file is fine.
pub fn remove_pid_file(path: &Path) {
    let _ = fs::remove_file(path);
}

// ------------------------------------------------------------------- reclaim

/// What [`reclaim_stale`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reclaim {
    /// No `gateway.pid`.
    NothingToDo,
    /// The record was unreadable and has been removed.
    CorruptRecord,
    /// The recorded process is gone; the record has been removed.
    StaleRecord,
    /// The recorded pid is alive but is not our sidecar. It was not touched; the record was removed.
    NotOurs { pid: u32 },
    /// A leftover sidecar of ours was ended.
    Reclaimed { pid: u32 },
    /// A leftover sidecar of ours could not be ended; the record stays.
    Failed { pid: u32 },
}

/// Whether two paths name the same file (symlinks resolved when both exist).
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn wait_for_exit(ops: &dyn ProcessOps, pid: u32, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if !ops.is_alive(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(EXIT_POLL);
    }
}

fn logger_note(pid: u32, exe: &Path) {
    eprintln!("[gateway] reclaiming a leftover gateway (pid {pid}) from an earlier install location: {}", exe.display());
}

/// Whether `path` has the shape of a bundled sidecar: `<...>/resources/gateway/kiro-gateway` (macOS: `gateway-<arch>/`)
/// (`kiro-gateway.exe` on Windows; the macOS bundle spells it `Contents/Resources/gateway/`).
///
/// Only the shape is checked, the file need not exist (the App may have been moved away).
pub fn is_bundled_sidecar_path(path: &Path) -> bool {
    let name_is = |component: Option<&std::ffi::OsStr>, wanted: &[&str]| {
        component.is_some_and(|c| wanted.iter().any(|w| c.to_string_lossy().eq_ignore_ascii_case(w)))
    };
    let gateway_dir = path.parent();
    name_is(path.file_name(), &["kiro-gateway", "kiro-gateway.exe"])
        && gateway_dir.and_then(Path::file_name).is_some_and(|d| super::sidecar::is_gateway_dir_name(&d.to_string_lossy()))
        && name_is(gateway_dir.and_then(Path::parent).and_then(Path::file_name), &["resources"])
}

/// End a gateway left behind by an earlier App run, if and only if it really is ours.
///
/// Args:
///     pid_file: `gateway.pid`.
///     our_exe: Path of this App's sidecar.
///     port: The port this App is configured to use.
///     ops: Process inspection and signalling.
///     ports: Port inspection, to see who listens on `port`.
///     grace: How long SIGTERM gets before SIGKILL (and how long to wait after SIGKILL).
///
/// Returns:
///     What happened. Nothing is signalled unless the pid is alive and either (a) its actual
///     executable equals the recorded path and the recorded path equals `our_exe`, or (b) both
///     paths are bundled-sidecar paths (the App was moved or renamed) and the process is the
///     one listening on `port`.
pub fn reclaim_stale(pid_file: &Path, our_exe: &Path, port: u16, ops: &dyn ProcessOps, ports: &dyn PortOps, grace: Duration) -> Reclaim {
    let record = match read_pid_file(pid_file) {
        PidFile::Missing => return Reclaim::NothingToDo,
        PidFile::Corrupt => {
            remove_pid_file(pid_file);
            return Reclaim::CorruptRecord;
        }
        PidFile::Valid(record) => record,
    };
    let pid = record.pid;
    if !is_signalable(pid) {
        remove_pid_file(pid_file);
        return Reclaim::NotOurs { pid };
    }
    if !ops.is_alive(pid) {
        remove_pid_file(pid_file);
        return Reclaim::StaleRecord;
    }
    let actual = ops.image_path(pid);
    let exact = actual.as_ref().is_some_and(|actual| same_path(actual, &record.exe)) && same_path(&record.exe, our_exe);
    // The App moved: the leftover runs from a bundle location that is not ours any more.
    let relocated = !exact
        && is_bundled_sidecar_path(&record.exe)
        && actual.as_ref().is_some_and(|actual| is_bundled_sidecar_path(actual))
        && ports.occupant(port).pid == Some(pid);
    if relocated {
        logger_note(pid, &record.exe);
    }
    if !(exact || relocated) {
        remove_pid_file(pid_file);
        return Reclaim::NotOurs { pid };
    }
    ops.terminate(pid);
    if !wait_for_exit(ops, pid, grace) {
        ops.kill(pid);
        if !wait_for_exit(ops, pid, grace) {
            return Reclaim::Failed { pid };
        }
    }
    remove_pid_file(pid_file);
    Reclaim::Reclaimed { pid }
}

#[cfg(test)]
mod tests;
