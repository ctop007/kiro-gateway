//! Main window behaviour. Tray logic lands in a later batch; for now closing the
//! main window only hides it so the gateway (once wired) keeps running.

use tauri::{Runtime, Window, WindowEvent};

/// Label of the single main window (must match `app.windows[0].label` in tauri.conf.json).
pub const MAIN_WINDOW_LABEL: &str = "main";

/// What to do when the user closes a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Prevent the close and hide the window instead.
    Hide,
    /// Let the window close normally.
    Close,
}

/// Decide the close behaviour for a window label. Pure, so it is unit-tested.
pub fn close_action(label: &str) -> CloseAction {
    if label == MAIN_WINDOW_LABEL {
        CloseAction::Hide
    } else {
        CloseAction::Close
    }
}

/// `Builder::on_window_event` hook: hide the main window instead of closing it.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if close_action(window.label()) == CloseAction::Hide {
            api.prevent_close();
            if let Err(err) = window.hide() {
                eprintln!("failed to hide main window: {err}");
            }
        }
    }
}

/// Bring the main window to the front (used by single-instance and, later, the tray).
pub fn show_main<R: Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let Some(w) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_main_window_hides_it() {
        assert_eq!(close_action(MAIN_WINDOW_LABEL), CloseAction::Hide);
    }

    #[test]
    fn closing_other_windows_closes_them() {
        assert_eq!(close_action("about"), CloseAction::Close);
        assert_eq!(close_action(""), CloseAction::Close);
    }

    #[test]
    fn label_match_is_exact() {
        assert_eq!(close_action("Main"), CloseAction::Close);
        assert_eq!(close_action("main "), CloseAction::Close);
    }
}
