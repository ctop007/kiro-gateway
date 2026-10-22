use super::*;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::tempdir;

fn leftovers(dir: &std::path::Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect()
}

#[test]
fn writes_new_file_with_exact_content() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.json");
    write_atomic(&path, b"{\"a\":1}", FileMode::Default).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"{\"a\":1}");
}

#[test]
fn replaces_existing_file_completely() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, "a much longer previous content than the new one").unwrap();
    write_atomic(&path, b"short", FileMode::Default).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"short");
}

#[test]
fn leaves_no_temporary_file_behind() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.json");
    write_atomic(&path, b"x", FileMode::Default).unwrap();
    write_atomic(&path, b"y", FileMode::Private).unwrap();
    assert_eq!(leftovers(dir.path()), vec!["config.json".to_string()]);
}

#[test]
fn creates_missing_parent_directories() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("a").join("b").join("secret");
    write_atomic(&path, b"x", FileMode::Private).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"x");
}

#[test]
fn empty_content_is_written_as_empty_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("empty");
    write_atomic(&path, b"", FileMode::Default).unwrap();
    assert_eq!(fs::read(&path).unwrap().len(), 0);
}

#[test]
fn failed_rename_cleans_up_and_reports_the_error() {
    let dir = tempdir().unwrap();
    // The target is a non-empty directory, so rename(temp, target) must fail.
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("inner"), "x").unwrap();
    let result = write_atomic(&target, b"data", FileMode::Default);
    assert!(result.is_err());
    assert_eq!(leftovers(dir.path()), vec!["target".to_string()]);
    assert!(target.join("inner").exists(), "existing directory must be untouched");
}

#[test]
fn unwritable_parent_is_an_error_not_a_panic() {
    let dir = tempdir().unwrap();
    // A regular file where the parent directory should be.
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "x").unwrap();
    let result = write_atomic(&blocker.join("child"), b"x", FileMode::Default);
    assert!(result.is_err());
}

#[cfg(unix)]
#[test]
fn private_files_are_0600_even_with_a_permissive_umask() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let path = dir.path().join("secret");
    write_atomic(&path, b"k", FileMode::Private).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn replacing_a_loose_file_with_private_mode_tightens_it() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let path = dir.path().join("secret");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    write_atomic(&path, b"new", FileMode::Private).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(fs::read(&path).unwrap(), b"new");
}

#[test]
fn concurrent_readers_never_see_a_partial_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("big");
    let a = vec![b'A'; 512 * 1024];
    let b = vec![b'B'; 512 * 1024];
    write_atomic(&path, &a, FileMode::Default).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let (path, stop, a, b) = (path.clone(), stop.clone(), a.clone(), b.clone());
        std::thread::spawn(move || {
            let mut reads = 0u32;
            while !stop.load(Ordering::Relaxed) {
                let got = fs::read(&path).unwrap();
                assert!(got == a || got == b, "saw a torn file of {} bytes", got.len());
                reads += 1;
            }
            reads
        })
    };
    for i in 0..60 {
        write_atomic(&path, if i % 2 == 0 { &b } else { &a }, FileMode::Default).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    assert!(reader.join().unwrap() > 0);
}

#[test]
fn two_writers_in_the_same_directory_do_not_collide_on_temp_names() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("shared");
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let path = path.clone();
            std::thread::spawn(move || {
                for _ in 0..25 {
                    write_atomic(&path, format!("writer-{i}").as_bytes(), FileMode::Default).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.starts_with("writer-"));
    assert_eq!(leftovers(dir.path()), vec!["shared".to_string()]);
}
