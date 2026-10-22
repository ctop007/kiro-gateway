//! End-to-end tests of the install / adopt / remove / status flow with an injected
//! backend (clock, verification, claude lookup) over temporary directories.

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::kclaude::rc::{self, BEGIN_MARKER, END_MARKER};
use crate::kclaude::types::{ErrorCode, KclaudeState, ShellKind, VerifyStatus};

struct FakeBackend {
    stamp: &'static str,
    claude: bool,
    verify_result: RefCell<KclaudeVerify>,
    verify_calls: Cell<u32>,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            stamp: "20261022-101500",
            claude: true,
            verify_result: RefCell::new(KclaudeVerify {
                status: VerifyStatus::Ok,
                detail: "ok".into(),
                fix: None,
            }),
            verify_calls: Cell::new(0),
        }
    }
}

impl Backend for FakeBackend {
    fn stamp(&self) -> String {
        self.stamp.to_string()
    }
    fn claude_installed(&self) -> bool {
        self.claude
    }
    fn verify(&self, _target: &VerifyTarget<'_>) -> KclaudeVerify {
        self.verify_calls.set(self.verify_calls.get() + 1);
        self.verify_result.borrow().clone()
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    data_dir: PathBuf,
    docs: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let data_dir = home.join("Library/Application Support/dev.kirogateway.desktop");
        let docs = home.join("Documents");
        fs::create_dir_all(&home).unwrap();
        Self { _tmp: tmp, home, data_dir, docs }
    }

    fn env(&self, platform: Platform, shell: Option<&str>) -> KclaudeEnv {
        KclaudeEnv {
            home: self.home.clone(),
            data_dir: self.data_dir.clone(),
            shell_var: shell.map(str::to_string),
            platform,
            documents_dir: Some(self.docs.clone()),
        }
    }

    fn zshrc(&self) -> PathBuf {
        self.home.join(".zshrc")
    }

    fn backups(&self, dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .map(|rd| rd.map(|e| e.unwrap().file_name().into_string().unwrap()).collect())
            .unwrap_or_default();
        v.retain(|n| n.contains(".bak-kiro-gateway-"));
        v.sort();
        v
    }
}

fn svc<'a>(env: KclaudeEnv, backend: &'a FakeBackend) -> KclaudeService<'a> {
    KclaudeService::new(env, backend)
}

// --- status / install on macOS zsh ---------------------------------------------------------

#[test]
fn first_run_with_no_zshrc_installs_script_and_block_and_reports_installed() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);

    assert_eq!(s.status().state, KclaudeState::NotInstalled);
    assert_eq!(backend.verify_calls.get(), 0, "no verification before install");

    let status = s.install().unwrap();
    assert_eq!(status.state, KclaudeState::Installed);
    assert_eq!(status.shell, ShellKind::Zsh);
    assert_eq!(status.rc_files, vec!["~/.zshrc".to_string()]);
    assert!(status.claude_installed);
    assert_eq!(status.verify.status, VerifyStatus::Ok);
    assert_eq!(backend.verify_calls.get(), 1);

    let rc_text = fs::read_to_string(fx.zshrc()).unwrap();
    assert!(rc_text.starts_with(BEGIN_MARKER) && rc_text.contains(END_MARKER));
    assert!(rc_text.contains("Application Support"));
    assert!(fx.data_dir.join("kclaude.zsh").exists());
    assert!(fx.backups(&fx.home).is_empty(), "nothing to back up for a new file");
}

#[test]
fn existing_zshrc_is_backed_up_and_only_gains_the_block() {
    let fx = Fixture::new();
    let original = "export PATH=$HOME/bin:$PATH\nalias ll='ls -l'\n";
    fs::write(fx.zshrc(), original).unwrap();
    let backend = FakeBackend::new();
    svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend).install().unwrap();

    let after = fs::read_to_string(fx.zshrc()).unwrap();
    assert!(after.starts_with(original));
    let backups = fx.backups(&fx.home);
    assert_eq!(backups, vec![".zshrc.bak-kiro-gateway-20261022-101500".to_string()]);
    assert_eq!(fs::read_to_string(fx.home.join(&backups[0])).unwrap(), original);
}

#[test]
fn installing_twice_writes_nothing_the_second_time() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    let rc_before = fs::read(fx.zshrc()).unwrap();
    let script_before = fs::read(fx.data_dir.join("kclaude.zsh")).unwrap();
    let mtime = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
    let (rc_m, script_m) = (mtime(&fx.zshrc()), mtime(&fx.data_dir.join("kclaude.zsh")));
    std::thread::sleep(std::time::Duration::from_millis(20));

    let status = s.install().unwrap();

    assert_eq!(status.state, KclaudeState::Installed);
    assert_eq!(fs::read(fx.zshrc()).unwrap(), rc_before);
    assert_eq!(fs::read(fx.data_dir.join("kclaude.zsh")).unwrap(), script_before);
    assert_eq!(mtime(&fx.zshrc()), rc_m);
    assert_eq!(mtime(&fx.data_dir.join("kclaude.zsh")), script_m);
    assert!(fx.backups(&fx.home).is_empty());
}

#[test]
fn stale_script_content_is_refreshed_without_touching_the_startup_file() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    fs::write(fx.data_dir.join("kclaude.zsh"), "old template").unwrap();
    let rc_before = fs::read(fx.zshrc()).unwrap();

    assert_eq!(s.status().state, KclaudeState::Outdated);
    let status = s.install().unwrap();

    assert_eq!(status.state, KclaudeState::Installed);
    assert_eq!(fs::read(fx.zshrc()).unwrap(), rc_before);
    assert_ne!(fs::read_to_string(fx.data_dir.join("kclaude.zsh")).unwrap(), "old template");
    assert!(fx.backups(&fx.home).is_empty());
}

#[test]
fn data_dir_move_rewrites_the_block_after_a_backup() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend).install().unwrap();

    let mut moved = fx.env(Platform::MacOs, Some("/bin/zsh"));
    moved.data_dir = fx.home.join("elsewhere dir");
    let s = svc(moved, &backend);
    assert_eq!(s.status().state, KclaudeState::Outdated);
    assert_eq!(s.install().unwrap().state, KclaudeState::Installed);

    let text = fs::read_to_string(fx.zshrc()).unwrap();
    assert!(text.contains("elsewhere dir") && !text.contains("Application Support"));
    assert_eq!(text.matches(BEGIN_MARKER).count(), 1);
    assert_eq!(fx.backups(&fx.home).len(), 1);
}

// --- foreign / adopt ----------------------------------------------------------------------

#[test]
fn foreign_kclaude_blocks_install_and_leaves_every_file_untouched() {
    let fx = Fixture::new();
    let original = "source \"$HOME/kiro-gateway/scripts/kclaude.zsh\"\n";
    fs::write(fx.zshrc(), original).unwrap();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);

    assert_eq!(s.status().state, KclaudeState::Foreign);
    let err = s.install().unwrap_err();

    assert_eq!(err.code, ErrorCode::ForeignPresent);
    assert_eq!(fs::read_to_string(fx.zshrc()).unwrap(), original);
    assert!(fx.backups(&fx.home).is_empty());
    assert!(!fx.data_dir.join("kclaude.zsh").exists(), "no script is written when install is refused");
    assert_eq!(backend.verify_calls.get(), 0);
}

#[test]
fn adopt_comments_out_the_old_line_adds_the_block_and_backs_up_first() {
    let fx = Fixture::new();
    let original = "export A=1\nsource \"$HOME/kiro-gateway/scripts/kclaude.zsh\"\n";
    fs::write(fx.zshrc(), original).unwrap();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);

    let status = s.adopt().unwrap();

    assert_eq!(status.state, KclaudeState::Installed);
    let after = fs::read_to_string(fx.zshrc()).unwrap();
    assert!(after.contains("# disabled by Kiro Gateway App: source \"$HOME/kiro-gateway/scripts/kclaude.zsh\"\n"));
    assert!(after.contains(BEGIN_MARKER));
    let backups = fx.backups(&fx.home);
    assert_eq!(backups.len(), 1, "one backup for the whole adopt, not one per step");
    assert_eq!(fs::read_to_string(fx.home.join(&backups[0])).unwrap(), original);
    assert!(fx.data_dir.join("kclaude.zsh").exists());
}

#[test]
fn adopt_without_a_foreign_kclaude_is_invalid_argument() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    for _ in 0..2 {
        let err = s.adopt().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        s.install().unwrap();
    }
}

#[test]
fn adopt_backup_failure_changes_nothing() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fixture::new();
        let original = "alias kclaude=claude\n";
        fs::write(fx.zshrc(), original).unwrap();
        fs::create_dir_all(&fx.data_dir).unwrap(); // the script can be written; only home is read-only
        let backend = FakeBackend::new();
        let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
        fs::set_permissions(&fx.home, fs::Permissions::from_mode(0o555)).unwrap();
        let root = fs::write(fx.home.join("probe"), b"").is_ok();
        let result = s.adopt();
        fs::set_permissions(&fx.home, fs::Permissions::from_mode(0o755)).unwrap();
        if !root {
            assert_eq!(result.unwrap_err().code, ErrorCode::BackupFailed);
            assert_eq!(fs::read_to_string(fx.zshrc()).unwrap(), original);
        }
    }
}

#[test]
fn damaged_markers_are_reported_as_foreign_and_never_edited() {
    let fx = Fixture::new();
    let original = format!("{BEGIN_MARKER}\nsource x\n");
    fs::write(fx.zshrc(), &original).unwrap();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    assert_eq!(s.status().state, KclaudeState::Foreign);
    assert_eq!(s.install().unwrap_err().code, ErrorCode::ForeignPresent);
    let err = s.adopt().unwrap_err();
    assert_eq!(err.code, ErrorCode::WriteFailed);
    assert!(err.message.contains("标记"), "{}", err.message);
    assert_eq!(s.remove().unwrap_err().code, ErrorCode::WriteFailed);
    assert_eq!(fs::read_to_string(fx.zshrc()).unwrap(), original);
}

// --- remove --------------------------------------------------------------------------------

#[test]
fn remove_restores_the_original_bytes_exactly_and_reports_not_installed() {
    let fx = Fixture::new();
    let original = b"# top\r\nexport A=1\r\n\n\xff raw byte\nlast line without newline";
    fs::write(fx.zshrc(), original).unwrap();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();

    let status = s.remove().unwrap();

    assert_eq!(status.state, KclaudeState::NotInstalled);
    assert_eq!(status.verify.status, VerifyStatus::Skipped);
    assert_eq!(fs::read(fx.zshrc()).unwrap(), original);
    assert_eq!(fx.backups(&fx.home).len(), 2, "one backup for install, one for remove (same second gets a -1 suffix)");
}

#[test]
fn remove_when_not_installed_is_a_no_op() {
    let fx = Fixture::new();
    fs::write(fx.zshrc(), "keep\n").unwrap();
    let backend = FakeBackend::new();
    let status = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend).remove().unwrap();
    assert_eq!(status.state, KclaudeState::NotInstalled);
    assert_eq!(fs::read_to_string(fx.zshrc()).unwrap(), "keep\n");
    assert!(fx.backups(&fx.home).is_empty());
}

#[test]
fn remove_with_a_foreign_kclaude_still_removes_only_our_block() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    let mut text = fs::read_to_string(fx.zshrc()).unwrap();
    text.insert_str(0, "source ~/x/kclaude.zsh\n");
    fs::write(fx.zshrc(), &text).unwrap();
    s.remove().unwrap();
    assert_eq!(fs::read_to_string(fx.zshrc()).unwrap(), "source ~/x/kclaude.zsh\n");
}

// --- shell selection -----------------------------------------------------------------------

#[test]
fn bash_user_on_macos_gets_bash_profile_and_the_bash_script() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/opt/homebrew/bin/bash")), &backend);
    let status = s.install().unwrap();
    assert_eq!(status.shell, ShellKind::Bash);
    assert_eq!(status.rc_files, vec!["~/.bash_profile".to_string()]);
    assert!(fx.home.join(".bash_profile").exists());
    assert!(!fx.zshrc().exists());
    assert!(fx.data_dir.join("kclaude.bash").exists());
    assert!(fs::read_to_string(fx.home.join(".bash_profile")).unwrap().contains("kclaude.bash"));
}

#[test]
fn bash_user_whose_profile_sources_bashrc_gets_bashrc() {
    let fx = Fixture::new();
    fs::write(fx.home.join(".bash_profile"), "[ -f ~/.bashrc ] && . ~/.bashrc\n").unwrap();
    let backend = FakeBackend::new();
    let status = svc(fx.env(Platform::MacOs, Some("/bin/bash")), &backend).install().unwrap();
    assert_eq!(status.rc_files, vec!["~/.bashrc".to_string()]);
    assert_eq!(
        fs::read_to_string(fx.home.join(".bash_profile")).unwrap(),
        "[ -f ~/.bashrc ] && . ~/.bashrc\n"
    );
}

#[test]
fn unknown_shell_falls_back_to_zsh() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let status = svc(fx.env(Platform::MacOs, Some("/usr/bin/fish")), &backend).install().unwrap();
    assert_eq!(status.shell, ShellKind::Zsh);
}

// --- status extras --------------------------------------------------------------------------

#[test]
fn claude_installed_flag_comes_from_the_backend() {
    let fx = Fixture::new();
    let mut backend = FakeBackend::new();
    backend.claude = false;
    let status = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend).status();
    assert!(!status.claude_installed);
}

#[test]
fn failed_verification_is_returned_as_is_after_install() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    *backend.verify_result.borrow_mut() = KclaudeVerify {
        status: VerifyStatus::Fail,
        detail: "新开的终端里找不到 kclaude".into(),
        fix: None,
    };
    let status = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend).install().unwrap();
    assert_eq!(status.state, KclaudeState::Installed);
    assert_eq!(status.verify.status, VerifyStatus::Fail);
}

#[test]
fn status_of_an_installed_setup_runs_verify_and_status_of_foreign_does_not() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    backend.verify_calls.set(0);
    fs::remove_file(fx.data_dir.join(VERIFY_CACHE)).unwrap();
    assert_eq!(s.status().verify.status, VerifyStatus::Ok);
    assert_eq!(backend.verify_calls.get(), 1);

    fs::write(fx.zshrc(), "alias kclaude=claude\n").unwrap();
    backend.verify_calls.set(0);
    let status = s.status();
    assert_eq!(status.state, KclaudeState::Foreign);
    assert_eq!(status.verify.status, VerifyStatus::Skipped);
    assert_eq!(backend.verify_calls.get(), 0);
}

#[test]
fn unreadable_startup_file_is_an_internal_status_not_a_panic() {
    let fx = Fixture::new();
    fs::create_dir(fx.zshrc()).unwrap(); // a directory where the file should be
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    assert_eq!(s.install().unwrap_err().code, ErrorCode::WriteFailed);
}

#[test]
fn rc_display_path_abbreviates_home_and_leaves_other_paths_alone() {
    let home = Path::new("/Users/me");
    assert_eq!(display_path(home, Path::new("/Users/me/.zshrc")), "~/.zshrc");
    assert_eq!(display_path(home, Path::new("/Users/me")), "~");
    assert_eq!(display_path(home, Path::new("/Users/meow/.zshrc")), "/Users/meow/.zshrc");
    assert_eq!(display_path(home, Path::new("/etc/zshrc")), "/etc/zshrc");
}

// --- startup auto-install -------------------------------------------------------------------

#[test]
fn auto_install_installs_when_missing_and_skips_foreign() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    let status = s.auto_install();
    assert_eq!(status.state, KclaudeState::Installed);

    let fx2 = Fixture::new();
    let original = "kclaude() { claude; }\n";
    fs::write(fx2.zshrc(), original).unwrap();
    let s2 = svc(fx2.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    let status2 = s2.auto_install();
    assert_eq!(status2.state, KclaudeState::Foreign);
    assert_eq!(fs::read_to_string(fx2.zshrc()).unwrap(), original);
}

#[test]
fn auto_install_does_not_resurrect_a_block_the_user_removed_when_the_script_is_current() {
    // auto_install runs at every App start; a removed block would come back each time.
    // The service therefore records removal in a marker file next to the scripts.
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    s.remove().unwrap();
    assert_eq!(s.auto_install().state, KclaudeState::NotInstalled);
    assert!(!fx.zshrc().exists() || !fs::read_to_string(fx.zshrc()).unwrap().contains(BEGIN_MARKER));
    // An explicit install brings it back and clears the marker.
    assert_eq!(s.install().unwrap().state, KclaudeState::Installed);
    assert_eq!(s.auto_install().state, KclaudeState::Installed);
}

// --- Windows: two profiles --------------------------------------------------------------------

#[test]
fn windows_install_writes_both_profiles_and_the_ps1_script() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::Windows, None), &backend);

    let status = s.install().unwrap();

    assert_eq!(status.shell, ShellKind::Powershell);
    assert_eq!(status.state, KclaudeState::Installed);
    assert_eq!(
        status.rc_files,
        vec![
            "~/Documents/WindowsPowerShell/profile.ps1".to_string(),
            "~/Documents/PowerShell/profile.ps1".to_string()
        ]
    );
    let ps5 = fs::read_to_string(fx.docs.join("WindowsPowerShell/profile.ps1")).unwrap();
    let ps7 = fs::read_to_string(fx.docs.join("PowerShell/profile.ps1")).unwrap();
    assert_eq!(ps5, ps7);
    assert!(ps5.contains("kclaude.ps1") && ps5.contains("Test-Path -LiteralPath"));
    let script = fs::read(fx.data_dir.join("kclaude.ps1")).unwrap();
    assert!(script.starts_with(b"\xEF\xBB\xBF"));
}

#[test]
fn windows_existing_profile_content_is_kept_and_backed_up() {
    let fx = Fixture::new();
    let p5 = fx.docs.join("WindowsPowerShell/profile.ps1");
    fs::create_dir_all(p5.parent().unwrap()).unwrap();
    fs::write(&p5, "Set-Alias ll Get-ChildItem\r\n").unwrap();
    let backend = FakeBackend::new();
    svc(fx.env(Platform::Windows, None), &backend).install().unwrap();
    assert!(fs::read_to_string(&p5).unwrap().starts_with("Set-Alias ll Get-ChildItem\r\n"));
    assert_eq!(fx.backups(p5.parent().unwrap()).len(), 1);
    assert!(fx.backups(&fx.docs.join("PowerShell")).is_empty());
}

#[test]
fn windows_foreign_in_one_profile_blocks_both_and_writes_nothing() {
    let fx = Fixture::new();
    let p7 = fx.docs.join("PowerShell/profile.ps1");
    fs::create_dir_all(p7.parent().unwrap()).unwrap();
    fs::write(&p7, "function kclaude { claude @args }\n").unwrap();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::Windows, None), &backend);
    assert_eq!(s.status().state, KclaudeState::Foreign);
    assert_eq!(s.install().unwrap_err().code, ErrorCode::ForeignPresent);
    assert!(!fx.docs.join("WindowsPowerShell/profile.ps1").exists());
}

#[test]
fn windows_adopt_comments_out_the_function_in_one_profile_and_installs_both() {
    let fx = Fixture::new();
    let p7 = fx.docs.join("PowerShell/profile.ps1");
    fs::create_dir_all(p7.parent().unwrap()).unwrap();
    fs::write(&p7, "function kclaude {\n  claude @args\n}\n").unwrap();
    let backend = FakeBackend::new();
    let status = svc(fx.env(Platform::Windows, None), &backend).adopt().unwrap();
    assert_eq!(status.state, KclaudeState::Installed);
    let text = fs::read_to_string(&p7).unwrap();
    assert!(text.starts_with("# disabled by Kiro Gateway App: function kclaude {\n"));
    assert!(fx.docs.join("WindowsPowerShell/profile.ps1").exists());
}

#[test]
fn windows_one_profile_missing_the_block_is_outdated_and_install_repairs_it() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::Windows, None), &backend);
    s.install().unwrap();
    fs::remove_file(fx.docs.join("PowerShell/profile.ps1")).unwrap();
    assert_eq!(s.status().state, KclaudeState::Outdated);
    assert_eq!(s.install().unwrap().state, KclaudeState::Installed);
}

#[test]
fn windows_remove_strips_both_profiles() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::Windows, None), &backend);
    s.install().unwrap();
    let status = s.remove().unwrap();
    assert_eq!(status.state, KclaudeState::NotInstalled);
    for p in [fx.docs.join("WindowsPowerShell/profile.ps1"), fx.docs.join("PowerShell/profile.ps1")] {
        assert!(!fs::read_to_string(p).unwrap().contains(BEGIN_MARKER));
    }
}

#[test]
fn windows_without_a_documents_folder_reports_an_error_not_a_guess() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let mut env = fx.env(Platform::Windows, None);
    env.documents_dir = None;
    let err = svc(env, &backend).install().unwrap_err();
    assert_eq!(err.code, ErrorCode::WriteFailed);
    assert!(err.message.contains("文档"), "{}", err.message);
}

#[test]
fn utf16_profile_is_left_alone() {
    let fx = Fixture::new();
    let p5 = fx.docs.join("WindowsPowerShell/profile.ps1");
    fs::create_dir_all(p5.parent().unwrap()).unwrap();
    let utf16: Vec<u8> = [0xFFu8, 0xFE].into_iter().chain("Get-Date\n".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
    fs::write(&p5, &utf16).unwrap();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::Windows, None), &backend);
    assert_eq!(s.install().unwrap_err().code, ErrorCode::WriteFailed);
    assert_eq!(fs::read(&p5).unwrap(), utf16);
    assert!(!fx.docs.join("PowerShell/profile.ps1").exists(), "all-or-nothing across profiles");
}

#[test]
fn rc_module_is_reachable_for_block_expectations() {
    // Guards the public surface the commands layer depends on.
    let _ = rc::BEGIN_MARKER;
}


// --- verification is cached ----------------------------------------------------------------

#[test]
fn consecutive_status_calls_do_not_start_a_login_shell_again() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    assert_eq!(backend.verify_calls.get(), 1, "install runs the check once");

    for _ in 0..3 {
        let status = s.status();
        assert_eq!(status.state, KclaudeState::Installed);
        assert_eq!(status.verify.status, VerifyStatus::Ok);
    }
    assert_eq!(backend.verify_calls.get(), 1, "status served from the cache");
}

#[test]
fn cached_result_survives_a_new_service_instance_like_an_app_restart() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend).install().unwrap();
    let later = FakeBackend::new();
    let status = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &later).status();
    assert_eq!(status.verify.status, VerifyStatus::Ok);
    assert_eq!(later.verify_calls.get(), 0);
}

#[test]
fn a_failed_check_is_cached_too_until_the_startup_file_changes() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    *backend.verify_result.borrow_mut() = KclaudeVerify {
        status: VerifyStatus::Fail,
        detail: "新开的终端里找不到 kclaude".into(),
        fix: None,
    };
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    assert_eq!(s.status().verify.status, VerifyStatus::Fail);
    assert_eq!(backend.verify_calls.get(), 1);
}

#[test]
fn rc_file_change_triggers_a_new_check() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    assert_eq!(backend.verify_calls.get(), 1);

    // The user edits the startup file below the managed block: mtime and size change.
    let mut text = fs::read_to_string(fx.zshrc()).unwrap();
    text.push_str("export EDITED_LATER=1\n");
    fs::write(fx.zshrc(), text).unwrap();
    *backend.verify_result.borrow_mut() = KclaudeVerify {
        status: VerifyStatus::Fail,
        detail: "新开的终端里 kclaude 被别名盖住了".into(),
        fix: None,
    };
    let status = s.status();
    assert_eq!(backend.verify_calls.get(), 2, "re-checked after the file changed");
    assert_eq!(status.verify.status, VerifyStatus::Fail);
    s.status();
    assert_eq!(backend.verify_calls.get(), 2, "and cached again");
}

#[test]
fn mtime_only_change_with_same_size_triggers_a_new_check() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    let file = fs::OpenOptions::new().write(true).open(fx.zshrc()).unwrap();
    let new_time = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    file.set_modified(new_time).unwrap();
    drop(file);
    s.status();
    assert_eq!(backend.verify_calls.get(), 2);
}

#[test]
fn corrupt_cache_file_is_ignored_and_rewritten() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    fs::write(fx.data_dir.join(VERIFY_CACHE), b"{ not json").unwrap();
    assert_eq!(s.status().verify.status, VerifyStatus::Ok);
    assert_eq!(backend.verify_calls.get(), 2);
    s.status();
    assert_eq!(backend.verify_calls.get(), 2);
}

#[test]
fn reinstall_after_outdated_script_forces_a_fresh_check() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    fs::write(fx.data_dir.join("kclaude.zsh"), "stale script\n").unwrap();
    assert_eq!(s.status().state, KclaudeState::Outdated);
    assert_eq!(backend.verify_calls.get(), 1, "status alone does not re-check");
    // App start repairs the script: that is an install, so the shell is asked again.
    assert_eq!(s.auto_install().state, KclaudeState::Installed);
    assert_eq!(backend.verify_calls.get(), 2);
}

#[test]
fn adopt_forces_a_fresh_check() {
    let fx = Fixture::new();
    fs::write(fx.zshrc(), "source \"$HOME/kiro-gateway/scripts/kclaude.zsh\"\n").unwrap();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.adopt().unwrap();
    assert_eq!(backend.verify_calls.get(), 1);
    s.status();
    assert_eq!(backend.verify_calls.get(), 1);
}

#[test]
fn status_without_a_cache_checks_once_then_caches() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let s = svc(fx.env(Platform::MacOs, Some("/bin/zsh")), &backend);
    s.install().unwrap();
    fs::remove_file(fx.data_dir.join(VERIFY_CACHE)).unwrap();
    s.status();
    s.status();
    assert_eq!(backend.verify_calls.get(), 2);
}
