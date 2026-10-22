//! PowerShell profile paths and execution-policy detection.
//!
//! Windows PowerShell 5 and PowerShell 7 each read their own `CurrentUserAllHosts`
//! profile, so kclaude is written to both. Paths are computed from the Documents folder
//! (which OneDrive may redirect, hence the KnownFolder lookup) without starting any
//! PowerShell process. The execution policy is only detected: when it would stop the
//! profile from loading, the UI gets a command to copy; the App never changes it.

use std::path::{Path, PathBuf};

use super::types::{FixAction, VerifyStatus};

/// `Set-ExecutionPolicy` command offered to the user (current user only, no admin needed).
pub const POLICY_FIX_COMMAND: &str =
    "Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned";

/// The two `CurrentUserAllHosts` profile files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilePaths {
    /// Windows PowerShell 5: `<Documents>\WindowsPowerShell\profile.ps1`.
    pub ps5: PathBuf,
    /// PowerShell 7: `<Documents>\PowerShell\profile.ps1`.
    pub ps7: PathBuf,
}

impl ProfilePaths {
    /// Both paths, PS5 first.
    pub fn all(&self) -> Vec<PathBuf> {
        vec![self.ps5.clone(), self.ps7.clone()]
    }
}

/// Compute both profile paths for `documents`.
///
/// Args:
///     documents: The user's Documents folder (injected so tests need no Windows).
pub fn profile_paths(documents: &Path) -> ProfilePaths {
    ProfilePaths {
        ps5: documents.join("WindowsPowerShell").join("profile.ps1"),
        ps7: documents.join("PowerShell").join("profile.ps1"),
    }
}

/// True for UTF-16 files (BOM `FF FE` / `FE FF`). Their bytes cannot be edited as
/// ASCII-compatible text, so callers must leave such a profile alone.
pub fn is_utf16(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF])
}

/// PowerShell execution policy values. Only the Windows registry reader builds the stricter
/// values, so other platforms see them as unused.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionPolicy {
    Undefined,
    Restricted,
    AllSigned,
    RemoteSigned,
    Unrestricted,
    Bypass,
}

impl ExecutionPolicy {
    /// Parse a policy name (case-insensitive). Unknown names give `None`.
    #[cfg(any(windows, test))]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "undefined" => Some(Self::Undefined),
            "restricted" => Some(Self::Restricted),
            "allsigned" => Some(Self::AllSigned),
            "remotesigned" => Some(Self::RemoteSigned),
            "unrestricted" => Some(Self::Unrestricted),
            "bypass" => Some(Self::Bypass),
            _ => None,
        }
    }

    /// Whether this policy stops an unsigned local profile from loading.
    pub fn blocks_local_scripts(self) -> bool {
        matches!(self, Self::Restricted | Self::AllSigned)
    }
}

/// Policy values by scope. Precedence follows PowerShell: machine group policy, user group
/// policy, current user, local machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PolicySources {
    pub machine_policy: Option<ExecutionPolicy>,
    pub user_policy: Option<ExecutionPolicy>,
    pub user: Option<ExecutionPolicy>,
    pub machine: Option<ExecutionPolicy>,
}

impl PolicySources {
    /// Effective policy. `Undefined` entries fall through; nothing set means `Restricted`,
    /// the Windows client default.
    pub fn effective(&self) -> ExecutionPolicy {
        [self.machine_policy, self.user_policy, self.user, self.machine]
            .into_iter()
            .flatten()
            .find(|p| *p != ExecutionPolicy::Undefined)
            .unwrap_or(ExecutionPolicy::Restricted)
    }

    fn enforced_by_group_policy(&self) -> bool {
        [self.machine_policy, self.user_policy]
            .into_iter()
            .flatten()
            .any(|p| p != ExecutionPolicy::Undefined)
    }
}

/// A failed-verification description produced by the policy check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyHint {
    pub status: VerifyStatus,
    pub detail: String,
    pub fix: Option<FixAction>,
}

/// Explain an execution policy that would block the profile, or `None` when profiles load.
///
/// When group policy sets the restriction, no copyable command is offered because it could
/// not override it.
pub fn execution_policy_hint(sources: &PolicySources) -> Option<PolicyHint> {
    if !sources.effective().blocks_local_scripts() {
        return None;
    }
    if sources.enforced_by_group_policy() {
        return Some(PolicyHint {
            status: VerifyStatus::Fail,
            detail: "PowerShell 执行策略由组策略限制，不能加载 profile。请联系管理员放行".to_string(),
            fix: None,
        });
    }
    Some(PolicyHint {
        status: VerifyStatus::Fail,
        detail: "PowerShell 执行策略不允许加载 profile，kclaude 在新终端里不会生效。复制下面的命令到 PowerShell 里运行"
            .to_string(),
        fix: Some(FixAction::CopyCommand {
            label: "复制".to_string(),
            command: POLICY_FIX_COMMAND.to_string(),
        }),
    })
}

/// The user's Documents folder as Windows reports it (follows OneDrive redirection).
///
/// Returns:
///     `None` when the shell API fails.
#[cfg(windows)]
pub fn documents_dir() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};

    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: SHGetKnownFolderPath writes a CoTaskMem-allocated, NUL-terminated UTF-16
    // string to `raw` on success (HRESULT 0); it is read once and then freed.
    unsafe {
        let hr = SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut raw);
        if hr != 0 || raw.is_null() {
            if !raw.is_null() {
                CoTaskMemFree(raw as *const _);
            }
            return None;
        }
        let mut len = 0usize;
        while *raw.add(len) != 0 {
            len += 1;
        }
        let path = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, len)));
        CoTaskMemFree(raw as *const _);
        Some(path)
    }
}

/// Read the execution policy from the registry (no PowerShell process is started).
///
/// Covers Windows PowerShell 5 settings; PowerShell 7 honours the same machine and user
/// registry values unless overridden by `powershell.config.json`, which is not consulted.
#[cfg(windows)]
pub fn read_policy_sources() -> PolicySources {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    const SHELL_IDS: &str = r"Software\Microsoft\PowerShell\1\ShellIds\Microsoft.PowerShell";
    const GPO: &str = r"Software\Policies\Microsoft\Windows\PowerShell";

    let gpo = |root| {
        if reg_read_dword(root, GPO, "EnableScripts") == Some(0) {
            return Some(ExecutionPolicy::Restricted);
        }
        reg_read_string(root, GPO, "ExecutionPolicy").and_then(|v| ExecutionPolicy::parse(&v))
    };
    PolicySources {
        machine_policy: gpo(HKEY_LOCAL_MACHINE),
        user_policy: gpo(HKEY_CURRENT_USER),
        user: reg_read_string(HKEY_CURRENT_USER, SHELL_IDS, "ExecutionPolicy")
            .and_then(|v| ExecutionPolicy::parse(&v)),
        machine: reg_read_string(HKEY_LOCAL_MACHINE, SHELL_IDS, "ExecutionPolicy")
            .and_then(|v| ExecutionPolicy::parse(&v)),
    }
}

#[cfg(windows)]
fn reg_read_raw(
    root: windows_sys::Win32::System::Registry::HKEY,
    subkey: &str,
    name: &str,
) -> Option<(u32, Vec<u8>)> {
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW, KEY_READ};

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let (key_w, name_w) = (wide(subkey), wide(name));
    // SAFETY: all pointers refer to live, NUL-terminated buffers or locals; the key handle
    // is closed on every path after a successful open.
    unsafe {
        let mut key = std::ptr::null_mut();
        if RegOpenKeyExW(root, key_w.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return None;
        }
        let (mut kind, mut size) = (0u32, 0u32);
        let mut out = None;
        if RegQueryValueExW(key, name_w.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut size) == 0 {
            let mut buf = vec![0u8; size as usize];
            if RegQueryValueExW(key, name_w.as_ptr(), std::ptr::null(), &mut kind, buf.as_mut_ptr(), &mut size) == 0 {
                buf.truncate(size as usize);
                out = Some((kind, buf));
            }
        }
        RegCloseKey(key);
        out
    }
}

#[cfg(windows)]
fn reg_read_string(root: windows_sys::Win32::System::Registry::HKEY, subkey: &str, name: &str) -> Option<String> {
    const REG_SZ: u32 = 1;
    let (kind, buf) = reg_read_raw(root, subkey, name)?;
    if kind != REG_SZ {
        return None;
    }
    let units: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
}

#[cfg(windows)]
fn reg_read_dword(root: windows_sys::Win32::System::Registry::HKEY, subkey: &str, name: &str) -> Option<u32> {
    const REG_DWORD: u32 = 4;
    let (kind, buf) = reg_read_raw(root, subkey, name)?;
    (kind == REG_DWORD && buf.len() == 4).then(|| u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))
}

#[cfg(test)]
mod tests;
