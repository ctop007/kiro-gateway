//! Thin Tauri glue for the environment check: one command, no logic.
//!
//! Everything is delegated to `report::run_env_check`.

use super::report::{run_env_check, EnvReport};
use serde::Serialize;

/// Error shape of every command (ipc-contract.md, "错误").
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

/// `env_check`: run all four checks and return the report.
///
/// The login shell probe and the sqlite read block for up to a few seconds, so
/// the work runs on the blocking thread pool instead of the async runtime.
#[tauri::command]
pub async fn env_check() -> Result<EnvReport, CommandError> {
    tauri::async_runtime::spawn_blocking(run_env_check).await.map_err(|_| CommandError {
        code: "internal",
        message: "环境检测没能完成，请重试".to_string(),
    })
}
