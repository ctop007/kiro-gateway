//! Log line sanitising for the excerpt shown in the UI (tasks 4.5, design risk 7).
//!
//! Steps for each line: strip terminal escape codes, replace the access key verbatim,
//! then mask every token-like run.
//!
//! A *token-like run* is a maximal run of `A-Za-z0-9+/_-` (plus up to two trailing `=`)
//! that is at least 32 characters long and looks like encoded data: it contains a digit,
//! `+`, `/` or `=`. Long identifiers made only of letters, `_` and `-` (for example
//! `_load_enterprise_device_registration`) are kept, because they appear in every log
//! line. A run that starts with `/` is a file path: its `/`-separated parts are checked
//! one by one, so a long path is not masked as a whole while a 32+ character token
//! inside it still is. The trade-off is documented in the tests: a base64 token made of
//! letters only (about 1 in 1000 random tokens) is not recognised by shape alone; the
//! access key itself is always caught by the exact match.

/// Replacement text for masked content.
const MASK: &str = "***";
/// Shortest run treated as a token.
const MIN_TOKEN_LEN: usize = 32;
/// Longest line shown in an excerpt, in characters.
pub const MAX_EXCERPT_LINE_CHARS: usize = 400;

/// Remove ANSI escape sequences (colors, cursor movement, OSC titles) and `\r`.
///
/// Truncated sequences at the end of the input are dropped rather than echoed.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {}
            '\u{1b}' => match chars.next() {
                // CSI: parameters, then one final byte in 0x40..=0x7E.
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                // Two-character escape such as ESC c: the second character belongs to it.
                Some(_) | None => {}
            },
            other => out.push(other),
        }
    }
    out
}

fn is_run_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '_' | '-')
}

/// Whether a run (without the path rule applied) is token-like.
fn looks_like_token(run: &str) -> bool {
    run.len() >= MIN_TOKEN_LEN && run.chars().any(|c| c.is_ascii_digit() || matches!(c, '+' | '/' | '='))
}

/// Mask the parts of a path-like run (`/a/b/c`): each part is judged on its own.
fn mask_path_parts(run: &str) -> String {
    run.split('/')
        .map(|part| if looks_like_token(part) { MASK.to_string() } else { part.to_string() })
        .collect::<Vec<_>>()
        .join("/")
}

/// Mask every token-like run in `line`.
fn mask_tokens(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if !is_run_char(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_run_char(chars[i]) {
            i += 1;
        }
        let mut padding = 0;
        while padding < 2 && i < chars.len() && chars[i] == '=' {
            i += 1;
            padding += 1;
        }
        let run: String = chars[start..i].iter().collect();
        let is_path = run.starts_with('/') && !run.contains('+');
        if is_path {
            out.push_str(&mask_path_parts(&run));
        } else if looks_like_token(&run) {
            out.push_str(MASK);
        } else {
            out.push_str(&run);
        }
    }
    out
}

/// Sanitise one log line.
///
/// Args:
///     line: Raw line from `gateway.log`.
///     secret: The access key; empty means "no key to look for".
///
/// Returns:
///     The line without escape codes, with the key and token-like runs replaced by `***`.
pub fn redact_line(line: &str, secret: &str) -> String {
    let plain = strip_ansi(line);
    let without_key = if secret.is_empty() { plain } else { plain.replace(secret, MASK) };
    mask_tokens(&without_key)
}

fn shorten(line: String) -> String {
    if line.chars().count() <= MAX_EXCERPT_LINE_CHARS {
        return line;
    }
    let mut short: String = line.chars().take(MAX_EXCERPT_LINE_CHARS).collect();
    short.push('…');
    short
}

/// The last `max_lines` non-blank lines, sanitised and shortened.
///
/// Blank lines are dropped before counting, so they never push real output out of the
/// window.
pub fn redact_tail(lines: &[String], max_lines: usize, secret: &str) -> Vec<String> {
    let cleaned: Vec<String> = lines
        .iter()
        .map(|l| redact_line(l, secret))
        .filter(|l| !l.trim().is_empty())
        .collect();
    let skip = cleaned.len().saturating_sub(max_lines);
    cleaned.into_iter().skip(skip).map(shorten).collect()
}

#[cfg(test)]
mod tests;
