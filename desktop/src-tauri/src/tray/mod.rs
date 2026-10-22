//! Status tray icon (tasks 4.6, 8.6).
//!
//! - [`model`]: the menu as plain data, a pure function of the gateway status, the update
//!   state, the forced-upgrade flag and today's credits.
//! - [`controller`]: the Tauri glue. It builds the icon, rebuilds the menu when one of the
//!   inputs changes (it listens to the `gateway-status`, `update-status` and
//!   `request-log-appended` events, so it never calls back into the gateway manager while
//!   that holds its lock) and runs the menu actions.

pub mod controller;
pub mod model;
