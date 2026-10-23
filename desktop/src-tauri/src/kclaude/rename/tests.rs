//! Tests for changing the command name: validation without side effects, the in-place
//! rewrite of the startup-file block, the legacy `kclaude` block migration, and the
//! coexistence with a user's own `source .../scripts/kclaude.zsh`.
//!
//! Real `zsh` / `bash` login shells run against a temporary HOME. Nothing here touches the
//! developer's own startup files or the App's real data directory.

use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDate;

use super::*;
use crate::app_config::{AppPaths, ConfigStore};
use crate::kclaude::rc::{BEGIN_MARKER, END_MARKER, LEGACY_BEGIN_MARKER, LEGACY_END_MARKER};
use crate::kclaude::service::{executable_in_dirs, Platform, VerifyTarget};
use crate::kclaude::types::{KclaudeState, KclaudeVerify, VerifyStatus};
use crate::kclaude::verify;

const MSG_FORMAT: &str = "命令名只能用字母、数字、- 和 _，以字母开头，2 到 32 个字符";
const MSG_RESERVED: &str = "这个名字是系统命令，换一个";
const MSG_TAKEN: &str = "这个名字已经被占用";

/// A backend that runs the real login shell check (without `ZDOTDIR`, so a developer's
/// own setup cannot leak in) and looks names up in a fixed list of directories.
struct RealBackend {
    path_dirs: Vec<PathBuf>,
}

impl Backend for RealBackend {
    fn stamp(&self) -> String {
        "20261023-120000".to_string()
    }
    fn claude_installed(&self) -> bool {
        true
    }
    fn verify(&self, target: &VerifyTarget<'_>) -> KclaudeVerify {
        match target {
            VerifyTarget::Posix { shell, home, command } => {
                let args = verify::login_type_args(command);
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let outcome =
                    verify::run_shell_with_env(shell, &refs, home, Duration::from_secs(20), &[("ZDOTDIR", None)]);
                verify::verify_posix(&outcome, command)
            }
            VerifyTarget::PowerShell { .. } => panic!("no PowerShell in these tests"),
        }
    }
    fn on_path(&self, name: &str) -> bool {
        executable_in_dirs(&self.path_dirs, name, false)
    }
}

/// A backend that records calls instead of starting shells.
struct FakeBackend {
    path_names: RefCell<Vec<String>>,
    verified: RefCell<Vec<String>>,
}

impl FakeBackend {
    fn new() -> Self {
        Self { path_names: RefCell::new(Vec::new()), verified: RefCell::new(Vec::new()) }
    }
}

impl Backend for FakeBackend {
    fn stamp(&self) -> String {
        "20261023-120000".to_string()
    }
    fn claude_installed(&self) -> bool {
        true
    }
    fn verify(&self, target: &VerifyTarget<'_>) -> KclaudeVerify {
        let command = match target {
            VerifyTarget::Posix { command, .. } | VerifyTarget::PowerShell { command } => *command,
        };
        self.verified.borrow_mut().push(command.to_string());
        KclaudeVerify { status: VerifyStatus::Ok, detail: "ok".into(), fix: None }
    }
    fn on_path(&self, name: &str) -> bool {
        self.path_names.borrow().iter().any(|n| n == name)
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    data_dir: PathBuf,
    docs: PathBuf,
    store: ConfigStore,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let data_dir = home.join("Library/Application Support/dev.kirogateway.desktop");
        let docs = home.join("Documents");
        fs::create_dir_all(&home).unwrap();
        let now = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap().and_hms_opt(12, 0, 0).unwrap();
        let (store, _) = ConfigStore::open(AppPaths::new(&data_dir), now).unwrap();
        // A port nothing listens on, so a script run here can never reach a real gateway.
        let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        let port = if port < 1024 { 40123 } else { port };
        store.update(&ConfigPatch { port: Some(i64::from(port)), ..Default::default() }, false).unwrap();
        Self { _tmp: tmp, home, data_dir, docs, store }
    }

    fn env(&self, shell: &str) -> KclaudeEnv {
        self.env_on(Platform::MacOs, shell)
    }

    fn env_on(&self, platform: Platform, shell: &str) -> KclaudeEnv {
        KclaudeEnv {
            home: self.home.clone(),
            data_dir: self.data_dir.clone(),
            command: self.store.get().command_name,
            shell_var: Some(shell.to_string()),
            platform,
            documents_dir: Some(self.docs.clone()),
        }
    }

    fn zshrc(&self) -> PathBuf {
        self.home.join(".zshrc")
    }

    fn rc_text(&self, name: &str) -> String {
        fs::read_to_string(self.home.join(name)).unwrap_or_default()
    }

    fn backups(&self, dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .map(|rd| rd.map(|e| e.unwrap().file_name().into_string().unwrap()).collect())
            .unwrap_or_default();
        v.retain(|n| n.contains(".bak-kiro-gateway-"));
        v.sort();
        v
    }

    fn script_files(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(&self.data_dir)
            .map(|rd| rd.map(|e| e.unwrap().file_name().into_string().unwrap()).collect())
            .unwrap_or_default();
        v.retain(|n| n.ends_with(".zsh") || n.ends_with(".bash") || n.ends_with(".ps1"));
        v.sort();
        v
    }

    fn install(&self, shell: &str, backend: &dyn Backend) -> KclaudeStatus {
        KclaudeService::new(self.env(shell), backend).install().unwrap()
    }

    fn rename(&self, shell: &str, backend: &dyn Backend, name: &str) -> Result<UpdateOutcome, ConfigError> {
        let patch = ConfigPatch { command_name: Some(name.to_string()), ..Default::default() };
        apply_update(&self.store, &self.env(shell), backend, &patch, false)
    }

    fn config_bytes(&self) -> Vec<u8> {
        fs::read(AppPaths::new(&self.data_dir).config_file()).unwrap()
    }
}

use super::super::types::KclaudeStatus;

fn type_in_login_shell(shell: &str, home: &Path, command: &str) -> String {
    let args = verify::login_type_args(command);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match verify::run_shell_with_env(Path::new(shell), &refs, home, Duration::from_secs(20), &[("ZDOTDIR", None)]) {
        verify::RunOutcome::Finished { stdout, stderr, .. } => format!("{stdout}{stderr}"),
        other => panic!("{other:?}"),
    }
}

fn run_in_login_shell(shell: &str, home: &Path, script: &str) -> (Option<i32>, String, String) {
    match verify::run_shell_with_env(Path::new(shell), &["-lic", script], home, Duration::from_secs(30), &[("ZDOTDIR", None)]) {
        verify::RunOutcome::Finished { code, stdout, stderr } => (code, stdout, stderr),
        other => panic!("{other:?}"),
    }
}

// ---- the happy path with real shells -------------------------------------------------------

#[test]
fn rename_with_real_zsh_and_bash_new_name_works_old_name_is_gone_one_block_and_a_backup() {
    for (shell, rc_name, script_ext) in [("/bin/zsh", ".zshrc", "zsh"), ("/bin/bash", ".bash_profile", "bash")] {
        let fx = Fixture::new();
        let backend = RealBackend { path_dirs: vec![] };
        let original = "# my rc\nexport EDITOR=vim\nalias ll='ls -l'\n";
        fs::write(fx.home.join(rc_name), original).unwrap();

        let installed = fx.install(shell, &backend);
        assert_eq!(installed.state, KclaudeState::Installed, "{shell}");
        assert_eq!(installed.command_name, "cclaude");
        assert!(type_in_login_shell(shell, &fx.home, "cclaude").contains("function"), "{shell}");

        let outcome = fx.rename(shell, &backend, "kclaude-test").unwrap();
        assert_eq!(outcome.config.command_name, "kclaude-test");

        // Startup file: the user's lines, exactly one App block, pointing at the new script.
        let rc = fx.rc_text(rc_name);
        assert!(rc.starts_with(original), "{shell}: {rc}");
        assert_eq!(rc.matches(BEGIN_MARKER).count(), 1, "{shell}: {rc}");
        assert_eq!(rc.matches(END_MARKER).count(), 1, "{shell}");
        assert!(rc.contains(&format!("kclaude-test.{script_ext}")), "{shell}: {rc}");
        assert!(!rc.contains(&format!("cclaude.{script_ext}")), "{shell}: {rc}");

        // Scripts: new one written, old one deleted, kclaude.env untouched in name.
        assert_eq!(fx.script_files(), vec![format!("kclaude-test.{script_ext}")], "{shell}");
        assert!(fx.data_dir.join("kclaude.env").exists());

        // A backup of the pre-rename file exists and holds the old block.
        let backups = fx.backups(&fx.home);
        assert!(!backups.is_empty(), "{shell}");
        let newest_with_old_block = backups
            .iter()
            .any(|b| fs::read_to_string(fx.home.join(b)).unwrap().contains(&format!("cclaude.{script_ext}")));
        assert!(newest_with_old_block, "{shell}: no backup holds the old block: {backups:?}");

        // A fresh login shell: new name is a function, old name is gone.
        let new_type = type_in_login_shell(shell, &fx.home, "kclaude-test");
        assert!(new_type.contains("kclaude-test is a") && new_type.contains("function"), "{shell}: {new_type}");
        let old_type = type_in_login_shell(shell, &fx.home, "cclaude");
        assert!(!old_type.contains("cclaude is a"), "{shell}: old name still defined: {old_type}");

        // The service agrees, and the status names the new command.
        let status = KclaudeService::new(fx.env(shell), &backend).status();
        assert_eq!(status.command_name, "kclaude-test");
        assert_eq!(status.state, KclaudeState::Installed, "{shell}");
        assert_eq!(status.verify.status, VerifyStatus::Ok, "{shell}: {:?}", status.verify);
    }
}

#[test]
fn the_renamed_command_prints_its_own_name_in_errors_when_run_in_a_real_shell() {
    for shell in ["/bin/zsh", "/bin/bash"] {
        let fx = Fixture::new();
        let backend = RealBackend { path_dirs: vec![] };
        let rc_name = if shell.ends_with("zsh") { ".zshrc" } else { ".bash_profile" };
        fs::write(fx.home.join(rc_name), "export A=1\n").unwrap();
        fx.install(shell, &backend);
        fx.rename(shell, &backend, "kclaude-test").unwrap();
        let (code, _out, err) = run_in_login_shell(shell, &fx.home, "kclaude-test --version");
        assert_ne!(code, Some(0), "{shell}");
        assert!(err.contains("kclaude-test: KiroGateway 未运行，请在 Kiro Gateway App 里打开开关"), "{shell}: {err}");
    }
}

#[test]
fn renaming_back_and_forth_always_leaves_one_block_and_one_script() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(fx.zshrc(), "export A=1\n").unwrap();
    fx.install("/bin/zsh", &backend);
    for name in ["kclaude-test", "cclaude", "my_cc", "cclaude"] {
        fx.rename("/bin/zsh", &backend, name).unwrap();
        let rc = fx.rc_text(".zshrc");
        assert_eq!(rc.matches(BEGIN_MARKER).count(), 1, "{name}: {rc}");
        assert!(rc.starts_with("export A=1\n"), "{name}");
        assert_eq!(fx.script_files(), vec![format!("{name}.zsh")], "{name}");
        assert_eq!(fx.store.get().command_name, name);
    }
}

#[test]
fn renaming_to_the_current_name_writes_nothing() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(fx.zshrc(), "export A=1\n").unwrap();
    fx.install("/bin/zsh", &backend);
    let rc_before = fs::read(fx.zshrc()).unwrap();
    let config_before = fx.config_bytes();
    let backups_before = fx.backups(&fx.home);
    fx.rename("/bin/zsh", &backend, "cclaude").unwrap();
    assert_eq!(fs::read(fx.zshrc()).unwrap(), rc_before);
    assert_eq!(fx.config_bytes(), config_before);
    assert_eq!(fx.backups(&fx.home), backups_before);
}

#[test]
fn a_patch_without_a_command_name_never_touches_the_startup_file() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(fx.zshrc(), "export A=1\n").unwrap();
    fx.install("/bin/zsh", &backend);
    let rc_before = fs::read(fx.zshrc()).unwrap();
    let patch = ConfigPatch { port: Some(9123), ..Default::default() };
    let outcome = apply_update(&fx.store, &fx.env("/bin/zsh"), &backend, &patch, true).unwrap();
    assert_eq!(outcome.config.port, 9123);
    assert!(outcome.restart_required);
    assert_eq!(fs::read(fx.zshrc()).unwrap(), rc_before);
    assert!(backend.verified.borrow().len() <= 1, "no login shell started by a port change");
}

#[test]
fn a_combined_patch_applies_port_and_name_together() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fx.install("/bin/zsh", &backend);
    let patch = ConfigPatch { port: Some(9124), command_name: Some("kclaude-test".into()), ..Default::default() };
    let outcome = apply_update(&fx.store, &fx.env("/bin/zsh"), &backend, &patch, false).unwrap();
    assert_eq!((outcome.config.port, outcome.config.command_name.as_str()), (9124, "kclaude-test"));
    let env_file = fs::read_to_string(fx.data_dir.join("kclaude.env")).unwrap();
    assert!(env_file.contains("KIRO_GATEWAY_PORT=9124\n") && env_file.contains("KIRO_GATEWAY_COMMAND=kclaude-test\n"));
}

// ---- refusals leave everything alone ------------------------------------------------------

fn snapshot(fx: &Fixture) -> (Vec<u8>, Vec<u8>, Vec<String>, Vec<String>, Vec<u8>) {
    (
        fs::read(fx.zshrc()).unwrap_or_default(),
        fx.config_bytes(),
        fx.script_files(),
        fx.backups(&fx.home),
        fs::read(fx.data_dir.join("kclaude.env")).unwrap(),
    )
}

#[test]
fn invalid_names_are_refused_with_the_reason_and_nothing_is_written() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(fx.zshrc(), "export A=1\n").unwrap();
    fx.install("/bin/zsh", &backend);
    let before = snapshot(&fx);

    let cases = [
        ("claude", MSG_RESERVED),
        ("Claude", MSG_RESERVED),
        ("ls", MSG_RESERVED),
        ("git", MSG_RESERVED),
        ("kiro-cli", MSG_RESERVED),
        ("9abc", MSG_FORMAT),
        ("a b", MSG_FORMAT),
        (" cc", MSG_FORMAT),
        ("x", MSG_FORMAT),
        ("", MSG_FORMAT),
        ("rm -rf", MSG_FORMAT),
        ("a;b", MSG_FORMAT),
        ("$(id)", MSG_FORMAT),
        ("名字名字", MSG_FORMAT),
        (&"x".repeat(33), MSG_FORMAT),
    ];
    for (name, message) in cases {
        let err = fx.rename("/bin/zsh", &backend, name).unwrap_err();
        assert_eq!(err.code(), "invalid_argument", "{name:?}");
        assert_eq!(err.message(), message, "{name:?}");
        assert_eq!(snapshot(&fx), before, "{name:?} changed something on disk");
        assert_eq!(fx.store.get().command_name, "cclaude", "{name:?}");
    }
}

#[test]
fn a_name_that_is_an_executable_on_the_path_is_taken() {
    let fx = Fixture::new();
    let bin = fx.home.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tool = bin.join("mytool");
    fs::write(&tool, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    // A plain file without the executable bit does not occupy the name.
    fs::write(bin.join("just-data"), "x").unwrap();
    let backend = RealBackend { path_dirs: vec![bin.clone(), PathBuf::from("relative/dir")] };
    fs::write(fx.zshrc(), "export A=1\n").unwrap();
    fx.install("/bin/zsh", &backend);
    let before = snapshot(&fx);

    let err = fx.rename("/bin/zsh", &backend, "mytool").unwrap_err();
    assert_eq!((err.code(), err.message()), ("invalid_argument", MSG_TAKEN));
    assert_eq!(snapshot(&fx), before);

    fx.rename("/bin/zsh", &backend, "just-data").unwrap();
    assert_eq!(fx.store.get().command_name, "just-data");
}

#[test]
fn a_name_defined_in_the_startup_file_is_taken_alias_function_and_source() {
    for line in [
        "alias taken-name='echo hi'",
        "taken-name() { echo hi; }",
        "function taken-name { echo hi; }",
        "source ~/scripts/taken-name.zsh",
        "[ -f ~/x/taken-name.sh ] && . ~/x/taken-name.sh",
    ] {
        let fx = Fixture::new();
        let backend = FakeBackend::new();
        fs::write(fx.zshrc(), "export A=1\n").unwrap();
        fx.install("/bin/zsh", &backend);
        let mut rc = fx.rc_text(".zshrc");
        rc.push_str(line);
        rc.push('\n');
        fs::write(fx.zshrc(), &rc).unwrap();
        let before = snapshot(&fx);

        let err = fx.rename("/bin/zsh", &backend, "taken-name").unwrap_err();
        assert_eq!((err.code(), err.message()), ("invalid_argument", MSG_TAKEN), "{line}");
        assert_eq!(snapshot(&fx), before, "{line}");
        assert_eq!(fx.store.get().command_name, "cclaude", "{line}");
    }
}

#[test]
fn commented_out_or_similar_looking_lines_do_not_occupy_a_name() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(
        fx.zshrc(),
        "# alias free-name=ls\nalias free-name2=ls\nfree-name-helper() { :; }\nsource ~/.nvm/nvm.sh\n",
    )
    .unwrap();
    fx.install("/bin/zsh", &backend);
    fx.rename("/bin/zsh", &backend, "free-name").unwrap();
    assert_eq!(fx.store.get().command_name, "free-name");
}

#[test]
fn the_apps_own_block_never_counts_as_an_occupant() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fx.install("/bin/zsh", &backend);
    // The block mentions cclaude.zsh; switching away and back must not trip over it.
    fx.rename("/bin/zsh", &backend, "kclaude-test").unwrap();
    fx.rename("/bin/zsh", &backend, "cclaude").unwrap();
    assert_eq!(fx.store.get().command_name, "cclaude");
}

// ---- user's own scripts/kclaude.zsh -----------------------------------------------------------

#[test]
fn manual_source_of_scripts_kclaude_zsh_does_not_block_cclaude_and_stays_untouched() {
    for (shell, rc_name) in [("/bin/zsh", ".zshrc"), ("/bin/bash", ".bash_profile")] {
        let fx = Fixture::new();
        let backend = RealBackend { path_dirs: vec![] };
        let manual = fx.home.join("work/scripts");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join("kclaude.zsh"), "kclaude() { echo MANUAL-KCLAUDE; }\n").unwrap();
        let original = "export A=1\nsource \"$HOME/work/scripts/kclaude.zsh\"\nexport B=2\n";
        fs::write(fx.home.join(rc_name), original).unwrap();

        let service = KclaudeService::new(fx.env(shell), &backend);
        assert_eq!(service.status().state, KclaudeState::NotInstalled, "{shell}: not foreign any more");
        let installed = service.install().unwrap();
        assert_eq!(installed.state, KclaudeState::Installed, "{shell}");
        assert_eq!(installed.command_name, "cclaude");

        let rc = fx.rc_text(rc_name);
        assert!(rc.starts_with(original), "{shell}: the old line must not move or change: {rc}");
        assert!(!rc.contains("disabled by Kiro Gateway App"), "{shell}");
        assert_eq!(rc.matches(BEGIN_MARKER).count(), 1, "{shell}");

        // zsh runs the user's zsh script; bash cannot define the zsh-only function but must still
        // load the App's bash script for cclaude.
        assert!(type_in_login_shell(shell, &fx.home, "cclaude").contains("function"), "{shell}");
        if shell.ends_with("zsh") {
            assert!(type_in_login_shell(shell, &fx.home, "kclaude").contains("function"), "{shell}: both coexist");
        }
    }
}

#[test]
fn naming_the_app_command_kclaude_while_the_manual_one_exists_is_refused_as_taken() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(fx.zshrc(), "source \"$HOME/work/scripts/kclaude.zsh\"\n").unwrap();
    fx.install("/bin/zsh", &backend);
    let before = snapshot(&fx);
    let err = fx.rename("/bin/zsh", &backend, "kclaude").unwrap_err();
    assert_eq!((err.code(), err.message()), ("invalid_argument", MSG_TAKEN));
    assert_eq!(snapshot(&fx), before);
}

// ---- legacy block migration -----------------------------------------------------------------

fn write_legacy_install(fx: &Fixture, rc_name: &str, before: &str, after: &str) -> String {
    let script = fx.data_dir.join("kclaude.zsh");
    fs::create_dir_all(&fx.data_dir).unwrap();
    // The legacy script file; its content does not matter, only that it exists.
    fs::write(&script, "# legacy kclaude function\nkclaude() { :; }\n").unwrap();
    let block = format!(
        "{LEGACY_BEGIN_MARKER}\n[ -r '{p}' ] && source '{p}'\n{LEGACY_END_MARKER}\n",
        p = script.display()
    );
    let text = format!("{before}{block}{after}");
    fs::write(fx.home.join(rc_name), &text).unwrap();
    text
}

#[test]
fn a_legacy_kclaude_block_is_replaced_in_place_and_the_old_script_is_removed() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    write_legacy_install(&fx, ".zshrc", "export A=1\n", "export B=2\n");
    let service = KclaudeService::new(fx.env("/bin/zsh"), &backend);
    assert_eq!(service.status().state, KclaudeState::Outdated);

    let status = service.install().unwrap();

    assert_eq!(status.state, KclaudeState::Installed);
    let rc = fx.rc_text(".zshrc");
    assert!(rc.starts_with("export A=1\n# >>> kiro-gateway-desktop command >>>\n"), "{rc}");
    assert!(rc.ends_with("# <<< kiro-gateway-desktop command <<<\nexport B=2\n"), "{rc}");
    assert!(!rc.contains(LEGACY_BEGIN_MARKER) && !rc.contains(LEGACY_END_MARKER), "{rc}");
    assert!(rc.contains("cclaude.zsh") && !rc.contains("kclaude.zsh"), "{rc}");
    assert_eq!(fx.script_files(), vec!["cclaude.zsh".to_string()]);
    assert_eq!(fx.backups(&fx.home).len(), 1, "the file is backed up before the replacement");
}

#[test]
fn auto_install_migrates_a_legacy_block_at_app_start() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    write_legacy_install(&fx, ".zshrc", "", "");
    let status = KclaudeService::new(fx.env("/bin/zsh"), &backend).auto_install();
    assert_eq!(status.state, KclaudeState::Installed);
    assert!(!fx.rc_text(".zshrc").contains(LEGACY_BEGIN_MARKER));
    assert_eq!(fx.script_files(), vec!["cclaude.zsh".to_string()]);
}

#[test]
fn a_legacy_block_with_real_shells_ends_with_cclaude_defined_and_kclaude_gone() {
    for (shell, rc_name, ext) in [("/bin/zsh", ".zshrc", "zsh"), ("/bin/bash", ".bash_profile", "bash")] {
        let fx = Fixture::new();
        let backend = RealBackend { path_dirs: vec![] };
        let script = fx.data_dir.join(format!("kclaude.{ext}"));
        fs::create_dir_all(&fx.data_dir).unwrap();
        // Render the old script the way the first build did: same template, name kclaude.
        script::install(if ext == "zsh" { script::ScriptKind::Zsh } else { script::ScriptKind::Bash }, "kclaude", &fx.data_dir).unwrap();
        let legacy = format!(
            "{LEGACY_BEGIN_MARKER}\n[ -r '{p}' ] && source '{p}'\n{LEGACY_END_MARKER}\n",
            p = script.display()
        );
        fs::write(fx.home.join(rc_name), format!("export A=1\n{legacy}")).unwrap();
        assert!(type_in_login_shell(shell, &fx.home, "kclaude").contains("function"), "{shell}: precondition");

        let status = KclaudeService::new(fx.env(shell), &backend).install().unwrap();

        assert_eq!(status.state, KclaudeState::Installed, "{shell}");
        assert_eq!(status.verify.status, VerifyStatus::Ok, "{shell}: {:?}", status.verify);
        assert!(!type_in_login_shell(shell, &fx.home, "kclaude").contains("kclaude is a"), "{shell}: kclaude must be gone");
        assert!(!script.exists(), "{shell}: the legacy script file is deleted");
    }
}

#[test]
fn keeping_the_name_kclaude_migrates_the_markers_but_keeps_the_script_file() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fx.store.update(&ConfigPatch { command_name: Some("kclaude".into()), ..Default::default() }, false).unwrap();
    let script = fx.data_dir.join("kclaude.zsh");
    script::install(script::ScriptKind::Zsh, "kclaude", &fx.data_dir).unwrap();
    fs::write(
        fx.zshrc(),
        format!("{LEGACY_BEGIN_MARKER}\n[ -r '{p}' ] && source '{p}'\n{LEGACY_END_MARKER}\n", p = script.display()),
    )
    .unwrap();
    let status = KclaudeService::new(fx.env("/bin/zsh"), &backend).install().unwrap();
    assert_eq!(status.state, KclaudeState::Installed);
    assert!(script.exists(), "the file the block points at must stay");
    assert!(fx.rc_text(".zshrc").contains(BEGIN_MARKER));
}

#[test]
fn rename_from_a_legacy_kclaude_setup_to_a_new_name_replaces_the_legacy_block() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    // The user's config says kclaude (as if an earlier build chose it) and the file has the legacy block.
    fx.store.update(&ConfigPatch { command_name: Some("kclaude".into()), ..Default::default() }, false).unwrap();
    write_legacy_install(&fx, ".zshrc", "export A=1\n", "");
    fx.rename("/bin/zsh", &backend, "kclaude-test").unwrap();
    let rc = fx.rc_text(".zshrc");
    assert_eq!(rc.matches(BEGIN_MARKER).count(), 1, "{rc}");
    assert!(!rc.contains(LEGACY_BEGIN_MARKER), "{rc}");
    assert_eq!(fx.script_files(), vec!["kclaude-test.zsh".to_string()]);
}

// ---- states where the name is only a setting ----------------------------------------------------

#[test]
fn a_foreign_definition_of_the_current_name_means_the_rename_is_only_saved() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let original = "alias cclaude='my-own-thing'\nexport A=1\n";
    fs::write(fx.zshrc(), original).unwrap();

    let outcome = fx.rename("/bin/zsh", &backend, "kclaude-test").unwrap();

    assert_eq!(outcome.config.command_name, "kclaude-test");
    assert_eq!(fx.rc_text(".zshrc"), original, "the startup file is not touched");
    assert!(fx.backups(&fx.home).is_empty());
    assert!(fx.script_files().is_empty(), "no script is written either");
    assert!(backend.verified.borrow().is_empty());
}

#[test]
fn a_user_removed_kclaude_stays_removed_after_a_rename() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(fx.zshrc(), "export A=1\n").unwrap();
    fx.install("/bin/zsh", &backend);
    KclaudeService::new(fx.env("/bin/zsh"), &backend).remove().unwrap();
    let rc_after_remove = fs::read(fx.zshrc()).unwrap();
    let scripts_after_remove = fx.script_files();

    let outcome = fx.rename("/bin/zsh", &backend, "kclaude-test").unwrap();

    assert_eq!(outcome.config.command_name, "kclaude-test");
    assert_eq!(fs::read(fx.zshrc()).unwrap(), rc_after_remove);
    assert_eq!(fx.script_files(), scripts_after_remove, "nothing new is written");
    let status = KclaudeService::new(fx.env("/bin/zsh"), &backend).status();
    assert_eq!(status.state, KclaudeState::NotInstalled);
    assert_eq!(status.command_name, "kclaude-test");
}

#[test]
fn a_damaged_marker_block_means_the_rename_is_only_saved() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let original = format!("{BEGIN_MARKER}\nsource x\n");
    fs::write(fx.zshrc(), &original).unwrap();
    let outcome = fx.rename("/bin/zsh", &backend, "kclaude-test").unwrap();
    assert_eq!(outcome.config.command_name, "kclaude-test");
    assert_eq!(fx.rc_text(".zshrc"), original);
}

// ---- failure rolls the setting back ----------------------------------------------------------------

#[test]
fn when_the_startup_file_cannot_be_backed_up_the_old_name_stays_in_force() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(fx.zshrc(), "export A=1\n").unwrap();
    fx.install("/bin/zsh", &backend);
    let rc_before = fs::read(fx.zshrc()).unwrap();

    fs::set_permissions(&fx.home, fs::Permissions::from_mode(0o555)).unwrap();
    let can_write_anyway = fs::write(fx.home.join("probe"), b"").is_ok(); // running as root
    let result = fx.rename("/bin/zsh", &backend, "kclaude-test");
    fs::set_permissions(&fx.home, fs::Permissions::from_mode(0o755)).unwrap();
    if can_write_anyway {
        return;
    }

    let err = result.unwrap_err();
    assert_eq!(err.code(), "write_failed");
    assert!(!err.message().contains('/'), "{}", err.message());
    assert_eq!(fx.store.get().command_name, "cclaude", "config rolled back");
    let on_disk: crate::app_config::model::AppConfig = serde_json::from_slice(&fx.config_bytes()).unwrap();
    assert_eq!(on_disk.command_name, "cclaude");
    assert!(fs::read_to_string(fx.data_dir.join("kclaude.env")).unwrap().contains("KIRO_GATEWAY_COMMAND=cclaude\n"));
    assert_eq!(fs::read(fx.zshrc()).unwrap(), rc_before);
    assert_eq!(fx.script_files(), vec!["cclaude.zsh".to_string()], "old script kept, new one removed");
}

// ---- Windows layout ---------------------------------------------------------------------------------

#[test]
fn windows_rename_rewrites_both_profiles_and_swaps_the_ps1_script() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let p5 = fx.docs.join("WindowsPowerShell/profile.ps1");
    let p7 = fx.docs.join("PowerShell/profile.ps1");
    fs::create_dir_all(p5.parent().unwrap()).unwrap();
    fs::write(&p5, "Set-Alias ll Get-ChildItem\r\n").unwrap();
    KclaudeService::new(fx.env_on(Platform::Windows, ""), &backend).install().unwrap();
    assert_eq!(fx.script_files(), vec!["cclaude.ps1".to_string()]);

    let patch = ConfigPatch { command_name: Some("kclaude-test".into()), ..Default::default() };
    apply_update(&fx.store, &fx.env_on(Platform::Windows, ""), &backend, &patch, false).unwrap();

    for profile in [&p5, &p7] {
        let text = fs::read_to_string(profile).unwrap();
        assert_eq!(text.matches(BEGIN_MARKER).count(), 1, "{text}");
        assert!(text.contains("kclaude-test.ps1") && !text.contains("cclaude.ps1"), "{text}");
    }
    assert!(fs::read_to_string(&p5).unwrap().starts_with("Set-Alias ll Get-ChildItem\r\n"));
    assert_eq!(fx.script_files(), vec!["kclaude-test.ps1".to_string()]);
    assert_eq!(backend.verified.borrow().last().map(String::as_str), Some("kclaude-test"));
}

#[test]
fn windows_function_in_a_profile_with_the_new_name_is_taken() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let p7 = fx.docs.join("PowerShell/profile.ps1");
    fs::create_dir_all(p7.parent().unwrap()).unwrap();
    fs::write(&p7, "function Taken-Name { 1 }\n").unwrap();
    let patch = ConfigPatch { command_name: Some("taken-name".into()), ..Default::default() };
    let err = apply_update(&fx.store, &fx.env_on(Platform::Windows, ""), &backend, &patch, false).unwrap_err();
    assert_eq!(err.message(), MSG_TAKEN);
}

// ---- name_problem / check_name directly ---------------------------------------------------------------

#[test]
fn name_problem_reports_format_reserved_taken_in_that_order_and_none_for_a_free_name() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    backend.path_names.borrow_mut().push("pathcmd".into());
    fs::write(fx.zshrc(), "alias rccmd=ls\n").unwrap();
    let service = KclaudeService::new(fx.env("/bin/zsh"), &backend);
    use crate::app_config::command_name::NameProblem as P;
    assert_eq!(service.name_problem("9x"), Some(P::Format));
    assert_eq!(service.name_problem("claude"), Some(P::Reserved));
    assert_eq!(service.name_problem("pathcmd"), Some(P::Taken));
    assert_eq!(service.name_problem("rccmd"), Some(P::Taken));
    assert_eq!(service.name_problem("free-one"), None);
    assert_eq!(service.name_problem("cclaude"), None, "the name in force is never taken by itself");
}

// ---- the verify cache follows the command name ---------------------------------------------------------

#[test]
fn the_verify_fingerprint_depends_on_the_command_name() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(fx.zshrc(), "export A=1\n").unwrap();
    let a = KclaudeService::new(fx.env("/bin/zsh").with_command("cclaude"), &backend);
    let b = KclaudeService::new(fx.env("/bin/zsh").with_command("kclaude-test"), &backend);
    assert_ne!(a.verify_fingerprint_for_tests(), b.verify_fingerprint_for_tests());
}

#[test]
fn a_cached_check_for_the_old_name_is_not_reused_for_the_new_one() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    fs::write(fx.zshrc(), "export A=1\n").unwrap();
    fx.install("/bin/zsh", &backend);
    assert_eq!(backend.verified.borrow().as_slice(), ["cclaude"]);
    // Same startup file bytes and mtime from here on: only the name differs.
    let status = KclaudeService::new(fx.env("/bin/zsh").with_command("cclaude"), &backend).status();
    assert_eq!(status.verify.status, VerifyStatus::Ok);
    assert_eq!(backend.verified.borrow().len(), 1, "same name: served from the cache");
    fs::write(fx.data_dir.join("kclaude-test.zsh"), "").unwrap();
    let _ = KclaudeService::new(fx.env("/bin/zsh").with_command("kclaude-test"), &backend).status();
    // Not installed under that name, so no check is run; installing it runs a fresh one.
    fx.rename("/bin/zsh", &backend, "kclaude-test").unwrap();
    assert_eq!(backend.verified.borrow().last().map(String::as_str), Some("kclaude-test"));
}

#[test]
fn skipped_verification_names_the_command() {
    let fx = Fixture::new();
    let backend = FakeBackend::new();
    let status = KclaudeService::new(fx.env("/bin/zsh"), &backend).status();
    assert_eq!(status.verify.status, VerifyStatus::Skipped);
    assert_eq!(status.verify.detail, "还没有安装 cclaude");
    assert_eq!(status.command_name, "cclaude");
}
