//! Thin Tauri glue for the configuration: `config_get`, `config_update` and the adapters
//! other modules need. No logic of its own; everything is in [`ConfigStore`].
//!

use super::model::{AppConfig, ConfigError, ConfigPatch};
use super::paths::AppPaths;
use super::store::{ConfigStore, OpenNote, UpdateOutcome};
use crate::gateway::service::GatewayService;
use chrono::Local;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_autostart::ManagerExt;

/// Error shape of every command (ipc-contract.md, "错误").
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

impl From<ConfigError> for CommandError {
    fn from(error: ConfigError) -> Self {
        Self { code: error.code(), message: error.message().to_string() }
    }
}

/// `ConfigUpdateResult` of the contract.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigUpdateResult {
    pub config: AppConfig,
    pub restart_required: bool,
}

/// Open the store in the App data directory and register it as managed state.
///
/// Call once from `setup`, before `gateway::commands::setup`.
///
/// Returns:
///     The store and the notes from opening it (first-run key, corrupt-config backup) for
///     the caller to log.
pub fn setup<R: Runtime>(app: &AppHandle<R>) -> Result<(Arc<ConfigStore>, Vec<OpenNote>), CommandError> {
    let dir = app.path().app_data_dir().map_err(|_| CommandError {
        code: "internal",
        message: "找不到应用数据目录".to_string(),
    })?;
    let (store, notes) = ConfigStore::open(AppPaths::new(dir), Local::now().naive_local())?;
    let store = Arc::new(store);
    app.manage(store.clone());
    Ok((store, notes))
}

/// `config_get`
#[tauri::command]
pub fn config_get(store: State<'_, Arc<ConfigStore>>) -> AppConfig {
    store.get()
}

/// `config_update`
///
/// The patch is validated first. The autostart toggle (a system call that can fail) runs
/// before anything is written, so a failure never leaves a half-applied patch. The stored
/// `autostart` is whatever the system reports afterwards.
///
/// A new `commandName` is also checked against the PATH and the startup files, and the
/// kclaude scripts and startup-file block are rewritten under the new name (see
/// `kclaude::rename`). That runs a login shell, so the command is `async` and executes on
/// the async runtime's thread pool instead of the main thread.
#[tauri::command(async)]
pub fn config_update<R: Runtime>(
    app: AppHandle<R>,
    store: State<'_, Arc<ConfigStore>>,
    gateway: State<'_, Arc<GatewayService>>,
    patch: ConfigPatch,
) -> Result<ConfigUpdateResult, CommandError> {
    store.check_patch(&patch)?;
    // Refuse a taken name before the autostart toggle, so a rejected patch changes nothing.
    let kclaude_machine = match patch.command_name.as_deref() {
        Some(name) if name != store.get().command_name => {
            let (env, backend) = crate::kclaude::commands::env_and_backend(&app)
                .map_err(|e| CommandError { code: "internal", message: e.message })?;
            let env = env.with_command(&store.get().command_name);
            crate::kclaude::rename::check_name(&env, &backend, name)?;
            Some((env, backend))
        }
        _ => None,
    };

    let mut actual_autostart = None;
    if let Some(wanted) = patch.autostart {
        let manager = app.autolaunch();
        let result = if wanted { manager.enable() } else { manager.disable() };
        if result.is_err() {
            return Err(CommandError { code: "write_failed", message: "没能修改开机自启，请检查系统设置".to_string() });
        }
        actual_autostart = Some(manager.is_enabled().unwrap_or(wanted));
    }

    let mut patch = patch;
    patch.autostart = None;
    let UpdateOutcome { config, restart_required } = match &kclaude_machine {
        Some((env, backend)) => crate::kclaude::rename::apply_update(&store, env, backend, &patch, gateway.is_active())?,
        None => store.update(&patch, gateway.is_active())?,
    };
    gateway.apply_port(config.port);
    let config = match actual_autostart {
        Some(actual) => store.sync_autostart(actual)?,
        None => config,
    };
    Ok(ConfigUpdateResult { config, restart_required })
}
