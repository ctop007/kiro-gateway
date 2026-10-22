//! Tauri glue for the kclaude commands (`kclaude_status`, `kclaude_install`, `kclaude_adopt`,
//! `kclaude_remove`) and the startup auto-install. All logic lives in [`super::service`].
//!
//! Errors reach the frontend as `{ code, message }` (see `docs/ipc-contract.md`).

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use super::service::{KclaudeEnv, KclaudeService, Platform, SystemBackend};
use super::types::{ErrorCode, KclaudeError, KclaudeStatus};

/// Directories searched for `claude`, set by the environment-check module once it has the
/// login-shell PATH. Until then the process PATH and well-known locations are used.
#[derive(Default)]
pub struct ClaudeSearchDirs(pub Mutex<Vec<PathBuf>>);

fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn default_claude_dirs(home: &std::path::Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend([
        home.join(".local/bin"),
        home.join(".claude/local"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            dirs.push(local.join("Programs").join("claude"));
        }
        if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
            dirs.push(appdata.join("npm"));
        }
        dirs.push(home.join(".local").join("bin"));
    }
    dirs
}

/// Point the `claude` lookup at the login-shell PATH (probed once and cached by the
/// environment check) plus the usual install locations. Call once after the environment
/// check, so a GUI launch with a short PATH still finds `claude`.
pub fn refresh_search_dirs(app: &AppHandle) {
    let Some(home) = home_dir() else { return };
    let mut dirs: Vec<PathBuf> = crate::env_check::which::cached_shell_path()
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(default_claude_dirs(&home));
    if let Ok(mut guard) = app.state::<ClaudeSearchDirs>().0.lock() {
        *guard = dirs;
    }
}

/// The machine description the service works from: user home, data directory, `$SHELL`.
/// Shared by the commands and the debug self-test (which points `HOME` at a temp dir).
pub fn make_env(home: PathBuf, data_dir: PathBuf) -> KclaudeEnv {
    KclaudeEnv {
        home,
        data_dir,
        shell_var: std::env::var("SHELL").ok(),
        platform: Platform::current(),
        #[cfg(windows)]
        documents_dir: super::ps_profile::documents_dir(),
        #[cfg(not(windows))]
        documents_dir: None,
    }
}

/// Directories searched for `claude`: the configured list, or the process PATH plus the
/// usual install locations when none was configured yet.
pub fn search_dirs(configured: &[PathBuf], home: &std::path::Path) -> Vec<PathBuf> {
    if configured.is_empty() {
        default_claude_dirs(home)
    } else {
        configured.to_vec()
    }
}

fn build(app: &AppHandle, dirs: &State<'_, ClaudeSearchDirs>) -> Result<(KclaudeEnv, SystemBackend), KclaudeError> {
    let internal = |m: &str| KclaudeError::new(ErrorCode::Internal, m);
    let home = home_dir().ok_or_else(|| internal("找不到用户目录"))?;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|_| internal("找不到 App 数据目录"))?;
    let configured = dirs.0.lock().map(|g| g.clone()).unwrap_or_default();
    let search = search_dirs(&configured, &home);
    Ok((make_env(home.clone(), data_dir), SystemBackend::new(search)))
}

async fn run_blocking<F>(app: AppHandle, f: F) -> Result<KclaudeStatus, KclaudeError>
where
    F: FnOnce(&KclaudeService<'_>) -> Result<KclaudeStatus, KclaudeError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let dirs = app.state::<ClaudeSearchDirs>();
        let (env, backend) = build(&app, &dirs)?;
        f(&KclaudeService::new(env, &backend))
    })
    .await
    .map_err(|_| KclaudeError::new(ErrorCode::Internal, "kclaude 操作异常中断"))?
}

#[tauri::command]
pub async fn kclaude_status(app: AppHandle) -> Result<KclaudeStatus, KclaudeError> {
    run_blocking(app, |s| Ok(s.status())).await
}

#[tauri::command]
pub async fn kclaude_install(app: AppHandle) -> Result<KclaudeStatus, KclaudeError> {
    run_blocking(app, |s| s.install()).await
}

#[tauri::command]
pub async fn kclaude_adopt(app: AppHandle) -> Result<KclaudeStatus, KclaudeError> {
    run_blocking(app, |s| s.adopt()).await
}

#[tauri::command]
pub async fn kclaude_remove(app: AppHandle) -> Result<KclaudeStatus, KclaudeError> {
    run_blocking(app, |s| s.remove()).await
}

/// App start: install when missing, repair stale pieces, never touch a foreign kclaude.
/// Call from a background thread (it may run the shell check, up to 5 seconds).
pub fn auto_install(app: &AppHandle) -> Result<KclaudeStatus, KclaudeError> {
    let dirs = app.state::<ClaudeSearchDirs>();
    let (env, backend) = build(app, &dirs)?;
    Ok(KclaudeService::new(env, &backend).auto_install())
}
