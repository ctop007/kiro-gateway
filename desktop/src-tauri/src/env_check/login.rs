//! Read-only probe of the kiro-cli login database (tasks 5.2).
//!
//! kiro-cli keeps its credentials in a sqlite file with one table
//! `auth_kv(key TEXT, value TEXT)`. The gateway reads the same rows, so the key
//! priority here mirrors `SQLITE_TOKEN_KEYS` in `kiro/auth.py`.
//!
//! Guarantees:
//! - The file is opened with a `file:<path>?mode=ro` URI plus `SQLITE_OPEN_READONLY`
//!   and a 2 second busy timeout; kiro-cli running at the same time (even with an
//!   open write transaction) neither blocks nor is blocked.
//! - Token strings never leave [`read_login`]: the public result types hold only
//!   the login method, the expiry and whether a refresh token exists, and their
//!   `Debug` / `Serialize` output cannot contain anything else.
//!
//! Database locations on Windows are NOT confirmed on a real machine yet.

use super::{Platform, UserDirs};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Serialize, Serializer};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a read waits for a lock held by kiro-cli (design: `busy_timeout`).
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(2000);

/// `auth_kv` keys in priority order, with the login method each one stands for.
const TOKEN_KEYS: [(&str, LoginMethod); 3] = [
    ("kirocli:social:token", LoginMethod::Social),
    ("kirocli:odic:token", LoginMethod::AwsSso),
    ("codewhisperer:odic:token", LoginMethod::AwsSso),
];

/// How the user signed in to kiro-cli.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginMethod {
    /// Google / GitHub / Microsoft social login.
    Social,
    /// AWS IAM Identity Center or Builder ID (OIDC).
    AwsSso,
}

impl LoginMethod {
    /// User-facing label, as shown in the check list.
    pub fn label(self) -> &'static str {
        match self {
            LoginMethod::Social => "社交账号登录",
            LoginMethod::AwsSso => "AWS SSO 登录",
        }
    }
}

fn serialize_expiry<S: Serializer>(
    value: &Option<DateTime<FixedOffset>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(t) => serializer.serialize_str(&t.to_rfc3339()),
        None => serializer.serialize_none(),
    }
}

/// What the check may know about a stored login. Deliberately has no token field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginInfo {
    pub method: LoginMethod,
    /// Access token expiry; `None` when missing or unparsable.
    #[serde(serialize_with = "serialize_expiry")]
    pub expires_at: Option<DateTime<FixedOffset>>,
    /// Whether a non-empty refresh token is stored.
    pub refreshable: bool,
}

impl LoginInfo {
    /// Whether the gateway can still use this login at `now`.
    ///
    /// A login is usable when the access token has not expired, or when it can
    /// be refreshed (the gateway refreshes on its own). An unknown expiry counts
    /// as expired, like in the gateway.
    pub fn is_valid(&self, now: DateTime<FixedOffset>) -> bool {
        if self.refreshable {
            return true;
        }
        match self.expires_at {
            Some(expiry) => expiry > now,
            None => false,
        }
    }
}

/// Outcome of probing one database file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginProbe {
    /// The file does not exist (or is not a regular file).
    NoDatabase,
    /// The file opens but holds no usable token row: no `auth_kv` table or none of the keys.
    NoCredentials,
    /// A token row exists but its value is not the JSON object kiro-cli writes.
    Corrupt,
    /// The file could not be read (not a sqlite file, locked longer than the timeout, I/O error).
    Unreadable,
    /// A token row was read.
    Found(LoginInfo),
}

/// Candidate database locations for `platform`, in priority order.
///
/// - macOS: `~/Library/Application Support/kiro-cli/data.sqlite3` is the real
///   location; the two `~/.local/share` paths are the XDG layout and the older
///   Amazon Q name, which the gateway docs mention.
/// - Windows: `%APPDATA%\kiro-cli\data.sqlite3` and `%LOCALAPPDATA%\kiro-cli\data.sqlite3`.
///   NOT confirmed on a real Windows machine.
pub fn database_candidates(platform: Platform, dirs: &UserDirs) -> Vec<PathBuf> {
    let mut out = Vec::new();
    match platform {
        Platform::MacOs => {
            if let Some(home) = &dirs.home {
                out.push(home.join("Library").join("Application Support").join("kiro-cli").join("data.sqlite3"));
                out.push(home.join(".local").join("share").join("kiro-cli").join("data.sqlite3"));
                out.push(home.join(".local").join("share").join("amazon-q").join("data.sqlite3"));
            }
        }
        Platform::Windows => {
            if let Some(roaming) = &dirs.app_data {
                out.push(roaming.join("kiro-cli").join("data.sqlite3"));
            }
            if let Some(local) = &dirs.local_app_data {
                out.push(local.join("kiro-cli").join("data.sqlite3"));
            }
        }
    }
    out
}

/// First candidate that exists as a file.
pub fn find_database(platform: Platform, dirs: &UserDirs) -> Option<PathBuf> {
    database_candidates(platform, dirs).into_iter().find(|p| p.is_file())
}

/// Build the `file:` URI sqlite needs for read-only access.
///
/// Everything except unreserved characters, `/` and `:` is percent-escaped (as
/// UTF-8 bytes), because `?`, `#` and `%` would otherwise end or corrupt the
/// path part. `windows_path` converts `C:\a\b` into `file:///C:/a/b`.
pub fn sqlite_read_only_uri(path: &Path, windows_path: bool) -> String {
    let raw = path.to_string_lossy();
    let normalized = if windows_path { raw.replace('\\', "/") } else { raw.to_string() };
    let mut escaped = String::with_capacity(normalized.len() + 16);
    for byte in normalized.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                escaped.push(byte as char)
            }
            other => escaped.push_str(&format!("%{other:02X}")),
        }
    }
    if windows_path && !escaped.starts_with('/') {
        format!("file:///{escaped}?mode=ro")
    } else {
        format!("file:{escaped}?mode=ro")
    }
}

/// Open `path` read-only with the given busy timeout.
pub fn open_read_only(path: &Path, busy_timeout: Duration) -> rusqlite::Result<Connection> {
    let uri = sqlite_read_only_uri(path, cfg!(windows));
    let conn = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(busy_timeout)?;
    Ok(conn)
}

/// Parse kiro-cli's `expires_at`: RFC 3339 with `Z`, an offset, or up to nanoseconds.
/// A value without any offset is read as UTC.
fn parse_expiry(raw: &str) -> Option<DateTime<FixedOffset>> {
    let raw = raw.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(raw) {
        return Some(t);
    }
    let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    Some(Utc.from_utc_datetime(&naive).fixed_offset())
}

fn non_empty_string(value: Option<&serde_json::Value>) -> bool {
    matches!(value, Some(serde_json::Value::String(s)) if !s.trim().is_empty())
}

/// Turn one stored value into [`LoginInfo`]. The token strings are dropped here.
fn interpret_value(method: LoginMethod, raw: &[u8]) -> Option<LoginInfo> {
    let parsed: serde_json::Value = serde_json::from_slice(raw).ok()?;
    let object = parsed.as_object()?;
    let refreshable = non_empty_string(object.get("refresh_token"));
    if !refreshable && !non_empty_string(object.get("access_token")) {
        return None;
    }
    let expires_at = object
        .get("expires_at")
        .and_then(serde_json::Value::as_str)
        .and_then(parse_expiry);
    Some(LoginInfo { method, expires_at, refreshable })
}

/// Read the token row from an open connection.
fn read_login(conn: &Connection) -> rusqlite::Result<LoginProbe> {
    let table: Option<String> = conn
        .query_row("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'auth_kv'", [], |r| r.get(0))
        .optional()?;
    if table.is_none() {
        return Ok(LoginProbe::NoCredentials);
    }
    for (key, method) in TOKEN_KEYS {
        let value: Option<Value> = conn
            .query_row("SELECT value FROM auth_kv WHERE key = ?1 LIMIT 1", [key], |r| r.get(0))
            .optional()?;
        // The first key that has a row decides, exactly like the gateway.
        let Some(value) = value else { continue };
        let bytes = match value {
            Value::Text(s) => s.into_bytes(),
            Value::Blob(b) => b,
            _ => return Ok(LoginProbe::Corrupt),
        };
        return Ok(match interpret_value(method, &bytes) {
            Some(info) => LoginProbe::Found(info),
            None => LoginProbe::Corrupt,
        });
    }
    Ok(LoginProbe::NoCredentials)
}

/// Probe one database file without ever writing to it.
///
/// Args:
///     path: The sqlite file.
///     busy_timeout: How long to wait for a lock held by kiro-cli.
///
/// Returns:
///     What was found; see [`LoginProbe`]. Never contains token strings.
pub fn probe_database(path: &Path, busy_timeout: Duration) -> LoginProbe {
    if !path.is_file() {
        return LoginProbe::NoDatabase;
    }
    let Ok(conn) = open_read_only(path, busy_timeout) else {
        return LoginProbe::Unreadable;
    };
    read_login(&conn).unwrap_or(LoginProbe::Unreadable)
}

/// Locate the kiro-cli database on this machine and probe it.
///
/// Returns [`LoginProbe::NoDatabase`] when no candidate exists.
pub fn probe_default(platform: Platform, dirs: &UserDirs) -> LoginProbe {
    match find_database(platform, dirs) {
        Some(path) => probe_database(&path, BUSY_TIMEOUT),
        None => LoginProbe::NoDatabase,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset, TimeZone, Utc};
    use rusqlite::Connection;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    const FAKE_ACCESS: &str = "FAKE-ACCESS-TOKEN-do-not-leak-1234567890";
    const FAKE_REFRESH: &str = "FAKE-REFRESH-TOKEN-do-not-leak-0987654321";
    const FAKE_SECRET: &str = "FAKE-CLIENT-SECRET-do-not-leak";
    const SOCIAL: &str = "kirocli:social:token";
    const ODIC: &str = "kirocli:odic:token";
    const LEGACY: &str = "codewhisperer:odic:token";
    const SHORT_WAIT: Duration = Duration::from_millis(150);

    fn now() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2026, 10, 22, 12, 0, 0).unwrap()
    }

    fn token_json(expires_at: &str, refresh: Option<&str>) -> String {
        let mut v = serde_json::json!({
            "access_token": FAKE_ACCESS,
            "expires_at": expires_at,
            "region": "us-east-1",
            "start_url": "https://example.awsapps.com/start",
            "client_secret": FAKE_SECRET,
        });
        if let Some(r) = refresh {
            v["refresh_token"] = serde_json::Value::String(r.to_string());
        }
        v.to_string()
    }

    fn db_with(dir: &Path, rows: &[(&str, &str)]) -> PathBuf {
        let path = dir.join("data.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute("CREATE TABLE auth_kv (key TEXT PRIMARY KEY, value TEXT)", []).unwrap();
        for (k, v) in rows {
            conn.execute("INSERT INTO auth_kv (key, value) VALUES (?1, ?2)", [k, v]).unwrap();
        }
        path
    }

    fn found(probe: LoginProbe) -> LoginInfo {
        match probe {
            LoginProbe::Found(info) => info,
            other => panic!("expected Found, got {other:?}"),
        }
    }

    // ---------- database situations of tasks 5.2 ----------

    #[test]
    fn probe_missing_database_is_no_database() {
        let tmp = tempfile::tempdir().unwrap();
        let got = probe_database(&tmp.path().join("nope.sqlite3"), SHORT_WAIT);
        assert_eq!(got, LoginProbe::NoDatabase);
    }

    #[test]
    fn probe_database_path_that_is_a_directory_is_no_database() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(probe_database(tmp.path(), SHORT_WAIT), LoginProbe::NoDatabase);
    }

    #[test]
    fn probe_database_without_auth_kv_table_is_no_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("data.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute("CREATE TABLE other (a TEXT)", []).unwrap();
        drop(conn);
        assert_eq!(probe_database(&path, SHORT_WAIT), LoginProbe::NoCredentials);
    }

    #[test]
    fn probe_empty_file_without_schema_is_no_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("data.sqlite3");
        fs::write(&path, b"").unwrap();
        assert_eq!(probe_database(&path, SHORT_WAIT), LoginProbe::NoCredentials);
    }

    #[test]
    fn probe_database_without_token_keys_is_no_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[("some.other.key", "{}"), ("kirocli:odic:device-registration", "{}")]);
        assert_eq!(probe_database(&path, SHORT_WAIT), LoginProbe::NoCredentials);
    }

    #[test]
    fn probe_file_that_is_not_sqlite_is_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("data.sqlite3");
        fs::write(&path, vec![b'x'; 4096]).unwrap();
        assert!(matches!(probe_database(&path, SHORT_WAIT), LoginProbe::Unreadable));
    }

    #[test]
    fn probe_value_that_is_not_json_is_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, "this is { not json")]);
        assert_eq!(probe_database(&path, SHORT_WAIT), LoginProbe::Corrupt);
    }

    #[test]
    fn probe_json_values_of_the_wrong_shape_are_corrupt() {
        for value in ["[]", "\"str\"", "null", "42", "{}", "{\"region\":\"us-east-1\"}"] {
            let tmp = tempfile::tempdir().unwrap();
            let path = db_with(tmp.path(), &[(SOCIAL, value)]);
            assert_eq!(probe_database(&path, SHORT_WAIT), LoginProbe::Corrupt, "value {value}");
        }
    }

    #[test]
    fn probe_null_value_is_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("data.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute("CREATE TABLE auth_kv (key TEXT, value TEXT)", []).unwrap();
        conn.execute("INSERT INTO auth_kv VALUES (?1, NULL)", [SOCIAL]).unwrap();
        drop(conn);
        assert_eq!(probe_database(&path, SHORT_WAIT), LoginProbe::Corrupt);
    }

    #[test]
    fn probe_blob_value_holding_json_is_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("data.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute("CREATE TABLE auth_kv (key TEXT, value BLOB)", []).unwrap();
        let json = token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH));
        conn.execute("INSERT INTO auth_kv VALUES (?1, ?2)", rusqlite::params![SOCIAL, json.as_bytes()]).unwrap();
        drop(conn);
        assert!(found(probe_database(&path, SHORT_WAIT)).refreshable);
    }

    // ---------- key priority and login method ----------

    #[test]
    fn social_key_means_social_login() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        assert_eq!(found(probe_database(&path, SHORT_WAIT)).method, LoginMethod::Social);
    }

    #[test]
    fn odic_and_legacy_keys_mean_aws_sso() {
        for key in [ODIC, LEGACY] {
            let tmp = tempfile::tempdir().unwrap();
            let path = db_with(tmp.path(), &[(key, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
            assert_eq!(found(probe_database(&path, SHORT_WAIT)).method, LoginMethod::AwsSso, "key {key}");
        }
    }

    #[test]
    fn social_wins_over_odic_and_odic_wins_over_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(
            tmp.path(),
            &[
                (LEGACY, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH))),
                (ODIC, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH))),
                (SOCIAL, &token_json("2031-01-01T00:00:00Z", None)),
            ],
        );
        let info = found(probe_database(&path, SHORT_WAIT));
        assert_eq!(info.method, LoginMethod::Social);
        assert!(!info.refreshable, "the social row has no refresh token, so no other row may fill in");

        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(
            tmp.path(),
            &[
                (LEGACY, &token_json("2030-01-01T00:00:00Z", None)),
                (ODIC, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH))),
            ],
        );
        assert!(found(probe_database(&path, SHORT_WAIT)).refreshable);
    }

    #[test]
    fn first_key_that_exists_decides_even_if_it_is_corrupt() {
        // The gateway reads the first existing row too, so the check must agree with it.
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(
            tmp.path(),
            &[(SOCIAL, "garbage"), (ODIC, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))],
        );
        assert_eq!(probe_database(&path, SHORT_WAIT), LoginProbe::Corrupt);
    }

    // ---------- expiry parsing ----------

    fn expiry_of(raw: &str) -> Option<DateTime<FixedOffset>> {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json(raw, Some(FAKE_REFRESH)))]);
        found(probe_database(&path, SHORT_WAIT)).expires_at
    }

    #[test]
    fn expiry_with_z_suffix_is_utc() {
        let got = expiry_of("2026-10-22T04:00:00Z").unwrap();
        assert_eq!(got, Utc.with_ymd_and_hms(2026, 10, 22, 4, 0, 0).unwrap());
    }

    #[test]
    fn expiry_with_positive_offset_is_the_same_instant() {
        let got = expiry_of("2026-10-22T12:00:00+08:00").unwrap();
        assert_eq!(got, Utc.with_ymd_and_hms(2026, 10, 22, 4, 0, 0).unwrap());
    }

    #[test]
    fn expiry_with_nanoseconds_parses() {
        let got = expiry_of("2026-10-22T04:00:00.123456789Z").unwrap();
        assert_eq!(got.timestamp(), Utc.with_ymd_and_hms(2026, 10, 22, 4, 0, 0).unwrap().timestamp());
        let got = expiry_of("2026-10-22T12:00:00.987654321+08:00").unwrap();
        assert_eq!(got.timestamp(), Utc.with_ymd_and_hms(2026, 10, 22, 4, 0, 0).unwrap().timestamp());
    }

    #[test]
    fn expiry_without_offset_is_read_as_utc() {
        let got = expiry_of("2026-10-22T04:00:00").unwrap();
        assert_eq!(got, Utc.with_ymd_and_hms(2026, 10, 22, 4, 0, 0).unwrap());
    }

    #[test]
    fn expiry_that_is_garbage_or_missing_is_none() {
        assert_eq!(expiry_of("tomorrow"), None);
        assert_eq!(expiry_of(""), None);
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &format!("{{\"access_token\":\"{FAKE_ACCESS}\"}}"))]);
        assert_eq!(found(probe_database(&path, SHORT_WAIT)).expires_at, None);
    }

    #[test]
    fn expiry_that_is_not_a_string_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &format!("{{\"access_token\":\"{FAKE_ACCESS}\",\"expires_at\":1790000000}}"))]);
        assert_eq!(found(probe_database(&path, SHORT_WAIT)).expires_at, None);
    }

    // ---------- validity: expired + refresh passes, expired without refresh fails ----------

    #[test]
    fn expired_with_refresh_token_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2020-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        let info = found(probe_database(&path, SHORT_WAIT));
        assert!(info.refreshable);
        assert!(info.is_valid(now()));
    }

    #[test]
    fn expired_without_refresh_token_is_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2020-01-01T00:00:00Z", None))]);
        let info = found(probe_database(&path, SHORT_WAIT));
        assert!(!info.refreshable);
        assert!(!info.is_valid(now()));
    }

    #[test]
    fn expired_with_empty_or_null_refresh_token_is_invalid() {
        for refresh in ["\"\"", "null", "\"   \"", "7"] {
            let json = format!(
                "{{\"access_token\":\"{FAKE_ACCESS}\",\"refresh_token\":{refresh},\"expires_at\":\"2020-01-01T00:00:00Z\"}}"
            );
            let tmp = tempfile::tempdir().unwrap();
            let path = db_with(tmp.path(), &[(SOCIAL, &json)]);
            let info = found(probe_database(&path, SHORT_WAIT));
            assert!(!info.refreshable, "refresh_token = {refresh}");
            assert!(!info.is_valid(now()));
        }
    }

    #[test]
    fn unexpired_without_refresh_token_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", None))]);
        assert!(found(probe_database(&path, SHORT_WAIT)).is_valid(now()));
    }

    #[test]
    fn unknown_expiry_without_refresh_token_is_invalid() {
        // The gateway treats an unknown expiry as expired, then has nothing to refresh with.
        let info = LoginInfo { method: LoginMethod::Social, expires_at: None, refreshable: false };
        assert!(!info.is_valid(now()));
        let info = LoginInfo { method: LoginMethod::Social, expires_at: None, refreshable: true };
        assert!(info.is_valid(now()));
    }

    #[test]
    fn expiry_exactly_now_counts_as_expired() {
        let info = LoginInfo { method: LoginMethod::Social, expires_at: Some(now()), refreshable: false };
        assert!(!info.is_valid(now()));
    }

    // ---------- concurrency and read-only behaviour ----------

    #[test]
    fn probe_reads_while_another_connection_holds_a_write_transaction() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE; INSERT INTO auth_kv VALUES ('pending', '{}');").unwrap();
        let start = Instant::now();
        let info = found(probe_database(&path, Duration::from_millis(2000)));
        assert!(info.refreshable);
        assert!(start.elapsed() < Duration::from_millis(1500), "must not wait for the writer");
        writer.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn probe_reads_while_a_wal_mode_writer_holds_a_transaction() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        let writer = Connection::open(&path).unwrap();
        let mode: String = writer.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        writer.execute_batch("BEGIN IMMEDIATE; INSERT INTO auth_kv VALUES ('pending', '{}');").unwrap();
        assert!(found(probe_database(&path, Duration::from_millis(2000))).refreshable);
        writer.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn probe_gives_up_with_unreadable_when_the_writer_holds_an_exclusive_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        let start = Instant::now();
        let got = probe_database(&path, SHORT_WAIT);
        assert!(matches!(got, LoginProbe::Unreadable), "got {got:?}");
        assert!(start.elapsed() < Duration::from_secs(3), "must not hang on a locked database");
        writer.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn busy_timeout_constant_is_two_seconds() {
        assert_eq!(BUSY_TIMEOUT, Duration::from_millis(2000));
    }

    #[test]
    fn probe_leaves_the_database_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        let listing = |dir: &Path| {
            let mut names: Vec<String> =
                fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
            names.sort();
            names
        };
        let before_bytes = fs::read(&path).unwrap();
        let before_files = listing(tmp.path());
        let _ = probe_database(&path, SHORT_WAIT);
        assert_eq!(fs::read(&path).unwrap(), before_bytes);
        assert_eq!(listing(tmp.path()), before_files, "no -journal / -wal / -shm files may appear");
    }

    #[test]
    fn probe_cannot_write_even_through_a_crafted_state() {
        // The connection is opened read-only: prove it by asking the same URI to write.
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        let conn = open_read_only(&path, SHORT_WAIT).unwrap();
        let err = conn.execute("DELETE FROM auth_kv", []).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("readonly"), "{err}");
    }

    #[test]
    fn probe_path_with_spaces_and_uri_special_characters() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Application Support").join("100% #1 what?");
        fs::create_dir_all(&dir).unwrap();
        let path = db_with(&dir, &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        assert!(found(probe_database(&path, SHORT_WAIT)).refreshable);
    }

    #[test]
    fn probe_path_with_non_ascii_characters() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("用户 数据");
        fs::create_dir_all(&dir).unwrap();
        let path = db_with(&dir, &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        assert!(found(probe_database(&path, SHORT_WAIT)).refreshable);
    }

    // ---------- URI building ----------

    #[test]
    fn uri_escapes_space_percent_question_and_hash() {
        let uri = sqlite_read_only_uri(Path::new("/Users/me/Library/Application Support/a%b?c#d.sqlite3"), false);
        assert_eq!(uri, "file:/Users/me/Library/Application%20Support/a%25b%3Fc%23d.sqlite3?mode=ro");
    }

    #[test]
    fn uri_escapes_non_ascii_as_utf8_bytes() {
        let uri = sqlite_read_only_uri(Path::new("/u/用"), false);
        assert_eq!(uri, "file:/u/%E7%94%A8?mode=ro");
    }

    #[test]
    fn uri_for_windows_drive_path_uses_three_slashes_and_forward_slashes() {
        let uri = sqlite_read_only_uri(Path::new(r"C:\Users\me\AppData\Local\kiro-cli\data.sqlite3"), true);
        assert_eq!(uri, "file:///C:/Users/me/AppData/Local/kiro-cli/data.sqlite3?mode=ro");
        let uri = sqlite_read_only_uri(Path::new(r"C:\Users\x y\data.sqlite3"), true);
        assert_eq!(uri, "file:///C:/Users/x%20y/data.sqlite3?mode=ro");
    }

    // ---------- database candidates ----------

    fn dirs(home: &str) -> UserDirs {
        UserDirs {
            home: Some(PathBuf::from(home)),
            local_app_data: Some(PathBuf::from(format!("{home}/AppData/Local"))),
            app_data: Some(PathBuf::from(format!("{home}/AppData/Roaming"))),
        }
    }

    #[test]
    fn macos_database_candidates_in_priority_order() {
        assert_eq!(
            database_candidates(Platform::MacOs, &dirs("/Users/me")),
            vec![
                PathBuf::from("/Users/me/Library/Application Support/kiro-cli/data.sqlite3"),
                PathBuf::from("/Users/me/.local/share/kiro-cli/data.sqlite3"),
                PathBuf::from("/Users/me/.local/share/amazon-q/data.sqlite3"),
            ]
        );
    }

    #[test]
    fn windows_database_candidates_are_appdata_then_localappdata() {
        assert_eq!(
            database_candidates(Platform::Windows, &dirs("/u/me")),
            vec![
                PathBuf::from("/u/me/AppData/Roaming").join("kiro-cli").join("data.sqlite3"),
                PathBuf::from("/u/me/AppData/Local").join("kiro-cli").join("data.sqlite3"),
            ]
        );
    }

    #[test]
    fn database_candidates_without_base_dirs_are_empty() {
        let none = UserDirs::default();
        assert!(database_candidates(Platform::MacOs, &none).is_empty());
        assert!(database_candidates(Platform::Windows, &none).is_empty());
    }

    #[test]
    fn find_database_returns_first_existing_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let second = home.join(".local/share/kiro-cli/data.sqlite3");
        let third = home.join(".local/share/amazon-q/data.sqlite3");
        for p in [&second, &third] {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"").unwrap();
        }
        let user = UserDirs { home: Some(home.to_path_buf()), ..UserDirs::default() };
        assert_eq!(find_database(Platform::MacOs, &user), Some(second));
    }

    #[test]
    fn find_database_none_when_nothing_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let user = UserDirs { home: Some(tmp.path().to_path_buf()), ..UserDirs::default() };
        assert_eq!(find_database(Platform::MacOs, &user), None);
    }

    #[test]
    fn find_database_skips_a_directory_with_the_database_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_named_like_db = tmp.path().join("Library/Application Support/kiro-cli/data.sqlite3");
        fs::create_dir_all(&dir_named_like_db).unwrap();
        let user = UserDirs { home: Some(tmp.path().to_path_buf()), ..UserDirs::default() };
        assert_eq!(find_database(Platform::MacOs, &user), None);
    }

    // ---------- no credential leaks ----------

    #[test]
    fn serialized_result_contains_no_token_material() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &token_json("2030-01-01T00:00:00Z", Some(FAKE_REFRESH)))]);
        let probe = probe_database(&path, SHORT_WAIT);
        let info = found(probe.clone());

        let info_json = serde_json::to_string(&info).unwrap();
        let probe_json = serde_json::to_string(&probe).unwrap();
        let debug = format!("{info:?} {probe:?}");
        for text in [&info_json, &probe_json, &debug] {
            for secret in [FAKE_ACCESS, FAKE_REFRESH, FAKE_SECRET, "FAKE-", "access_token", "refresh_token"] {
                assert!(!text.contains(secret), "leaked {secret} in {text}");
            }
        }
        let value: serde_json::Value = serde_json::from_str(&info_json).unwrap();
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, vec!["expiresAt", "method", "refreshable"]);
    }

    #[test]
    fn corrupt_and_unreadable_results_do_not_echo_the_raw_value() {
        let tmp = tempfile::tempdir().unwrap();
        let path = db_with(tmp.path(), &[(SOCIAL, &format!("not json {FAKE_ACCESS}"))]);
        let probe = probe_database(&path, SHORT_WAIT);
        assert_eq!(probe, LoginProbe::Corrupt);
        assert!(!format!("{probe:?}").contains("FAKE-"));
        assert!(!serde_json::to_string(&probe).unwrap().contains("FAKE-"));
    }
}
