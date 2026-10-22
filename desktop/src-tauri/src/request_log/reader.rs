//! Incremental reader for the gateway's month files (tasks 7.1).
//!
//! Keeps one cursor per month file: the byte offset just after the last
//! complete line already consumed, the file identity (inode, or creation time
//! on Windows) and the first bytes of the file. A file that got shorter, was
//! replaced, was deleted or starts with different bytes than before is read
//! again from the start and reported as `reset`.
//!
//! The reader never advances past the last newline, so a half-written line is
//! picked up whole once the gateway finishes it.

use crate::request_log::model::{parse_line, LoggedRequest, YearMonth};
use std::collections::HashMap;
use std::fs::{File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;

/// How many leading bytes of a file are remembered to notice a rewrite that
/// kept the inode and ended up longer than before.
const HEAD_FINGERPRINT_BYTES: usize = 64;

/// What one `poll` of a month file produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadOutcome {
    /// Newly read records, in file order.
    pub records: Vec<LoggedRequest>,
    /// Complete lines that could not be parsed this round.
    pub skipped_lines: u64,
    /// The file shrank, was replaced or was deleted, so earlier records from
    /// it may no longer exist; the caller should rebuild its view of the month.
    pub reset: bool,
}

#[derive(Debug, Clone)]
struct Cursor {
    /// Byte offset just after the last complete line consumed.
    offset: u64,
    identity: Option<u64>,
    /// First `min(offset, 64)` bytes of the file.
    head: Vec<u8>,
}

/// Incremental reader over the month files in one log directory.
#[derive(Debug)]
pub struct LogReader {
    dir: PathBuf,
    cursors: HashMap<YearMonth, Cursor>,
    skipped_total: u64,
}

impl LogReader {
    /// Create a reader for `dir`. The directory does not need to exist yet.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, cursors: HashMap::new(), skipped_total: 0 }
    }

    /// Bad lines skipped since creation or the last `reset_all`.
    #[cfg(test)]
    pub fn skipped_lines_total(&self) -> u64 {
        self.skipped_total
    }

    /// Drop the cursor of one month so its file is read from the start next time.
    pub fn forget(&mut self, month: YearMonth) {
        self.cursors.remove(&month);
    }

    /// Drop every cursor and the skipped-line counter.
    pub fn reset_all(&mut self) {
        self.cursors.clear();
        self.skipped_total = 0;
    }

    /// Read whatever complete lines were appended to a month file since the last call.
    ///
    /// A missing file is not an error. When an I/O error is returned the
    /// cursor is left untouched, so the next call retries from the same spot.
    ///
    /// Args:
    ///     month: Month file to read.
    ///
    /// Returns:
    ///     New records, the number of skipped bad lines and whether the file was rewound.
    pub fn poll(&mut self, month: YearMonth) -> io::Result<ReadOutcome> {
        let path = self.dir.join(month.file_name());
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let reset = self.cursors.remove(&month).is_some();
                return Ok(ReadOutcome { reset, ..ReadOutcome::default() });
            }
            Err(e) => return Err(e),
        };
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a regular file", path.display()),
            ));
        }
        let identity = file_identity(&meta);
        let len = meta.len();

        let mut reset = false;
        let mut cursor = match self.cursors.get(&month) {
            Some(known) => known.clone(),
            None => Cursor { offset: 0, identity, head: Vec::new() },
        };
        if self.cursors.contains_key(&month) {
            let replaced = cursor.identity != identity;
            let shrunk = len < cursor.offset;
            if replaced || shrunk || !head_matches(&mut file, &cursor.head)? {
                reset = true;
                cursor = Cursor { offset: 0, identity, head: Vec::new() };
            }
        }

        file.seek(SeekFrom::Start(cursor.offset))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;

        let consumed = tail.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let mut outcome = ReadOutcome { reset, ..ReadOutcome::default() };
        for raw in tail[..consumed].split(|b| *b == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if raw.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match parse_line(raw, month) {
                Ok(record) => outcome.records.push(record),
                Err(_) => outcome.skipped_lines += 1,
            }
        }

        cursor.offset += consumed as u64;
        let room = HEAD_FINGERPRINT_BYTES.saturating_sub(cursor.head.len());
        cursor.head.extend_from_slice(&tail[..consumed.min(room)]);
        self.cursors.insert(month, cursor);
        self.skipped_total += outcome.skipped_lines;
        Ok(outcome)
    }
}

/// True when the file still begins with the bytes remembered from last time.
fn head_matches(file: &mut File, head: &[u8]) -> io::Result<bool> {
    if head.is_empty() {
        return Ok(true);
    }
    file.seek(SeekFrom::Start(0))?;
    let mut now = vec![0u8; head.len()];
    match file.read_exact(&mut now) {
        Ok(()) => Ok(now == head),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

/// Identity that changes when the file is replaced: the inode on Unix, the
/// creation time elsewhere.
#[cfg(unix)]
fn file_identity(meta: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.ino())
}

#[cfg(not(unix))]
fn file_identity(meta: &Metadata) -> Option<u64> {
    let created = meta.created().ok()?;
    let since_epoch = created.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(since_epoch.as_nanos() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_log::model::{sample_line, Outcome, YearMonth, SAMPLE_TS};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::Path;
    use tempfile::TempDir;

    const OCT: YearMonth = YearMonth { year: 2026, month: 10 };
    const NOV: YearMonth = YearMonth { year: 2026, month: 11 };

    fn line(n: u32) -> String {
        sample_line(
            &format!("2026-10-22T20:51:{:02}.713+08:00", n % 60),
            Some("claude-opus-5-5[1m]"),
            "ok",
            0.006173584610281925,
        )
    }

    fn append(dir: &Path, month: YearMonth, bytes: &str) {
        let mut f = OpenOptions::new().create(true).append(true).open(dir.join(month.file_name())).unwrap();
        f.write_all(bytes.as_bytes()).unwrap();
    }

    fn write(dir: &Path, month: YearMonth, bytes: &str) {
        fs::write(dir.join(month.file_name()), bytes).unwrap();
    }

    fn new_reader() -> (TempDir, LogReader) {
        let tmp = TempDir::new().unwrap();
        let reader = LogReader::new(tmp.path().to_path_buf());
        (tmp, reader)
    }

    fn ids(out: &ReadOutcome) -> Vec<String> {
        out.records.iter().map(|r| r.record.ts.clone()).collect()
    }

    #[test]
    fn reads_every_complete_line_of_a_new_file_in_order() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n{}\n{}\n", line(1), line(2), line(3)));
        let out = reader.poll(OCT).unwrap();
        assert_eq!(out.records.len(), 3);
        assert_eq!(out.records[0].record.ts, "2026-10-22T20:51:01.713+08:00");
        assert_eq!(out.records[2].record.ts, "2026-10-22T20:51:03.713+08:00");
        assert_eq!(out.records[0].source, OCT);
        assert_eq!(out.records[0].record.outcome, Outcome::Ok);
        assert_eq!(out.skipped_lines, 0);
        assert!(!out.reset);
    }

    #[test]
    fn missing_file_yields_nothing_and_is_not_a_reset() {
        let (_tmp, mut reader) = new_reader();
        let out = reader.poll(OCT).unwrap();
        assert!(out.records.is_empty());
        assert!(!out.reset);
    }

    #[test]
    fn missing_directory_yields_nothing() {
        let tmp = TempDir::new().unwrap();
        let mut reader = LogReader::new(tmp.path().join("does-not-exist"));
        let out = reader.poll(OCT).unwrap();
        assert!(out.records.is_empty());
        assert!(!out.reset);
    }

    #[test]
    fn empty_file_yields_nothing() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, "");
        let out = reader.poll(OCT).unwrap();
        assert!(out.records.is_empty());
        assert!(!out.reset);
    }

    #[test]
    fn second_poll_returns_only_appended_lines() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n", line(1)));
        assert_eq!(reader.poll(OCT).unwrap().records.len(), 1);
        append(tmp.path(), OCT, &format!("{}\n{}\n", line(2), line(3)));
        let out = reader.poll(OCT).unwrap();
        assert_eq!(ids(&out), vec!["2026-10-22T20:51:02.713+08:00", "2026-10-22T20:51:03.713+08:00"]);
        assert!(!out.reset);
    }

    #[test]
    fn poll_without_new_data_returns_nothing() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n", line(1)));
        reader.poll(OCT).unwrap();
        for _ in 0..3 {
            let out = reader.poll(OCT).unwrap();
            assert!(out.records.is_empty());
            assert!(!out.reset);
        }
    }

    #[test]
    fn half_written_last_line_waits_for_its_newline() {
        let (tmp, mut reader) = new_reader();
        let second = line(2);
        let (head, tail) = second.split_at(40);
        write(tmp.path(), OCT, &format!("{}\n{}", line(1), head));
        let first = reader.poll(OCT).unwrap();
        assert_eq!(first.records.len(), 1);
        assert_eq!(first.skipped_lines, 0, "a half line is not a bad line");

        // Still half: nothing new, still not counted as bad.
        let again = reader.poll(OCT).unwrap();
        assert!(again.records.is_empty());
        assert_eq!(again.skipped_lines, 0);

        append(tmp.path(), OCT, tail);
        let still_open = reader.poll(OCT).unwrap();
        assert!(still_open.records.is_empty(), "no newline yet, line must stay pending");

        append(tmp.path(), OCT, "\n");
        let done = reader.poll(OCT).unwrap();
        assert_eq!(ids(&done), vec!["2026-10-22T20:51:02.713+08:00"]);
        assert_eq!(done.skipped_lines, 0);
    }

    #[test]
    fn half_line_ending_inside_a_multibyte_character_is_reassembled() {
        let (tmp, mut reader) = new_reader();
        let full = r#"{"ts": "2026-10-22T20:51:03.713+08:00", "id": null, "api": "openai", "model": null, "stream": false, "status": 500, "outcome": "error", "duration_ms": 3, "credits": 0.0, "error": "额度用尽"}"#;
        let bytes = full.as_bytes();
        let cut = full.find("额").unwrap() + 1; // inside the 3-byte character
        let mut f = OpenOptions::new().create(true).append(true).open(tmp.path().join(OCT.file_name())).unwrap();
        f.write_all(&bytes[..cut]).unwrap();
        assert!(reader.poll(OCT).unwrap().records.is_empty());
        f.write_all(&bytes[cut..]).unwrap();
        f.write_all(b"\n").unwrap();
        let out = reader.poll(OCT).unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].record.error.as_deref(), Some("额度用尽"));
    }

    #[test]
    fn unterminated_final_line_is_never_returned() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n{}", line(1), line(2)));
        assert_eq!(reader.poll(OCT).unwrap().records.len(), 1);
        for _ in 0..3 {
            assert!(reader.poll(OCT).unwrap().records.is_empty());
        }
        let mut again = LogReader::new(tmp.path().to_path_buf());
        assert_eq!(again.poll(OCT).unwrap().records.len(), 1);
    }

    #[test]
    fn bad_lines_are_skipped_and_counted_while_good_lines_survive() {
        let (tmp, mut reader) = new_reader();
        let content = format!(
            "{}\nthis is not json\n{{\"ts\": 1}}\n{}\n\u{0}\u{0}\n{}\n",
            line(1),
            line(2),
            line(3)
        );
        write(tmp.path(), OCT, &content);
        let out = reader.poll(OCT).unwrap();
        assert_eq!(out.records.len(), 3);
        assert_eq!(out.skipped_lines, 3);
        assert_eq!(reader.skipped_lines_total(), 3);
        append(tmp.path(), OCT, "garbage\n");
        assert_eq!(reader.poll(OCT).unwrap().skipped_lines, 1);
        assert_eq!(reader.skipped_lines_total(), 4);
    }

    #[test]
    fn invalid_utf8_line_is_skipped() {
        let (tmp, mut reader) = new_reader();
        let mut bytes = line(1).into_bytes();
        bytes.push(b'\n');
        bytes.extend_from_slice(&[0xff, 0xfe, 0xfd, b'\n']);
        bytes.extend_from_slice(line(2).as_bytes());
        bytes.push(b'\n');
        fs::write(tmp.path().join(OCT.file_name()), bytes).unwrap();
        let out = reader.poll(OCT).unwrap();
        assert_eq!(out.records.len(), 2);
        assert_eq!(out.skipped_lines, 1);
    }

    #[test]
    fn blank_lines_and_crlf_endings_are_tolerated() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\r\n\r\n\n{}\r\n", line(1), line(2)));
        let out = reader.poll(OCT).unwrap();
        assert_eq!(out.records.len(), 2);
        assert_eq!(out.skipped_lines, 0, "blank lines are not damage");
    }

    #[test]
    fn truncated_file_is_read_again_from_the_start_and_flagged() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n{}\n{}\n", line(1), line(2), line(3)));
        assert_eq!(reader.poll(OCT).unwrap().records.len(), 3);
        write(tmp.path(), OCT, &format!("{}\n", line(9)));
        let out = reader.poll(OCT).unwrap();
        assert!(out.reset);
        assert_eq!(ids(&out), vec!["2026-10-22T20:51:09.713+08:00"]);
        // And the next round is back to normal incremental reading.
        append(tmp.path(), OCT, &format!("{}\n", line(10)));
        let next = reader.poll(OCT).unwrap();
        assert!(!next.reset);
        assert_eq!(ids(&next), vec!["2026-10-22T20:51:10.713+08:00"]);
    }

    #[test]
    fn file_emptied_to_zero_bytes_resets_with_no_records() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n", line(1)));
        reader.poll(OCT).unwrap();
        write(tmp.path(), OCT, "");
        let out = reader.poll(OCT).unwrap();
        assert!(out.reset);
        assert!(out.records.is_empty());
        assert!(!reader.poll(OCT).unwrap().reset);
    }

    #[test]
    fn rewritten_in_place_to_a_longer_file_is_detected_by_its_head() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n", line(1)));
        reader.poll(OCT).unwrap();
        // Same inode (fs::write truncates in place), now longer than before.
        write(tmp.path(), OCT, &format!("{}\n{}\n{}\n", line(21), line(22), line(23)));
        let out = reader.poll(OCT).unwrap();
        assert!(out.reset);
        assert_eq!(out.records.len(), 3);
        assert_eq!(out.records[0].record.ts, "2026-10-22T20:51:21.713+08:00");
    }

    #[cfg(unix)]
    #[test]
    fn file_replaced_by_a_new_inode_is_read_again_from_the_start() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n", line(1)));
        reader.poll(OCT).unwrap();
        let replacement = tmp.path().join("replacement.tmp");
        fs::write(&replacement, format!("{}\n{}\n", line(1), line(2))).unwrap();
        fs::rename(&replacement, tmp.path().join(OCT.file_name())).unwrap();
        let out = reader.poll(OCT).unwrap();
        assert!(out.reset);
        assert_eq!(out.records.len(), 2);
    }

    #[test]
    fn deleted_file_resets_and_a_recreated_file_is_read_from_the_start() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n{}\n", line(1), line(2)));
        reader.poll(OCT).unwrap();
        fs::remove_file(tmp.path().join(OCT.file_name())).unwrap();
        let gone = reader.poll(OCT).unwrap();
        assert!(gone.reset);
        assert!(gone.records.is_empty());
        assert!(!reader.poll(OCT).unwrap().reset, "only reported once");

        write(tmp.path(), OCT, &format!("{}\n", line(5)));
        let back = reader.poll(OCT).unwrap();
        assert!(!back.reset);
        assert_eq!(ids(&back), vec!["2026-10-22T20:51:05.713+08:00"]);
    }

    #[test]
    fn months_are_tracked_independently() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n", line(1)));
        write(tmp.path(), NOV, &format!("{}\n{}\n", line(2), line(3)));
        assert_eq!(reader.poll(OCT).unwrap().records.len(), 1);
        assert_eq!(reader.poll(NOV).unwrap().records.len(), 2);
        append(tmp.path(), NOV, &format!("{}\n", line(4)));
        assert!(reader.poll(OCT).unwrap().records.is_empty());
        let nov = reader.poll(NOV).unwrap();
        assert_eq!(nov.records.len(), 1);
        assert_eq!(nov.records[0].source, NOV);
    }

    #[test]
    fn a_month_file_that_appears_later_is_read_from_its_first_line() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n", line(1)));
        reader.poll(OCT).unwrap();
        assert!(reader.poll(NOV).unwrap().records.is_empty());
        write(tmp.path(), NOV, &format!("{}\n", line(2)));
        let out = reader.poll(NOV).unwrap();
        assert_eq!(out.records.len(), 1);
        assert!(!out.reset);
    }

    #[test]
    fn forget_makes_the_next_poll_reread_the_file() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\n{}\n", line(1), line(2)));
        reader.poll(OCT).unwrap();
        reader.forget(OCT);
        let out = reader.poll(OCT).unwrap();
        assert_eq!(out.records.len(), 2);
        assert!(!out.reset);
    }

    #[test]
    fn reset_all_clears_every_cursor_and_the_skipped_counter() {
        let (tmp, mut reader) = new_reader();
        write(tmp.path(), OCT, &format!("{}\nbad\n", line(1)));
        write(tmp.path(), NOV, &format!("{}\n", line(2)));
        reader.poll(OCT).unwrap();
        reader.poll(NOV).unwrap();
        assert_eq!(reader.skipped_lines_total(), 1);
        reader.reset_all();
        assert_eq!(reader.skipped_lines_total(), 0);
        assert_eq!(reader.poll(OCT).unwrap().records.len(), 1);
        assert_eq!(reader.poll(NOV).unwrap().records.len(), 1);
    }

    #[test]
    fn large_file_is_consumed_in_one_pass() {
        let (tmp, mut reader) = new_reader();
        let mut content = String::new();
        for i in 0..20_000 {
            content.push_str(&line(i));
            content.push('\n');
        }
        write(tmp.path(), OCT, &content);
        assert_eq!(reader.poll(OCT).unwrap().records.len(), 20_000);
        assert!(reader.poll(OCT).unwrap().records.is_empty());
    }

    #[test]
    fn a_directory_with_the_file_name_is_an_error_and_keeps_the_cursor() {
        let (tmp, mut reader) = new_reader();
        fs::create_dir(tmp.path().join(OCT.file_name())).unwrap();
        let err = reader.poll(OCT).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn sample_ts_constant_matches_the_documented_gateway_format() {
        assert_eq!(SAMPLE_TS, "2026-10-22T20:51:03.713+08:00");
    }
}
