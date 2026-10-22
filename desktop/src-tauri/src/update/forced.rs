//! Forced-upgrade lock (tasks 8.4).
//!
//! While an upgrade is mandatory, starting the gateway is refused: by the
//! `gateway_start` command and the tray item (both call [`guard_gateway_start`]),
//! and for `kclaude` through `upgradeRequired: true` in `config.json`
//! (written through [`super::ports::UpgradeFlag`]).

use super::ports::UpgradeFlag;
use super::types::{UpdateError, UpdateErrorCode, UpdateState};

/// Displayable refusal text.
pub const UPGRADE_REQUIRED_MESSAGE: &str = "需要先升级到新版本才能使用";

/// Refuses gateway start while a forced upgrade is in progress.
///
/// The persisted flag is consulted as well as the in-memory state, so the lock
/// also holds right after launch, before the first manifest check finished.
///
/// Args:
///     state: Current update state.
///     flag: Persisted `upgradeRequired` flag.
///
/// Returns:
///     `Err` with code `upgrade_required` when locked.
pub fn guard_gateway_start(state: &UpdateState, flag: &dyn UpgradeFlag) -> Result<(), UpdateError> {
    if matches!(state, UpdateState::Forced { .. }) || flag.is_set() {
        Err(UpdateError::new(UpdateErrorCode::UpgradeRequired, UPGRADE_REQUIRED_MESSAGE))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::types::DownloadProgress;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Flag(AtomicBool);
    impl UpgradeFlag for Flag {
        fn is_set(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
        fn set(&self, value: bool) -> Result<(), String> {
            self.0.store(value, Ordering::SeqCst);
            Ok(())
        }
    }

    fn forced() -> UpdateState {
        UpdateState::Forced {
            version: "0.0.3".into(),
            progress: None,
            error: Some("下载失败".into()),
            manual_url: "https://x".into(),
        }
    }

    #[test]
    fn forced_state_blocks_start_even_with_flag_unset() {
        let err = guard_gateway_start(&forced(), &Flag(AtomicBool::new(false))).unwrap_err();
        assert_eq!(err.code, UpdateErrorCode::UpgradeRequired);
        assert!(!err.message.contains("http"));
    }

    #[test]
    fn persisted_flag_blocks_start_before_first_check() {
        let err = guard_gateway_start(&UpdateState::None, &Flag(AtomicBool::new(true))).unwrap_err();
        assert_eq!(err.code, UpdateErrorCode::UpgradeRequired);
    }

    #[test]
    fn every_non_forced_state_with_flag_unset_allows_start() {
        let flag = Flag(AtomicBool::new(false));
        let states = [
            UpdateState::None,
            UpdateState::Downloading { version: "0.0.2".into(), progress: DownloadProgress::start() },
            UpdateState::Ready { version: "0.0.2".into() },
            UpdateState::InstallFailed { version: "0.0.2".into(), error: "e".into(), manual_url: "u".into() },
        ];
        for s in states {
            assert!(guard_gateway_start(&s, &flag).is_ok(), "{s:?}");
        }
    }
}
