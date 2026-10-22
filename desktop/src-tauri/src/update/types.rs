//! IPC types of the update module (see `desktop/docs/ipc-contract.md` section 6).
//!
//! JSON rules: field names are camelCase, enum values are snake_case strings,
//! optional fields are `null` (never omitted).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Download progress of an update package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    /// Bytes received so far.
    pub downloaded_bytes: u64,
    /// Total size, `None` when the server did not send a length.
    pub total_bytes: Option<u64>,
}

impl DownloadProgress {
    /// Progress at the very beginning of a download.
    pub fn start() -> Self {
        Self { downloaded_bytes: 0, total_bytes: None }
    }
}

/// State of the update machinery as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum UpdateState {
    /// Up to date, or the check failed (failures are never surfaced).
    None,
    /// A normal update is being downloaded silently.
    Downloading { version: String, progress: DownloadProgress },
    /// Downloaded and verified; installed on the next start or on "restart now".
    Ready { version: String },
    /// Start-up is installing the previously downloaded update; the window shows
    /// "正在完成更新" and the gateway stays off until this ends (the app restarts on success).
    Installing { version: String },
    /// The running version is below the manifest's `minRequiredVersion`.
    Forced {
        version: String,
        progress: Option<DownloadProgress>,
        error: Option<String>,
        manual_url: String,
    },
    /// Installing the pending update failed; the app runs the current version.
    InstallFailed { version: String, error: String, manual_url: String },
}

/// `UpdateState` plus the always-present bookkeeping fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    #[serde(flatten)]
    pub state: UpdateState,
    /// For example "0.0.1".
    pub current_version: String,
    /// Time of the last successful manifest check, ISO 8601 with offset.
    pub last_checked_at: Option<String>,
}

/// Stable error codes of the update commands (subset of the contract table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateErrorCode {
    InvalidArgument,
    UpgradeRequired,
    NoPendingUpdate,
    Internal,
}

/// Command error: `{ "code": "...", "message": "..." }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateError {
    pub code: UpdateErrorCode,
    /// One displayable Chinese sentence without URLs, stack traces or secrets.
    pub message: String,
}

impl UpdateError {
    /// Builds an error from a code and a displayable message.
    pub fn new(code: UpdateErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for UpdateError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(state: UpdateState) -> serde_json::Value {
        serde_json::to_value(UpdateStatus {
            state,
            current_version: "0.0.1".into(),
            last_checked_at: None,
        })
        .unwrap()
    }

    #[test]
    fn none_state_serializes_flat_with_null_last_checked() {
        assert_eq!(
            status(UpdateState::None),
            json!({ "state": "none", "currentVersion": "0.0.1", "lastCheckedAt": null })
        );
    }

    #[test]
    fn downloading_state_matches_contract_shape() {
        let v = status(UpdateState::Downloading {
            version: "0.0.2".into(),
            progress: DownloadProgress { downloaded_bytes: 10, total_bytes: None },
        });
        assert_eq!(v["state"], "downloading");
        assert_eq!(v["version"], "0.0.2");
        assert_eq!(v["progress"], json!({ "downloadedBytes": 10, "totalBytes": null }));
    }

    #[test]
    fn forced_state_keeps_null_progress_and_error_keys() {
        let v = status(UpdateState::Forced {
            version: "0.0.3".into(),
            progress: None,
            error: None,
            manual_url: "https://x/y.dmg".into(),
        });
        assert_eq!(v["state"], "forced");
        assert!(v.as_object().unwrap().contains_key("progress"));
        assert!(v["progress"].is_null());
        assert!(v["error"].is_null());
        assert_eq!(v["manualUrl"], "https://x/y.dmg");
    }

    #[test]
    fn install_failed_and_ready_use_snake_case_state_names() {
        assert_eq!(status(UpdateState::Ready { version: "0.0.2".into() })["state"], "ready");
        let installing = status(UpdateState::Installing { version: "0.0.2".into() });
        assert_eq!(installing["state"], "installing");
        assert_eq!(installing["version"], "0.0.2");
        let v = status(UpdateState::InstallFailed {
            version: "0.0.2".into(),
            error: "更新安装失败".into(),
            manual_url: "https://x".into(),
        });
        assert_eq!(v["state"], "install_failed");
        assert_eq!(v["manualUrl"], "https://x");
    }

    #[test]
    fn status_round_trips_through_json() {
        let original = UpdateStatus {
            state: UpdateState::Forced {
                version: "0.0.3".into(),
                progress: Some(DownloadProgress { downloaded_bytes: 5, total_bytes: Some(9) }),
                error: Some("下载失败".into()),
                manual_url: "https://x".into(),
            },
            current_version: "0.0.1".into(),
            last_checked_at: Some("2026-10-22T14:41:08+08:00".into()),
        };
        let text = serde_json::to_string(&original).unwrap();
        assert_eq!(serde_json::from_str::<UpdateStatus>(&text).unwrap(), original);
    }

    #[test]
    fn error_serializes_as_code_and_message() {
        let e = UpdateError::new(UpdateErrorCode::NoPendingUpdate, "没有已下载的更新");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({ "code": "no_pending_update", "message": "没有已下载的更新" })
        );
        assert_eq!(
            serde_json::to_value(UpdateErrorCode::UpgradeRequired).unwrap(),
            json!("upgrade_required")
        );
    }
}
