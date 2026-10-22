//! Record and payload types for the request log (ipc-contract.md section 5).

#[cfg(test)]
use chrono::TimeZone;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Label used for records whose `model` is null.
pub const UNKNOWN_MODEL_LABEL: &str = "未知模型";

/// Default `limit` of `request_log_snapshot`.
pub const DEFAULT_LIMIT: usize = 200;

/// Largest accepted `limit` of `request_log_snapshot`.
pub const MAX_LIMIT: usize = 1000;

const FILE_PREFIX: &str = "requests-";
const FILE_SUFFIX: &str = ".jsonl";

/// API surface a request came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKind {
    Openai,
    Anthropic,
}

/// How a request ended, as classified by the gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Error,
    Refused,
    ClientDisconnected,
}

/// One log line, in the shape the frontend receives (camelCase).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRecord {
    /// Original `ts` string of the line (ISO 8601 with offset).
    pub ts: String,
    pub id: Option<String>,
    pub api: ApiKind,
    pub model: Option<String>,
    pub stream: bool,
    pub status: u16,
    pub outcome: Outcome,
    pub duration_ms: u64,
    /// Credits billed for the request; 0 when nothing was billed.
    pub credits: f64,
    pub error: Option<String>,
}

/// A parsed record plus the bookkeeping the store needs.
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedRequest {
    pub record: RequestRecord,
    /// `ts` as an instant, used for ordering and day / month bucketing.
    pub at: DateTime<Utc>,
    /// Month file the line was read from.
    pub source: YearMonth,
}

/// Credits and request count.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotal {
    pub credits: f64,
    pub requests: u64,
}

/// Month total; `month` looks like `2026-10`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthUsage {
    pub credits: f64,
    pub requests: u64,
    pub month: String,
}

/// Per-model usage of the current month.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub credits: f64,
    pub requests: u64,
}

/// Totals shown at the top of the log page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub today: UsageTotal,
    pub month: MonthUsage,
    pub by_model: Vec<ModelUsage>,
}

/// Result of `request_log_snapshot` / `request_log_clear`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogSnapshot {
    #[serde(flatten)]
    pub summary: UsageSummary,
    /// Newest first.
    pub records: Vec<RequestRecord>,
    pub generated_at: String,
}

/// Payload of the `request-log-appended` event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogAppended {
    #[serde(flatten)]
    pub summary: UsageSummary,
    /// Records read this round, newest first.
    pub records: Vec<RequestRecord>,
    /// True when a month file was deleted, truncated or replaced: the list
    /// the UI holds may contain records that no longer exist, so it should
    /// reload with `request_log_snapshot`. Not part of ipc-contract v1.
    pub reset: bool,
}

/// Why a line could not be turned into a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineError {
    /// Not valid UTF-8 JSON, wrong shape, or a missing / mistyped field.
    Json,
    /// `ts` is not an ISO 8601 timestamp with a UTC offset.
    Timestamp,
    /// `credits` is negative.
    NegativeCredits,
}

#[derive(Deserialize)]
struct RawLine {
    ts: String,
    #[serde(default)]
    id: Option<String>,
    api: ApiKind,
    #[serde(default)]
    model: Option<String>,
    stream: bool,
    status: u16,
    outcome: Outcome,
    duration_ms: u64,
    credits: f64,
    #[serde(default)]
    error: Option<String>,
}

/// Parse one complete log line (without its line terminator).
///
/// Args:
///     line: Raw bytes of the line.
///     source: Month file the line came from.
///
/// Returns:
///     The record, or the reason the line is unusable.
pub fn parse_line(line: &[u8], source: YearMonth) -> Result<LoggedRequest, LineError> {
    let raw: RawLine = serde_json::from_slice(line).map_err(|_| LineError::Json)?;
    // Python writes NaN / Infinity as bare tokens, which serde_json already
    // rejects; this guards the remaining negative case.
    if !(raw.credits >= 0.0) || !raw.credits.is_finite() {
        return Err(LineError::NegativeCredits);
    }
    let at = DateTime::parse_from_rfc3339(&raw.ts)
        .map_err(|_| LineError::Timestamp)?
        .with_timezone(&Utc);
    Ok(LoggedRequest {
        record: RequestRecord {
            ts: raw.ts,
            id: raw.id,
            api: raw.api,
            model: raw.model,
            stream: raw.stream,
            status: raw.status,
            outcome: raw.outcome,
            duration_ms: raw.duration_ms,
            credits: raw.credits,
            error: raw.error,
        },
        at,
        source,
    })
}

/// A calendar month, ordered chronologically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct YearMonth {
    pub year: i32,
    /// 1..=12
    pub month: u32,
}

impl YearMonth {
    /// Build a month; returns `None` when `month` is outside 1..=12.
    pub fn new(year: i32, month: u32) -> Option<Self> {
        (1..=12).contains(&month).then_some(Self { year, month })
    }

    /// Month containing a calendar date.
    pub fn of_date(date: NaiveDate) -> Self {
        Self { year: date.year(), month: date.month() }
    }

    /// Month containing an instant, in the given timezone.
    #[cfg(test)]
    pub fn of_instant<Tz: TimeZone>(at: DateTime<Utc>, tz: &Tz) -> Self {
        Self::of_date(at.with_timezone(tz).date_naive())
    }

    /// The month before this one.
    pub fn prev(self) -> Self {
        if self.month == 1 {
            Self { year: self.year - 1, month: 12 }
        } else {
            Self { year: self.year, month: self.month - 1 }
        }
    }

    /// `2026-10`
    pub fn label(self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }

    /// `requests-2026-10.jsonl`
    pub fn file_name(self) -> String {
        format!("{FILE_PREFIX}{}{FILE_SUFFIX}", self.label())
    }

    /// Inverse of `file_name`; `None` for any other file name.
    pub fn from_file_name(name: &str) -> Option<Self> {
        let body = name.strip_prefix(FILE_PREFIX)?.strip_suffix(FILE_SUFFIX)?;
        let bytes = body.as_bytes();
        if bytes.len() != 7 || bytes[4] != b'-' {
            return None;
        }
        let digits = |range: &[u8]| range.iter().all(u8::is_ascii_digit);
        if !digits(&bytes[..4]) || !digits(&bytes[5..]) {
            return None;
        }
        Self::new(body[..4].parse().ok()?, body[5..].parse().ok()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    pub(crate) const SAMPLE_TS: &str = "2026-10-22T20:51:03.713+08:00";

    pub(crate) fn sample_line(ts: &str, model: Option<&str>, outcome: &str, credits: f64) -> String {
        let model = model.map_or("null".to_string(), |m| format!("\"{m}\""));
        let status = if outcome == "ok" { 200 } else { 402 };
        let error = if outcome == "ok" { "null".to_string() } else { "\"quota exhausted\"".to_string() };
        format!(
            "{{\"ts\": \"{ts}\", \"id\": \"msg_01\", \"api\": \"anthropic\", \"model\": {model}, \"stream\": true, \"status\": {status}, \"outcome\": \"{outcome}\", \"duration_ms\": 5894, \"credits\": {credits}, \"error\": {error}}}"
        )
    }

    const OCT: YearMonth = YearMonth { year: 2026, month: 10 };

    #[test]
    fn parse_line_reads_a_real_gateway_line() {
        let line = sample_line(SAMPLE_TS, Some("claude-opus-5-5[1m]"), "ok", 0.006173584610281925);
        let parsed = parse_line(line.as_bytes(), OCT).unwrap();
        assert_eq!(parsed.record.ts, SAMPLE_TS);
        assert_eq!(parsed.record.id.as_deref(), Some("msg_01"));
        assert_eq!(parsed.record.api, ApiKind::Anthropic);
        assert_eq!(parsed.record.model.as_deref(), Some("claude-opus-5-5[1m]"));
        assert!(parsed.record.stream);
        assert_eq!(parsed.record.status, 200);
        assert_eq!(parsed.record.outcome, Outcome::Ok);
        assert_eq!(parsed.record.duration_ms, 5894);
        assert_eq!(parsed.record.credits, 0.006173584610281925);
        assert_eq!(parsed.record.error, None);
        assert_eq!(parsed.at, DateTime::parse_from_rfc3339("2026-10-22T12:51:03.713Z").unwrap());
        assert_eq!(parsed.source, OCT);
    }

    #[test]
    fn parse_line_accepts_null_model_null_id_and_integer_credits() {
        let line = r#"{"ts": "2026-10-22T20:51:03.713+08:00", "id": null, "api": "openai", "model": null, "stream": false, "status": 500, "outcome": "error", "duration_ms": 12, "credits": 0, "error": "boom"}"#;
        let parsed = parse_line(line.as_bytes(), OCT).unwrap();
        assert_eq!(parsed.record.id, None);
        assert_eq!(parsed.record.model, None);
        assert_eq!(parsed.record.credits, 0.0);
        assert_eq!(parsed.record.api, ApiKind::Openai);
    }

    #[test]
    fn parse_line_accepts_zero_offset_and_ignores_unknown_fields() {
        let line = r#"{"ts": "2026-10-22T12:51:03.713+00:00", "api": "openai", "stream": false, "status": 200, "outcome": "refused", "duration_ms": 1, "credits": 0.0, "future": [1,2]}"#;
        let parsed = parse_line(line.as_bytes(), OCT).unwrap();
        assert_eq!(parsed.record.outcome, Outcome::Refused);
        assert_eq!(parsed.record.id, None);
        assert_eq!(parsed.record.error, None);
    }

    #[test]
    fn parse_line_rejects_malformed_input() {
        let ok = sample_line(SAMPLE_TS, None, "ok", 0.1);
        let cases: Vec<(Vec<u8>, LineError)> = vec![
            (b"".to_vec(), LineError::Json),
            (b"not json".to_vec(), LineError::Json),
            (b"null".to_vec(), LineError::Json),
            (b"[]".to_vec(), LineError::Json),
            (b"{}".to_vec(), LineError::Json),
            (ok[..ok.len() - 10].as_bytes().to_vec(), LineError::Json),
            (ok.replace("\"ok\"", "\"weird\"").into_bytes(), LineError::Json),
            (ok.replace("\"anthropic\"", "\"gemini\"").into_bytes(), LineError::Json),
            (ok.replace("\"stream\": true", "\"stream\": \"yes\"").into_bytes(), LineError::Json),
            (ok.replace("5894", "-5").into_bytes(), LineError::Json),
            (ok.replace("\"status\": 200", "\"status\": 70000").into_bytes(), LineError::Json),
            (ok.replace("0.1", "NaN").into_bytes(), LineError::Json),
            (ok.replace("0.1", "-0.5").into_bytes(), LineError::NegativeCredits),
            (ok.replace(SAMPLE_TS, "yesterday").into_bytes(), LineError::Timestamp),
            (ok.replace(SAMPLE_TS, "2026-10-22T20:51:03.713").into_bytes(), LineError::Timestamp),
            (vec![0xff, 0xfe, b'{', b'}'], LineError::Json),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_line(&line, OCT).unwrap_err(), expected, "line: {:?}", String::from_utf8_lossy(&line));
        }
    }

    #[test]
    fn record_serializes_to_camel_case_with_null_keys_present() {
        let line = sample_line(SAMPLE_TS, None, "error", 0.1);
        let record = parse_line(line.as_bytes(), OCT).unwrap().record;
        let value = serde_json::to_value(&record).unwrap();
        let keys: Vec<&str> = {
            let mut k: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
            k.sort();
            k
        };
        assert_eq!(
            keys,
            vec!["api", "credits", "durationMs", "error", "id", "model", "outcome", "status", "stream", "ts"]
        );
        assert_eq!(value["model"], serde_json::Value::Null);
        assert_eq!(value["outcome"], "error");
        assert_eq!(value["durationMs"], 5894);
    }

    #[test]
    fn outcome_values_serialize_to_contract_strings() {
        let s = |o| serde_json::to_string(&o).unwrap();
        assert_eq!(s(Outcome::Ok), "\"ok\"");
        assert_eq!(s(Outcome::Error), "\"error\"");
        assert_eq!(s(Outcome::Refused), "\"refused\"");
        assert_eq!(s(Outcome::ClientDisconnected), "\"client_disconnected\"");
        assert_eq!(serde_json::to_string(&ApiKind::Openai).unwrap(), "\"openai\"");
    }

    #[test]
    fn year_month_prev_wraps_across_years() {
        assert_eq!(YearMonth::new(2026, 1).unwrap().prev(), YearMonth::new(2025, 12).unwrap());
        assert_eq!(YearMonth::new(2026, 10).unwrap().prev(), YearMonth::new(2026, 9).unwrap());
        assert!(YearMonth::new(2025, 12).unwrap() < YearMonth::new(2026, 1).unwrap());
    }

    #[test]
    fn year_month_file_name_round_trips() {
        assert_eq!(OCT.file_name(), "requests-2026-10.jsonl");
        assert_eq!(OCT.label(), "2026-10");
        assert_eq!(YearMonth::from_file_name("requests-2026-10.jsonl"), Some(OCT));
        assert_eq!(YearMonth::new(2026, 3).unwrap().file_name(), "requests-2026-03.jsonl");
    }

    #[test]
    fn year_month_from_file_name_rejects_lookalikes() {
        for name in [
            "requests-2026-13.jsonl",
            "requests-2026-00.jsonl",
            "requests-2026-1.jsonl",
            "requests-26-10.jsonl",
            "requests-2026-10.jsonl.bak",
            "requests-2026-10.json",
            "request-2026-10.jsonl",
            "Requests-2026-10.jsonl",
            "requests-2026-1a.jsonl",
            "requests-+026-10.jsonl",
            "requests-2026_10.jsonl",
            "requests-.jsonl",
            "",
        ] {
            assert_eq!(YearMonth::from_file_name(name), None, "{name}");
        }
    }

    #[test]
    fn year_month_of_instant_uses_the_given_timezone() {
        let tz = FixedOffset::east_opt(8 * 3600).unwrap();
        let at = DateTime::parse_from_rfc3339("2026-10-31T17:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(YearMonth::of_instant(at, &Utc), OCT);
        assert_eq!(YearMonth::of_instant(at, &tz), YearMonth::new(2026, 11).unwrap());
    }
}

#[cfg(test)]
pub(crate) use tests::{sample_line, SAMPLE_TS};
