//! Changing the command name (`config_update` with `commandName`).
//!
//! The flow, in order:
//!
//! 1. The name is checked: format and reserved names (config layer), then whether it is
//!    already an executable on the PATH or a function / alias / `source` in the startup
//!    file ([`check_name`]). Nothing is written when it fails.
//! 2. `config.json` and `kclaude.env` get the new name (one commit in the config store).
//! 3. The kclaude service rewrites the scripts and the startup-file block under the new
//!    name ([`KclaudeService::rename_from`]): one block, one backup per file, old scripts
//!    deleted last. A foreign or user-removed command is only saved, not written.
//! 4. If step 3 fails, the previous name is restored in the config so the settings and the
//!    startup files keep agreeing, and the error is returned.
//!
//! Plain Rust: the machine description, the backend and the store are injected.

use crate::app_config::command_name::NameProblem;
use crate::app_config::model::{command_name_error, ConfigError, ConfigPatch};
use crate::app_config::store::{ConfigStore, UpdateOutcome};

use super::script;
use super::service::{Backend, KclaudeEnv, KclaudeService};
use super::types::{ErrorCode, KclaudeError};

/// Check that `new_name` can be used on this machine.
///
/// Args:
///     env: The machine with the command name currently in effect.
///     backend: PATH lookup and the other side effects.
///     new_name: The requested name.
///
/// Returns:
///     `invalid_argument` with the reason in Chinese (format, system command, or already
///     taken by a PATH executable or a startup-file definition).
pub fn check_name(env: &KclaudeEnv, backend: &dyn Backend, new_name: &str) -> Result<(), ConfigError> {
    match KclaudeService::new(env.clone(), backend).name_problem(new_name) {
        None => Ok(()),
        Some(problem) => Err(command_name_error(problem)),
    }
}

/// Map a kclaude failure onto the two error codes `config_update` documents.
fn config_error(error: KclaudeError) -> ConfigError {
    match error.code {
        ErrorCode::InvalidArgument | ErrorCode::ForeignPresent => {
            ConfigError::InvalidArgument(NameProblem::Taken.message().to_string())
        }
        ErrorCode::BackupFailed | ErrorCode::WriteFailed | ErrorCode::Internal => ConfigError::WriteFailed(error.message),
    }
}

/// Apply a patch that may carry `commandName`, keeping config and startup files in step.
///
/// Args:
///     store: The configuration store (holds the name currently in effect).
///     env: The machine with that current name in `env.command`.
///     backend: Side effects.
///     patch: The patch from the frontend; `autostart` must already be handled by the caller.
///     gateway_running: Whether the gateway is starting or running (for `restart_required`).
///
/// Returns:
///     The new config (after the rewrite) and `restart_required`. On any error the config is
///     as it was before the call, except that a failed rollback is silent.
pub fn apply_update(
    store: &ConfigStore,
    env: &KclaudeEnv,
    backend: &dyn Backend,
    patch: &ConfigPatch,
    gateway_running: bool,
) -> Result<UpdateOutcome, ConfigError> {
    let old = store.get().command_name;
    let renamed_to = patch.command_name.as_deref().filter(|name| *name != old);
    if let Some(name) = renamed_to {
        check_name(env, backend, name)?;
    }
    let outcome = store.update(patch, gateway_running)?;
    let Some(new_name) = renamed_to else {
        return Ok(outcome);
    };

    let service = KclaudeService::new(env.with_command(new_name), backend);
    match service.rename_from(&old) {
        Ok(_) => Ok(outcome),
        Err(error) => {
            let back = ConfigPatch { command_name: Some(old), ..Default::default() };
            let _ = store.update(&back, gateway_running);
            // The script of the new name may have been written before the failure.
            script::remove_all(new_name, &env.data_dir);
            Err(config_error(error))
        }
    }
}

// The tests drive real zsh / bash login shells and set Unix permissions.
#[cfg(all(test, unix))]
mod tests;
