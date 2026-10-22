//! Tests for PowerShell profile paths and execution-policy hints
//! (`cargo test kclaude::ps_profile`).

use std::path::{Path, PathBuf};

use super::*;

#[test]
fn profile_paths_are_derived_from_the_injected_documents_dir() {
    let docs = Path::new("/inj/Documents");
    let p = profile_paths(docs);
    assert_eq!(p.ps5, docs.join("WindowsPowerShell").join("profile.ps1"));
    assert_eq!(p.ps7, docs.join("PowerShell").join("profile.ps1"));
    assert_eq!(p.all(), vec![p.ps5.clone(), p.ps7.clone()]);
}

#[test]
fn documents_redirected_to_onedrive_or_with_spaces_and_unicode_is_used_verbatim() {
    for dir in ["/Users/小明/OneDrive - Corp/文档", "/Users/a b/OneDrive/Documents"] {
        let p = profile_paths(Path::new(dir));
        assert_eq!(p.ps5, PathBuf::from(dir).join("WindowsPowerShell/profile.ps1"));
        assert_eq!(p.ps7, PathBuf::from(dir).join("PowerShell/profile.ps1"));
    }
}

#[test]
fn profile_file_name_is_current_user_all_hosts() {
    let p = profile_paths(Path::new("/d"));
    assert_eq!(p.ps5.file_name().unwrap(), "profile.ps1");
    assert_ne!(p.ps5.file_name().unwrap(), "Microsoft.PowerShell_profile.ps1");
}

#[test]
fn policy_parsing_is_case_insensitive_and_unknown_values_are_none() {
    assert_eq!(ExecutionPolicy::parse("Restricted"), Some(ExecutionPolicy::Restricted));
    assert_eq!(ExecutionPolicy::parse(" allsigned "), Some(ExecutionPolicy::AllSigned));
    assert_eq!(ExecutionPolicy::parse("REMOTESIGNED"), Some(ExecutionPolicy::RemoteSigned));
    assert_eq!(ExecutionPolicy::parse("Unrestricted"), Some(ExecutionPolicy::Unrestricted));
    assert_eq!(ExecutionPolicy::parse("Bypass"), Some(ExecutionPolicy::Bypass));
    assert_eq!(ExecutionPolicy::parse("Undefined"), Some(ExecutionPolicy::Undefined));
    assert_eq!(ExecutionPolicy::parse(""), None);
    assert_eq!(ExecutionPolicy::parse("Sometimes"), None);
}

#[test]
fn only_restricted_and_allsigned_block_a_local_profile() {
    use ExecutionPolicy::*;
    for (policy, blocks) in [
        (Restricted, true),
        (AllSigned, true),
        (RemoteSigned, false),
        (Unrestricted, false),
        (Bypass, false),
    ] {
        assert_eq!(policy.blocks_local_scripts(), blocks, "{policy:?}");
    }
}

#[test]
fn machine_policy_group_policy_beats_user_group_policy_beats_user_beats_machine() {
    use ExecutionPolicy::*;
    let mut s = PolicySources::default();
    s.machine = Some(Bypass);
    assert_eq!(s.effective(), Bypass);
    s.user = Some(RemoteSigned);
    assert_eq!(s.effective(), RemoteSigned);
    s.user_policy = Some(AllSigned);
    assert_eq!(s.effective(), AllSigned);
    s.machine_policy = Some(Restricted);
    assert_eq!(s.effective(), Restricted);
}

#[test]
fn undefined_values_fall_through_and_nothing_set_means_the_client_default_restricted() {
    use ExecutionPolicy::*;
    let mut s = PolicySources::default();
    assert_eq!(s.effective(), Restricted);
    s.user = Some(Undefined);
    s.machine = Some(RemoteSigned);
    assert_eq!(s.effective(), RemoteSigned);
    s.machine = Some(Undefined);
    assert_eq!(s.effective(), Restricted);
}

#[test]
fn execution_policy_hint_is_none_when_scripts_are_allowed() {
    let s = PolicySources { user: Some(ExecutionPolicy::RemoteSigned), ..Default::default() };
    assert_eq!(execution_policy_hint(&s), None);
}

#[test]
fn execution_policy_hint_gives_a_copyable_command_and_never_runs_it() {
    let s = PolicySources::default();
    let hint = execution_policy_hint(&s).expect("Restricted blocks profiles");
    assert_eq!(hint.status, VerifyStatus::Fail);
    assert!(hint.detail.contains("执行策略"), "{}", hint.detail);
    match hint.fix {
        Some(FixAction::CopyCommand { command, label }) => {
            assert_eq!(command, "Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned");
            assert!(!label.is_empty());
        }
        other => panic!("expected a copy_command fix, got {other:?}"),
    }
}

#[test]
fn group_policy_enforced_restriction_explains_that_the_command_will_not_help() {
    let s = PolicySources { machine_policy: Some(ExecutionPolicy::AllSigned), ..Default::default() };
    let hint = execution_policy_hint(&s).unwrap();
    assert!(hint.detail.contains("组策略"), "{}", hint.detail);
    assert!(hint.fix.is_none(), "a command cannot override group policy");
}

#[test]
fn policy_strings_never_leak_paths_or_secrets_into_the_hint() {
    let hint = execution_policy_hint(&PolicySources::default()).unwrap();
    assert!(!hint.detail.contains("http"));
    assert!(!hint.detail.contains('\\'));
}

#[test]
fn utf16_profiles_are_detected_so_they_are_never_edited_as_bytes() {
    assert!(is_utf16(b"\xFF\xFEa\x00"));
    assert!(is_utf16(b"\xFE\xFF\x00a"));
    assert!(!is_utf16(b"\xEF\xBB\xBFfunction x {}"));
    assert!(!is_utf16(b"plain"));
    assert!(!is_utf16(b""));
}
