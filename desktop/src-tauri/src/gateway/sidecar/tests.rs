use super::*;
use std::ffi::OsString;
use std::fs;
use tempfile::tempdir;

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
    let exe = res.path().join("gateway").join(exe_name(cfg!(windows)));
    make_exe(&exe);
    let found = resolve(res.path(), None).unwrap();
    assert_eq!(found.exe, exe);
}

#[test]
fn tiktoken_cache_is_picked_up_when_it_sits_next_to_the_executable() {
    let res = tempdir().unwrap();
    make_exe(&res.path().join("gateway").join(exe_name(cfg!(windows))));
    assert_eq!(resolve(res.path(), None).unwrap().tiktoken_cache, None);
    fs::create_dir(res.path().join("gateway").join("tiktoken_cache")).unwrap();
    assert_eq!(
        resolve(res.path(), None).unwrap().tiktoken_cache,
        Some(res.path().join("gateway").join("tiktoken_cache"))
    );
}

#[test]
fn override_wins_over_the_resource_dir() {
    let res = tempdir().unwrap();
    make_exe(&res.path().join("gateway").join(exe_name(cfg!(windows))));
    let other = tempdir().unwrap();
    let custom = other.path().join("my-gateway");
    make_exe(&custom);
    let found = resolve(res.path(), Some(custom.as_os_str())).unwrap();
    assert_eq!(found.exe, custom);
}

#[test]
fn override_uses_the_tiktoken_cache_next_to_the_overriding_executable() {
    let res = tempdir().unwrap();
    let other = tempdir().unwrap();
    let custom = other.path().join("bin").join("my-gateway");
    make_exe(&custom);
    fs::create_dir(other.path().join("bin").join("tiktoken_cache")).unwrap();
    let found = resolve(res.path(), Some(custom.as_os_str())).unwrap();
    assert_eq!(found.tiktoken_cache, Some(other.path().join("bin").join("tiktoken_cache")));
}

#[test]
fn empty_override_is_ignored() {
    let res = tempdir().unwrap();
    let exe = res.path().join("gateway").join(exe_name(cfg!(windows)));
    make_exe(&exe);
    assert_eq!(resolve(res.path(), Some(&OsString::from(""))).unwrap().exe, exe);
}

#[test]
fn missing_default_executable_is_a_clear_error() {
    let res = tempdir().unwrap();
    let err = resolve(res.path(), None).unwrap_err();
    assert_eq!(err, SidecarError::Missing);
    assert!(err.message().contains("重新安装"));
    assert!(!err.message().contains('/'));
}

#[test]
fn missing_override_target_is_an_error_not_a_silent_fallback() {
    let res = tempdir().unwrap();
    make_exe(&res.path().join("gateway").join(exe_name(cfg!(windows))));
    let err = resolve(res.path(), Some(OsString::from("/definitely/not/here").as_os_str())).unwrap_err();
    assert_eq!(err, SidecarError::OverrideMissing);
    assert!(err.message().contains(OVERRIDE_VAR));
}

#[test]
fn a_directory_is_not_an_executable() {
    let res = tempdir().unwrap();
    fs::create_dir_all(res.path().join("gateway").join(exe_name(cfg!(windows)))).unwrap();
    assert_eq!(resolve(res.path(), None).unwrap_err(), SidecarError::Missing);
}

#[cfg(unix)]
#[test]
fn a_file_without_the_execute_bit_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let res = tempdir().unwrap();
    let exe = res.path().join("gateway").join(exe_name(false));
    make_exe(&exe);
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(resolve(res.path(), None).unwrap_err(), SidecarError::NotExecutable);
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
