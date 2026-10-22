use super::*;
use std::fs;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    v.sort();
    v
}

#[test]
fn constants_match_the_design_five_megabytes_three_files() {
    assert_eq!(MAX_LOG_BYTES, 5 * 1024 * 1024);
    assert_eq!(LOG_FILES, 3);
}

#[test]
fn creates_the_file_and_parent_directory_and_appends() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("logs").join("gateway.log");
    let mut log = RotatingLog::open(&path, 1000, 3).unwrap();
    log.write_line(b"first").unwrap();
    log.write_line(b"second").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\n");
}

#[test]
fn reopening_appends_to_the_existing_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    RotatingLog::open(&path, 1000, 3).unwrap().write_line(b"one").unwrap();
    RotatingLog::open(&path, 1000, 3).unwrap().write_line(b"two").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
}

#[test]
fn rotates_before_exceeding_the_limit_and_keeps_whole_lines() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let mut log = RotatingLog::open(&path, 20, 3).unwrap();
    log.write_line(b"aaaaaaaaa").unwrap(); // 10 bytes with newline
    log.write_line(b"bbbbbbbbb").unwrap(); // 20 bytes, fits exactly
    log.write_line(b"ccccccccc").unwrap(); // would be 30: rotate first
    assert_eq!(fs::read_to_string(&path).unwrap(), "ccccccccc\n");
    assert_eq!(fs::read_to_string(rotated(&path, 1)).unwrap(), "aaaaaaaaa\nbbbbbbbbb\n");
}

#[test]
fn keeps_at_most_the_configured_number_of_files() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let mut log = RotatingLog::open(&path, 10, 3).unwrap();
    for i in 0..10 {
        log.write_line(format!("line-{i:03}").as_bytes()).unwrap(); // 9 bytes each, one per file
    }
    assert_eq!(names(dir.path()), vec!["gateway.log", "gateway.log.1", "gateway.log.2"]);
    assert_eq!(fs::read_to_string(&path).unwrap(), "line-009\n");
    assert_eq!(fs::read_to_string(rotated(&path, 1)).unwrap(), "line-008\n");
    assert_eq!(fs::read_to_string(rotated(&path, 2)).unwrap(), "line-007\n");
}

#[test]
fn total_size_stays_below_files_times_limit() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let mut log = RotatingLog::open(&path, 1000, 3).unwrap();
    for i in 0..500 {
        log.write_line(format!("a reasonably long line number {i} with padding ........").as_bytes()).unwrap();
    }
    let total: u64 = names(dir.path()).iter().map(|n| fs::metadata(dir.path().join(n)).unwrap().len()).sum();
    assert!(total <= 3 * 1000, "total {total}");
}

#[test]
fn a_single_line_longer_than_the_limit_is_still_written_whole() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let mut log = RotatingLog::open(&path, 10, 3).unwrap();
    log.write_line(&[b'x'; 50]).unwrap();
    log.write_line(b"next").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"next\n");
    assert_eq!(fs::read(rotated(&path, 1)).unwrap().len(), 51);
}

#[test]
fn a_single_file_configuration_just_truncates() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let mut log = RotatingLog::open(&path, 10, 1).unwrap();
    log.write_line(b"aaaaaaaaa").unwrap();
    log.write_line(b"bbbbbbbbb").unwrap();
    assert_eq!(names(dir.path()), vec!["gateway.log"]);
    assert_eq!(fs::read_to_string(&path).unwrap(), "bbbbbbbbb\n");
}

#[test]
fn rotation_starts_correctly_when_reopening_a_nearly_full_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    fs::write(&path, "aaaaaaaaa\n").unwrap();
    let mut log = RotatingLog::open(&path, 15, 3).unwrap();
    log.write_line(b"bbbbbbbbb").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "bbbbbbbbb\n");
    assert_eq!(fs::read_to_string(rotated(&path, 1)).unwrap(), "aaaaaaaaa\n");
}

#[test]
fn works_when_the_log_file_was_deleted_while_running() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let mut log = RotatingLog::open(&path, 1000, 3).unwrap();
    log.write_line(b"before").unwrap();
    fs::remove_file(&path).unwrap();
    log.write_line(b"after").unwrap();
    // The handle keeps pointing at the unlinked inode on Unix; a rotation re-creates the path.
    let mut log2 = RotatingLog::open(&path, 1000, 3).unwrap();
    log2.write_line(b"fresh").unwrap();
    assert!(fs::read_to_string(&path).unwrap().contains("fresh"));
}

#[test]
fn pump_copies_lines_and_tolerates_invalid_utf8_and_missing_final_newline() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let log = Arc::new(Mutex::new(RotatingLog::open(&path, 1000, 3).unwrap()));
    let input = Cursor::new(b"one\ntwo\xff\xfe\nthree".to_vec());
    pump(input, &log);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes, b"one\ntwo\xff\xfe\nthree\n");
}

#[test]
fn pump_splits_overlong_lines_instead_of_buffering_forever() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let log = Arc::new(Mutex::new(RotatingLog::open(&path, 10 * 1024 * 1024, 3).unwrap()));
    let long = vec![b'z'; MAX_LINE_BYTES * 2 + 10];
    pump(Cursor::new(long), &log);
    let text = fs::read(&path).unwrap();
    assert!(text.iter().filter(|b| **b == b'\n').count() >= 3);
    assert_eq!(text.iter().filter(|b| **b == b'z').count(), MAX_LINE_BYTES * 2 + 10);
}

#[test]
fn two_pumps_never_interleave_inside_a_line() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let log = Arc::new(Mutex::new(RotatingLog::open(&path, 10 * 1024 * 1024, 3).unwrap()));
    let a: Vec<u8> = (0..500).flat_map(|i| format!("AAAA-{i}-AAAA\n").into_bytes()).collect();
    let b: Vec<u8> = (0..500).flat_map(|i| format!("BBBB-{i}-BBBB\n").into_bytes()).collect();
    let (la, lb) = (log.clone(), log.clone());
    let ta = std::thread::spawn(move || pump(Cursor::new(a), &la));
    let tb = std::thread::spawn(move || pump(Cursor::new(b), &lb));
    ta.join().unwrap();
    tb.join().unwrap();
    for line in fs::read_to_string(&path).unwrap().lines() {
        assert!(
            (line.starts_with("AAAA-") && line.ends_with("-AAAA")) || (line.starts_with("BBBB-") && line.ends_with("-BBBB")),
            "mixed line: {line}"
        );
    }
}

#[test]
fn tail_returns_the_last_lines_after_the_last_run_marker() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let mut log = RotatingLog::open(&path, 100_000, 3).unwrap();
    log.write_line(b"old run line 1").unwrap();
    log.write_line(RUN_MARKER.as_bytes()).unwrap();
    log.write_line(b"new run line 1").unwrap();
    log.write_line(b"new run line 2").unwrap();
    assert_eq!(read_run_tail(&path, 50), vec!["new run line 1", "new run line 2"]);
}

#[test]
fn tail_without_a_marker_returns_the_last_n_lines() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let lines: Vec<String> = (1..=30).map(|n| format!("l{n}")).collect();
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    let tail = read_run_tail(&path, 5);
    assert_eq!(tail, vec!["l26", "l27", "l28", "l29", "l30"]);
}

#[test]
fn tail_of_a_missing_or_empty_file_is_empty() {
    let dir = tempdir().unwrap();
    assert!(read_run_tail(&dir.path().join("nope.log"), 20).is_empty());
    let empty = dir.path().join("empty.log");
    fs::write(&empty, "").unwrap();
    assert!(read_run_tail(&empty, 20).is_empty());
}

#[test]
fn tail_of_a_huge_file_reads_only_the_end() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let mut text = String::new();
    for i in 0..60_000 {
        text.push_str(&format!("line {i} {}\n", "x".repeat(40)));
    }
    fs::write(&path, &text).unwrap();
    let tail = read_run_tail(&path, 3);
    assert_eq!(tail.len(), 3);
    assert!(tail[2].starts_with("line 59999 "));
}

#[test]
fn tail_drops_a_partial_first_line_cut_by_the_read_window() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    let mut text = String::new();
    for i in 0..30_000 {
        text.push_str(&format!("line {i:05}\n"));
    }
    fs::write(&path, &text).unwrap();
    // Ask for more lines than the window can hold: every returned line must be whole.
    for line in read_run_tail(&path, 100_000) {
        assert!(line.starts_with("line ") && line.len() == 10, "{line:?}");
    }
}

#[test]
fn tail_replaces_invalid_utf8_instead_of_failing() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.log");
    fs::write(&path, b"ok\nbad \xff\xfe bytes\n").unwrap();
    let tail = read_run_tail(&path, 5);
    assert_eq!(tail.len(), 2);
    assert!(tail[1].starts_with("bad "));
}

#[test]
fn marker_line_is_free_of_secrets_and_paths() {
    assert!(RUN_MARKER.is_ascii());
    assert!(!RUN_MARKER.contains('/'));
}
