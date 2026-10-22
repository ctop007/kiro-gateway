//! Clean shutdown on SIGTERM / SIGINT / SIGHUP (macOS, tasks 4.6).
//!
//! `kill <pid>`, a terminal Ctrl+C or a logout can end the app without going through the
//! tray's quit item. Without a handler the process would die at once and leave the gateway
//! running as an orphan (the next launch would reclaim it, but it would hold the port until
//! then). The signals are blocked in every thread and received by one dedicated thread, which
//! stops the gateway and exits normally.
//!
//! [`install`] must run before the first thread is created: threads inherit the signal mask.

use std::sync::OnceLock;

use tauri::AppHandle;

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Exit status used when a signal arrives before the app exists (128 + SIGTERM).
const EXIT_BEFORE_READY: i32 = 143;

fn handled_signals() -> libc::sigset_t {
    // SAFETY: `sigemptyset` / `sigaddset` only write into the zero-initialised set we own.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, signal);
        }
        set
    }
}

/// Block the signals in the calling thread (and every thread created from it afterwards)
/// and start the thread that waits for them.
pub fn install() {
    let set = handled_signals();
    // SAFETY: `set` is a valid, initialised signal set; the old mask is not needed.
    let blocked = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) } == 0;
    if !blocked {
        eprintln!("[signals] could not block the termination signals; the gateway may outlive a kill");
        return;
    }
    let spawned = std::thread::Builder::new().name("signals".into()).spawn(move || {
        let set = handled_signals();
        let mut received: libc::c_int = 0;
        // SAFETY: `set` is valid and `received` is a live out-parameter.
        let status = unsafe { libc::sigwait(&set, &mut received) };
        if status != 0 {
            return;
        }
        eprintln!("[signals] signal {received}: stopping the gateway and exiting");
        match APP.get() {
            Some(app) => {
                crate::gateway::commands::shutdown(app);
                app.exit(0);
            }
            None => std::process::exit(EXIT_BEFORE_READY),
        }
    });
    if spawned.is_err() {
        eprintln!("[signals] could not start the signal thread");
    }
}

/// Hand the running app to the signal thread. Until this is called, a signal ends the
/// process immediately (nothing has been started yet).
pub fn set_app(app: &AppHandle) {
    let _ = APP.set(app.clone());
}
