#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_config;
mod env_check;
mod gateway;
mod kclaude;
mod request_log;
#[cfg(debug_assertions)]
mod selftest;
#[cfg(unix)]
mod signals;
#[cfg(target_os = "macos")]
mod macos_state;
mod tray;
mod update;
mod window;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod scaffold_tests;

use std::sync::Arc;

use tauri::{Manager, RunEvent};
use tauri_plugin_autostart::MacosLauncher;

/// Extra argument the OS passes when it launches the app at login.
const AUTOSTART_ARG: &str = "--autostarted";

/// Debug builds only: run the headless self-test instead of the app (see `selftest`).
#[cfg(debug_assertions)]
const SELFTEST_ARG: &str = "--selftest";

/// Start-up work that can block for seconds (login shell PATH, sqlite, `type kclaude`).
/// It runs on its own thread so the window and the tray are usable meanwhile.
///
/// Order: update scheduler (its first tick is the first manifest check, so it runs alongside
/// everything else) → environment check (fills the login-shell PATH cache) → kclaude install /
/// repair → restore the gateway switch, once the start-up install and the first manifest check
/// (at most `FIRST_CHECK_WAIT`) have ended and did not demand an upgrade → request-log poller.
fn finish_startup(app: tauri::AppHandle, update: Arc<update::UpdateService>) {
    if let Err(err) = update::commands::spawn_scheduler(update.clone()) {
        eprintln!("[startup] update scheduler: {err}");
    }
    let report = env_check::report::run_env_check();
    kclaude::commands::refresh_search_dirs(&app);
    match kclaude::commands::auto_install(&app) {
        Ok(status) => eprintln!("[startup] kclaude: {:?}", status.state),
        Err(err) => eprintln!("[startup] kclaude auto-install failed: {err}"),
    }
    if let Some(gateway) = app.try_state::<Arc<gateway::service::GatewayService>>() {
        let blocked = matches!(
            env_check::report::gateway_gate(&report),
            env_check::report::GatewayGate::Blocked { .. }
        );
        if !blocked || !gateway.config().get().gateway_wanted {
            let verdict = update.resume_gateway_after_startup(update::schedule::FIRST_CHECK_WAIT);
            eprintln!("[startup] first update check: {verdict:?}");
        }
    }
    request_log::commands::spawn_poller(&app);
}

fn main() {
    #[cfg(debug_assertions)]
    if std::env::args().any(|a| a == SELFTEST_ARG) {
        std::process::exit(selftest::run());
    }

    // Before any other thread exists: they inherit the blocked signal mask.
    #[cfg(unix)]
    signals::install();

    // Before NSApplication launches: no "reopen windows?" alert after a signal-terminated run.
    #[cfg(target_os = "macos")]
    macos_state::disable_window_restoration();

    let autostarted = std::env::args().any(|a| a == AUTOSTART_ARG);

    tauri::Builder::default()
        // Must be the first plugin: a second launch only brings the existing window forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Autostart: LaunchAgent on macOS, registry Run key on Windows; off by default.
        // Rust owns it (config_update), the frontend has no autostart permission.
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        // Both only used from Rust (update module); the frontend gets no permission for them.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(kclaude::commands::ClaudeSearchDirs::default())
        .on_window_event(window::on_window_event)
        .setup(move |app| {
            let handle = app.handle().clone();
            #[cfg(unix)]
            signals::set_app(&handle);

            eprintln!("[startup] Kiro Gateway {}", handle.package_info().version);

            // 1. Configuration and access key.
            let (config, notes) = app_config::commands::setup(&handle).map_err(|e| e.message)?;
            for note in notes {
                eprintln!("[startup] config: {note:?}");
            }
            // The log state is needed by the tray and the commands; its poller starts later.
            request_log::commands::init(&handle)?;

            // 2. Gateway service, then the update service: installing a pending update
            //    happens here, before anything else is started (it restarts the app).
            let gateway = gateway::commands::setup(&handle, config.clone()).map_err(|e| e.message)?;
            let update = update::commands::setup(
                &handle,
                Arc::new(gateway::commands::UpdateGatewayControl(gateway.clone())),
                Arc::new(gateway::commands::UpdateUpgradeFlag(config.clone())),
            )
            .map_err(|e| e.message)?;
            // Every start path, the automatic resume included, asks the update service too.
            let guard_source = update.clone();
            gateway.set_start_guard(Arc::new(move || guard_source.guard_gateway_start().is_err()));

            // 3. Tray, then the slow start-up work in the background.
            tray::controller::TrayController::install(&handle, config.clone(), &gateway, &update).map(|tray| {
                handle.manage(tray);
            })?;
            if autostarted {
                if let Some(main) = app.get_webview_window(window::MAIN_WINDOW_LABEL) {
                    let _ = main.hide();
                }
            }
            let worker = handle.clone();
            std::thread::Builder::new()
                .name("startup".into())
                .spawn(move || finish_startup(worker, update))?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_config::commands::config_get,
            app_config::commands::config_update,
            gateway::commands::gateway_status,
            gateway::commands::gateway_start,
            gateway::commands::gateway_stop,
            gateway::commands::gateway_models,
            gateway::commands::gateway_reveal_log,
            env_check::commands::env_check,
            kclaude::commands::kclaude_status,
            kclaude::commands::kclaude_install,
            kclaude::commands::kclaude_adopt,
            kclaude::commands::kclaude_remove,
            request_log::commands::request_log_snapshot,
            request_log::commands::request_log_clear,
            update::commands::update_status,
            update::commands::update_check_now,
            update::commands::update_restart_now,
            update::commands::update_retry,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            // The gateway must be gone before the process is, whichever way the app ends.
            RunEvent::Exit => gateway::commands::shutdown(app),
            // macOS: after hiding the window, clicking the Dock icon only fires Reopen.
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => window::show_main(app),
            _ => {}
        });
}
