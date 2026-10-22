//! Retention and clearing of month files (tasks 7.3).
//!
//! The gateway reopens the month file for every write, so deleting a file here
//! is safe: the next request simply recreates it.

use crate::request_log::model::YearMonth;
use std::fs;
use std::io;
use std::path::Path;

/// What `prune_old_months` did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Months whose files were deleted, oldest first.
    pub removed: Vec<YearMonth>,
    /// Months whose files could not be deleted (permissions, file in use), oldest first.
    pub failed: Vec<YearMonth>,
}

/// Every `requests-YYYY-MM.jsonl` regular file in `dir`, oldest month first.
/// A missing directory has none.
fn month_files(dir: &Path) -> io::Result<Vec<YearMonth>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut months = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(month) = name.to_str().and_then(YearMonth::from_file_name) else {
            continue;
        };
        if entry.file_type()?.is_file() {
            months.push(month);
        }
    }
    months.sort();
    Ok(months)
}

/// Delete month files older than the month before `current`.
///
/// The current and previous month are always kept so the month totals stay
/// exact; future months and files that do not match the gateway's naming are
/// never touched. A file that cannot be deleted is reported in `failed` and
/// does not stop the others.
///
/// Args:
///     dir: Request log directory (may not exist).
///     current: The current local month.
///
/// Returns:
///     Which months were removed and which could not be.
pub fn prune_old_months(dir: &Path, current: YearMonth) -> io::Result<PruneReport> {
    let keep_from = current.prev();
    let mut report = PruneReport::default();
    for month in month_files(dir)?.into_iter().filter(|m| *m < keep_from) {
        match fs::remove_file(dir.join(month.file_name())) {
            Ok(()) => report.removed.push(month),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => report.failed.push(month),
        }
    }
    Ok(report)
}

/// Delete every month file in `dir`; other files stay.
///
/// Args:
///     dir: Request log directory (may not exist).
///
/// Returns:
///     The number of files deleted, or the first deletion error.
pub fn clear_all(dir: &Path) -> io::Result<usize> {
    let mut removed = 0;
    for month in month_files(dir)? {
        match fs::remove_file(dir.join(month.file_name())) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_log::model::{sample_line, YearMonth};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn month(year: i32, m: u32) -> YearMonth {
        YearMonth::new(year, m).unwrap()
    }

    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), format!("{}\n", sample_line("2026-10-22T20:51:03.713+08:00", None, "ok", 0.1))).unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> =
            fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        out.sort();
        out
    }

    #[test]
    fn prune_keeps_current_and_previous_month_and_deletes_older_ones() {
        let tmp = TempDir::new().unwrap();
        for name in [
            "requests-2026-10.jsonl",
            "requests-2026-09.jsonl",
            "requests-2026-08.jsonl",
            "requests-2026-01.jsonl",
            "requests-2025-12.jsonl",
        ] {
            touch(tmp.path(), name);
        }
        let report = prune_old_months(tmp.path(), month(2026, 10)).unwrap();
        assert_eq!(names(tmp.path()), vec!["requests-2026-09.jsonl", "requests-2026-10.jsonl"]);
        assert_eq!(report.removed, vec![month(2025, 12), month(2026, 1), month(2026, 8)]);
        assert!(report.failed.is_empty());
    }

    #[test]
    fn prune_in_january_keeps_december_of_last_year() {
        let tmp = TempDir::new().unwrap();
        for name in ["requests-2027-01.jsonl", "requests-2026-12.jsonl", "requests-2026-11.jsonl"] {
            touch(tmp.path(), name);
        }
        let report = prune_old_months(tmp.path(), month(2027, 1)).unwrap();
        assert_eq!(names(tmp.path()), vec!["requests-2026-12.jsonl", "requests-2027-01.jsonl"]);
        assert_eq!(report.removed, vec![month(2026, 11)]);
    }

    #[test]
    fn prune_never_touches_future_months_or_unrelated_files() {
        let tmp = TempDir::new().unwrap();
        for name in [
            "requests-2026-11.jsonl",
            "requests-2030-01.jsonl",
            "requests-2026-05.jsonl.bak",
            "requests-2026-13.jsonl",
            "notes.txt",
            "gateway.log",
        ] {
            touch(tmp.path(), name);
        }
        fs::create_dir(tmp.path().join("requests-2020-01.jsonl.d")).unwrap();
        fs::create_dir(tmp.path().join("requests-2020-02.jsonl")).unwrap();
        let report = prune_old_months(tmp.path(), month(2026, 10)).unwrap();
        assert!(report.removed.is_empty(), "future months and look-alikes stay");
        assert!(tmp.path().join("requests-2020-02.jsonl").is_dir(), "a directory is not a log file");
        assert_eq!(names(tmp.path()).len(), 8);
    }

    #[test]
    fn prune_is_a_no_op_on_a_missing_or_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let report = prune_old_months(&tmp.path().join("nope"), month(2026, 10)).unwrap();
        assert!(report.removed.is_empty() && report.failed.is_empty());
        let report = prune_old_months(tmp.path(), month(2026, 10)).unwrap();
        assert!(report.removed.is_empty());
    }

    #[test]
    fn prune_twice_removes_nothing_the_second_time() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "requests-2026-01.jsonl");
        touch(tmp.path(), "requests-2026-10.jsonl");
        assert_eq!(prune_old_months(tmp.path(), month(2026, 10)).unwrap().removed.len(), 1);
        assert!(prune_old_months(tmp.path(), month(2026, 10)).unwrap().removed.is_empty());
    }

    #[test]
    fn prune_reports_files_it_cannot_delete_and_still_removes_the_rest() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "requests-2026-01.jsonl");
        touch(tmp.path(), "requests-2026-02.jsonl");
        // Read-only directory: deletions fail on Unix. Skip when running as root.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let original = fs::metadata(tmp.path()).unwrap().permissions();
            fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o500)).unwrap();
            let probe = fs::File::create(tmp.path().join("probe"));
            if probe.is_ok() {
                fs::set_permissions(tmp.path(), original).unwrap();
                return; // running as root, permissions are not enforced
            }
            let report = prune_old_months(tmp.path(), month(2026, 10)).unwrap();
            fs::set_permissions(tmp.path(), original).unwrap();
            assert!(report.removed.is_empty());
            assert_eq!(report.failed, vec![month(2026, 1), month(2026, 2)]);
        }
    }

    #[test]
    fn clear_all_deletes_every_month_file_and_nothing_else() {
        let tmp = TempDir::new().unwrap();
        for name in ["requests-2026-10.jsonl", "requests-2026-09.jsonl", "requests-2024-01.jsonl", "gateway.log", "keep.txt"] {
            touch(tmp.path(), name);
        }
        let removed = clear_all(tmp.path()).unwrap();
        assert_eq!(removed, 3);
        assert_eq!(names(tmp.path()), vec!["gateway.log", "keep.txt"]);
    }

    #[test]
    fn clear_all_on_missing_or_empty_directory_removes_nothing() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(clear_all(&tmp.path().join("nope")).unwrap(), 0);
        assert_eq!(clear_all(tmp.path()).unwrap(), 0);
    }

    #[test]
    fn clear_all_fails_loudly_when_a_file_cannot_be_removed() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "requests-2026-10.jsonl");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let original = fs::metadata(tmp.path()).unwrap().permissions();
            fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o500)).unwrap();
            if fs::File::create(tmp.path().join("probe")).is_ok() {
                fs::set_permissions(tmp.path(), original).unwrap();
                return;
            }
            let err = clear_all(tmp.path()).unwrap_err();
            fs::set_permissions(tmp.path(), original).unwrap();
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        }
    }
}
