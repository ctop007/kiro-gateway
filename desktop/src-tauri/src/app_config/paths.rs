//! File layout below the App data directory (design decision 3).

use std::path::{Path, PathBuf};

/// Every path the App uses below its data directory.
///
/// Built from an injected root so tests can point it at a temporary directory;
/// production passes Tauri's `app_data_dir()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    /// Create the layout for `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The App data directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `config.json`.
    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.json")
    }

    /// `secret`: the access key, 0600.
    pub fn secret_file(&self) -> PathBuf {
        self.root.join("secret")
    }

    /// `kclaude.env`: sourced by the kclaude scripts.
    pub fn kclaude_env_file(&self) -> PathBuf {
        self.root.join("kclaude.env")
    }

    /// `logs/`.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// `logs/gateway.log`: stdout and stderr of the sidecar.
    pub fn gateway_log(&self) -> PathBuf {
        self.logs_dir().join("gateway.log")
    }

    /// `logs/requests/`: where the gateway writes its structured request log.
    pub fn request_log_dir(&self) -> PathBuf {
        self.logs_dir().join("requests")
    }

    /// `runtime/`: the sidecar's working directory, kept free of any `.env`.
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }

    /// `gateway.pid`: pid and executable path of the sidecar the App started.
    pub fn pid_file(&self) -> PathBuf {
        self.root.join("gateway.pid")
    }
}
