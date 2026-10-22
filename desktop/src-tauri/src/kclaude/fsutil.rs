//! Small file helpers: atomic write that follows symlinks, and numbered backups.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Replace `path` with `bytes` atomically (temp file in the same directory, then rename).
///
/// A symlinked `path` (dotfile managers do this to `~/.zshrc`) is resolved first so the
/// link itself survives. Permissions of an existing file are kept. Missing parent
/// directories are created.
///
/// Args:
///     path: Destination file.
///     bytes: New content.
///
/// Returns:
///     `Ok(())` once the content is in place.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let tmp = dir.join(format!(".{name}.kgw-tmp-{}", std::process::id()));

    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Ok(meta) = fs::metadata(&target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Pick `<file>.bak-kiro-gateway-<stamp>`, adding `-1`, `-2` ... when it already exists so
/// an earlier backup is never overwritten.
///
/// Args:
///     file: The file being backed up.
///     stamp: `YYYYMMDD-HHMMSS`, supplied by the caller.
///
/// Returns:
///     A path that does not exist yet.
pub fn backup_path(file: &Path, stamp: &str) -> PathBuf {
    let base = format!(
        "{}.bak-kiro-gateway-{}",
        file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
        stamp
    );
    let first = file.with_file_name(&base);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    (1u32..)
        .map(|n| file.with_file_name(format!("{base}-{n}")))
        .find(|p| fs::symlink_metadata(p).is_err())
        .expect("an unused backup name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/profile.ps1");
        write_atomic(&path, b"x").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"x");
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_keeps_symlink_and_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles-zshrc");
        fs::write(&real, b"old").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join(".zshrc");
        symlink(&real, &link).unwrap();

        write_atomic(&link, b"new").unwrap();

        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"new");
        assert_eq!(fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn write_atomic_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        write_atomic(&path, b"1").unwrap();
        write_atomic(&path, b"2").unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["f".to_string()]);
    }

    #[test]
    fn backup_path_never_reuses_an_existing_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".zshrc");
        let first = backup_path(&file, "20261022-101500");
        assert_eq!(
            first.file_name().unwrap().to_str().unwrap(),
            ".zshrc.bak-kiro-gateway-20261022-101500"
        );
        fs::write(&first, b"").unwrap();
        let second = backup_path(&file, "20261022-101500");
        assert_eq!(
            second.file_name().unwrap().to_str().unwrap(),
            ".zshrc.bak-kiro-gateway-20261022-101500-1"
        );
        fs::write(&second, b"").unwrap();
        assert!(backup_path(&file, "20261022-101500")
            .to_str()
            .unwrap()
            .ends_with("-2"));
    }
}
