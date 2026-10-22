//! Locating the sidecar executable and preparing its working directory.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Environment variable that points the App at another sidecar (development, tests).
pub const OVERRIDE_VAR: &str = "KIRO_GATEWAY_SIDECAR";
/// Directory below the Tauri resource directory (`bundle.resources`: `gateway/`).
const RESOURCE_SUBDIR: &str = "gateway";
/// Directory with the bundled tiktoken BPE file, next to the executable.
const TIKTOKEN_SUBDIR: &str = "tiktoken_cache";

/// A usable sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidecar {
    pub exe: PathBuf,
    /// `tiktoken_cache/` next to the executable, when it exists.
    pub tiktoken_cache: Option<PathBuf>,
}

/// Why no sidecar could be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarError {
    /// The bundled executable is not where the installer puts it.
    Missing,
    /// `KIRO_GATEWAY_SIDECAR` points at something that is not a file.
    OverrideMissing,
    /// The file exists but cannot be executed (permissions).
    NotExecutable,
}

impl SidecarError {
    /// Chinese one-liner for the UI: what is wrong and what to do, no paths.
    pub fn message(self) -> String {
        match self {
            SidecarError::Missing => "没找到内置的网关程序，请重新安装 Kiro Gateway".to_string(),
            SidecarError::OverrideMissing => format!("环境变量 {OVERRIDE_VAR} 指向的文件不存在"),
            SidecarError::NotExecutable => "网关程序没有执行权限，请重新安装 Kiro Gateway".to_string(),
        }
    }
}

/// File name of the sidecar executable.
pub fn exe_name(windows: bool) -> &'static str {
    if windows {
        "kiro-gateway.exe"
    } else {
        "kiro-gateway"
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

/// Find the sidecar.
///
/// Args:
///     resource_dir: Tauri's resource directory.
///     override_path: Value of `KIRO_GATEWAY_SIDECAR`; empty counts as unset. When set it
///         wins and there is no fallback to the bundled copy: a wrong override should be
///         noticed, not hidden.
///
/// Returns:
///     The executable and its tiktoken cache, or why there is none.
pub fn resolve(resource_dir: &Path, override_path: Option<&OsStr>) -> Result<Sidecar, SidecarError> {
    let override_path = override_path.filter(|p| !p.is_empty());
    let (exe, missing) = match override_path {
        Some(p) => (PathBuf::from(p), SidecarError::OverrideMissing),
        None => (resource_dir.join(RESOURCE_SUBDIR).join(exe_name(cfg!(windows))), SidecarError::Missing),
    };
    if !exe.is_file() {
        return Err(missing);
    }
    if !is_executable(&exe) {
        return Err(SidecarError::NotExecutable);
    }
    let cache = exe.parent().map(|dir| dir.join(TIKTOKEN_SUBDIR)).filter(|dir| dir.is_dir());
    Ok(Sidecar { exe, tiktoken_cache: cache })
}

/// Create the sidecar's working directory and make sure no `.env` is in it.
///
/// The gateway runs `load_dotenv()` from its working directory at import time; an empty
/// directory guarantees it reads nothing the App did not give it.
pub fn prepare_runtime_dir(dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    match fs::remove_file(dir.join(".env")) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(dir.to_path_buf())
}

#[cfg(test)]
mod tests;
