//! What the tray menu shows, as plain data (tasks 4.6, 8.6).
//!
//! The mockup's tray menu is the reference:
//!
//! ```text
//! KiroGateway 运行中
//! 今日 1.84 credits
//! ----------------
//! 停止网关
//! 打开主窗口
//! 重启以完成更新      (only when an update is ready)
//! ----------------
//! 退出
//! ```
//!
//! During a forced upgrade the first line is 「需要升级到 x.y.z」 and the start item is
//! disabled. Everything here is a pure function of [`TrayInputs`], so it is unit-tested
//! without a Tauri runtime.

use crate::gateway::{GatewayState, GatewayStatus};
use crate::update::UpdateState;

/// What the user can trigger from the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayAction {
    /// Start the gateway when it is off, stop it when it is on.
    ToggleGateway,
    OpenMain,
    /// Install the downloaded update and restart.
    RestartForUpdate,
    Quit,
}

impl TrayAction {
    /// Stable menu item id.
    pub fn id(self) -> &'static str {
        match self {
            TrayAction::ToggleGateway => "tray-toggle-gateway",
            TrayAction::OpenMain => "tray-open-main",
            TrayAction::RestartForUpdate => "tray-restart-for-update",
            TrayAction::Quit => "tray-quit",
        }
    }

    /// Inverse of [`TrayAction::id`].
    pub fn from_id(id: &str) -> Option<TrayAction> {
        [TrayAction::ToggleGateway, TrayAction::OpenMain, TrayAction::RestartForUpdate, TrayAction::Quit]
            .into_iter()
            .find(|a| a.id() == id)
    }
}

/// One line of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayItem {
    /// Disabled text line.
    Head(String),
    Separator,
    Action { action: TrayAction, label: String, enabled: bool },
}

/// Everything the menu depends on.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayInputs {
    pub gateway: GatewayStatus,
    /// Update state as shown in the app.
    pub update: UpdateState,
    /// The persisted forced-upgrade flag (also set before the first manifest check).
    pub upgrade_required: bool,
    /// Credits used today; `None` until the request log is loaded.
    pub today_credits: Option<f64>,
}

/// The rendered menu.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayModel {
    pub tooltip: String,
    /// Text next to the icon on macOS: empty when all is well, 「!」 when the gateway failed.
    pub title: String,
    pub items: Vec<TrayItem>,
}

/// Two decimals, never shows a non-zero amount as 0 (same rule as the summary cards).
pub fn format_credits(value: f64) -> String {
    if !value.is_finite() || value <= 0.0 {
        return "0".to_string();
    }
    if value < 0.005 {
        return "<0.01".to_string();
    }
    format!("{value:.2}")
}

/// Short state word used in the menu head and the tooltip.
pub fn state_word(state: GatewayState) -> &'static str {
    match state {
        GatewayState::Running => "运行中",
        GatewayState::Starting => "启动中",
        GatewayState::Stopped => "已停止",
        GatewayState::StartFailed => "没能启动",
        GatewayState::Crashed => "意外退出",
    }
}

/// Build the menu for the given inputs.
pub fn build(inputs: &TrayInputs) -> TrayModel {
    let forced_version = match &inputs.update {
        UpdateState::Forced { version, .. } => Some(version.clone()),
        _ => None,
    };
    let forced = forced_version.is_some() || inputs.upgrade_required;
    let ready = matches!(inputs.update, UpdateState::Ready { .. }) && !forced;

    let head = match &forced_version {
        Some(version) => format!("需要升级到 {version}"),
        None if forced => "需要先升级".to_string(),
        None => format!("KiroGateway {}", state_word(inputs.gateway.state)),
    };

    let mut items = vec![TrayItem::Head(head.clone())];
    if let Some(credits) = inputs.today_credits {
        items.push(TrayItem::Head(format!("今日 {} credits", format_credits(credits))));
    }
    items.push(TrayItem::Separator);

    let active = inputs.gateway.is_active();
    let (toggle_label, toggle_enabled) = if active {
        ("停止网关".to_string(), true)
    } else if forced {
        ("启动网关（需要先升级）".to_string(), false)
    } else {
        ("启动网关".to_string(), true)
    };
    items.push(TrayItem::Action { action: TrayAction::ToggleGateway, label: toggle_label, enabled: toggle_enabled });
    items.push(TrayItem::Action { action: TrayAction::OpenMain, label: "打开主窗口".to_string(), enabled: true });
    if ready {
        items.push(TrayItem::Action {
            action: TrayAction::RestartForUpdate,
            label: "重启以完成更新".to_string(),
            enabled: true,
        });
    }
    items.push(TrayItem::Separator);
    items.push(TrayItem::Action { action: TrayAction::Quit, label: "退出".to_string(), enabled: true });

    let failed = matches!(inputs.gateway.state, GatewayState::StartFailed | GatewayState::Crashed);
    TrayModel {
        tooltip: if forced { head } else { format!("Kiro Gateway：{}", state_word(inputs.gateway.state)) },
        title: if failed { "!".to_string() } else { String::new() },
        items,
    }
}

#[cfg(test)]
mod tests;
