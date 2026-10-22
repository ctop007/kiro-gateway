//! Ties reader, aggregation and retention together.
//!
//! `LogStore` owns the records of the current and the previous local month.
//! Time never comes from the system clock here: every method that needs "now"
//! takes it as an argument and the timezone is a type parameter, so tests run
//! at any instant in any timezone. The Tauri glue passes `Utc::now()` and
//! `chrono::Local`.

use crate::request_log::agg::{recent, summarize, RecordFilter};
use crate::request_log::model::{
    LoggedRequest, RequestLogAppended, RequestLogSnapshot, YearMonth, DEFAULT_LIMIT, MAX_LIMIT,
};
use crate::request_log::reader::LogReader;
use crate::request_log::retention::{clear_all, prune_old_months};
use chrono::{DateTime, NaiveDate, SecondsFormat, TimeZone, Utc};
use std::fmt;
use std::io;
use std::path::PathBuf;

/// Result of one polling round.
#[derive(Debug, Clone, PartialEq)]
pub struct TickResult {
    /// Payload for the `request-log-appended` event; `None` when nothing
    /// changed for the UI.
    pub appended: Option<RequestLogAppended>,
    /// Number of month files that failed to read this round (retried next round).
    pub io_errors: u32,
}

/// `limit` outside 1..=1000.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidLimit(pub usize);

impl fmt::Display for InvalidLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "limit must be between 1 and {MAX_LIMIT}, got {}", self.0)
    }
}

impl std::error::Error for InvalidLimit {}

/// Records of the current and previous month plus the files they came from.
pub struct LogStore<Tz: TimeZone> {
    dir: PathBuf,
    tz: Tz,
    reader: LogReader,
    records: Vec<LoggedRequest>,
    current: YearMonth,
    last_day: NaiveDate,
}

impl<Tz: TimeZone> LogStore<Tz>
where
    Tz::Offset: fmt::Display,
{
    /// Prune old month files, then load the current and previous month.
    ///
    /// A missing directory is fine (the gateway creates it on the first
    /// request). Pruning problems do not stop the load.
    ///
    /// Args:
    ///     dir: Request log directory.
    ///     now: The current instant.
    ///     tz: Timezone that defines day and month.
    pub fn open(dir: PathBuf, now: DateTime<Utc>, tz: Tz) -> Self {
        let local = now.with_timezone(&tz).date_naive();
        let current = YearMonth::of_date(local);
        let mut store = Self {
            reader: LogReader::new(dir.clone()),
            dir,
            tz,
            records: Vec::new(),
            current,
            last_day: local,
        };
        let _ = prune_old_months(&store.dir, current);
        store.tick(now);
        store
    }

    /// Read new lines, handle day / month rollover and report what changed.
    ///
    /// Both the current and the previous month file are checked: a request
    /// that started just before midnight on the last day of a month is
    /// written to the old month's file after the new month has begun.
    ///
    /// Args:
    ///     now: The current instant.
    ///
    /// Returns:
    ///     An event payload when there are new records, when the local day or
    ///     month changed, or when a file was rewound or deleted (then with an
    ///     empty `records` and `reset` set).
    pub fn tick(&mut self, now: DateTime<Utc>) -> TickResult {
        let today = now.with_timezone(&self.tz).date_naive();
        let month = YearMonth::of_date(today);
        let mut changed = false;
        let mut rewound = false;
        let mut io_errors = 0;

        if month != self.current {
            self.current = month;
            let _ = prune_old_months(&self.dir, month);
            self.drop_months_outside_window();
            changed = true;
        }
        if today != self.last_day {
            self.last_day = today;
            changed = true;
        }

        let mut fresh: Vec<LoggedRequest> = Vec::new();
        for m in [self.current.prev(), self.current] {
            match self.reader.poll(m) {
                Ok(outcome) => {
                    if outcome.reset {
                        rewound = true;
                        self.records.retain(|r| r.source != m);
                    }
                    self.records.extend(outcome.records.iter().cloned());
                    fresh.extend(outcome.records);
                }
                Err(_) => io_errors += 1,
            }
        }

        if rewound {
            // Re-read records are not "new": the UI must reload the list.
            fresh.clear();
        }
        let appended = (!fresh.is_empty() || changed || rewound).then(|| RequestLogAppended {
            summary: summarize(&self.records, now, &self.tz),
            records: recent(&fresh, fresh.len(), &RecordFilter::default()),
            reset: rewound,
        });
        TickResult { appended, io_errors }
    }

    /// Summary plus the newest `limit` records.
    ///
    /// Args:
    ///     limit: `None` means 200; otherwise 1..=1000.
    ///     now: The current instant.
    ///
    /// Returns:
    ///     The snapshot, or `InvalidLimit`.
    pub fn snapshot(&self, limit: Option<usize>, now: DateTime<Utc>) -> Result<RequestLogSnapshot, InvalidLimit> {
        let limit = limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(InvalidLimit(limit));
        }
        Ok(self.build_snapshot(limit, now))
    }

    /// Delete every month file and forget everything read so far.
    ///
    /// On a deletion error the in-memory state is rebuilt from what is still
    /// on disk, so the view never shows records the user cleared nor hides
    /// ones that survived.
    ///
    /// Args:
    ///     now: The current instant.
    ///
    /// Returns:
    ///     An empty snapshot, or the deletion error.
    pub fn clear(&mut self, now: DateTime<Utc>) -> io::Result<RequestLogSnapshot> {
        let result = clear_all(&self.dir);
        self.reader.reset_all();
        self.records.clear();
        if result.is_err() {
            self.tick(now);
        }
        result.map(|_| self.build_snapshot(DEFAULT_LIMIT, now))
    }

    /// Bad lines skipped so far (for diagnostics).
    #[cfg(test)]
    pub fn skipped_lines(&self) -> u64 {
        self.reader.skipped_lines_total()
    }

    fn build_snapshot(&self, limit: usize, now: DateTime<Utc>) -> RequestLogSnapshot {
        RequestLogSnapshot {
            summary: summarize(&self.records, now, &self.tz),
            records: recent(&self.records, limit, &RecordFilter::default()),
            generated_at: now.with_timezone(&self.tz).to_rfc3339_opts(SecondsFormat::Secs, false),
        }
    }

    fn drop_months_outside_window(&mut self) {
        let previous = self.current.prev();
        let current = self.current;
        let in_window = |m: YearMonth| m == previous || m == current;
        let mut stale: Vec<YearMonth> = self.records.iter().map(|r| r.source).filter(|m| !in_window(*m)).collect();
        stale.sort();
        stale.dedup();
        for month in stale {
            self.reader.forget(month);
        }
        self.records.retain(|r| in_window(r.source));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_log::model::{sample_line, UNKNOWN_MODEL_LABEL};
    use chrono::FixedOffset;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::Path;
    use tempfile::TempDir;

    fn tz8() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn append(dir: &Path, file: &str, ts: &str, model: Option<&str>, outcome: &str, credits: f64) {
        let mut f = OpenOptions::new().create(true).append(true).open(dir.join(file)).unwrap();
        writeln!(f, "{}", sample_line(ts, model, outcome, credits)).unwrap();
    }

    const OPUS: Option<&str> = Some("claude-opus-5-5[1m]");

    #[test]
    fn open_loads_current_and_previous_month_and_deletes_older_files() {
        let tmp = TempDir::new().unwrap();
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-22T20:51:03.713+08:00", OPUS, "ok", 0.5);
        append(tmp.path(), "requests-2026-09.jsonl", "2026-09-30T23:59:00.000+08:00", OPUS, "ok", 1.0);
        append(tmp.path(), "requests-2026-08.jsonl", "2026-08-30T23:59:00.000+08:00", OPUS, "ok", 8.0);
        let now = at("2026-10-22T13:00:00Z");
        let store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        assert!(!tmp.path().join("requests-2026-08.jsonl").exists());
        assert!(tmp.path().join("requests-2026-09.jsonl").exists());
        let snap = store.snapshot(None, now).unwrap();
        assert_eq!(snap.records.len(), 2);
        assert_eq!(snap.records[0].ts, "2026-10-22T20:51:03.713+08:00");
        assert_eq!(snap.summary.month.credits, 0.5, "last month's credits are not this month's");
        assert_eq!(snap.summary.today.requests, 1);
        assert_eq!(snap.generated_at, "2026-10-22T21:00:00+08:00");
    }

    #[test]
    fn open_on_a_missing_directory_gives_an_empty_snapshot() {
        let tmp = TempDir::new().unwrap();
        let now = at("2026-10-22T13:00:00Z");
        let store = LogStore::open(tmp.path().join("logs/requests"), now, tz8());
        let snap = store.snapshot(None, now).unwrap();
        assert!(snap.records.is_empty());
        assert_eq!(snap.summary.month.month, "2026-10");
        assert_eq!(snap.summary.month.requests, 0);
    }

    #[test]
    fn tick_reports_new_records_once_newest_first_with_a_fresh_summary() {
        let tmp = TempDir::new().unwrap();
        let now = at("2026-10-22T13:00:00Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        assert_eq!(store.tick(now).appended, None, "quiet round sends nothing");

        let f = "requests-2026-10.jsonl";
        append(tmp.path(), f, "2026-10-22T20:59:01.000+08:00", OPUS, "ok", 0.25);
        append(tmp.path(), f, "2026-10-22T20:59:02.000+08:00", None, "error", 0.5);
        let tick = store.tick(now);
        let appended = tick.appended.unwrap();
        assert_eq!(appended.records.len(), 2);
        assert_eq!(appended.records[0].ts, "2026-10-22T20:59:02.000+08:00");
        assert!(!appended.reset);
        assert_eq!(appended.summary.month.requests, 2);
        assert_eq!(appended.summary.month.credits, 0.75);
        assert_eq!(appended.summary.by_model.len(), 2);
        assert_eq!(appended.summary.by_model[1].model.as_str(), "claude-opus-5-5[1m]");
        assert_eq!(appended.summary.by_model[0].model, UNKNOWN_MODEL_LABEL);
        assert_eq!(store.tick(now).appended, None);
    }

    #[test]
    fn tick_ignores_half_written_and_corrupt_lines() {
        let tmp = TempDir::new().unwrap();
        let now = at("2026-10-22T13:00:00Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        let path = tmp.path().join("requests-2026-10.jsonl");
        let good = sample_line("2026-10-22T20:59:01.000+08:00", OPUS, "ok", 0.25);
        fs::write(&path, format!("garbage
{good}
{}", &good[..30])).unwrap();
        let appended = store.tick(now).appended.unwrap();
        assert_eq!(appended.records.len(), 1);
        assert_eq!(appended.summary.month.requests, 1);
        assert_eq!(store.skipped_lines(), 1);
    }

    #[test]
    fn crossing_midnight_sends_an_empty_event_that_zeroes_today_only() {
        let tmp = TempDir::new().unwrap();
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-22T23:59:00.000+08:00", OPUS, "ok", 1.5);
        let before = at("2026-10-22T15:59:30Z"); // 23:59:30 local
        let mut store = LogStore::open(tmp.path().to_path_buf(), before, tz8());
        assert_eq!(store.snapshot(None, before).unwrap().summary.today.requests, 1);
        assert_eq!(store.tick(before).appended, None);

        let after = at("2026-10-22T16:00:00Z"); // 00:00 local on the 23rd
        let appended = store.tick(after).appended.unwrap();
        assert!(appended.records.is_empty());
        assert_eq!(appended.summary.today.requests, 0);
        assert_eq!(appended.summary.month.credits, 1.5);
        assert_eq!(store.tick(after).appended, None, "only once");
    }

    #[test]
    fn crossing_into_a_new_month_resets_the_month_and_prunes_the_oldest_file() {
        let tmp = TempDir::new().unwrap();
        append(tmp.path(), "requests-2026-09.jsonl", "2026-09-20T10:00:00.000+08:00", OPUS, "ok", 9.0);
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-31T23:59:00.000+08:00", OPUS, "ok", 1.5);
        let before = at("2026-10-31T15:59:30Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), before, tz8());
        assert_eq!(store.snapshot(None, before).unwrap().summary.month.credits, 1.5);

        let after = at("2026-10-31T16:00:00Z"); // 2026-11-01 00:00 local
        let appended = store.tick(after).appended.unwrap();
        assert_eq!(appended.summary.month.month, "2026-11");
        assert_eq!(appended.summary.month.credits, 0.0);
        assert_eq!(appended.summary.today.requests, 0);
        assert!(!tmp.path().join("requests-2026-09.jsonl").exists(), "September is now two months back");
        let snap = store.snapshot(None, after).unwrap();
        assert_eq!(snap.records.len(), 1, "October stays for the list; September is gone");
    }

    #[test]
    fn a_late_line_written_to_last_months_file_after_rollover_is_still_picked_up() {
        let tmp = TempDir::new().unwrap();
        let before = at("2026-10-31T15:59:30Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), before, tz8());
        let after = at("2026-10-31T16:00:30Z");
        store.tick(after);
        // The gateway finishes a request that started at 23:59:58 and files it under October.
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-31T23:59:58.000+08:00", OPUS, "ok", 2.0);
        let appended = store.tick(after).appended.unwrap();
        assert_eq!(appended.records.len(), 1);
        assert_eq!(appended.summary.month.credits, 0.0, "it belongs to October, not November");
    }

    #[test]
    fn first_request_of_a_new_month_is_read_from_the_new_file() {
        let tmp = TempDir::new().unwrap();
        let now = at("2026-11-01T01:00:00Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        append(tmp.path(), "requests-2026-11.jsonl", "2026-11-01T09:00:10.000+08:00", OPUS, "ok", 0.3);
        let appended = store.tick(now).appended.unwrap();
        assert_eq!(appended.summary.month.requests, 1);
        assert_eq!(appended.summary.today.requests, 1);
    }

    #[test]
    fn a_truncated_month_file_is_reloaded_and_the_event_is_flagged_as_reset() {
        let tmp = TempDir::new().unwrap();
        let f = "requests-2026-10.jsonl";
        append(tmp.path(), f, "2026-10-22T20:00:00.000+08:00", OPUS, "ok", 1.0);
        append(tmp.path(), f, "2026-10-22T20:01:00.000+08:00", OPUS, "ok", 1.0);
        let now = at("2026-10-22T13:00:00Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        fs::write(tmp.path().join(f), format!("{}
", sample_line("2026-10-22T20:30:00.000+08:00", OPUS, "ok", 0.1))).unwrap();
        let appended = store.tick(now).appended.unwrap();
        assert!(appended.reset);
        assert!(appended.records.is_empty());
        assert_eq!(appended.summary.month.requests, 1);
        let snap = store.snapshot(None, now).unwrap();
        assert_eq!(snap.records.len(), 1);
        assert_eq!(snap.records[0].ts, "2026-10-22T20:30:00.000+08:00");
    }

    #[test]
    fn an_externally_deleted_month_file_empties_the_view() {
        let tmp = TempDir::new().unwrap();
        let f = "requests-2026-10.jsonl";
        append(tmp.path(), f, "2026-10-22T20:00:00.000+08:00", OPUS, "ok", 1.0);
        let now = at("2026-10-22T13:00:00Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        fs::remove_file(tmp.path().join(f)).unwrap();
        let appended = store.tick(now).appended.unwrap();
        assert!(appended.reset);
        assert_eq!(appended.summary.month.requests, 0);
        assert!(store.snapshot(None, now).unwrap().records.is_empty());
    }

    #[test]
    fn snapshot_limit_defaults_to_200_and_rejects_out_of_range_values() {
        let tmp = TempDir::new().unwrap();
        for i in 0..250 {
            append(
                tmp.path(),
                "requests-2026-10.jsonl",
                &format!("2026-10-22T10:{:02}:{:02}.000+08:00", i / 60, i % 60),
                OPUS,
                "ok",
                0.01,
            );
        }
        let now = at("2026-10-22T13:00:00Z");
        let store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        assert_eq!(store.snapshot(None, now).unwrap().records.len(), 200);
        assert_eq!(store.snapshot(Some(1), now).unwrap().records.len(), 1);
        assert_eq!(store.snapshot(Some(1000), now).unwrap().records.len(), 250);
        assert_eq!(store.snapshot(Some(0), now).unwrap_err(), InvalidLimit(0));
        assert_eq!(store.snapshot(Some(1001), now).unwrap_err(), InvalidLimit(1001));
        assert_eq!(store.snapshot(None, now).unwrap().summary.month.requests, 250, "limit does not affect totals");
    }

    #[test]
    fn clear_removes_all_files_resets_the_view_and_later_requests_are_recorded() {
        let tmp = TempDir::new().unwrap();
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-22T20:00:00.000+08:00", OPUS, "ok", 1.0);
        append(tmp.path(), "requests-2026-09.jsonl", "2026-09-22T20:00:00.000+08:00", OPUS, "ok", 1.0);
        let now = at("2026-10-22T13:00:00Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        let snap = store.clear(now).unwrap();
        assert!(snap.records.is_empty());
        assert_eq!(snap.summary.month.requests, 0);
        assert_eq!(snap.summary.today.credits, 0.0);
        assert!(snap.summary.by_model.is_empty());
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
        assert_eq!(store.tick(now).appended, None);

        // The gateway recreates the file; the offset was reset so the first line is not lost.
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-22T21:00:00.000+08:00", OPUS, "ok", 0.4);
        let appended = store.tick(now).appended.unwrap();
        assert_eq!(appended.records.len(), 1);
        assert_eq!(appended.summary.month.credits, 0.4);
    }

    #[cfg(unix)]
    #[test]
    fn clear_failure_keeps_showing_what_is_still_on_disk() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-22T20:00:00.000+08:00", OPUS, "ok", 1.0);
        let now = at("2026-10-22T13:00:00Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        let original = fs::metadata(tmp.path()).unwrap().permissions();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o500)).unwrap();
        if fs::File::create(tmp.path().join("probe")).is_ok() {
            fs::set_permissions(tmp.path(), original).unwrap();
            return; // root ignores permissions
        }
        let err = store.clear(now).unwrap_err();
        fs::set_permissions(tmp.path(), original).unwrap();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        let snap = store.snapshot(None, now).unwrap();
        assert_eq!(snap.records.len(), 1, "the file survived, so the record is still shown");
    }

    #[test]
    fn io_errors_are_counted_and_retried_without_losing_state() {
        let tmp = TempDir::new().unwrap();
        let now = at("2026-10-22T13:00:00Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        fs::create_dir(tmp.path().join("requests-2026-10.jsonl")).unwrap();
        let tick = store.tick(now);
        assert_eq!(tick.io_errors, 1);
        assert_eq!(tick.appended, None);
        fs::remove_dir(tmp.path().join("requests-2026-10.jsonl")).unwrap();
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-22T20:00:00.000+08:00", OPUS, "ok", 1.0);
        let tick = store.tick(now);
        assert_eq!(tick.io_errors, 0);
        assert_eq!(tick.appended.unwrap().records.len(), 1);
    }

    #[test]
    fn snapshot_json_matches_the_ipc_contract_shape() {
        let tmp = TempDir::new().unwrap();
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-22T20:51:03.713+08:00", OPUS, "ok", 0.006173584610281925);
        let now = at("2026-10-22T13:00:00Z");
        let store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        let v = serde_json::to_value(store.snapshot(None, now).unwrap()).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, vec!["byModel", "generatedAt", "month", "records", "today"]);
        assert_eq!(v["month"]["month"], "2026-10");
        assert_eq!(v["month"]["requests"], 1);
        assert_eq!(v["today"]["credits"], 0.006173584610281925);
        assert_eq!(v["byModel"][0]["model"], "claude-opus-5-5[1m]");
        assert_eq!(v["records"][0]["durationMs"], 5894);
    }

    #[test]
    fn appended_json_has_summary_fields_and_records() {
        let tmp = TempDir::new().unwrap();
        let now = at("2026-10-22T13:00:00Z");
        let mut store = LogStore::open(tmp.path().to_path_buf(), now, tz8());
        append(tmp.path(), "requests-2026-10.jsonl", "2026-10-22T20:51:03.713+08:00", None, "error", 0.1);
        let v = serde_json::to_value(store.tick(now).appended.unwrap()).unwrap();
        for key in ["today", "month", "byModel", "records"] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert_eq!(v["byModel"][0]["model"], UNKNOWN_MODEL_LABEL);
    }
}
