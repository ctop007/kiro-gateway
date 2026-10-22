//! In-memory aggregation of request records (tasks 7.2).
//!
//! All day and month bucketing is done on the record's own instant (`ts`)
//! converted to the injected timezone, never on the month file it came from,
//! so a request that started at 23:59 on the last day of a month is counted in
//! that month whatever file it ended up in.

use crate::request_log::model::{
    LoggedRequest, ModelUsage, MonthUsage, Outcome, RequestRecord, UsageSummary, UsageTotal, YearMonth,
    UNKNOWN_MODEL_LABEL,
};
use chrono::{DateTime, TimeZone, Utc};
use std::collections::HashMap;

/// Narrowing of the record list. It never affects `summarize`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordFilter {
    /// Keep only records whose outcome is not `ok`.
    pub failed_only: bool,
    /// Keep only records of this model; `未知模型` selects records without one.
    pub model: Option<String>,
}

impl RecordFilter {
    fn matches(&self, record: &RequestRecord) -> bool {
        if self.failed_only && record.outcome == Outcome::Ok {
            return false;
        }
        match &self.model {
            Some(wanted) => model_label(record) == wanted,
            None => true,
        }
    }
}

/// Group name of a record: its model, or `未知模型` when missing or empty.
fn model_label(record: &RequestRecord) -> &str {
    match record.model.as_deref() {
        Some(name) if !name.is_empty() => name,
        _ => UNKNOWN_MODEL_LABEL,
    }
}

/// Totals for the local day and month containing `now`, plus this month per model.
///
/// Every record counts, whatever its outcome, so credits billed to failed
/// requests are included. `month.credits` is the sum of the `by_model` entries
/// in their listed order, so the groups always add up to it exactly.
///
/// Args:
///     records: All records in memory (current and previous month files).
///     now: The current instant.
///     tz: Timezone that defines "day" and "month".
///
/// Returns:
///     The summary; `by_model` is sorted by credits, then requests, descending,
///     then by name.
pub fn summarize<Tz: TimeZone>(records: &[LoggedRequest], now: DateTime<Utc>, tz: &Tz) -> UsageSummary {
    let today = now.with_timezone(tz).date_naive();
    let month = YearMonth::of_date(today);

    let mut today_total = UsageTotal { credits: 0.0, requests: 0 };
    let mut groups: HashMap<&str, (f64, u64)> = HashMap::new();
    for entry in records {
        let local = entry.at.with_timezone(tz).date_naive();
        if YearMonth::of_date(local) != month {
            continue;
        }
        let group = groups.entry(model_label(&entry.record)).or_insert((0.0, 0));
        group.0 += entry.record.credits;
        group.1 += 1;
        if local == today {
            today_total.credits += entry.record.credits;
            today_total.requests += 1;
        }
    }

    let mut by_model: Vec<ModelUsage> = groups
        .into_iter()
        .map(|(name, (credits, requests))| ModelUsage { model: name.to_string(), credits, requests })
        .collect();
    by_model.sort_by(|a, b| {
        b.credits
            .total_cmp(&a.credits)
            .then(b.requests.cmp(&a.requests))
            .then_with(|| a.model.cmp(&b.model))
    });
    let credits = by_model.iter().map(|m| m.credits).sum();
    let requests = by_model.iter().map(|m| m.requests).sum();

    UsageSummary {
        today: today_total,
        month: MonthUsage { credits, requests, month: month.label() },
        by_model,
    }
}

/// The newest `limit` records matching `filter`, newest first.
///
/// Ordered by instant; records with the same instant keep "later arrival
/// first" so the list reads like the log tail.
pub fn recent(records: &[LoggedRequest], limit: usize, filter: &RecordFilter) -> Vec<RequestRecord> {
    let mut picked: Vec<(usize, &LoggedRequest)> =
        records.iter().enumerate().filter(|(_, r)| filter.matches(&r.record)).collect();
    picked.sort_by(|(ia, a), (ib, b)| b.at.cmp(&a.at).then(ib.cmp(ia)));
    picked.into_iter().take(limit).map(|(_, r)| r.record.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_log::model::{parse_line, sample_line, LoggedRequest, YearMonth};
    use chrono::{DateTime, FixedOffset, Utc};

    fn tz8() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn rec(ts: &str, model: Option<&str>, outcome: &str, credits: f64) -> LoggedRequest {
        let line = sample_line(ts, model, outcome, credits);
        let at = DateTime::parse_from_rfc3339(ts).unwrap().with_timezone(&Utc);
        parse_line(line.as_bytes(), YearMonth::of_instant(at, &Utc)).unwrap()
    }

    fn opus(ts: &str, credits: f64) -> LoggedRequest {
        rec(ts, Some("claude-opus-5-5[1m]"), "ok", credits)
    }

    // ---- summarize ----------------------------------------------------

    #[test]
    fn empty_input_gives_zero_totals_and_no_groups() {
        let s = summarize(&[], at("2026-10-22T12:00:00Z"), &tz8());
        assert_eq!(s.today, UsageTotal { credits: 0.0, requests: 0 });
        assert_eq!(s.month.credits, 0.0);
        assert_eq!(s.month.requests, 0);
        assert_eq!(s.month.month, "2026-10");
        assert!(s.by_model.is_empty());
    }

    #[test]
    fn today_and_month_totals_use_the_local_calendar() {
        let rows = vec![
            opus("2026-10-22T20:51:03.713+08:00", 0.25),
            opus("2026-10-22T00:00:00.000+08:00", 0.5),
            opus("2026-10-21T23:59:59.999+08:00", 1.0),
            opus("2026-10-01T00:00:00.000+08:00", 2.0),
            opus("2026-09-30T23:59:59.999+08:00", 4.0),
        ];
        let s = summarize(&rows, at("2026-10-22T13:00:00Z"), &tz8()); // 21:00 local
        assert_eq!(s.today.requests, 2);
        assert_eq!(s.today.credits, 0.75);
        assert_eq!(s.month.requests, 4);
        assert_eq!(s.month.credits, 3.75);
        assert_eq!(s.month.month, "2026-10");
    }

    #[test]
    fn one_minute_before_and_after_local_midnight_land_on_different_days() {
        let rows = vec![
            opus("2026-10-22T23:59:00.000+08:00", 1.0),
            opus("2026-10-23T00:00:00.000+08:00", 2.0),
        ];
        let before = summarize(&rows, at("2026-10-22T15:59:30Z"), &tz8()); // 23:59:30 local on the 22nd
        assert_eq!((before.today.requests, before.today.credits), (1, 1.0));
        let after = summarize(&rows, at("2026-10-22T16:00:00Z"), &tz8()); // 00:00:00 local on the 23rd
        assert_eq!((after.today.requests, after.today.credits), (1, 2.0));
        assert_eq!(after.month.requests, 2, "month total is unchanged by the day rollover");
    }

    #[test]
    fn last_day_of_month_rolls_over_into_a_fresh_month_total() {
        let rows = vec![
            opus("2026-10-31T23:59:59.000+08:00", 1.5),
            opus("2026-11-01T00:00:00.000+08:00", 0.5),
        ];
        let oct = summarize(&rows, at("2026-10-31T15:59:59Z"), &tz8());
        assert_eq!(oct.month.month, "2026-10");
        assert_eq!((oct.month.requests, oct.month.credits), (1, 1.5));
        assert_eq!((oct.today.requests, oct.today.credits), (1, 1.5));
        let nov = summarize(&rows, at("2026-10-31T16:00:00Z"), &tz8());
        assert_eq!(nov.month.month, "2026-11");
        assert_eq!((nov.month.requests, nov.month.credits), (1, 0.5));
        assert_eq!((nov.today.requests, nov.today.credits), (1, 0.5));
    }

    #[test]
    fn year_end_rolls_over_to_january() {
        let rows = vec![
            opus("2026-12-31T23:30:00.000+08:00", 1.0),
            opus("2027-01-01T00:30:00.000+08:00", 2.0),
        ];
        let s = summarize(&rows, at("2026-12-31T16:30:00Z"), &tz8());
        assert_eq!(s.month.month, "2027-01");
        assert_eq!(s.month.credits, 2.0);
    }

    #[test]
    fn records_written_in_another_offset_are_bucketed_in_the_injected_timezone() {
        // 2026-10-22T23:30-05:00 is 2026-10-23T12:30+08:00.
        let rows = vec![opus("2026-10-22T23:30:00.000-05:00", 3.0)];
        let local_day = summarize(&rows, at("2026-10-23T04:00:00Z"), &tz8()); // 12:00 local on the 23rd
        assert_eq!(local_day.today.requests, 1);
        let west = FixedOffset::west_opt(5 * 3600).unwrap();
        let in_west = summarize(&rows, at("2026-10-23T04:00:00Z"), &west); // 23:00 on the 22nd
        assert_eq!(in_west.today.requests, 1);
        let next_day_west = summarize(&rows, at("2026-10-23T06:00:00Z"), &west); // 01:00 on the 23rd
        assert_eq!(next_day_west.today.requests, 0);
        assert_eq!(next_day_west.month.requests, 1);
    }

    #[test]
    fn failed_refused_and_disconnected_requests_still_count_their_credits() {
        let rows = vec![
            rec("2026-10-22T10:00:00.000+08:00", Some("m"), "error", 0.1),
            rec("2026-10-22T10:01:00.000+08:00", Some("m"), "refused", 0.2),
            rec("2026-10-22T10:02:00.000+08:00", Some("m"), "client_disconnected", 0.4),
            rec("2026-10-22T10:03:00.000+08:00", Some("m"), "ok", 0.8),
        ];
        let s = summarize(&rows, at("2026-10-22T08:00:00Z"), &tz8());
        assert_eq!(s.today.requests, 4);
        assert!((s.today.credits - 1.5).abs() < 1e-12);
        assert!((s.month.credits - 1.5).abs() < 1e-12);
    }

    #[test]
    fn zero_credit_requests_count_as_requests() {
        let rows = vec![rec("2026-10-22T10:00:00.000+08:00", Some("m"), "error", 0.0)];
        let s = summarize(&rows, at("2026-10-22T08:00:00Z"), &tz8());
        assert_eq!((s.today.requests, s.today.credits), (1, 0.0));
        assert_eq!(s.by_model.len(), 1);
    }

    #[test]
    fn by_model_groups_sorted_by_credits_descending_and_sum_to_the_month_total() {
        let rows = vec![
            rec("2026-10-22T10:00:00.000+08:00", Some("claude-haiku-4-5"), "ok", 0.1),
            rec("2026-10-22T10:01:00.000+08:00", Some("claude-opus-5-5[1m]"), "ok", 0.006173584610281925),
            rec("2026-10-22T10:02:00.000+08:00", Some("claude-opus-5-5[1m]"), "error", 0.3),
            rec("2026-10-22T10:03:00.000+08:00", Some("claude-sonnet-5-5[1m]"), "ok", 0.25),
            rec("2026-10-22T10:04:00.000+08:00", Some("claude-haiku-4-5"), "ok", 0.1),
            rec("2026-09-22T10:04:00.000+08:00", Some("claude-haiku-4-5"), "ok", 99.0),
        ];
        let s = summarize(&rows, at("2026-10-22T08:00:00Z"), &tz8());
        let names: Vec<&str> = s.by_model.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(names, vec!["claude-opus-5-5[1m]", "claude-sonnet-5-5[1m]", "claude-haiku-4-5"]);
        assert_eq!(s.by_model[0].requests, 2);
        assert_eq!(s.by_model[2].requests, 2);
        let credits_sum: f64 = s.by_model.iter().map(|m| m.credits).sum();
        let requests_sum: u64 = s.by_model.iter().map(|m| m.requests).sum();
        assert_eq!(credits_sum, s.month.credits, "bit-exact, summed in listed order");
        assert_eq!(requests_sum, s.month.requests);
    }

    #[test]
    fn group_sum_equals_total_for_awkward_floats() {
        let mut rows = Vec::new();
        for i in 0..500u32 {
            let model = ["a", "b", "c", "d", "e"][(i % 5) as usize];
            let credits = 0.006173584610281925 * f64::from(i % 17 + 1) / 3.0;
            rows.push(rec(&format!("2026-10-{:02}T10:00:00.000+08:00", i % 28 + 1), Some(model), "ok", credits));
        }
        let s = summarize(&rows, at("2026-10-28T08:00:00Z"), &tz8());
        let sum: f64 = s.by_model.iter().map(|m| m.credits).sum();
        assert_eq!(sum, s.month.credits);
        assert_eq!(s.month.requests, 500);
    }

    #[test]
    fn null_and_empty_model_are_grouped_as_unknown_model() {
        let rows = vec![
            rec("2026-10-22T10:00:00.000+08:00", None, "error", 0.5),
            rec("2026-10-22T10:01:00.000+08:00", Some(""), "error", 0.25),
            rec("2026-10-22T10:02:00.000+08:00", Some("claude-haiku-4-5"), "ok", 0.1),
        ];
        let s = summarize(&rows, at("2026-10-22T08:00:00Z"), &tz8());
        assert_eq!(s.by_model.len(), 2);
        assert_eq!(s.by_model[0].model, "未知模型");
        assert_eq!(s.by_model[0].requests, 2);
        assert_eq!(s.by_model[0].credits, 0.75);
    }

    #[test]
    fn equal_credits_order_by_requests_then_name_so_output_is_stable() {
        let rows = vec![
            rec("2026-10-22T10:00:00.000+08:00", Some("b"), "ok", 0.0),
            rec("2026-10-22T10:01:00.000+08:00", Some("a"), "ok", 0.0),
            rec("2026-10-22T10:02:00.000+08:00", Some("c"), "ok", 0.0),
            rec("2026-10-22T10:03:00.000+08:00", Some("c"), "ok", 0.0),
        ];
        let s = summarize(&rows, at("2026-10-22T08:00:00Z"), &tz8());
        let names: Vec<&str> = s.by_model.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(names, vec!["c", "a", "b"]);
    }

    #[test]
    fn records_from_last_month_never_enter_month_groups() {
        let rows = vec![opus("2026-09-30T23:59:59.999+08:00", 5.0)];
        let s = summarize(&rows, at("2026-10-01T00:00:01Z"), &tz8());
        assert!(s.by_model.is_empty());
        assert_eq!(s.month.requests, 0);
    }

    // ---- recent / filter ------------------------------------------------

    fn ts_of(rows: &[crate::request_log::model::RequestRecord]) -> Vec<String> {
        rows.iter().map(|r| r.ts.clone()).collect()
    }

    #[test]
    fn recent_returns_newest_first_and_at_most_limit() {
        let rows = vec![
            opus("2026-10-22T10:00:00.000+08:00", 0.1),
            opus("2026-10-22T10:02:00.000+08:00", 0.1),
            opus("2026-10-22T10:01:00.000+08:00", 0.1),
        ];
        let all = recent(&rows, 10, &RecordFilter::default());
        assert_eq!(
            ts_of(&all),
            vec!["2026-10-22T10:02:00.000+08:00", "2026-10-22T10:01:00.000+08:00", "2026-10-22T10:00:00.000+08:00"]
        );
        let two = recent(&rows, 2, &RecordFilter::default());
        assert_eq!(two.len(), 2);
        assert_eq!(two[0].ts, "2026-10-22T10:02:00.000+08:00");
        assert!(recent(&rows, 0, &RecordFilter::default()).is_empty());
        assert!(recent(&[], 5, &RecordFilter::default()).is_empty());
    }

    #[test]
    fn recent_orders_by_instant_not_by_text_when_offsets_differ() {
        // 10:00+08:00 is 02:00Z, which is earlier than 05:00-05:00 = 10:00Z.
        let rows = vec![opus("2026-10-22T10:00:00.000+08:00", 0.1), opus("2026-10-22T05:00:00.000-05:00", 0.1)];
        let out = recent(&rows, 5, &RecordFilter::default());
        assert_eq!(out[0].ts, "2026-10-22T05:00:00.000-05:00");
    }

    #[test]
    fn recent_puts_later_arrivals_first_when_timestamps_tie() {
        let mut a = opus("2026-10-22T10:00:00.000+08:00", 0.1);
        a.record.id = Some("first".into());
        let mut b = opus("2026-10-22T10:00:00.000+08:00", 0.1);
        b.record.id = Some("second".into());
        let out = recent(&[a, b], 5, &RecordFilter::default());
        assert_eq!(out[0].id.as_deref(), Some("second"));
    }

    #[test]
    fn failed_only_filter_keeps_everything_that_is_not_ok() {
        let rows = vec![
            rec("2026-10-22T10:00:00.000+08:00", Some("m"), "ok", 0.1),
            rec("2026-10-22T10:01:00.000+08:00", Some("m"), "error", 0.1),
            rec("2026-10-22T10:02:00.000+08:00", Some("m"), "refused", 0.1),
            rec("2026-10-22T10:03:00.000+08:00", Some("m"), "client_disconnected", 0.1),
        ];
        let filter = RecordFilter { failed_only: true, model: None };
        let out = recent(&rows, 10, &filter);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|r| r.outcome != crate::request_log::model::Outcome::Ok));
    }

    #[test]
    fn model_filter_matches_exact_name_and_the_unknown_label() {
        let rows = vec![
            rec("2026-10-22T10:00:00.000+08:00", Some("claude-haiku-4-5"), "ok", 0.1),
            rec("2026-10-22T10:01:00.000+08:00", Some("claude-opus-5-5[1m]"), "ok", 0.1),
            rec("2026-10-22T10:02:00.000+08:00", None, "error", 0.1),
        ];
        let haiku = RecordFilter { failed_only: false, model: Some("claude-haiku-4-5".into()) };
        assert_eq!(recent(&rows, 10, &haiku).len(), 1);
        let unknown = RecordFilter { failed_only: false, model: Some("未知模型".into()) };
        assert_eq!(recent(&rows, 10, &unknown).len(), 1);
        let none = RecordFilter { failed_only: false, model: Some("claude-haiku".into()) };
        assert!(recent(&rows, 10, &none).is_empty());
        let both = RecordFilter { failed_only: true, model: Some("claude-haiku-4-5".into()) };
        assert!(recent(&rows, 10, &both).is_empty());
    }

    #[test]
    fn filtering_the_list_never_changes_the_summary() {
        let rows = vec![
            rec("2026-10-22T10:00:00.000+08:00", Some("claude-haiku-4-5"), "ok", 0.1),
            rec("2026-10-22T10:01:00.000+08:00", Some("claude-opus-5-5[1m]"), "error", 0.4),
        ];
        let now = at("2026-10-22T08:00:00Z");
        let before = summarize(&rows, now, &tz8());
        let filtered = recent(&rows, 10, &RecordFilter { failed_only: true, model: Some("claude-opus-5-5[1m]".into()) });
        assert_eq!(filtered.len(), 1);
        assert_eq!(summarize(&rows, now, &tz8()), before);
        assert_eq!(before.today.requests, 2);
    }
}
