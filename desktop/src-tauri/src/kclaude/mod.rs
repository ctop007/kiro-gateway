//! kclaude command management: script templates, shell startup file blocks, PowerShell
//! profiles and the post-install check. See `docs/ipc-contract.md` section 4.
//!
//! Everything except `commands` is plain Rust with injected directories, clocks and
//! processes so it can be tested without a running Tauri app. The Tauri glue lives in
//! `commands`.

pub mod fsutil;
pub mod ps_profile;
pub mod rc;
pub mod script;
pub mod service;
pub mod types;
pub mod verify;

pub mod commands;

#[cfg(test)]
mod script_tests;
