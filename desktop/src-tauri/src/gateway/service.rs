//! What `gateway_start`, `gateway_stop` and `gateway_models` do around the manager.
//!
//! The manager only runs and watches a process. This layer decides *whether* to start
//! (forced upgrade, environment check), prepares what a start needs (sidecar location,
//! environment variables from the stored configuration and key) and keeps `gatewayWanted`
//! in sync. It is plain Rust: the environment check is behind [`EnvGate`], the process
//! environment is injected, so the tests run without Tauri.

use super::env::{build_env, EnvInputs};
use super::health::ModelsError;
use super::manager::{GatewayManager, StartError, StartRequest};
use super::sidecar::{self, OVERRIDE_VAR};
use super::types::{GatewayState, GatewayStatus};
use crate::app_config::ConfigStore;
use serde::Serialize;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Time limit of `GET /v1/models` (contract: 5 seconds).
pub const MODELS_TIMEOUT: Duration = Duration::from_secs(5);

/// `GatewayModel` of the IPC contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GatewayModel {
    pub id: String,
}

/// The environment check as far as starting the gateway is concerned.
pub trait EnvGate: Send + Sync {
    /// Run the check now.
    ///
    /// Returns:
    ///     The kiro-cli database path to give the gateway, or the Chinese reason the
    ///     switch must stay off (`env_not_ready`).
    fn check(&self) -> Result<PathBuf, String>;
}

/// A start request that was rejected (as opposed to a start that failed, which is a state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    UpgradeRequired,
    EnvNotReady(String),
    Internal(String),
}

impl ServiceError {
    /// The `code` of the IPC error shape.
    pub fn code(&self) -> &'static str {
        match self {
            ServiceError::UpgradeRequired => "upgrade_required",
            ServiceError::EnvNotReady(_) => "env_not_ready",
            ServiceError::Internal(_) => "internal",
        }
    }

    /// One Chinese sentence, safe to show.
    pub fn message(&self) -> String {
        match self {
            ServiceError::UpgradeRequired => StartError::UpgradeRequired.message(),
            ServiceError::EnvNotReady(m) | ServiceError::Internal(m) => m.clone(),
        }
    }
}

impl From<StartError> for ServiceError {
    fn from(error: StartError) -> Self {
        match error {
            StartError::UpgradeRequired => ServiceError::UpgradeRequired,
            StartError::Internal(m) => ServiceError::Internal(m),
        }
    }
}

/// Answers "is a forced upgrade in progress?" from outside the service (the update module's
/// in-memory state). The persisted `upgradeRequired` flag is checked in addition.
pub type StartGuard = Arc<dyn Fn() -> bool + Send + Sync>;

type EnvSnapshot = Box<dyn Fn() -> Vec<(OsString, OsString)> + Send + Sync>;

/// Gateway control as the commands, the tray and the update module see it.
pub struct GatewayService {
    config: Arc<ConfigStore>,
    manager: GatewayManager,
    gate: Arc<dyn EnvGate>,
    resource_dir: PathBuf,
    sidecar_override: Option<OsString>,
    inherited_env: EnvSnapshot,
    /// Set once the update service exists (it is built after this one, since it needs it).
    start_guard: OnceLock<StartGuard>,
}

impl GatewayService {
    /// Args:
    ///     config: Configuration and key storage.
    ///     manager: The process supervisor (its port should match the stored config).
    ///     gate: Environment check run before every start.
    ///     resource_dir: Tauri's resource directory (the sidecar is `gateway/kiro-gateway` below it).
    ///     sidecar_override: Value of `KIRO_GATEWAY_SIDECAR`, if set.
    ///     inherited_env: The App's environment at the time of a start (`std::env::vars_os`).
    pub fn new(
        config: Arc<ConfigStore>,
        manager: GatewayManager,
        gate: Arc<dyn EnvGate>,
        resource_dir: PathBuf,
        sidecar_override: Option<OsString>,
        inherited_env: EnvSnapshot,
    ) -> Self {
        Self { config, manager, gate, resource_dir, sidecar_override, inherited_env, start_guard: OnceLock::new() }
    }

    /// Replace the sidecar override (builder style, for tests).
    #[cfg(test)]
    pub fn with_sidecar_override(mut self, value: Option<OsString>) -> Self {
        self.sidecar_override = value;
        self
    }

    /// Replace the inherited environment source (builder style, for tests).
    #[cfg(test)]
    pub fn with_inherited_env(mut self, source: EnvSnapshot) -> Self {
        self.inherited_env = source;
        self
    }

    /// Install the forced-upgrade guard consulted by every start, including the automatic
    /// ones (`resume_if_wanted`). Only the first call takes effect.
    pub fn set_start_guard(&self, guard: StartGuard) {
        let _ = self.start_guard.set(guard);
    }

    fn upgrade_forced(&self) -> bool {
        self.config.upgrade_required() || self.start_guard.get().is_some_and(|forced| forced())
    }

    /// The configuration store.
    pub fn config(&self) -> &Arc<ConfigStore> {
        &self.config
    }

    /// The process supervisor, for listeners.
    pub fn manager(&self) -> &GatewayManager {
        &self.manager
    }

    /// Current status.
    pub fn status(&self) -> GatewayStatus {
        self.manager.status()
    }

    /// Whether a process is starting or running (decides `restartRequired`).
    pub fn is_active(&self) -> bool {
        self.status().is_active()
    }

    /// Tell the manager the configured port changed.
    pub fn apply_port(&self, port: u16) {
        self.manager.set_port(port);
    }

    /// `gateway_start`.
    ///
    /// Order: forced upgrade (`upgrade_required`; the persisted flag or the injected guard), then the environment check
    /// (`env_not_ready`, run again on every start), then the sidecar lookup. A missing or
    /// unusable sidecar and everything that happens after the launch are *states*
    /// (`start_failed`), not errors. `gatewayWanted` becomes true once the start was
    /// accepted, whatever its outcome: the user asked for the gateway.
    ///
    /// On an already active gateway this returns the current status without any check.
    pub fn start(&self) -> Result<GatewayStatus, ServiceError> {
        if self.upgrade_forced() {
            return Err(ServiceError::UpgradeRequired);
        }
        if self.is_active() {
            return Ok(self.status());
        }
        let kiro_cli_db = self.gate.check().map_err(ServiceError::EnvNotReady)?;
        let config = self.config.get();
        let paths = self.config.paths();

        self.config
            .set_gateway_wanted(true)
            .map_err(|_| ServiceError::Internal("没能保存开关状态，请检查磁盘空间和权限".to_string()))?;

        let found = sidecar::resolve(&self.resource_dir, self.sidecar_override.as_deref(), std::env::consts::ARCH);
        let sidecar = match found {
            Ok(sidecar) => sidecar,
            Err(error) => {
                self.manager.report_start_problem(config.port, error.message());
                return Ok(self.status());
            }
        };

        let inputs = EnvInputs {
            secret: self.config.secret(),
            kiro_cli_db,
            port: config.port,
            request_log_dir: paths.request_log_dir(),
            tiktoken_cache_dir: sidecar.tiktoken_cache.clone(),
        };
        let request = StartRequest {
            env: build_env(&inputs, (self.inherited_env)()),
            sidecar,
            port: config.port,
            secret: inputs.secret.clone(),
            upgrade_required: false,
        };
        self.manager.start(request).map_err(ServiceError::from)
    }

    /// `gateway_stop`: stop, clear any failure, and record that the user turned the switch off.
    pub fn stop(&self) -> Result<GatewayStatus, ServiceError> {
        let status = self.manager.stop();
        self.config
            .set_gateway_wanted(false)
            .map_err(|_| ServiceError::Internal("没能保存开关状态，请检查磁盘空间和权限".to_string()))?;
        Ok(status)
    }

    /// Stop the process when the App quits. `gatewayWanted` stays as it is, so the next launch
    /// (or the restart after an update) brings the gateway back.
    pub fn shutdown(&self) {
        self.manager.stop();
    }

    /// Start the gateway again if the user had it on and nothing forbids it. Quiet when it
    /// cannot: a forced upgrade or a blocked environment leaves it stopped, and
    /// `gatewayWanted` stays set for the next try.
    pub fn resume_if_wanted(&self) {
        if !self.config.get().gateway_wanted {
            return;
        }
        let _ = self.start();
    }

    /// `gateway_models`.
    pub fn models(&self) -> Result<Vec<GatewayModel>, ModelsError> {
        if self.status().state != GatewayState::Running {
            return Err(ModelsError::NotRunning);
        }
        let ids = self.manager.models(&self.config.secret(), MODELS_TIMEOUT)?;
        Ok(ids.into_iter().map(|id| GatewayModel { id }).collect())
    }
}

/// Name of the variable that overrides the sidecar path (re-exported for the commands layer).
pub const SIDECAR_OVERRIDE_VAR: &str = OVERRIDE_VAR;

#[cfg(test)]
mod tests;
