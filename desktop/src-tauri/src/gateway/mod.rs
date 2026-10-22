//! Gateway process control for the desktop app (tasks 4.2 to 4.5 and 5.4,
//! `docs/ipc-contract.md` section 2).
//!
//! Starts the PyInstaller sidecar, watches it, and reports a truthful status.
//!
//! Layout:
//! - `types`: `GatewayStatus` and friends, the JSON shapes of the contract
//! - `env`: environment assembly for the sidecar (design decision 2)
//! - `sidecar`: where the sidecar executable lives (`KIRO_GATEWAY_SIDECAR` override)
//! - `logfile`: `gateway.log` writer with 5 MB x 3 rotation
//! - `redact`: ANSI stripping and token / secret masking of log lines (tasks 4.5)
//! - `classify`: failure kind from the sidecar log (tasks 5.4)
//! - `port`: bind probe, listener lookup, `gateway.pid` and stale-process reclaim (tasks 4.4)
//! - `sys`: the few OS calls (signals, process image path, Job Object)
//! - `health`: `/health` probe and the `/v1/models` request made on behalf of the frontend
//! - `manager`: the state machine (tasks 4.3)
//! - `service`: what `gateway_start` does around the manager: upgrade gate, environment gate,
//!   sidecar lookup, environment assembly, `gatewayWanted`
//! - `commands`: `#[tauri::command]` functions and event wiring
//!
//! Everything except `commands` is plain Rust with no Tauri dependency. Directories,
//! clock, health probe, process inspector and the status callback are injected, so the
//! tests drive the real code with fake sidecars (shell scripts) in temporary
//! directories and never touch the real App data directory or port 8000.

pub mod classify;
pub mod env;
pub mod health;
pub mod logfile;
pub mod manager;
pub mod port;
pub mod real;
pub mod redact;
pub mod service;
pub mod sidecar;
pub mod sys;
pub mod types;

#[cfg(test)]
mod testing;

pub mod commands;

pub use types::{GatewayState, GatewayStatus};
