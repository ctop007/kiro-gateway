//! Thin Tauri glue for the request log: two commands and the 1 second poller.
//!
//! No logic lives here. Everything is delegated to `LogStore`, which takes the
//! directory, clock and timezone as arguments.

use crate::request_log::model::RequestLogSnapshot;
use crate::request_log::store::{InvalidLimit, LogStore};
use chrono::{Local, Utc};
use serde::Serialize;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Event name from ipc-contract.md section 5.
pub const APPENDED_EVENT: &str = "request-log-appended";

/// Polling interval of the month file.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Directory below the App data directory where the gateway writes its log.
const LOG_SUBDIR: [&str; 2] = ["logs", "requests"];

/// Error shape of every command (ipc-contract.md, "错误").
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

impl From<InvalidLimit> for CommandError {
    fn from(_: InvalidLimit) -> Self {
        Self { code: "invalid_argument", message: "limit 需要在 1 到 1000 之间".to_string() }
    }
}

/// Managed state: the store behind a mutex shared by the commands and the poller.
pub struct RequestLogState(Mutex<LogStore<Local>>);

fn lock(state: &RequestLogState) -> MutexGuard<'_, LogStore<Local>> {
    // A poisoned lock only means a previous holder panicked; the store itself
    // is still consistent because every mutation is a single method call.
    state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Directory the gateway writes request logs to, below the App data directory.
pub fn log_dir(app_data_dir: PathBuf) -> PathBuf {
    LOG_SUBDIR.iter().fold(app_data_dir, |dir, part| dir.join(part))
}

/// Load the log and register it as managed state. Call once from `setup`; the commands
/// and the tray need the state, the poller ([`spawn_poller`]) starts later.
///
/// Args:
///     app: The running app; its data directory holds `logs/requests`.
///
/// Returns:
///     An error when the App data directory cannot be resolved.
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let dir = log_dir(app.path().app_data_dir()?);
    app.manage(RequestLogState(Mutex::new(LogStore::open(dir, Utc::now(), Local))));
    Ok(())
}

/// Start the 1 second poller that emits `request-log-appended`. Call once after [`init`].
pub fn spawn_poller(app: &AppHandle) {
    let handle = app.clone();
    thread::spawn(move || loop {
        thread::sleep(POLL_INTERVAL);
        let state = handle.state::<RequestLogState>();
        let tick = lock(&state).tick(Utc::now());
        if let Some(payload) = tick.appended {
            // Emit failures mean the webview is gone; the next tick will try again.
            let _ = handle.emit(APPENDED_EVENT, payload);
        }
    });
}

/// Credits used today (local day), for the tray menu. `None` before the log is loaded.
pub fn today_credits(app: &AppHandle) -> Option<f64> {
    let state = app.try_state::<RequestLogState>()?;
    let guard = lock(&state);
    guard.snapshot(Some(1), Utc::now()).ok().map(|s| s.summary.today.credits)
}

/// `request_log_snapshot { limit? }`
#[tauri::command]
pub fn request_log_snapshot(
    state: State<'_, RequestLogState>,
    limit: Option<usize>,
) -> Result<RequestLogSnapshot, CommandError> {
    lock(&state).snapshot(limit, Utc::now()).map_err(CommandError::from)
}

/// `request_log_clear`
#[tauri::command]
pub fn request_log_clear(state: State<'_, RequestLogState>) -> Result<RequestLogSnapshot, CommandError> {
    lock(&state).clear(Utc::now()).map_err(|e: io::Error| CommandError {
        code: "write_failed",
        message: format!("没能清空请求日志：{}", io_reason(&e)),
    })
}

/// Short Chinese reason for a failed deletion, without paths.
fn io_reason(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "没有权限删除日志文件",
        _ => "日志文件正被占用或磁盘出错，稍后重试",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dir_is_logs_requests_below_the_data_dir() {
        let dir = log_dir(PathBuf::from("/data/app"));
        assert_eq!(dir, PathBuf::from("/data/app/logs/requests"));
    }

    #[test]
    fn invalid_limit_maps_to_invalid_argument() {
        let error = CommandError::from(InvalidLimit(0));
        assert_eq!(error.code, "invalid_argument");
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["code"], "invalid_argument");
        assert!(json["message"].as_str().unwrap().contains("1000"));
    }

    #[test]
    fn io_reason_never_leaks_paths_or_os_text() {
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "/Users/x/secret: denied");
        assert!(!io_reason(&denied).contains('/'));
        let other = io::Error::new(io::ErrorKind::Other, "/Users/x/secret");
        assert!(!io_reason(&other).contains("secret"));
    }
}
