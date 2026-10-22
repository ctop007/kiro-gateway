//! Thin Tauri glue for the gateway: `gateway_*` commands, the `gateway-status` event and the
//! adapters other modules need. No logic of its own; everything is in [`GatewayService`].
//!

use super::health::ModelsError;
use super::manager::{Deps, GatewayManager, ManagerSettings, SystemClock};
use super::port::SystemPorts;
use super::service::{EnvGate, GatewayModel, GatewayService, ServiceError, SIDECAR_OVERRIDE_VAR};
use super::sys::SystemProcessOps;
use super::types::GatewayStatus;
use crate::app_config::ConfigStore;
use crate::env_check::report::{gateway_gate, run_env_check, GatewayGate};
use crate::env_check::{login, UserDirs};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_opener::OpenerExt;

/// Event name from ipc-contract.md section 2.
pub const STATUS_EVENT: &str = "gateway-status";

/// Timeout of one `/health` request.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);

/// Error shape of every command (ipc-contract.md, "错误").
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

impl From<ServiceError> for CommandError {
    fn from(error: ServiceError) -> Self {
        Self { code: error.code(), message: error.message() }
    }
}

impl From<ModelsError> for CommandError {
    fn from(error: ModelsError) -> Self {
        Self { code: error.code(), message: error.message().to_string() }
    }
}

/// The real environment check: `env_check` decides whether the switch may go on, and the
/// database path comes from the same probe the check used.
struct EnvCheckGate;

impl EnvGate for EnvCheckGate {
    fn check(&self) -> Result<PathBuf, String> {
        let report = run_env_check();
        if let GatewayGate::Blocked { reason } = gateway_gate(&report) {
            return Err(reason);
        }
        login::find_database(crate::env_check::Platform::current(), &UserDirs::from_process())
            .ok_or_else(|| "先登录 kiro-cli 才能打开".to_string())
    }
}

/// Create the gateway service, wire the status event and register everything as managed state.
///
/// Call once from `setup`, after `app_config::commands::setup`.
///
/// Args:
///     app: The app handle.
///     config: The store returned by `app_config::commands::setup`.
///
/// Returns:
///     The service, for the update module (`GatewayControl`), the tray and the exit hook.
pub fn setup<R: Runtime>(app: &AppHandle<R>, config: Arc<ConfigStore>) -> Result<Arc<GatewayService>, CommandError> {
    let resource_dir = app.path().resource_dir().map_err(|_| CommandError {
        code: "internal",
        message: "找不到应用资源目录".to_string(),
    })?;
    let service = build_service(config, resource_dir, Arc::new(EnvCheckGate));
    let handle = app.clone();
    service.manager().add_listener(Box::new(move |status: &GatewayStatus| {
        // A failed emit means the webview is gone; the next transition tries again.
        let _ = handle.emit(STATUS_EVENT, status.clone());
    }));
    app.manage(service.clone());
    Ok(service)
}

/// The gateway service as the app runs it: real clock, health probe, process inspector and
/// port checks, configured port from the store. Shared by [`setup`] and the debug self-test.
///
/// Args:
///     config: Configuration and key storage.
///     resource_dir: Directory that holds `gateway/kiro-gateway`.
///     gate: Environment check run before every start.
pub fn build_service(config: Arc<ConfigStore>, resource_dir: PathBuf, gate: Arc<dyn EnvGate>) -> Arc<GatewayService> {
    let deps = Deps {
        clock: Arc::new(SystemClock),
        health: Arc::new(super::health::HttpHealth::new(HEALTH_TIMEOUT)),
        ops: Arc::new(SystemProcessOps),
        ports: Arc::new(SystemPorts),
    };
    let manager = GatewayManager::new(config.paths().clone(), deps, ManagerSettings::default(), config.get().port);
    Arc::new(GatewayService::new(
        config,
        manager,
        gate,
        resource_dir,
        std::env::var_os(SIDECAR_OVERRIDE_VAR),
        Box::new(|| std::env::vars_os().collect()),
    ))
}

/// The real environment gate (`env_check`), for the debug self-test.
#[cfg(debug_assertions)]
pub fn env_check_gate() -> Arc<dyn EnvGate> {
    Arc::new(EnvCheckGate)
}

/// `gateway_status`
#[tauri::command]
pub fn gateway_status(service: State<'_, Arc<GatewayService>>) -> GatewayStatus {
    service.status()
}

/// Start the gateway on behalf of a command or the tray.
///
/// The update module is asked first: a forced upgrade in progress refuses the start even
/// when the persisted flag could not be written. Blocking: call from a worker thread.
pub fn start_guarded<R: Runtime>(app: &AppHandle<R>, service: &GatewayService) -> Result<GatewayStatus, CommandError> {
    if let Some(update) = app.try_state::<Arc<crate::update::UpdateService>>() {
        if let Err(error) = update.guard_gateway_start() {
            return Err(CommandError { code: "upgrade_required", message: error.message });
        }
    }
    service.start().map_err(CommandError::from)
}

/// `gateway_start`
///
/// Runs on the blocking pool: the environment check can take a few seconds and a start
/// first ends a leftover gateway of an earlier run.
#[tauri::command]
pub async fn gateway_start<R: Runtime>(
    app: AppHandle<R>,
    service: State<'_, Arc<GatewayService>>,
) -> Result<GatewayStatus, CommandError> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || start_guarded(&app, &service))
        .await
        .map_err(|_| CommandError { code: "internal", message: "没能启动网关".to_string() })?
}

/// `gateway_stop`
#[tauri::command]
pub async fn gateway_stop(service: State<'_, Arc<GatewayService>>) -> Result<GatewayStatus, CommandError> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.stop())
        .await
        .map_err(|_| CommandError { code: "internal", message: "没能停止网关".to_string() })?
        .map_err(CommandError::from)
}

/// `gateway_models`
#[tauri::command]
pub async fn gateway_models(service: State<'_, Arc<GatewayService>>) -> Result<Vec<GatewayModel>, CommandError> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.models())
        .await
        .map_err(|_| CommandError { code: "internal", message: "没能读取模型列表".to_string() })?
        .map_err(CommandError::from)
}

/// `gateway_reveal_log`: open `logs/gateway.log` with the default program.
#[tauri::command]
pub fn gateway_reveal_log<R: Runtime>(
    app: AppHandle<R>,
    service: State<'_, Arc<GatewayService>>,
) -> Result<(), CommandError> {
    let log = service.config().paths().gateway_log();
    if !log.is_file() {
        return Err(CommandError { code: "internal", message: "还没有网关日志".to_string() });
    }
    app.opener()
        .open_path(log.to_string_lossy().to_string(), None::<&str>)
        .map_err(|_| CommandError { code: "internal", message: "没能打开日志文件".to_string() })
}

/// Stop the gateway process synchronously. Call from the tray "quit" item and `RunEvent::Exit`.
pub fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    if let Some(service) = app.try_state::<Arc<GatewayService>>() {
        service.shutdown();
    }
}

/// Adapter for the update module: it can stop the gateway and bring it back afterwards.
pub struct UpdateGatewayControl(pub Arc<GatewayService>);

impl crate::update::ports::GatewayControl for UpdateGatewayControl {
    fn stop(&self) {
        self.0.shutdown();
    }

    fn resume_if_wanted(&self) {
        self.0.resume_if_wanted();
    }
}

/// Adapter for the update module: the persisted `upgradeRequired` flag.
pub struct UpdateUpgradeFlag(pub Arc<ConfigStore>);

impl crate::update::ports::UpgradeFlag for UpdateUpgradeFlag {
    fn is_set(&self) -> bool {
        self.0.upgrade_required()
    }

    fn set(&self, value: bool) -> Result<(), String> {
        self.0.set_upgrade_required(value).map_err(|e| e.message().to_string())
    }
}
