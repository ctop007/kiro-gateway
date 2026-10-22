#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod window;
#[cfg(test)]
mod scaffold_tests;

use tauri_plugin_autostart::MacosLauncher;

/// Extra argument the OS passes when it launches the app at login.
const AUTOSTART_ARG: &str = "--autostarted";

fn main() {
    tauri::Builder::default()
        // Must be the first plugin: a second launch only brings the existing window forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Autostart: LaunchAgent on macOS, registry Run key on Windows; off by default.
        // Rust owns it (config_update), the frontend has no autostart permission.
        // AUTOSTART_ARG lets a later batch tell an autostart launch from a manual one.
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        .on_window_event(window::on_window_event)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            // macOS: after hiding the window, clicking the Dock icon only fires Reopen.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                window::show_main(_app);
            }
        });
}
