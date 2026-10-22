//! Wire types of the gateway section of `docs/ipc-contract.md`.

use serde::{Deserialize, Serialize};

/// `GatewayState`: what the switch and the tray show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayState {
    Stopped,
    Starting,
    Running,
    StartFailed,
    Crashed,
}

/// `FailureKind`: why the gateway did not start or went away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    PortInUse,
    LoginExpired,
    Timeout,
    Other,
}

/// `PortOccupant`: the program that holds the port. Never killed, only reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortOccupant {
    /// Process name, for example `python3.13`; `None` when it could not be found out.
    pub name: Option<String>,
    pub pid: Option<u32>,
}

/// `GatewayFailure`, present in `start_failed` and `crashed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayFailure {
    pub kind: FailureKind,
    /// One Chinese sentence, safe to show: no URL, stack trace, token or key.
    pub message: String,
    /// Only for `port_in_use`.
    pub occupant: Option<PortOccupant>,
    /// Last (at most 20) log lines of this run, already redacted.
    pub log_excerpt: Vec<String>,
    /// When the failure or exit happened (local time, ISO 8601 with offset).
    pub at: String,
}

/// `GatewayStatus`: payload of `gateway_status` and of the `gateway-status` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub state: GatewayState,
    /// Port in use, or the configured port while stopped.
    pub port: u16,
    /// Set while `starting` / `running`.
    pub pid: Option<u32>,
    /// When the current state began (local time, ISO 8601 with offset).
    pub since: Option<String>,
    /// Set in `start_failed` / `crashed` only.
    pub failure: Option<GatewayFailure>,
}

impl GatewayStatus {
    /// A `stopped` status on `port`, with no history.
    pub fn stopped(port: u16) -> Self {
        Self { state: GatewayState::Stopped, port, pid: None, since: None, failure: None }
    }

    /// Whether a sidecar process is (or is about to be) alive.
    pub fn is_active(&self) -> bool {
        matches!(self.state, GatewayState::Starting | GatewayState::Running)
    }
}
