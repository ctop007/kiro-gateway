//! Locating the sidecar executable and preparing its working directory.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Environment variable that points the App at another sidecar (development, tests).
pub const OVERRIDE_VAR: &str = "KIRO_GATEWAY_SIDECAR";
/// Name of the sidecar directory below the Tauri resource directory.
///
/// Windows ships one `gateway/`. macOS ships one directory per CPU architecture
/// (`gateway-aarch64/`, `gateway-x86_64/`): a universal bundle carries both and the App picks
/// the one matching the architecture it runs as; a single-architecture bundle carries one.
const RESOURCE_SUBDIR_BASE: &str = "gateway";
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
    /// The bundle has no sidecar for the CPU architecture the App is running on.
    ArchMissing,
}

impl SidecarError {
    /// Chinese one-liner for the UI: what is wrong and what to do, no paths.
    pub fn message(self) -> String {
        match self {
            SidecarError::Missing => "没找到内置的网关程序，请重新安装 Kiro Gateway".to_string(),
            SidecarError::OverrideMissing => format!("环境变量 {OVERRIDE_VAR} 指向的文件不存在"),
            SidecarError::NotExecutable => "网关程序没有执行权限，请重新安装 Kiro Gateway".to_string(),
            SidecarError::ArchMissing => {
                "这个安装包里没有适用于当前 CPU 的网关程序，请重新下载对应版本（Intel 与 Apple 芯片通用版）".to_string()
            }
        }
    }
}

/// Directory (below the resource directory) that holds the sidecar for `arch`.
///
/// Args:
///     windows: Whether the App is the Windows build.
///     arch: CPU architecture as spelled by `std::env::consts::ARCH` (`aarch64`, `x86_64`).
///
/// Returns:
///     `gateway` on Windows, `gateway-<arch>` elsewhere.
pub fn resource_subdir(windows: bool, arch: &str) -> String {
    if windows {
        RESOURCE_SUBDIR_BASE.to_string()
    } else {
        format!("{RESOURCE_SUBDIR_BASE}-{arch}")
    }
}

/// Whether `name` is a sidecar directory name of any layout ever shipped:
/// `gateway` (Windows, early macOS builds) or `gateway-<arch>`.
pub fn is_gateway_dir_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match lower.strip_prefix("gateway-") {
        None => lower == RESOURCE_SUBDIR_BASE,
        Some(arch) => !arch.is_empty() && arch.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
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
///     arch: CPU architecture of the running App (`std::env::consts::ARCH`); selects the
///         `gateway-<arch>/` directory on macOS.
///     override_path: Value of `KIRO_GATEWAY_SIDECAR`; empty counts as unset. When set it
///         wins and there is no fallback to the bundled copy: a wrong override should be
///         noticed, not hidden.
///
/// Returns:
///     The executable and its tiktoken cache, or why there is none.
pub fn resolve(resource_dir: &Path, override_path: Option<&OsStr>, arch: &str) -> Result<Sidecar, SidecarError> {
    let override_path = override_path.filter(|p| !p.is_empty());
    let (exe, missing) = match override_path {
        Some(p) => (PathBuf::from(p), SidecarError::OverrideMissing),
        None => {
            let dir = resource_dir.join(resource_subdir(cfg!(windows), arch));
            let missing = if dir.is_dir() { SidecarError::Missing } else { SidecarError::ArchMissing };
            (dir.join(exe_name(cfg!(windows))), missing)
        }
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
