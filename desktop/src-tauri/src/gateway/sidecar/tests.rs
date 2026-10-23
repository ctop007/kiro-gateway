use super::*;
use std::ffi::OsString;
use std::fs;
use tempfile::tempdir;

const ARCH_UNDER_TEST: &str = std::env::consts::ARCH;

/// Where the installer puts the sidecar of the architecture this test binary runs on.
fn bundled(res: &std::path::Path) -> std::path::PathBuf {
    res.join(resource_subdir(cfg!(windows), ARCH_UNDER_TEST))
}

fn make_exe(path: &std::path::Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn executable_name_is_platform_specific() {
    assert_eq!(exe_name(false), "kiro-gateway");
    assert_eq!(exe_name(true), "kiro-gateway.exe");
}

#[test]
fn default_location_is_gateway_dir_inside_the_resource_dir() {
    let res = tempdir().unwrap();
    let exe = bundled(res.path()).join(exe_name(cfg!(windows)));
    make_exe(&exe);
    let found = resolve(res.path(), None, ARCH_UNDER_TEST).unwrap();
    assert_eq!(found.exe, exe);
}

#[test]
fn tiktoken_cache_is_picked_up_when_it_sits_next_to_the_executable() {
    let res = tempdir().unwrap();
    make_exe(&bundled(res.path()).join(exe_name(cfg!(windows))));
    assert_eq!(resolve(res.path(), None, ARCH_UNDER_TEST).unwrap().tiktoken_cache, None);
    fs::create_dir(bundled(res.path()).join("tiktoken_cache")).unwrap();
    assert_eq!(
        resolve(res.path(), None, ARCH_UNDER_TEST).unwrap().tiktoken_cache,
        Some(bundled(res.path()).join("tiktoken_cache"))
    );
}

#[test]
fn override_wins_over_the_resource_dir() {
    let res = tempdir().unwrap();
    make_exe(&bundled(res.path()).join(exe_name(cfg!(windows))));
    let other = tempdir().unwrap();
    let custom = other.path().join("my-gateway");
    make_exe(&custom);
    let found = resolve(res.path(), Some(custom.as_os_str()), ARCH_UNDER_TEST).unwrap();
    assert_eq!(found.exe, custom);
}

#[test]
fn override_uses_the_tiktoken_cache_next_to_the_overriding_executable() {
    let res = tempdir().unwrap();
    let other = tempdir().unwrap();
    let custom = other.path().join("bin").join("my-gateway");
    make_exe(&custom);
    fs::create_dir(other.path().join("bin").join("tiktoken_cache")).unwrap();
    let found = resolve(res.path(), Some(custom.as_os_str()), ARCH_UNDER_TEST).unwrap();
    assert_eq!(found.tiktoken_cache, Some(other.path().join("bin").join("tiktoken_cache")));
}

#[test]
fn empty_override_is_ignored() {
    let res = tempdir().unwrap();
    let exe = bundled(res.path()).join(exe_name(cfg!(windows)));
    make_exe(&exe);
    assert_eq!(resolve(res.path(), Some(&OsString::from("")), ARCH_UNDER_TEST).unwrap().exe, exe);
}

#[test]
fn missing_default_executable_is_a_clear_error() {
    let res = tempdir().unwrap();
    fs::create_dir_all(bundled(res.path())).unwrap();
    let err = resolve(res.path(), None, ARCH_UNDER_TEST).unwrap_err();
    assert_eq!(err, SidecarError::Missing);
    assert!(err.message().contains("重新安装"));
    assert!(!err.message().contains('/'));
}

#[test]
fn missing_override_target_is_an_error_not_a_silent_fallback() {
    let res = tempdir().unwrap();
    make_exe(&bundled(res.path()).join(exe_name(cfg!(windows))));
    let err = resolve(res.path(), Some(OsString::from("/definitely/not/here").as_os_str()), ARCH_UNDER_TEST).unwrap_err();
    assert_eq!(err, SidecarError::OverrideMissing);
    assert!(err.message().contains(OVERRIDE_VAR));
}

#[test]
fn a_directory_is_not_an_executable() {
    let res = tempdir().unwrap();
    fs::create_dir_all(bundled(res.path()).join(exe_name(cfg!(windows)))).unwrap();
    assert_eq!(resolve(res.path(), None, ARCH_UNDER_TEST).unwrap_err(), SidecarError::Missing);
}

#[cfg(unix)]
#[test]
fn a_file_without_the_execute_bit_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let res = tempdir().unwrap();
    let exe = bundled(res.path()).join(exe_name(false));
    make_exe(&exe);
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(resolve(res.path(), None, ARCH_UNDER_TEST).unwrap_err(), SidecarError::NotExecutable);
}

#[test]
fn working_directory_of_the_sidecar_is_the_empty_runtime_dir() {
    let data = tempdir().unwrap();
    let runtime = data.path().join("runtime");
    let made = prepare_runtime_dir(&runtime).unwrap();
    assert_eq!(made, runtime);
    assert!(runtime.is_dir());
}

#[test]
fn a_stray_env_file_in_runtime_is_removed_so_dotenv_cannot_read_it() {
    let data = tempdir().unwrap();
    let runtime = data.path().join("runtime");
    fs::create_dir(&runtime).unwrap();
    fs::write(runtime.join(".env"), "PROXY_API_KEY=evil\n").unwrap();
    fs::write(runtime.join("keep.txt"), "x").unwrap();
    prepare_runtime_dir(&runtime).unwrap();
    assert!(!runtime.join(".env").exists());
    assert!(runtime.join("keep.txt").exists(), "only .env is removed, nothing else");
}

#[test]
fn preparing_runtime_twice_is_fine() {
    let data = tempdir().unwrap();
    let runtime = data.path().join("runtime");
    prepare_runtime_dir(&runtime).unwrap();
    prepare_runtime_dir(&runtime).unwrap();
}

// ---- per-architecture layout (macOS universal bundle) ----

#[test]
fn macos_subdir_is_named_after_the_architecture() {
    assert_eq!(resource_subdir(false, "aarch64"), "gateway-aarch64");
    assert_eq!(resource_subdir(false, "x86_64"), "gateway-x86_64");
}

#[test]
fn windows_keeps_the_single_gateway_dir_whatever_the_architecture() {
    assert_eq!(resource_subdir(true, "x86_64"), "gateway");
    assert_eq!(resource_subdir(true, "aarch64"), "gateway");
}

#[cfg(unix)]
#[test]
fn each_architecture_picks_its_own_directory_from_a_universal_bundle() {
    let res = tempdir().unwrap();
    let arm = res.path().join("gateway-aarch64").join("kiro-gateway");
    let intel = res.path().join("gateway-x86_64").join("kiro-gateway");
    make_exe(&arm);
    make_exe(&intel);
    assert_eq!(resolve(res.path(), None, "aarch64").unwrap().exe, arm);
    assert_eq!(resolve(res.path(), None, "x86_64").unwrap().exe, intel);
}

#[cfg(unix)]
#[test]
fn tiktoken_cache_comes_from_the_chosen_architecture_dir() {
    let res = tempdir().unwrap();
    make_exe(&res.path().join("gateway-aarch64").join("kiro-gateway"));
    make_exe(&res.path().join("gateway-x86_64").join("kiro-gateway"));
    fs::create_dir(res.path().join("gateway-x86_64").join("tiktoken_cache")).unwrap();
    assert_eq!(resolve(res.path(), None, "aarch64").unwrap().tiktoken_cache, None);
    assert_eq!(
        resolve(res.path(), None, "x86_64").unwrap().tiktoken_cache,
        Some(res.path().join("gateway-x86_64").join("tiktoken_cache"))
    );
}

#[cfg(unix)]
#[test]
fn a_bundle_without_this_architecture_says_so_instead_of_a_generic_missing() {
    let res = tempdir().unwrap();
    make_exe(&res.path().join("gateway-aarch64").join("kiro-gateway"));
    let err = resolve(res.path(), None, "x86_64").unwrap_err();
    assert_eq!(err, SidecarError::ArchMissing);
    assert!(err.message().contains("CPU"));
    assert!(err.message().contains("重新下载"));
    assert!(!err.message().contains('/'));
}

#[cfg(unix)]
#[test]
fn an_empty_architecture_dir_is_missing_not_arch_missing() {
    let res = tempdir().unwrap();
    fs::create_dir_all(res.path().join("gateway-x86_64")).unwrap();
    assert_eq!(resolve(res.path(), None, "x86_64").unwrap_err(), SidecarError::Missing);
}

#[cfg(unix)]
#[test]
fn the_old_single_gateway_dir_is_not_used_on_macos() {
    let res = tempdir().unwrap();
    make_exe(&res.path().join("gateway").join("kiro-gateway"));
    assert_eq!(resolve(res.path(), None, "aarch64").unwrap_err(), SidecarError::ArchMissing);
}

#[cfg(unix)]
#[test]
fn override_still_wins_over_a_missing_architecture_dir() {
    let res = tempdir().unwrap();
    let other = tempdir().unwrap();
    let custom = other.path().join("my-gateway");
    make_exe(&custom);
    assert_eq!(resolve(res.path(), Some(custom.as_os_str()), "x86_64").unwrap().exe, custom);
}

#[test]
fn gateway_dir_names_cover_every_layout_ever_shipped() {
    assert!(is_gateway_dir_name("gateway"));
    assert!(is_gateway_dir_name("gateway-aarch64"));
    assert!(is_gateway_dir_name("gateway-x86_64"));
    assert!(is_gateway_dir_name("Gateway-X86_64"));
    assert!(!is_gateway_dir_name("gateway-"));
    assert!(!is_gateway_dir_name("gateway-arm64-extra dir"));
    assert!(!is_gateway_dir_name("gateways"));
    assert!(!is_gateway_dir_name(""));
}
