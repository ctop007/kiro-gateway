//! In-app update: manifest decision, pending-package store, scheduler and the
//! forced-upgrade lock.
//!
//! Everything except `commands` is plain Rust with an injected clock, manifest
//! source / downloader / installer, gateway control and upgrade flag, so it can
//! be tested without a Tauri runtime. `commands` holds the thin
//! `tauri-plugin-updater` glue.

pub mod decide;
pub mod forced;
pub mod pending;
pub mod ports;
pub mod schedule;
pub mod store;
#[cfg(test)]
pub(crate) mod testing;
pub mod types;

pub mod commands;

pub use schedule::UpdateService;
pub use types::{UpdateState, UpdateStatus};
