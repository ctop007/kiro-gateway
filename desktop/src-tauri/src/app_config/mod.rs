//! App configuration for the desktop app (tasks 4.1, `docs/ipc-contract.md` section 1).
//!
//! Owns everything the App keeps in its data directory about the gateway setup:
//!
//! - `config.json`: port, model mapping, autostart, `gatewayWanted`, `upgradeRequired`
//! - `secret`: the access key (the gateway's `PROXY_API_KEY`), 32 random bytes as hex, 0600
//! - `kclaude.env`: `KEY=VALUE` lines the `kclaude` scripts source on every run, rewritten
//!   whenever the configuration or the key changes
//!
//! Layout:
//! - `paths`: every file and directory below the App data directory
//! - `atomic`: temporary file + rename writes (optionally 0600 from the first byte)
//! - `model`: `AppConfig`, `ConfigPatch`, validation and the error type
//! - `env_file`: rendering of `kclaude.env`
//! - `secret`: key generation and validation
//! - `store`: `ConfigStore`, the only thing other modules talk to
//! - `commands`: `config_get` / `config_update`, no logic of its own
//!
//! Everything except `commands` is plain Rust with no Tauri dependency; the data
//! directory and the clock are injected, so tests never touch real files.

pub mod atomic;
pub mod env_file;
pub mod model;
pub mod paths;
pub mod secret;
pub mod store;

pub mod commands;

#[cfg(any(test, debug_assertions))]
pub use model::ConfigPatch;
pub use paths::AppPaths;
pub use store::ConfigStore;
