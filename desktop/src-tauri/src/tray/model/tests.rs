use super::*;
use crate::update::types::DownloadProgress;

fn gw(state: GatewayState) -> GatewayStatus {
    GatewayStatus { state, port: 8000, pid: None, since: None, failure: None }
}

fn inputs(state: GatewayState) -> TrayInputs {
    TrayInputs { gateway: gw(state), update: UpdateState::None, upgrade_required: false, today_credits: Some(1.84) }
}

fn heads(model: &TrayModel) -> Vec<String> {
    model
        .items
        .iter()
        .filter_map(|i| match i {
            TrayItem::Head(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

fn action(model: &TrayModel, which: TrayAction) -> Option<(String, bool)> {
    model.items.iter().find_map(|i| match i {
        TrayItem::Action { action, label, enabled } if *action == which => Some((label.clone(), *enabled)),
        _ => None,
    })
}

#[test]
fn running_menu_matches_the_mockup() {
    let model = build(&inputs(GatewayState::Running));
    assert_eq!(heads(&model), vec!["KiroGateway 运行中", "今日 1.84 credits"]);
    assert_eq!(action(&model, TrayAction::ToggleGateway), Some(("停止网关".into(), true)));
    assert_eq!(action(&model, TrayAction::OpenMain), Some(("打开主窗口".into(), true)));
    assert_eq!(action(&model, TrayAction::Quit), Some(("退出".into(), true)));
    assert_eq!(action(&model, TrayAction::RestartForUpdate), None);
    assert_eq!(model.tooltip, "Kiro Gateway：运行中");
    assert_eq!(model.title, "");
}

#[test]
fn every_gateway_state_has_its_word_and_the_right_toggle() {
    let cases = [
        (GatewayState::Stopped, "已停止", "启动网关"),
        (GatewayState::Starting, "启动中", "停止网关"),
        (GatewayState::Running, "运行中", "停止网关"),
        (GatewayState::StartFailed, "没能启动", "启动网关"),
        (GatewayState::Crashed, "意外退出", "启动网关"),
    ];
    for (state, word, toggle) in cases {
        let model = build(&inputs(state));
        assert_eq!(heads(&model)[0], format!("KiroGateway {word}"), "{state:?}");
        assert_eq!(action(&model, TrayAction::ToggleGateway), Some((toggle.into(), true)), "{state:?}");
    }
}

#[test]
fn failed_states_mark_the_icon_title() {
    assert_eq!(build(&inputs(GatewayState::StartFailed)).title, "!");
    assert_eq!(build(&inputs(GatewayState::Crashed)).title, "!");
    for ok in [GatewayState::Stopped, GatewayState::Starting, GatewayState::Running] {
        assert_eq!(build(&inputs(ok)).title, "", "{ok:?}");
    }
}

#[test]
fn forced_upgrade_replaces_the_head_and_disables_start() {
    let mut i = inputs(GatewayState::Stopped);
    i.update = UpdateState::Forced {
        version: "0.0.3".into(),
        progress: Some(DownloadProgress::start()),
        error: None,
        manual_url: "https://example.invalid/x.dmg".into(),
    };
    let model = build(&i);
    assert_eq!(heads(&model)[0], "需要升级到 0.0.3");
    assert_eq!(action(&model, TrayAction::ToggleGateway), Some(("启动网关（需要先升级）".into(), false)));
    assert_eq!(model.tooltip, "需要升级到 0.0.3");
    // The window and quit stay available.
    assert_eq!(action(&model, TrayAction::OpenMain), Some(("打开主窗口".into(), true)));
    assert_eq!(action(&model, TrayAction::Quit), Some(("退出".into(), true)));
}

#[test]
fn the_persisted_flag_alone_disables_start_before_the_first_check() {
    let mut i = inputs(GatewayState::Stopped);
    i.upgrade_required = true;
    let model = build(&i);
    assert_eq!(heads(&model)[0], "需要先升级");
    assert_eq!(action(&model, TrayAction::ToggleGateway), Some(("启动网关（需要先升级）".into(), false)));
}

#[test]
fn a_running_gateway_can_still_be_stopped_during_a_forced_upgrade() {
    let mut i = inputs(GatewayState::Running);
    i.upgrade_required = true;
    assert_eq!(action(&build(&i), TrayAction::ToggleGateway), Some(("停止网关".into(), true)));
}

#[test]
fn a_ready_update_adds_the_restart_item_between_open_and_quit() {
    let mut i = inputs(GatewayState::Running);
    i.update = UpdateState::Ready { version: "0.0.2".into() };
    let model = build(&i);
    assert_eq!(action(&model, TrayAction::RestartForUpdate), Some(("重启以完成更新".into(), true)));
    let order: Vec<TrayAction> = model
        .items
        .iter()
        .filter_map(|i| match i {
            TrayItem::Action { action, .. } => Some(*action),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        vec![TrayAction::ToggleGateway, TrayAction::OpenMain, TrayAction::RestartForUpdate, TrayAction::Quit]
    );
}

#[test]
fn other_update_states_do_not_add_the_restart_item() {
    for update in [
        UpdateState::None,
        UpdateState::Downloading { version: "0.0.2".into(), progress: DownloadProgress::start() },
        UpdateState::InstallFailed { version: "0.0.2".into(), error: "e".into(), manual_url: "u".into() },
    ] {
        let mut i = inputs(GatewayState::Running);
        i.update = update.clone();
        assert_eq!(action(&build(&i), TrayAction::RestartForUpdate), None, "{update:?}");
    }
}

#[test]
fn a_ready_update_never_shows_restart_while_an_upgrade_is_forced() {
    let mut i = inputs(GatewayState::Stopped);
    i.update = UpdateState::Ready { version: "0.0.2".into() };
    i.upgrade_required = true;
    assert_eq!(action(&build(&i), TrayAction::RestartForUpdate), None);
}

#[test]
fn the_credits_line_is_left_out_until_the_log_is_loaded() {
    let mut i = inputs(GatewayState::Running);
    i.today_credits = None;
    assert_eq!(heads(&build(&i)), vec!["KiroGateway 运行中"]);
}

#[test]
fn credits_formatting_follows_the_summary_cards() {
    assert_eq!(format_credits(0.0), "0");
    assert_eq!(format_credits(-1.0), "0");
    assert_eq!(format_credits(f64::NAN), "0");
    assert_eq!(format_credits(f64::INFINITY), "0");
    assert_eq!(format_credits(0.001), "<0.01");
    assert_eq!(format_credits(0.005), "0.01");
    assert_eq!(format_credits(1.84), "1.84");
    assert_eq!(format_credits(1234.5678), "1234.57");
}

#[test]
fn action_ids_round_trip_and_unknown_ids_are_rejected() {
    for a in [TrayAction::ToggleGateway, TrayAction::OpenMain, TrayAction::RestartForUpdate, TrayAction::Quit] {
        assert_eq!(TrayAction::from_id(a.id()), Some(a));
    }
    assert_eq!(TrayAction::from_id(""), None);
    assert_eq!(TrayAction::from_id("quit"), None);
}

#[test]
fn the_menu_always_ends_with_a_separator_and_quit() {
    for state in [GatewayState::Stopped, GatewayState::Running, GatewayState::Crashed] {
        let model = build(&inputs(state));
        let n = model.items.len();
        assert_eq!(model.items[n - 2], TrayItem::Separator);
        assert!(matches!(model.items[n - 1], TrayItem::Action { action: TrayAction::Quit, .. }));
    }
}
