//! IPC types for the kclaude module (`KclaudeStatus` and friends).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Whether the App-managed kclaude block is in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KclaudeState {
    Installed,
    NotInstalled,
    Foreign,
    Outdated,
}

/// Shell family kclaude is installed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellKind {
    Zsh,
    Bash,
    Powershell,
}

/// Result class of the fresh-login-shell check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyStatus {
    Ok,
    Fail,
    Skipped,
}

/// A fix the UI can offer. Only the copyable command is needed by kclaude.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FixAction {
    CopyCommand { label: String, command: String },
}

/// Outcome of running `type kclaude` in a fresh login shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KclaudeVerify {
    pub status: VerifyStatus,
    pub detail: String,
    pub fix: Option<FixAction>,
}

impl KclaudeVerify {
    /// Verification was not run because kclaude is not installed.
    pub fn skipped() -> Self {
        Self {
            status: VerifyStatus::Skipped,
            detail: "还没有安装 kclaude".to_string(),
            fix: None,
        }
    }
}

/// Everything the kclaude section of the UI needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KclaudeStatus {
    pub state: KclaudeState,
    pub shell: ShellKind,
    pub rc_files: Vec<String>,
    pub claude_installed: bool,
    pub verify: KclaudeVerify,
}

/// Stable error codes from the IPC contract that kclaude can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidArgument,
    ForeignPresent,
    BackupFailed,
    WriteFailed,
    Internal,
}

/// Command error: serialises to `{ "code": ..., "message": ... }`.
///
/// Messages never contain paths, URLs or secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KclaudeError {
    pub code: ErrorCode,
    pub message: String,
}

impl KclaudeError {
    pub fn new(code: ErrorCode, message: &str) -> Self {
        Self {
            code,
            message: message.to_string(),
        }
    }
}

impl fmt::Display for KclaudeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for KclaudeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_with_contract_field_names() {
        let status = KclaudeStatus {
            state: KclaudeState::NotInstalled,
            shell: ShellKind::Powershell,
            rc_files: vec!["~/a".to_string()],
            claude_installed: true,
            verify: KclaudeVerify {
                status: VerifyStatus::Fail,
                detail: "x".to_string(),
                fix: Some(FixAction::CopyCommand {
                    label: "复制".to_string(),
                    command: "cmd".to_string(),
                }),
            },
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["state"], "not_installed");
        assert_eq!(value["shell"], "powershell");
        assert_eq!(value["rcFiles"][0], "~/a");
        assert_eq!(value["claudeInstalled"], true);
        assert_eq!(value["verify"]["status"], "fail");
        assert_eq!(value["verify"]["fix"]["kind"], "copy_command");
        assert_eq!(value["verify"]["fix"]["command"], "cmd");
    }

    #[test]
    fn skipped_verify_has_null_fix_key() {
        let value = serde_json::to_value(KclaudeVerify::skipped()).unwrap();
        assert_eq!(value["status"], "skipped");
        assert!(value.get("fix").is_some() && value["fix"].is_null());
    }

    #[test]
    fn error_serializes_to_code_and_message() {
        let err = KclaudeError::new(ErrorCode::BackupFailed, "备份失败");
        let value = serde_json::to_value(&err).unwrap();
        assert_eq!(value["code"], "backup_failed");
        assert_eq!(value["message"], "备份失败");
        assert_eq!(value.as_object().unwrap().len(), 2);
    }

    #[test]
    fn every_error_code_uses_snake_case() {
        for (code, expected) in [
            (ErrorCode::InvalidArgument, "invalid_argument"),
            (ErrorCode::ForeignPresent, "foreign_present"),
            (ErrorCode::WriteFailed, "write_failed"),
            (ErrorCode::Internal, "internal"),
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), expected);
        }
    }
}
