//! Request log reader, aggregator and retention for the desktop app.
//!
//! The gateway appends one JSON line per model request to
//! `<dir>/requests-YYYY-MM.jsonl` (see `kiro/request_log.py`). This module
//! reads those files incrementally, keeps the records of the current and the
//! previous month in memory, and turns them into the `RequestLogSnapshot` /
//! `RequestLogAppended` shapes of `docs/ipc-contract.md` section 5.
//!
//! Layout:
//! - `model`: record and payload types, line parsing, month arithmetic
//! - `reader` (tasks 7.1): per-file offsets, partial lines, bad lines, rewinds
//! - `agg` (tasks 7.2): today / month totals, per-model groups, recent list
//! - `retention` (tasks 7.3): delete old month files, clear everything
//! - `store`: ties the three together behind a clock- and timezone-injected API
//! - `commands`: `#[tauri::command]` functions and the
//!   1 second polling thread; no logic of its own
//!
//! Everything except `commands` is plain Rust with no Tauri dependency, so it
//! is tested with an injected directory, clock and timezone.

pub mod agg;
pub mod model;
pub mod reader;
pub mod retention;
pub mod store;

pub mod commands;
