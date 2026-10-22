//! The kclaude install / adopt / remove / status flow.
//!
//! `KclaudeService` ties the pieces together (script templates, marker blocks, shell file
//! selection, PowerShell profiles, post-install check). Everything outside this module's
//! `Backend` (clock, shell verification, `claude` lookup) is injected, so the whole flow runs
//! in tests over temporary directories.
//!
//! Safety rules:
//! - All startup files are read and planned before anything is written; a refusal
//!   (foreign kclaude, damaged markers, UTF-16 profile) leaves every file untouched.
//! - The script in the App data directory is written before the startup file that points
//!   at it.
//! - Removing the block leaves a `kclaude.removed` marker so the automatic install at App
//!   start does not bring it back; an explicit install clears it.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::fsutil::write_atomic;
use super::ps_profile::{self, profile_paths};
use super::rc::{self, Dialect, RcState, Refusal};
use super::script::{self, ScriptKind};
use super::types::{
    ErrorCode, KclaudeError, KclaudeState, KclaudeStatus, KclaudeVerify, ShellKind,
};
use super::verify;

/// File in the data directory recording that the user removed kclaude.
pub const REMOVED_MARKER: &str = "kclaude.removed";
/// File in the data directory caching the last login-shell check.
pub const VERIFY_CACHE: &str = "kclaude-verify.json";

/// The last login-shell check and what the startup files looked like when it ran.
#[derive(Debug, Serialize, Deserialize)]
struct VerifyCache {
    fingerprint: String,
    verify: KclaudeVerify,
}

/// Operating system family, as far as startup-file handling is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    Other,
}

impl Platform {
    /// The platform this binary runs on.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Other
        }
    }
}

/// Everything about the machine the service needs, injected.
#[derive(Debug, Clone)]
pub struct KclaudeEnv {
    pub home: PathBuf,
    /// App data directory: holds `kclaude.*` scripts and `kclaude.env`.
    pub data_dir: PathBuf,
    /// Value of `$SHELL`.
    pub shell_var: Option<String>,
    pub platform: Platform,
    /// Windows Documents folder (KnownFolder); `None` when unavailable.
    pub documents_dir: Option<PathBuf>,
}

/// What the post-install check should run.
#[derive(Debug, Clone, Copy)]
pub enum VerifyTarget<'a> {
    Posix { shell: &'a Path, home: &'a Path },
    PowerShell,
}

/// Side effects the service delegates, so tests can replace them.
pub trait Backend {
    /// Timestamp for backup names, `YYYYMMDD-HHMMSS` in local time.
    fn stamp(&self) -> String;
    /// Whether Claude Code is installed.
    fn claude_installed(&self) -> bool;
    /// Check that a fresh shell knows `kclaude`.
    fn verify(&self, target: &VerifyTarget<'_>) -> KclaudeVerify;
}

/// One startup file under management.
struct RcTarget {
    path: PathBuf,
    content: Vec<u8>,
    read_error: bool,
}

/// The install / adopt / remove / status operations.
pub struct KclaudeService<'a> {
    env: KclaudeEnv,
    backend: &'a dyn Backend,
}

fn err(code: ErrorCode, message: &str) -> KclaudeError {
    KclaudeError::new(code, message)
}

const MSG_DAMAGED: &str = "启动文件里的 kclaude 标记不完整，App 没有改动它。删除残缺的标记行后重试";
const MSG_UNPARSABLE: &str = "启动文件里已有的 kclaude 写法 App 看不懂，没有改动它。请手动处理后重试";
const MSG_NO_DOCUMENTS: &str = "找不到「文档」文件夹，没能确定 PowerShell profile 的位置";
const MSG_UTF16: &str = "PowerShell profile 是 UTF-16 编码，App 没有改动它。请另存为 UTF-8 后重试";
const MSG_READ: &str = "读取启动文件失败。检查文件权限后重试";

impl<'a> KclaudeService<'a> {
    pub fn new(env: KclaudeEnv, backend: &'a dyn Backend) -> Self {
        Self { env, backend }
    }

    fn shell_kind(&self) -> ShellKind {
        match self.env.platform {
            Platform::Windows => ShellKind::Powershell,
            _ => rc::shell_from_env(self.env.shell_var.as_deref()),
        }
    }

    fn script_kind(&self) -> ScriptKind {
        match self.shell_kind() {
            ShellKind::Zsh => ScriptKind::Zsh,
            ShellKind::Bash => ScriptKind::Bash,
            ShellKind::Powershell => ScriptKind::PowerShell,
        }
    }

    fn dialect(&self) -> Dialect {
        match self.shell_kind() {
            ShellKind::Powershell => Dialect::PowerShell,
            _ => Dialect::Posix,
        }
    }

    fn script_path(&self) -> PathBuf {
        self.env.data_dir.join(self.script_kind().file_name())
    }

    fn expected_block(&self) -> String {
        rc::block_text(self.dialect(), &self.script_path())
    }

    fn rc_paths(&self) -> Result<Vec<PathBuf>, KclaudeError> {
        match self.shell_kind() {
            ShellKind::Powershell => {
                let docs = self
                    .env
                    .documents_dir
                    .as_deref()
                    .ok_or_else(|| err(ErrorCode::WriteFailed, MSG_NO_DOCUMENTS))?;
                Ok(profile_paths(docs).all())
            }
            kind => Ok(vec![rc::choose_rc_file(
                kind,
                &self.env.home,
                self.env.platform == Platform::MacOs,
            )]),
        }
    }

    fn load_targets(&self) -> Result<Vec<RcTarget>, KclaudeError> {
        Ok(self
            .rc_paths()?
            .into_iter()
            .map(|path| match rc::read_rc(&path) {
                Ok(content) => RcTarget { path, content, read_error: false },
                Err(_) => RcTarget { path, content: Vec::new(), read_error: true },
            })
            .collect())
    }

    fn check_writable(&self, targets: &[RcTarget]) -> Result<(), KclaudeError> {
        for t in targets {
            if t.read_error {
                return Err(err(ErrorCode::WriteFailed, MSG_READ));
            }
            if ps_profile::is_utf16(&t.content) {
                return Err(err(ErrorCode::WriteFailed, MSG_UTF16));
            }
        }
        Ok(())
    }

    fn rc_display(&self) -> Vec<String> {
        self.rc_paths()
            .unwrap_or_default()
            .iter()
            .map(|p| display_path(&self.env.home, p))
            .collect()
    }

    fn removed_marker(&self) -> PathBuf {
        self.env.data_dir.join(REMOVED_MARKER)
    }

    /// Current state without changing anything. Runs the shell check when kclaude is (or was)
    /// installed.
    pub fn status(&self) -> KclaudeStatus {
        self.status_with(false)
    }

    /// Like [`status`](Self::status); `force_verify` runs the login-shell check even when a
    /// cached result still matches (install, adopt and remove always re-check).
    fn status_with(&self, force_verify: bool) -> KclaudeStatus {
        let block = self.expected_block();
        let dialect = self.dialect();
        let states: Vec<RcState> = self
            .load_targets()
            .map(|targets| {
                targets
                    .iter()
                    .map(|t| rc::analyze(&t.content, dialect).state(&block))
                    .collect()
            })
            .unwrap_or_default();
        let script_current = script::is_current(self.script_kind(), &self.env.data_dir);
        let state = combine_states(&states, script_current);
        let verify = match state {
            KclaudeState::Installed | KclaudeState::Outdated => self.verify_cached(force_verify),
            _ => KclaudeVerify::skipped(),
        };
        KclaudeStatus {
            state,
            shell: self.shell_kind(),
            rc_files: self.rc_display(),
            claude_installed: self.backend.claude_installed(),
            verify,
        }
    }

    /// The login-shell check, spawning a shell only when there is no cached result for the
    /// current startup files (or `force` is set). `$SHELL -lic` runs the user's startup
    /// scripts and can take seconds, so `kclaude_status` must not do that on every call.
    fn verify_cached(&self, force: bool) -> KclaudeVerify {
        let fingerprint = self.verify_fingerprint();
        let path = self.env.data_dir.join(VERIFY_CACHE);
        if !force {
            let cached = fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<VerifyCache>(&b).ok());
            if let Some(c) = cached.filter(|c| c.fingerprint == fingerprint) {
                return c.verify;
            }
        }
        let verify = self.run_verify();
        let entry = VerifyCache { fingerprint, verify: verify.clone() };
        if let Ok(bytes) = serde_json::to_vec(&entry) {
            // Best effort: without the cache the next status simply re-checks.
            let _ = write_atomic(&path, &bytes);
        }
        verify
    }

    /// Identifies what the check depends on: shell, startup files (modification time and
    /// size) and the script kind. A change to any of them invalidates the cached result.
    fn verify_fingerprint(&self) -> String {
        let mut parts = vec![
            format!("{:?}", self.shell_kind()),
            self.env.shell_var.clone().unwrap_or_default(),
        ];
        for path in self.rc_paths().unwrap_or_default() {
            let stamp = match fs::metadata(&path) {
                Ok(meta) => {
                    let nanos = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    format!("{nanos}:{}", meta.len())
                }
                Err(_) => "missing".to_string(),
            };
            parts.push(format!("{}={stamp}", path.display()));
        }
        parts.join("|")
    }

    fn run_verify(&self) -> KclaudeVerify {
        match self.shell_kind() {
            ShellKind::Powershell => self.backend.verify(&VerifyTarget::PowerShell),
            kind => {
                let named = self
                    .env
                    .shell_var
                    .as_deref()
                    .filter(|s| rc::shell_from_env(Some(s)) == kind && is_known_shell_name(s));
                let shell = named.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/bin/zsh"));
                self.backend.verify(&VerifyTarget::Posix { shell: &shell, home: &self.env.home })
            }
        }
    }

    /// Install the script and the marker block. Idempotent: nothing is written when both are
    /// current.
    ///
    /// Returns:
    ///     The state after the write, including the shell check.
    ///     `ForeignPresent` when another kclaude exists; `BackupFailed` / `WriteFailed` otherwise.
    pub fn install(&self) -> Result<KclaudeStatus, KclaudeError> {
        self.write_all(Mode::Install)?;
        Ok(self.status_with(true))
    }

    /// Comment out the foreign kclaude lines, then install. Only valid in state `foreign`.
    pub fn adopt(&self) -> Result<KclaudeStatus, KclaudeError> {
        self.write_all(Mode::Adopt)?;
        Ok(self.status_with(true))
    }

    /// Remove the marker block from every startup file; all other bytes stay as they were.
    pub fn remove(&self) -> Result<KclaudeStatus, KclaudeError> {
        let dialect = self.dialect();
        let targets = self.load_targets()?;
        self.check_writable(&targets)?;
        let mut plans = Vec::new();
        for t in &targets {
            match rc::plan_remove(&t.content, dialect) {
                Ok(Some(bytes)) => plans.push((t, bytes)),
                Ok(None) => {}
                Err(_) => return Err(err(ErrorCode::WriteFailed, MSG_DAMAGED)),
            }
        }
        let stamp = self.backend.stamp();
        for (t, bytes) in plans {
            rc::apply(&t.path, &bytes, &stamp)?;
        }
        // Best effort: without the marker the next App start would simply install again.
        let _ = write_atomic(&self.removed_marker(), b"removed by the user\n");
        Ok(self.status_with(true))
    }

    /// What App start does: install when missing (unless the user removed it), never touch a
    /// foreign kclaude, repair outdated pieces.
    pub fn auto_install(&self) -> KclaudeStatus {
        let current = self.status();
        match current.state {
            KclaudeState::Installed | KclaudeState::Foreign => current,
            KclaudeState::NotInstalled if self.removed_marker().exists() => current,
            KclaudeState::NotInstalled | KclaudeState::Outdated => {
                self.install().unwrap_or_else(|_| self.status())
            }
        }
    }

    fn write_all(&self, mode: Mode) -> Result<(), KclaudeError> {
        let dialect = self.dialect();
        let block = self.expected_block();
        let targets = self.load_targets()?;
        self.check_writable(&targets)?;

        if mode == Mode::Adopt {
            let any_foreign = targets.iter().any(|t| {
                matches!(rc::analyze(&t.content, dialect).state(&block), RcState::Foreign | RcState::Damaged)
            });
            if !any_foreign {
                return Err(err(ErrorCode::InvalidArgument, "没有检测到需要接管的 kclaude"));
            }
        }

        // Plan every file first; any refusal aborts before the first write.
        let mut plans: Vec<(&RcTarget, Vec<u8>)> = Vec::new();
        for t in &targets {
            let planned = match mode {
                Mode::Install => rc::plan_install(&t.content, dialect, &block),
                Mode::Adopt => rc::plan_adopt(&t.content, dialect, &block),
            };
            match planned {
                Ok(Some(bytes)) => plans.push((t, bytes)),
                Ok(None) => {}
                Err(Refusal::Foreign) => {
                    return Err(err(
                        ErrorCode::ForeignPresent,
                        "检测到已有 kclaude，未改动。点「改由 App 管理」后再安装",
                    ))
                }
                Err(Refusal::Damaged) if mode == Mode::Install => {
                    return Err(err(ErrorCode::ForeignPresent, MSG_DAMAGED))
                }
                Err(Refusal::Damaged) => return Err(err(ErrorCode::WriteFailed, MSG_DAMAGED)),
                Err(Refusal::UnparsableForeign) => {
                    return Err(err(ErrorCode::WriteFailed, MSG_UNPARSABLE))
                }
            }
        }

        script::install(self.script_kind(), &self.env.data_dir)
            .map_err(|_| err(ErrorCode::WriteFailed, "写入 kclaude 脚本失败。检查磁盘空间后重试"))?;
        let stamp = self.backend.stamp();
        for (t, bytes) in plans {
            rc::apply(&t.path, &bytes, &stamp)?;
        }
        let _ = fs::remove_file(self.removed_marker());
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Install,
    Adopt,
}

fn is_known_shell_name(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("");
    name.starts_with("zsh") || name.starts_with("bash")
}

/// Merge per-file states (one on macOS, two on Windows) and script freshness into the
/// state the UI shows.
pub fn combine_states(states: &[RcState], script_current: bool) -> KclaudeState {
    if states.iter().any(|s| matches!(s, RcState::Foreign | RcState::Damaged)) {
        return KclaudeState::Foreign;
    }
    let installed = states.iter().filter(|s| **s == RcState::Installed).count();
    let not_installed = states.iter().filter(|s| **s == RcState::NotInstalled).count();
    if states.is_empty() || not_installed == states.len() {
        return KclaudeState::NotInstalled;
    }
    if installed == states.len() && script_current {
        KclaudeState::Installed
    } else {
        KclaudeState::Outdated
    }
}

/// Abbreviate the home directory as `~` for display.
pub fn display_path(home: &Path, path: &Path) -> String {
    if path == home {
        return "~".to_string();
    }
    match path.strip_prefix(home) {
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Look for a `claude` executable in `dirs`.
///
/// Args:
///     dirs: Directories to search (login-shell PATH entries plus well-known locations).
///     windows: Also try `.exe` / `.cmd` / `.ps1` suffixes.
pub fn find_claude(dirs: &[PathBuf], windows: bool) -> Option<PathBuf> {
    let names: &[&str] = if windows {
        &["claude.exe", "claude.cmd", "claude.ps1", "claude"]
    } else {
        &["claude"]
    };
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| is_executable_file(p))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else { return false };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The production backend: local clock, real shells, a caller-supplied `claude` lookup.
pub struct SystemBackend {
    claude_dirs: Vec<PathBuf>,
}

impl SystemBackend {
    /// Args:
    ///     claude_dirs: Directories searched for `claude` (the login-shell PATH and defaults).
    pub fn new(claude_dirs: Vec<PathBuf>) -> Self {
        Self { claude_dirs }
    }
}

impl Backend for SystemBackend {
    fn stamp(&self) -> String {
        chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
    }

    fn claude_installed(&self) -> bool {
        find_claude(&self.claude_dirs, cfg!(windows)).is_some()
    }

    fn verify(&self, target: &VerifyTarget<'_>) -> KclaudeVerify {
        match target {
            VerifyTarget::Posix { shell, home } => {
                let outcome = verify::run_shell(shell, &verify::LOGIN_TYPE_ARGS, home, verify::VERIFY_TIMEOUT);
                verify::verify_posix(&outcome)
            }
            VerifyTarget::PowerShell => {
                let hint = ps_profile::execution_policy_hint(&current_policy_sources());
                let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
                let args = ["-NoLogo", "-NonInteractive", "-Command", "(Get-Command kclaude -ErrorAction SilentlyContinue).CommandType"];
                let outcome = verify::run_shell(Path::new("powershell.exe"), &args, &home, verify::VERIFY_TIMEOUT);
                verify::verify_powershell(&outcome, hint)
            }
        }
    }
}

#[cfg(windows)]
fn current_policy_sources() -> ps_profile::PolicySources {
    ps_profile::read_policy_sources()
}

#[cfg(not(windows))]
fn current_policy_sources() -> ps_profile::PolicySources {
    ps_profile::PolicySources {
        user: Some(ps_profile::ExecutionPolicy::RemoteSigned),
        ..Default::default()
    }
}


#[cfg(test)]
mod tests;
