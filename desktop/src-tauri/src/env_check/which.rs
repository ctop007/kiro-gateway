//! Locate `kiro-cli` and `claude` (tasks 5.1).
//!
//! A GUI app started from Finder does not inherit the terminal PATH, so the
//! search order is: login shell PATH (`$SHELL -lic`, 3 s timeout, cached), the
//! process PATH, then per-platform default install directories. Only absolute
//! directories are searched, so an empty or `.` PATH entry can never resolve
//! against the app's own working directory.
//!
//! The lookup itself ([`find_tool`], [`default_dirs`], [`executable_names`]) is
//! a pure function of [`SearchInputs`], so the Windows rules are tested on any
//! host.
//!
//! Default locations on Windows are NOT confirmed on a real machine yet.

use super::{Platform, UserDirs};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Time budget for `$SHELL -lic 'echo $PATH'` (design: about 3 seconds).
pub const SHELL_PATH_TIMEOUT: Duration = Duration::from_secs(3);

/// The two executables the environment check looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    KiroCli,
    Claude,
}

impl Tool {
    /// Base command name, without extension.
    pub fn command_name(self) -> &'static str {
        match self {
            Tool::KiroCli => "kiro-cli",
            Tool::Claude => "claude",
        }
    }
}

/// Everything the pure lookup needs; no globals are read.
#[derive(Debug, Clone)]
pub struct SearchInputs {
    pub platform: Platform,
    /// PATH reported by the login shell, if the probe succeeded.
    pub shell_path: Option<OsString>,
    /// PATH of this process.
    pub process_path: Option<OsString>,
    /// Default install directories, already computed by [`default_dirs`].
    pub defaults: Vec<PathBuf>,
}

/// File names that count as the tool on `platform`, in priority order.
pub fn executable_names(platform: Platform, tool: Tool) -> Vec<String> {
    let base = tool.command_name();
    match platform {
        Platform::MacOs => vec![base.to_string()],
        Platform::Windows => vec![format!("{base}.exe"), format!("{base}.cmd"), format!("{base}.bat")],
    }
}

/// Default install directories for `tool` on `platform`.
///
/// - macOS: `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`.
/// - Windows (not confirmed on a real machine): `%LOCALAPPDATA%\Programs\<tool>`,
///   `%USERPROFILE%\.local\bin`, and for Claude Code the npm global directory
///   `%APPDATA%\npm`.
///
/// Directories whose base environment variable is missing are skipped.
pub fn default_dirs(platform: Platform, tool: Tool, dirs: &UserDirs) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    match platform {
        Platform::MacOs => {
            if let Some(home) = &dirs.home {
                out.push(home.join(".local").join("bin"));
            }
            out.push(PathBuf::from("/opt/homebrew/bin"));
            out.push(PathBuf::from("/usr/local/bin"));
        }
        Platform::Windows => {
            if let Some(local) = &dirs.local_app_data {
                out.push(local.join("Programs").join(tool.command_name()));
            }
            if let Some(home) = &dirs.home {
                out.push(home.join(".local").join("bin"));
            }
            if tool == Tool::Claude {
                if let Some(roaming) = &dirs.app_data {
                    out.push(roaming.join("npm"));
                }
            }
        }
    }
    out
}

/// Whether `path` is a regular file the OS would run.
///
/// Windows has no executable bit, so a file with the right extension counts.
fn is_runnable(platform: Platform, path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    match platform {
        Platform::Windows => true,
        Platform::MacOs => {
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
    }
}

/// Find `tool`: shell PATH first, then process PATH, then default directories.
///
/// Returns the absolute path of the first runnable match.
pub fn find_tool(tool: Tool, inputs: &SearchInputs) -> Option<PathBuf> {
    let names = executable_names(inputs.platform, tool);
    let mut searched: Vec<PathBuf> = Vec::new();
    for list in [&inputs.shell_path, &inputs.process_path].into_iter().flatten() {
        searched.extend(std::env::split_paths(list));
    }
    searched.extend(inputs.defaults.iter().cloned());
    for dir in searched {
        // Relative entries (including "" and ".") would resolve against our cwd.
        if !dir.is_absolute() {
            continue;
        }
        for name in &names {
            let candidate = dir.join(name);
            if is_runnable(inputs.platform, &candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Arguments that make `shell` print its PATH in `:` separated form.
///
/// fish stores PATH as a list, so `echo $PATH` would print it space separated.
fn path_probe_args(shell: &Path) -> [&'static str; 2] {
    let is_fish = shell.file_name().and_then(OsStr::to_str) == Some("fish");
    if is_fish {
        ["-lic", "string join : $PATH"]
    } else {
        ["-lic", "echo \"$PATH\""]
    }
}

/// Run `shell -lic 'echo $PATH'` and return its PATH, or `None` on any failure.
///
/// The shell is killed when it does not finish within `timeout` (a slow
/// `.zshrc` must not freeze the check). Rc files may print banners, so only the
/// last non-empty line of stdout is used.
pub fn probe_shell_path(shell: &Path, timeout: Duration) -> Option<OsString> {
    let mut child = Command::new(shell)
        .args(path_probe_args(shell))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Drain stdout on a helper thread. A grandchild left behind by an rc file may
    // keep the pipe open forever, so the thread is never joined.
    let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let finished = Arc::new(AtomicBool::new(false));
    if let Some(mut stdout) = child.stdout.take() {
        let buffer = Arc::clone(&buffer);
        let finished = Arc::clone(&finished);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut b) = buffer.lock() {
                            b.extend_from_slice(&chunk[..n]);
                        }
                    }
                }
            }
            finished.store(true, Ordering::SeqCst);
        });
    }

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
    // Give the reader a moment to take the last bytes after the shell exited.
    let grace = Instant::now() + Duration::from_millis(300);
    while !finished.load(Ordering::SeqCst) && Instant::now() < grace {
        std::thread::sleep(Duration::from_millis(5));
    }

    let bytes = buffer.lock().ok()?.clone();
    let text = String::from_utf8_lossy(&bytes);
    let line = text.lines().map(str::trim).filter(|l| !l.is_empty()).last()?;
    Some(OsString::from(line))
}

/// Remembers the outcome of the login shell probe, including a failed one.
#[derive(Debug, Default)]
pub struct PathCache {
    slot: Mutex<Option<Option<OsString>>>,
}

impl PathCache {
    /// Return the cached value, running `probe` only on the first call.
    pub fn get_or_probe<F>(&self, probe: F) -> Option<OsString>
    where
        F: FnOnce() -> Option<OsString>,
    {
        let mut slot = match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(cached) = slot.as_ref() {
            return cached.clone();
        }
        let value = probe();
        *slot = Some(value.clone());
        value
    }
}

static SHELL_PATH_CACHE: OnceLock<PathCache> = OnceLock::new();

/// The user's login shell: `$SHELL`, falling back to `/bin/zsh` on macOS.
fn login_shell() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    match std::env::var_os("SHELL") {
        Some(s) if !s.is_empty() && Path::new(&s).is_absolute() => Some(PathBuf::from(s)),
        _ => Some(PathBuf::from("/bin/zsh")),
    }
}

/// Login shell PATH, probed once per process and cached.
pub fn cached_shell_path() -> Option<OsString> {
    SHELL_PATH_CACHE
        .get_or_init(PathCache::default)
        .get_or_probe(|| login_shell().and_then(|sh| probe_shell_path(&sh, SHELL_PATH_TIMEOUT)))
}

/// Locate `tool` on this machine with the real environment.
pub fn locate(tool: Tool) -> Option<PathBuf> {
    let platform = Platform::current();
    let dirs = UserDirs::from_process();
    let inputs = SearchInputs {
        platform,
        shell_path: cached_shell_path(),
        process_path: std::env::var_os("PATH"),
        defaults: default_dirs(platform, tool, &dirs),
    };
    find_tool(tool, &inputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    fn make_exe(dir: &Path, name: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    fn inputs(shell: Option<&Path>, process: Option<&Path>, defaults: Vec<PathBuf>) -> SearchInputs {
        SearchInputs {
            platform: Platform::MacOs,
            shell_path: shell.map(|p| OsString::from(p)),
            process_path: process.map(|p| OsString::from(p)),
            defaults,
        }
    }

    // ---------- find_tool: the four situations of tasks 5.1 ----------

    #[test]
    fn find_tool_only_in_default_location() {
        let tmp = tempfile::tempdir().unwrap();
        let default_dir = tmp.path().join("local-bin");
        let exe = make_exe(&default_dir, "kiro-cli");
        let found = find_tool(Tool::KiroCli, &inputs(None, None, vec![default_dir]));
        assert_eq!(found, Some(exe));
    }

    #[test]
    fn find_tool_only_in_path() {
        let tmp = tempfile::tempdir().unwrap();
        let path_dir = tmp.path().join("custom");
        let exe = make_exe(&path_dir, "claude");
        let found = find_tool(Tool::Claude, &inputs(Some(&path_dir), None, vec![]));
        assert_eq!(found, Some(exe));
    }

    #[test]
    fn find_tool_in_process_path_when_shell_path_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let path_dir = tmp.path().join("proc");
        let exe = make_exe(&path_dir, "kiro-cli");
        let found = find_tool(Tool::KiroCli, &inputs(None, Some(&path_dir), vec![]));
        assert_eq!(found, Some(exe));
    }

    #[test]
    fn find_tool_nowhere_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        let found = find_tool(
            Tool::KiroCli,
            &inputs(Some(&empty), Some(&empty), vec![tmp.path().join("missing")]),
        );
        assert_eq!(found, None);
    }

    #[test]
    fn find_tool_path_wins_over_default_location() {
        let tmp = tempfile::tempdir().unwrap();
        let path_dir = tmp.path().join("a");
        let default_dir = tmp.path().join("b");
        let in_path = make_exe(&path_dir, "kiro-cli");
        make_exe(&default_dir, "kiro-cli");
        let found = find_tool(Tool::KiroCli, &inputs(Some(&path_dir), None, vec![default_dir]));
        assert_eq!(found, Some(in_path));
    }

    #[test]
    fn find_tool_shell_path_wins_over_process_path() {
        let tmp = tempfile::tempdir().unwrap();
        let shell_dir = tmp.path().join("shell");
        let proc_dir = tmp.path().join("proc");
        let in_shell = make_exe(&shell_dir, "claude");
        make_exe(&proc_dir, "claude");
        let found = find_tool(Tool::Claude, &inputs(Some(&shell_dir), Some(&proc_dir), vec![]));
        assert_eq!(found, Some(in_shell));
    }

    #[test]
    fn find_tool_returns_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("bin");
        make_exe(&dir, "kiro-cli");
        let found = find_tool(Tool::KiroCli, &inputs(None, None, vec![dir])).unwrap();
        assert!(found.is_absolute());
    }

    // ---------- find_tool: hostile PATH contents ----------

    #[test]
    fn find_tool_ignores_relative_path_entries() {
        // A relative entry would resolve against the app's cwd.
        let tmp = tempfile::tempdir().unwrap();
        make_exe(&tmp.path().join("rel"), "kiro-cli");
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let path = OsString::from(":.:rel:");
        let search = SearchInputs {
            platform: Platform::MacOs,
            shell_path: Some(path),
            process_path: None,
            defaults: vec![],
        };
        let found = find_tool(Tool::KiroCli, &search);
        std::env::set_current_dir(prev).unwrap();
        assert_eq!(found, None);
    }

    #[test]
    fn find_tool_ignores_directory_with_tool_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("bin");
        fs::create_dir_all(dir.join("kiro-cli")).unwrap();
        assert_eq!(find_tool(Tool::KiroCli, &inputs(Some(&dir), None, vec![])), None);
    }

    #[cfg(unix)]
    #[test]
    fn find_tool_ignores_non_executable_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("bin");
        let p = make_exe(&dir, "kiro-cli");
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(find_tool(Tool::KiroCli, &inputs(Some(&dir), None, vec![])), None);
    }

    #[test]
    fn find_tool_does_not_confuse_claude_with_kiro_cli() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("bin");
        make_exe(&dir, "kiro-cli");
        assert_eq!(find_tool(Tool::Claude, &inputs(Some(&dir), None, vec![])), None);
    }

    #[test]
    fn find_tool_directory_with_spaces_works() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("my tools").join("bin");
        let exe = make_exe(&dir, "kiro-cli");
        assert_eq!(find_tool(Tool::KiroCli, &inputs(Some(&dir), None, vec![])), Some(exe));
    }

    // ---------- Windows rules as pure functions ----------

    #[test]
    fn windows_find_tool_uses_exe_and_cmd_names_without_exec_bit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("bin");
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("claude.cmd");
        fs::write(&exe, "@echo off").unwrap(); // mode 0644 on unix: must still count
        let search = SearchInputs {
            platform: Platform::Windows,
            shell_path: None,
            process_path: Some(OsString::from(&dir)),
            defaults: vec![],
        };
        assert_eq!(find_tool(Tool::Claude, &search), Some(exe));
    }

    #[test]
    fn windows_executable_names_cover_exe_cmd_bat() {
        assert_eq!(
            executable_names(Platform::Windows, Tool::KiroCli),
            vec!["kiro-cli.exe", "kiro-cli.cmd", "kiro-cli.bat"]
        );
        assert_eq!(executable_names(Platform::MacOs, Tool::Claude), vec!["claude"]);
    }

    #[test]
    fn windows_default_dirs_under_localappdata_programs_and_userprofile() {
        let dirs = UserDirs {
            home: Some(PathBuf::from("/u/me")),
            local_app_data: Some(PathBuf::from("/u/me/AppData/Local")),
            app_data: Some(PathBuf::from("/u/me/AppData/Roaming")),
        };
        let kiro = default_dirs(Platform::Windows, Tool::KiroCli, &dirs);
        assert!(kiro.contains(&PathBuf::from("/u/me/AppData/Local").join("Programs").join("kiro-cli")));
        assert!(kiro.contains(&PathBuf::from("/u/me").join(".local").join("bin")));
        let claude = default_dirs(Platform::Windows, Tool::Claude, &dirs);
        assert!(claude.contains(&PathBuf::from("/u/me").join(".local").join("bin")));
        assert!(claude.contains(&PathBuf::from("/u/me/AppData/Roaming").join("npm")));
        // Nothing from the macOS list leaks into Windows.
        assert!(!kiro.contains(&PathBuf::from("/opt/homebrew/bin")));
    }

    #[test]
    fn windows_default_dirs_skip_missing_env_dirs() {
        let dirs = UserDirs { home: None, local_app_data: None, app_data: None };
        assert!(default_dirs(Platform::Windows, Tool::KiroCli, &dirs).is_empty());
    }

    #[test]
    fn macos_default_dirs_are_local_bin_homebrew_and_usr_local() {
        let dirs = UserDirs { home: Some(PathBuf::from("/Users/me")), ..UserDirs::default() };
        assert_eq!(
            default_dirs(Platform::MacOs, Tool::KiroCli, &dirs),
            vec![
                PathBuf::from("/Users/me/.local/bin"),
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ]
        );
    }

    #[test]
    fn macos_default_dirs_without_home_keep_system_dirs() {
        let dirs = UserDirs::default();
        assert_eq!(
            default_dirs(Platform::MacOs, Tool::Claude, &dirs),
            vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]
        );
    }

    // ---------- login shell PATH probe ----------

    #[cfg(unix)]
    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        make_exe(dir, "fakesh");
        let p = dir.join("fakesh");
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn probe_shell_path_reads_stdout_of_login_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let sh = fake_shell(tmp.path(), "echo /opt/a:/opt/b");
        let got = probe_shell_path(&sh, Duration::from_secs(3));
        assert_eq!(got, Some(OsString::from("/opt/a:/opt/b")));
    }

    #[cfg(unix)]
    #[test]
    fn probe_shell_path_passes_login_interactive_command_flags() {
        let tmp = tempfile::tempdir().unwrap();
        // Echo back the arguments instead of a PATH.
        let sh = fake_shell(tmp.path(), "echo \"$1|$2\"");
        let got = probe_shell_path(&sh, Duration::from_secs(3)).unwrap();
        assert_eq!(got, OsString::from("-lic|echo \"$PATH\""));
    }

    #[cfg(unix)]
    #[test]
    fn probe_shell_path_uses_last_line_when_rc_files_print_banners() {
        let tmp = tempfile::tempdir().unwrap();
        let sh = fake_shell(tmp.path(), "echo 'Welcome back'; echo; echo /x/bin:/y/bin; echo");
        let got = probe_shell_path(&sh, Duration::from_secs(3));
        assert_eq!(got, Some(OsString::from("/x/bin:/y/bin")));
    }

    #[cfg(unix)]
    #[test]
    fn probe_shell_path_times_out_and_kills_the_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let sh = fake_shell(tmp.path(), "exec sleep 30");
        let start = Instant::now();
        let got = probe_shell_path(&sh, Duration::from_millis(300));
        assert_eq!(got, None);
        assert!(start.elapsed() < Duration::from_secs(5), "must not wait for the shell");
    }

    #[cfg(unix)]
    #[test]
    fn probe_shell_path_nonzero_exit_without_output_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let sh = fake_shell(tmp.path(), "exit 3");
        assert_eq!(probe_shell_path(&sh, Duration::from_secs(3)), None);
    }

    #[test]
    fn probe_shell_path_missing_shell_is_none() {
        let got = probe_shell_path(Path::new("/definitely/not/a/shell"), Duration::from_secs(1));
        assert_eq!(got, None);
    }

    #[cfg(unix)]
    #[test]
    fn shell_timeout_still_finds_tool_in_default_location() {
        // The "shell timeout" situation of tasks 5.1 end to end.
        let tmp = tempfile::tempdir().unwrap();
        let sh = fake_shell(tmp.path(), "exec sleep 30");
        let default_dir = tmp.path().join("defaults");
        let exe = make_exe(&default_dir, "kiro-cli");
        let shell_path = probe_shell_path(&sh, Duration::from_millis(200));
        let search = SearchInputs {
            platform: Platform::MacOs,
            shell_path,
            process_path: None,
            defaults: vec![default_dir],
        };
        assert_eq!(find_tool(Tool::KiroCli, &search), Some(exe));
    }

    // ---------- cache ----------

    #[test]
    fn path_cache_probes_once_and_caches_failure_too() {
        let cache = PathCache::default();
        let mut calls = 0;
        let first = cache.get_or_probe(|| {
            calls += 1;
            None
        });
        let second = cache.get_or_probe(|| {
            calls += 1;
            Some(OsString::from("/never"))
        });
        assert_eq!(first, None);
        assert_eq!(second, None);
        assert_eq!(calls, 1);
    }

    #[test]
    fn path_cache_returns_cached_success() {
        let cache = PathCache::default();
        assert_eq!(cache.get_or_probe(|| Some(OsString::from("/a"))), Some(OsString::from("/a")));
        assert_eq!(cache.get_or_probe(|| panic!("must not probe again")), Some(OsString::from("/a")));
    }
}
