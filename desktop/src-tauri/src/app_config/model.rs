//! `AppConfig` and friends: the JSON shapes of `docs/ipc-contract.md` section 1.

use super::command_name::{self, NameProblem, DEFAULT_COMMAND_NAME};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;

/// Lowest port the App accepts (no privileged ports).
pub const MIN_PORT: i64 = 1024;
/// Highest TCP port.
pub const MAX_PORT: i64 = 65535;
/// Default gateway port.
pub const DEFAULT_PORT: u16 = 6868;
/// Longest model name accepted.
const MAX_MODEL_LEN: usize = 128;

/// One of the three Claude Code model tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Opus,
    Sonnet,
    Haiku,
}

/// Kiro model used for each tier. Defaults match `scripts/kclaude.zsh`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Models {
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
}

impl Default for Models {
    fn default() -> Self {
        Self {
            opus: "claude-opus-5-5[1m]".to_string(),
            sonnet: "claude-sonnet-5-5[1m]".to_string(),
            haiku: "claude-haiku-4-5".to_string(),
        }
    }
}

impl Models {
    /// The model name of `tier`.
    pub fn get(&self, tier: ModelTier) -> &str {
        match tier {
            ModelTier::Opus => &self.opus,
            ModelTier::Sonnet => &self.sonnet,
            ModelTier::Haiku => &self.haiku,
        }
    }
}

/// `AppConfig` of the IPC contract; persisted as `config.json`.
///
/// Unknown keys in the file are ignored and missing keys take their defaults, so an
/// older or newer App version can read the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    /// 1024..=65535, default 6868. A port already saved in `config.json` is kept as it is.
    pub port: u16,
    /// Name of the kclaude shell command, default `cclaude`; see [`super::command_name`].
    pub command_name: String,
    pub models: Models,
    /// Start the App at login; mirrors the system state.
    pub autostart: bool,
    /// Whether the user last turned the gateway switch on.
    pub gateway_wanted: bool,
    /// Forced-upgrade flag, written by the update module only.
    pub upgrade_required: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            command_name: DEFAULT_COMMAND_NAME.to_string(),
            models: Models::default(),
            autostart: false,
            gateway_wanted: false,
            upgrade_required: false,
        }
    }
}

impl AppConfig {
    /// Check a whole config, used on files that may have been edited by hand.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_port(i64::from(self.port))?;
        validate_command_name(&self.command_name)?;
        for tier in [ModelTier::Opus, ModelTier::Sonnet, ModelTier::Haiku] {
            validate_model(self.models.get(tier))?;
        }
        Ok(())
    }
}

/// Models of a [`ConfigPatch`]: only the tiers present are changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialModels {
    pub opus: Option<String>,
    pub sonnet: Option<String>,
    pub haiku: Option<String>,
}

/// `ConfigPatch` of the IPC contract.
///
/// `port` is a wide integer on purpose: an out-of-range value from the frontend must
/// reach [`validate_port`] and come back as a readable `invalid_argument`, instead of
/// failing JSON decoding with a type error. There is deliberately no `upgradeRequired`
/// or `gatewayWanted` here; unknown keys are ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigPatch {
    pub port: Option<i64>,
    pub models: Option<PartialModels>,
    pub autostart: Option<bool>,
    #[serde(rename = "commandName")]
    pub command_name: Option<String>,
}

/// Failure of a config operation; maps onto the error `code`s of the IPC contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A value is out of range or unsafe to write.
    InvalidArgument(String),
    /// A file could not be written.
    WriteFailed(String),
    /// Anything else (data directory unusable, no random source).
    Internal(String),
}

impl ConfigError {
    /// The stable `code` of the IPC error shape.
    pub fn code(&self) -> &'static str {
        match self {
            ConfigError::InvalidArgument(_) => "invalid_argument",
            ConfigError::WriteFailed(_) => "write_failed",
            ConfigError::Internal(_) => "internal",
        }
    }

    /// A one-sentence Chinese message that is safe to show: no paths, no OS text.
    pub fn message(&self) -> &str {
        match self {
            ConfigError::InvalidArgument(m) | ConfigError::WriteFailed(m) | ConfigError::Internal(m) => m,
        }
    }

    /// A [`ConfigError::WriteFailed`] with a short reason derived from the I/O error kind.
    pub fn write_failed(error: &io::Error) -> Self {
        ConfigError::WriteFailed(format!("没能保存配置：{}", io_reason(error)))
    }

    /// A [`ConfigError::Internal`] with a short reason derived from the I/O error kind.
    pub fn internal(what: &str, error: &io::Error) -> Self {
        ConfigError::Internal(format!("{what}：{}", io_reason(error)))
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for ConfigError {}

/// Short Chinese reason for an I/O failure, without paths or OS error text.
fn io_reason(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "没有写入权限",
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => "磁盘空间不足",
        io::ErrorKind::NotFound => "目录不存在",
        _ => match error.raw_os_error() {
            Some(28) | Some(122) => "磁盘空间不足",
            _ => "文件被占用或磁盘出错，稍后重试",
        },
    }
}

/// Check a port from the frontend.
///
/// Returns:
///     The port as `u16`, or `invalid_argument` outside 1024..=65535.
pub fn validate_port(port: i64) -> Result<u16, ConfigError> {
    if (MIN_PORT..=MAX_PORT).contains(&port) {
        Ok(port as u16)
    } else {
        Err(ConfigError::InvalidArgument(format!("端口需要在 {MIN_PORT} 到 {MAX_PORT} 之间")))
    }
}

/// Check a command name from the frontend (format and reserved names only).
///
/// Whether the name is already used by another command on this machine is checked by the
/// kclaude service, which knows the PATH and the startup files.
///
/// Returns:
///     The name, or `invalid_argument` with the reason in Chinese.
pub fn validate_command_name(name: &str) -> Result<String, ConfigError> {
    match command_name::validate(name) {
        Ok(()) => Ok(name.to_string()),
        Err(problem) => Err(command_name_error(problem)),
    }
}

/// The `invalid_argument` error for a command-name problem.
pub fn command_name_error(problem: NameProblem) -> ConfigError {
    ConfigError::InvalidArgument(problem.message().to_string())
}

/// Check a model name and return it trimmed.
///
/// The name is written unquoted into `kclaude.env`, so only characters that no shell
/// treats specially are allowed: ASCII letters, digits and `. _ - : [ ] /`. That covers
/// every real model id (`claude-opus-5-5[1m]`, `us.anthropic.claude-haiku-4-5:0`).
pub fn validate_model(name: &str) -> Result<String, ConfigError> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.len() <= MAX_MODEL_LEN
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '[' | ']' | '/'));
    if ok {
        Ok(name.to_string())
    } else {
        Err(ConfigError::InvalidArgument("模型名不能为空，且只能包含字母、数字和 . _ - : [ ] /".to_string()))
    }
}

/// Apply `patch` to `base`; all-or-nothing.
///
/// Returns:
///     The new config, or the first validation error with `base` untouched.
pub fn apply_patch(base: &AppConfig, patch: &ConfigPatch) -> Result<AppConfig, ConfigError> {
    let mut next = base.clone();
    if let Some(port) = patch.port {
        next.port = validate_port(port)?;
    }
    if let Some(models) = &patch.models {
        if let Some(name) = &models.opus {
            next.models.opus = validate_model(name)?;
        }
        if let Some(name) = &models.sonnet {
            next.models.sonnet = validate_model(name)?;
        }
        if let Some(name) = &models.haiku {
            next.models.haiku = validate_model(name)?;
        }
    }
    if let Some(name) = &patch.command_name {
        next.command_name = validate_command_name(name)?;
    }
    if let Some(autostart) = patch.autostart {
        next.autostart = autostart;
    }
    Ok(next)
}

#[cfg(test)]
mod tests;
