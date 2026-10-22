//! Tauri glue of the tray: icon, menu rebuilds and menu actions. No decisions of its own;
//! what the menu shows comes from [`super::model::build`].

use std::sync::{Arc, Mutex, MutexGuard};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Listener, Manager};

use super::model::{build, TrayAction, TrayInputs, TrayItem, TrayModel};
use crate::app_config::ConfigStore;
use crate::gateway::service::GatewayService;
use crate::gateway::{GatewayState, GatewayStatus};
use crate::update::{UpdateService, UpdateStatus};

/// Id of the single tray icon.
const TRAY_ID: &str = "main";

/// The icon shown in the menu bar / notification area (placeholder art from `icons/`).
const ICON_PNG: &[u8] = include_bytes!("../../icons/32x32.png");

/// Owns the tray icon and the inputs its menu is built from.
pub struct TrayController {
    app: AppHandle,
    config: Arc<ConfigStore>,
    inputs: Mutex<TrayInputs>,
    tray: TrayIcon,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn render(app: &AppHandle, model: &TrayModel) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for item in &model.items {
        match item {
            TrayItem::Head(text) => {
                menu.append(&MenuItem::new(app, text, false, None::<&str>)?)?;
            }
            TrayItem::Separator => {
                menu.append(&PredefinedMenuItem::separator(app)?)?;
            }
            TrayItem::Action { action, label, enabled } => {
                menu.append(&MenuItem::with_id(app, action.id(), label, *enabled, None::<&str>)?)?;
            }
        }
    }
    Ok(menu)
}

impl TrayController {
    /// Create the tray icon and subscribe to the events that change its menu.
    ///
    /// Args:
    ///     app: The app handle.
    ///     config: Configuration store (the persisted `upgradeRequired` flag).
    ///     gateway: Gateway service, for the initial status.
    ///     update: Update service, for the initial state.
    pub fn install(
        app: &AppHandle,
        config: Arc<ConfigStore>,
        gateway: &GatewayService,
        update: &UpdateService,
    ) -> tauri::Result<Arc<Self>> {
        let inputs = TrayInputs {
            gateway: gateway.status(),
            update: update.status().state,
            upgrade_required: config.upgrade_required(),
            today_credits: crate::request_log::commands::today_credits(app),
        };
        let model = build(&inputs);
        let menu = render(app, &model)?;
        let tray = TrayIconBuilder::with_id(TRAY_ID)
            .icon(Image::from_bytes(ICON_PNG)?)
            .tooltip(&model.tooltip)
            .title(&model.title)
            .menu(&menu)
            .on_menu_event(|app, event| {
                if let Some(action) = TrayAction::from_id(event.id().as_ref()) {
                    run_action(app, action);
                }
            })
            .build(app)?;
        let controller = Arc::new(Self { app: app.clone(), config, inputs: Mutex::new(inputs), tray });
        controller.subscribe();
        Ok(controller)
    }

    fn subscribe(self: &Arc<Self>) {
        let me = self.clone();
        self.app.listen(crate::gateway::commands::STATUS_EVENT, move |event| {
            if let Ok(status) = serde_json::from_str::<GatewayStatus>(event.payload()) {
                me.update(|i| i.gateway = status);
            }
        });
        let me = self.clone();
        self.app.listen(crate::update::commands::UPDATE_STATUS_EVENT, move |event| {
            if let Ok(status) = serde_json::from_str::<UpdateStatus>(event.payload()) {
                me.update(|i| i.update = status.state);
            }
        });
        let me = self.clone();
        self.app.listen(crate::request_log::commands::APPENDED_EVENT, move |_| {
            let credits = crate::request_log::commands::today_credits(&me.app);
            me.update(|i| i.today_credits = credits);
        });
    }

    /// Apply a change to the inputs and redraw when the menu would differ.
    fn update(&self, change: impl FnOnce(&mut TrayInputs)) {
        let (model, changed) = {
            let mut inputs = lock(&self.inputs);
            let before = build(&inputs);
            change(&mut inputs);
            inputs.upgrade_required = self.config.upgrade_required();
            let after = build(&inputs);
            let changed = after != before;
            (after, changed)
        };
        if !changed {
            return;
        }
        let tray = self.tray.clone();
        let app = self.app.clone();
        // Menus are built on the main thread; the listeners run on whichever thread emitted.
        let _ = self.app.run_on_main_thread(move || {
            match render(&app, &model) {
                Ok(menu) => {
                    let _ = tray.set_menu(Some(menu));
                }
                Err(err) => eprintln!("[tray] could not rebuild the menu: {err}"),
            }
            let _ = tray.set_tooltip(Some(model.tooltip));
            let _ = tray.set_title(Some(model.title));
        });
    }
}

/// Run a menu action. Anything that can block (starting needs the environment check,
/// the update restart stops the gateway first) runs on its own thread.
fn run_action(app: &AppHandle, action: TrayAction) {
    match action {
        TrayAction::OpenMain => crate::window::show_main(app),
        TrayAction::ToggleGateway => {
            let app = app.clone();
            std::thread::spawn(move || toggle_gateway(&app));
        }
        TrayAction::RestartForUpdate => {
            let app = app.clone();
            std::thread::spawn(move || {
                if let Some(update) = app.try_state::<Arc<UpdateService>>() {
                    if let Err(err) = update.restart_now() {
                        eprintln!("[tray] restart for update failed: {err}");
                    }
                }
            });
        }
        TrayAction::Quit => {
            let app = app.clone();
            // Synchronous: the gateway must be gone before the process is.
            std::thread::spawn(move || {
                crate::gateway::commands::shutdown(&app);
                app.exit(0);
            });
        }
    }
}

fn toggle_gateway(app: &AppHandle) {
    let Some(service) = app.try_state::<Arc<GatewayService>>() else { return };
    let status = service.status();
    let result: Result<(), String> = if status.is_active() {
        service.stop().map(|_| ()).map_err(|e| e.message())
    } else {
        crate::gateway::commands::start_guarded(app, &service).map(|_| ()).map_err(|e| e.message)
    };
    if let Err(message) = result {
        // The tray has nowhere to show a message; the window shows the state and the reason.
        eprintln!("[tray] gateway toggle refused: {message}");
        if matches!(status.state, GatewayState::Stopped | GatewayState::StartFailed | GatewayState::Crashed) {
            crate::window::show_main(app);
        }
    }
}
