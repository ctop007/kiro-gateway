//! Environment check for the desktop app (`docs/ipc-contract.md` section 3).
//!
//! Answers four questions: is kiro-cli installed, is it logged in, is the login
//! still usable, is Claude Code installed.
//!
//! Layout:
//! - `which` (tasks 5.1): locate `kiro-cli` / `claude` via the login shell PATH,
//!   the process PATH and per-platform default install directories
//! - `login` (tasks 5.2): read-only probe of the kiro-cli sqlite database,
//!   returns only login method, expiry and refreshability (never a token)
//! - `report`: compose the `EnvReport` of the IPC contract and the gateway gate
//! - `commands`: the `#[tauri::command]` wrapper, no logic
//!
//! Everything except `commands` is plain Rust with no Tauri dependency; all
//! inputs (home directory, PATH, clock, database path) are injectable so the
//! tests never touch the developer's real files.

pub mod login;
pub mod report;
pub mod which;

pub mod commands;

use std::path::PathBuf;

/// Platform family used to pick default locations and executable names.
///
/// Only macOS and Windows are shipped; any other Unix is treated like macOS so
/// that `cargo test` also works on a Linux CI box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
}

impl Platform {
    /// The platform this binary was compiled for.
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::MacOs
        }
    }
}

/// The user directories the lookup rules are computed from.
///
/// On macOS only `home` is used. On Windows `home` is `%USERPROFILE%`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserDirs {
    /// `$HOME` / `%USERPROFILE%`.
    pub home: Option<PathBuf>,
    /// `%LOCALAPPDATA%` (Windows only).
    pub local_app_data: Option<PathBuf>,
    /// `%APPDATA%` (Windows only).
    pub app_data: Option<PathBuf>,
}

impl UserDirs {
    /// Read the directories from the process environment.
    ///
    /// Empty and relative values are ignored so a bogus `HOME=""` can never
    /// turn a lookup into a search of the current directory.
    pub fn from_process() -> UserDirs {
        fn abs_env(name: &str) -> Option<PathBuf> {
            let value = std::env::var_os(name)?;
            let path = PathBuf::from(value);
            if path.is_absolute() {
                Some(path)
            } else {
                None
            }
        }
        let home = if cfg!(windows) {
            abs_env("USERPROFILE")
        } else {
            abs_env("HOME")
        };
        UserDirs {
            home,
            local_app_data: abs_env("LOCALAPPDATA"),
            app_data: abs_env("APPDATA"),
        }
    }
}
