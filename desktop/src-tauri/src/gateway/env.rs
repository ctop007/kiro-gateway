//! Environment of the sidecar (design decision 2).
//!
//! The process is started with a cleared environment plus the result of [`build_env`]:
//! only an allow-list of system variables is inherited from the App's own environment, so
//! no gateway setting a user happens to have exported (`DEBUG_MODE`, `VPN_PROXY_URL`,
//! `REFRESH_TOKEN`, ...) can change which account, key, address or log level the gateway
//! uses. Proxy variables stay: users in restricted networks need them to reach Kiro.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// Inherited variables with these prefixes pass through (case-insensitive): locale settings.
/// Test builds also let the `KGW_FAKE_*` switches of the fake sidecars through.
#[cfg(not(test))]
const ALLOWED_PREFIXES: [&str; 1] = ["LC_"];
#[cfg(test)]
const ALLOWED_PREFIXES: [&str; 2] = ["LC_", "KGW_FAKE_"];

/// Inherited variables that pass through (case-insensitive, so `https_proxy` and `HTTPS_PROXY`
/// are both covered, and Windows' mixed-case names match). Everything else is dropped.
const ALLOWED_NAMES: [&str; 25] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "LANG",
    "SHELL",
    "SYSTEMROOT",
    "WINDIR",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "TEMP",
    "TMP",
    "COMSPEC",
    "PATHEXT",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "PROGRAMDATA",
    "SYSTEMDRIVE",
    "__CF_USER_TEXT_ENCODING",
];

/// Everything the environment depends on.
#[derive(Debug, Clone)]
pub struct EnvInputs {
    /// The access key; becomes `PROXY_API_KEY`.
    pub secret: String,
    /// kiro-cli database found by the environment check.
    pub kiro_cli_db: PathBuf,
    pub port: u16,
    /// `logs/requests` below the App data directory.
    pub request_log_dir: PathBuf,
    /// Bundled tiktoken BPE cache; `None` leaves tiktoken to its own default.
    pub tiktoken_cache_dir: Option<PathBuf>,
}

/// Variables the App sets itself, in the order they are appended.
fn explicit(inputs: &EnvInputs) -> Vec<(&'static str, OsString)> {
    let mut vars: Vec<(&'static str, OsString)> = vec![
        ("PROXY_API_KEY", inputs.secret.clone().into()),
        ("KIRO_CLI_DB_FILE", inputs.kiro_cli_db.clone().into_os_string()),
        ("SERVER_HOST", "127.0.0.1".into()),
        ("SERVER_PORT", inputs.port.to_string().into()),
        ("FAKE_REASONING", "false".into()),
        ("REQUEST_LOG_DIR", inputs.request_log_dir.clone().into_os_string()),
        ("LOG_LEVEL", "INFO".into()),
        ("DESKTOP_API_ENABLED", "true".into()),
        // The startup banner has emoji; a redirected stdout on Windows would otherwise use
        // the ANSI code page and raise UnicodeEncodeError (found by the sidecar smoke test).
        ("PYTHONUTF8", "1".into()),
        ("PYTHONIOENCODING", "utf-8".into()),
        // Lines reach gateway.log as soon as they are printed, so a crash excerpt is complete.
        ("PYTHONUNBUFFERED", "1".into()),
    ];
    if let Some(dir) = &inputs.tiktoken_cache_dir {
        vars.push(("TIKTOKEN_CACHE_DIR", dir.clone().into_os_string()));
    }
    vars
}

fn is_allowed(key: &OsStr) -> bool {
    let upper = key.to_string_lossy().to_ascii_uppercase();
    ALLOWED_PREFIXES.iter().any(|p| upper.starts_with(p)) || ALLOWED_NAMES.contains(&upper.as_str())
}

/// Assemble the complete environment of the sidecar.
///
/// Args:
///     inputs: The values the App chooses.
///     inherited: The App's own environment (`std::env::vars_os()` in production).
///
/// Returns:
///     The variables to give to a command whose environment was cleared: the inherited
///     ones on the allow-list (system basics, locale, certificates, proxies) minus anything
///     the App sets itself, followed by the App's variables. No key appears twice.
pub fn build_env<I>(inputs: &EnvInputs, inherited: I) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let ours = explicit(inputs);
    let ours_upper: Vec<String> = ours.iter().map(|(k, _)| k.to_ascii_uppercase()).collect();
    let overridden = |key: &OsStr| ours_upper.contains(&key.to_string_lossy().to_ascii_uppercase());

    let mut result: Vec<(OsString, OsString)> = Vec::new();
    for (key, value) in inherited {
        if !is_allowed(&key) || overridden(&key) {
            continue;
        }
        result.retain(|(existing, _)| existing != &key);
        result.push((key, value));
    }
    result.extend(ours.into_iter().map(|(k, v)| (OsString::from(k), v)));
    result
}

#[cfg(test)]
mod tests;
