//! `ConfigStore`: the one owner of `config.json`, `secret` and `kclaude.env`.
//!
//! Every change goes through one mutex, so the three files never disagree about the
//! port, the models or the key. `config.json` is committed first and `kclaude.env`
//! right after it; both are atomic writes.

use super::atomic::{write_atomic, FileMode};
use super::env_file;
use super::model::{apply_patch, AppConfig, ConfigError, ConfigPatch};
use super::paths::AppPaths;
use super::secret::{self, SecretOrigin};
use chrono::NaiveDateTime;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// Something worth telling the user happened while opening the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenNote {
    /// First run: a new access key was generated.
    SecretCreated,
    /// The `secret` file was empty or malformed and was replaced with a new key.
    /// Terminals keep working because they read `kclaude.env` on every run.
    SecretRegenerated,
    /// `config.json` could not be used; defaults were restored. `backup_kept` tells
    /// whether the old file was saved as `config.json.bak-<YYYYMMDD-HHMMSS>`.
    ConfigCorrupt { backup_kept: bool },
}

/// Result of [`ConfigStore::update`] (`ConfigUpdateResult` of the contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOutcome {
    pub config: AppConfig,
    /// True when the port changed while the gateway is running.
    pub restart_required: bool,
}

struct Inner {
    config: AppConfig,
    secret: String,
}

/// Thread-safe configuration and key storage.
pub struct ConfigStore {
    paths: AppPaths,
    inner: Mutex<Inner>,
}

enum Loaded {
    Missing,
    Valid(AppConfig),
    Corrupt,
}

fn load_config(path: &Path) -> Result<Loaded, io::Error> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Missing),
        Err(e) => return Err(e),
    };
    match serde_json::from_slice::<AppConfig>(&bytes) {
        Ok(config) if config.validate().is_ok() => Ok(Loaded::Valid(config)),
        _ => Ok(Loaded::Corrupt),
    }
}

/// Copy `config.json` to `config.json.bak-<stamp>`, never overwriting an older backup.
fn backup_config(paths: &AppPaths, now: NaiveDateTime) -> bool {
    let stamp = now.format("%Y%m%d-%H%M%S").to_string();
    let source = paths.config_file();
    for attempt in 0..100u32 {
        let name = if attempt == 0 {
            format!("config.json.bak-{stamp}")
        } else {
            format!("config.json.bak-{stamp}-{attempt}")
        };
        let target = paths.root().join(name);
        if target.exists() {
            continue;
        }
        return fs::copy(&source, &target).is_ok();
    }
    false
}

fn config_bytes(config: &AppConfig) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(config).expect("AppConfig always serializes");
    text.push('\n');
    text.into_bytes()
}

fn write_env_if_changed(paths: &AppPaths, config: &AppConfig, secret: &str) -> io::Result<()> {
    let wanted = env_file::render(config, secret);
    let file = paths.kclaude_env_file();
    if fs::read(&file).map(|existing| existing == wanted.as_bytes()).unwrap_or(false) {
        return Ok(());
    }
    write_atomic(&file, wanted.as_bytes(), FileMode::Private)
}

impl ConfigStore {
    /// Open the store: load or create `config.json` and `secret`, refresh `kclaude.env`.
    ///
    /// Nothing is rewritten when everything is already up to date. A corrupt
    /// `config.json` is backed up and replaced by defaults.
    ///
    /// Args:
    ///     paths: App data layout (created if missing).
    ///     now: Local time, used only for the backup file name.
    ///
    /// Returns:
    ///     The store and the notes the UI may want to show, or `internal` /
    ///     `write_failed` when the data directory cannot be used.
    pub fn open(paths: AppPaths, now: NaiveDateTime) -> Result<(Self, Vec<OpenNote>), ConfigError> {
        let mut notes = Vec::new();
        fs::create_dir_all(paths.root()).map_err(|e| ConfigError::internal("没能创建数据目录", &e))?;

        let (config, config_needs_write) = match load_config(&paths.config_file())
            .map_err(|e| ConfigError::internal("没能读取配置", &e))?
        {
            Loaded::Valid(config) => (config, false),
            Loaded::Missing => (AppConfig::default(), true),
            Loaded::Corrupt => {
                let backup_kept = backup_config(&paths, now);
                notes.push(OpenNote::ConfigCorrupt { backup_kept });
                (AppConfig::default(), true)
            }
        };

        let (secret, origin) =
            secret::load_or_create(&paths).map_err(|e| ConfigError::internal("没能准备访问密钥", &e))?;
        match origin {
            SecretOrigin::Created => notes.push(OpenNote::SecretCreated),
            SecretOrigin::Regenerated => notes.push(OpenNote::SecretRegenerated),
            SecretOrigin::Existing => {}
        }

        if config_needs_write {
            write_atomic(&paths.config_file(), &config_bytes(&config), FileMode::Default)
                .map_err(|e| ConfigError::write_failed(&e))?;
        }
        write_env_if_changed(&paths, &config, &secret).map_err(|e| ConfigError::write_failed(&e))?;

        Ok((Self { paths, inner: Mutex::new(Inner { config, secret }) }, notes))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock only means a holder panicked between two complete commits.
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The data layout this store works in.
    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    /// A copy of the current configuration.
    pub fn get(&self) -> AppConfig {
        self.lock().config.clone()
    }

    /// The access key. For the gateway environment only; never send it over IPC.
    pub fn secret(&self) -> String {
        self.lock().secret.clone()
    }

    /// Whether the forced-upgrade flag is set.
    pub fn upgrade_required(&self) -> bool {
        self.lock().config.upgrade_required
    }

    /// Persist `next` (`config.json`, then `kclaude.env`) and make it current.
    ///
    /// If `config.json` cannot be written nothing changes. If only `kclaude.env` fails,
    /// `config.json` is already committed, so memory follows it and the error is returned.
    fn commit(&self, inner: &mut Inner, next: AppConfig, refresh_env: bool) -> Result<(), ConfigError> {
        if next == inner.config {
            return Ok(());
        }
        write_atomic(&self.paths.config_file(), &config_bytes(&next), FileMode::Default)
            .map_err(|e| ConfigError::write_failed(&e))?;
        inner.config = next;
        if refresh_env {
            write_env_if_changed(&self.paths, &inner.config, &inner.secret)
                .map_err(|e| ConfigError::write_failed(&e))?;
        }
        Ok(())
    }

    /// Apply a patch from the frontend (`config_update`).
    ///
    /// Args:
    ///     patch: Fields to change; absent fields stay.
    ///     gateway_running: Whether the gateway is `starting` / `running`.
    ///
    /// Returns:
    ///     The new config and whether a restart is needed, or `invalid_argument` /
    ///     `write_failed`. On error nothing is changed (except as noted in `commit`).
    pub fn update(&self, patch: &ConfigPatch, gateway_running: bool) -> Result<UpdateOutcome, ConfigError> {
        let mut inner = self.lock();
        let next = apply_patch(&inner.config, patch)?;
        let restart_required = gateway_running && next.port != inner.config.port;
        self.commit(&mut inner, next, true)?;
        Ok(UpdateOutcome { config: inner.config.clone(), restart_required })
    }

    /// Validate a patch against the current configuration without applying it.
    ///
    /// Lets the caller do other work that can fail (the autostart toggle) before committing,
    /// so a rejected patch never half-applies.
    pub fn check_patch(&self, patch: &ConfigPatch) -> Result<(), ConfigError> {
        apply_patch(&self.lock().config, patch).map(|_| ())
    }

    /// Record whether the user wants the gateway on (`gatewayWanted`).
    pub fn set_gateway_wanted(&self, wanted: bool) -> Result<(), ConfigError> {
        let mut inner = self.lock();
        let mut next = inner.config.clone();
        next.gateway_wanted = wanted;
        self.commit(&mut inner, next, false)
    }

    /// Set the forced-upgrade flag, in `config.json` and `kclaude.env` (for the update module).
    pub fn set_upgrade_required(&self, required: bool) -> Result<(), ConfigError> {
        let mut inner = self.lock();
        let mut next = inner.config.clone();
        next.upgrade_required = required;
        self.commit(&mut inner, next, true)
    }

    /// Store the autostart state the system actually has (it may differ from what was asked).
    pub fn sync_autostart(&self, actual: bool) -> Result<AppConfig, ConfigError> {
        let mut inner = self.lock();
        let mut next = inner.config.clone();
        next.autostart = actual;
        self.commit(&mut inner, next, false)?;
        Ok(inner.config.clone())
    }
}

#[cfg(test)]
mod tests;
