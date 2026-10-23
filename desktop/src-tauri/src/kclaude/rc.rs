//! Marker-block management inside shell startup files (`~/.zshrc`, `~/.bashrc`,
//! `~/.bash_profile`) and PowerShell profiles.
//!
//! The pure part (`analyze`, `plan_*`) works on bytes and returns the new file content, so
//! every outcome can be asserted byte for byte. The IO part (`apply`) takes a backup first
//! and refuses to write when the backup fails.
//!
//! Rules enforced here:
//! - Content outside the marker block is never changed, except by `plan_adopt`, which
//!   comments out a pre-existing foreign kclaude definition.
//! - `plan_remove` is the exact inverse of appending: install then remove gives back the
//!   original bytes, including a missing trailing newline.
//! - An existing definition of the command (named by the `command` argument) without our
//!   markers is `Foreign` and is never overwritten.
//! - The legacy marker pair (`... kclaude >>>`, written by builds before the command name
//!   became configurable) is our own block too: it is replaced in place on install and
//!   removed on remove, and never counts as foreign.

use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::fsutil::{backup_path, write_atomic};
use super::script::{ps_string_expr, sh_quote};
use super::types::{ErrorCode, KclaudeError, ShellKind};

pub const BEGIN_MARKER: &str = "# >>> kiro-gateway-desktop command >>>";
pub const END_MARKER: &str = "# <<< kiro-gateway-desktop command <<<";
/// Marker pair written by the first builds, when the command was always `kclaude`.
pub const LEGACY_BEGIN_MARKER: &str = "# >>> kiro-gateway-desktop kclaude >>>";
pub const LEGACY_END_MARKER: &str = "# <<< kiro-gateway-desktop kclaude <<<";
pub const DISABLED_PREFIX: &str = "# disabled by Kiro Gateway App: ";

/// Syntax family of the startup file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Posix,
    PowerShell,
}

/// Classification of one startup file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RcState {
    NotInstalled,
    Installed,
    Outdated,
    /// The command is defined in the file without our markers.
    Foreign,
    /// Unbalanced or nested markers: the extent of our block is unknown, so nothing is touched.
    Damaged,
}

/// Why an edit was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Foreign,
    Damaged,
    /// A foreign function definition whose end could not be determined.
    UnparsableForeign,
}

#[derive(Debug, Clone, Copy)]
struct Line {
    start: usize,
    /// End of the content, before the line terminator.
    end: usize,
    /// Start of the next line (after the terminator).
    next: usize,
}

impl Line {
    fn has_terminator(&self) -> bool {
        self.next > self.end
    }
}

fn split_lines(bytes: &[u8]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        match bytes[start..].iter().position(|&b| b == b'\n') {
            Some(offset) => {
                let nl = start + offset;
                lines.push(Line { start, end: nl, next: nl + 1 });
                start = nl + 1;
            }
            None => {
                lines.push(Line { start, end: bytes.len(), next: bytes.len() });
                start = bytes.len();
            }
        }
    }
    lines
}

fn line_text<'a>(bytes: &'a [u8], line: &Line) -> Cow<'a, str> {
    let mut end = line.end;
    if end > line.start && bytes[end - 1] == b'\r' {
        end -= 1;
    }
    String::from_utf8_lossy(&bytes[line.start..end])
}

fn detect_eol(bytes: &[u8]) -> &'static str {
    match bytes.iter().position(|&b| b == b'\n') {
        Some(i) if i > 0 && bytes[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

/// Text of the marker block (three lines, `\n` terminated) that references `script_path`.
///
/// Args:
///     dialect: Posix or PowerShell.
///     script_path: Absolute path of `<command>.zsh` / `.bash` / `.ps1`.
pub fn block_text(dialect: Dialect, script_path: &Path) -> String {
    let path = script_path.to_string_lossy();
    let body = match dialect {
        Dialect::Posix => {
            let q = sh_quote(&path);
            format!("[ -r {q} ] && source {q}")
        }
        Dialect::PowerShell => {
            let e = ps_string_expr(&path);
            format!("if (Test-Path -LiteralPath {e}) {{ . {e} }}")
        }
    };
    format!("{BEGIN_MARKER}\n{body}\n{END_MARKER}\n")
}

/// Result of scanning a startup file.
#[derive(Debug)]
pub struct Analysis<'a> {
    bytes: &'a [u8],
    lines: Vec<Line>,
    /// Inclusive line ranges of marker blocks.
    blocks: Vec<(usize, usize)>,
    damaged: bool,
    /// Inclusive line ranges of foreign definitions; `Err` when one cannot be delimited.
    foreign: Result<Vec<(usize, usize)>, ()>,
    dialect: Dialect,
}

/// Scan `bytes` for our marker blocks (current and legacy) and for foreign definitions of
/// `command`.
///
/// Args:
///     bytes: Startup file content.
///     dialect: Syntax family.
///     command: The command name being managed (`cclaude` by default).
pub fn analyze<'a>(bytes: &'a [u8], dialect: Dialect, command: &str) -> Analysis<'a> {
    let lines = split_lines(bytes);
    let mut blocks = Vec::new();
    // Line of the open marker and whether it is the legacy kind.
    let mut open: Option<(usize, bool)> = None;
    let mut damaged = false;
    for (i, line) in lines.iter().enumerate() {
        let text = line_text(bytes, line);
        let trimmed = text.trim();
        if trimmed == BEGIN_MARKER || trimmed == LEGACY_BEGIN_MARKER {
            if open.is_some() {
                damaged = true;
            }
            open = Some((i, trimmed == LEGACY_BEGIN_MARKER));
        } else if trimmed == END_MARKER || trimmed == LEGACY_END_MARKER {
            match open.take() {
                // A begin and an end of different generations do not belong together.
                Some((b, legacy)) if legacy == (trimmed == LEGACY_END_MARKER) => blocks.push((b, i)),
                Some(_) | None => damaged = true,
            }
        }
    }
    if open.is_some() {
        damaged = true;
    }
    let foreign = find_foreign(bytes, &lines, &blocks, dialect, command);
    Analysis { bytes, lines, blocks, damaged, foreign, dialect }
}

impl Analysis<'_> {
    /// Classify the file against the block that would be written now.
    pub fn state(&self, expected_block: &str) -> RcState {
        if self.damaged {
            return RcState::Damaged;
        }
        if !matches!(&self.foreign, Ok(v) if v.is_empty()) {
            return RcState::Foreign;
        }
        match self.blocks.as_slice() {
            [] => RcState::NotInstalled,
            [(b0, b1)] => {
                if self.block_lines_match(*b0, *b1, expected_block) {
                    RcState::Installed
                } else {
                    RcState::Outdated
                }
            }
            _ => RcState::Outdated,
        }
    }

    fn block_lines_match(&self, b0: usize, b1: usize, expected: &str) -> bool {
        let actual: Vec<String> = (b0..=b1)
            .map(|i| line_text(self.bytes, &self.lines[i]).trim().to_string())
            .collect();
        let want: Vec<&str> = expected.lines().map(str::trim).collect();
        actual.iter().map(String::as_str).eq(want.iter().copied())
    }

    /// True when a block written under the legacy marker pair is present.
    pub fn has_legacy_block(&self) -> bool {
        self.blocks.iter().any(|&(b0, _)| line_text(self.bytes, &self.lines[b0]).trim() == LEGACY_BEGIN_MARKER)
    }

    /// True when the file defines the command outside our blocks: a function, an alias or
    /// a `source` of a file named after it. A definition whose extent cannot be determined
    /// counts as a definition.
    pub fn defines_command(&self) -> bool {
        !matches!(&self.foreign, Ok(v) if v.is_empty())
    }

    fn refusal(&self) -> Option<Refusal> {
        if self.damaged {
            Some(Refusal::Damaged)
        } else if self.foreign.is_err() {
            Some(Refusal::UnparsableForeign)
        } else if matches!(&self.foreign, Ok(v) if !v.is_empty()) {
            Some(Refusal::Foreign)
        } else {
            None
        }
    }

    /// 1-based line numbers where foreign definitions start (for diagnostics and tests).
    #[cfg(test)]
    pub fn foreign_line_numbers(&self) -> Vec<usize> {
        self.foreign
            .as_ref()
            .map(|v| v.iter().map(|(a, _)| a + 1).collect())
            .unwrap_or_default()
    }
}

/// Compute the file content after an install.
///
/// Args:
///     bytes: Current content (empty for a missing file).
///     dialect: Syntax family.
///     command: The command name being managed.
///     expected_block: Output of [`block_text`].
///
/// Returns:
///     `Ok(None)` when the file is already current (no write), `Ok(Some(new))` otherwise.
///     `Err` when a foreign kclaude or damaged markers make writing unsafe.
pub fn plan_install(
    bytes: &[u8],
    dialect: Dialect,
    command: &str,
    expected_block: &str,
) -> Result<Option<Vec<u8>>, Refusal> {
    let analysis = analyze(bytes, dialect, command);
    if let Some(r) = analysis.refusal() {
        return Err(r);
    }
    match analysis.state(expected_block) {
        RcState::Installed => Ok(None),
        RcState::NotInstalled => Ok(Some(append_block(bytes, expected_block))),
        RcState::Outdated => Ok(Some(replace_blocks(&analysis, expected_block))),
        RcState::Foreign | RcState::Damaged => unreachable!("refusal() covers these"),
    }
}

/// Compute the file content after "let the App manage it": foreign definitions are
/// commented out with [`DISABLED_PREFIX`], then the block is installed.
///
/// Returns:
///     `Ok(None)` when there is nothing to change. `Err(Damaged)` / `Err(UnparsableForeign)`
///     when the file cannot be edited safely; `Err(Foreign)` is never returned.
pub fn plan_adopt(
    bytes: &[u8],
    dialect: Dialect,
    command: &str,
    expected_block: &str,
) -> Result<Option<Vec<u8>>, Refusal> {
    let analysis = analyze(bytes, dialect, command);
    match analysis.refusal() {
        Some(Refusal::Damaged) => return Err(Refusal::Damaged),
        Some(Refusal::UnparsableForeign) => return Err(Refusal::UnparsableForeign),
        _ => {}
    }
    let regions = analysis.foreign.clone().unwrap_or_default();
    let commented = if regions.is_empty() {
        None
    } else {
        Some(comment_regions(&analysis, &regions))
    };
    let base: &[u8] = commented.as_deref().unwrap_or(bytes);
    match plan_install(base, dialect, command, expected_block)? {
        Some(next) => Ok(Some(next)),
        None => Ok(commented),
    }
}

/// Compute the file content after removing our block(s). Everything else stays byte for
/// byte. Returns `Ok(None)` when there is no block, `Err(Damaged)` for unbalanced markers.
pub fn plan_remove(bytes: &[u8], dialect: Dialect, command: &str) -> Result<Option<Vec<u8>>, Refusal> {
    let analysis = analyze(bytes, dialect, command);
    if analysis.damaged {
        return Err(Refusal::Damaged);
    }
    if analysis.blocks.is_empty() {
        return Ok(None);
    }
    Ok(Some(delete_blocks(&analysis)))
}

fn append_block(bytes: &[u8], block: &str) -> Vec<u8> {
    let eol = detect_eol(bytes);
    let mut lines: Vec<&str> = block.lines().collect();
    let mut out = bytes.to_vec();
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        // Keep removal an exact inverse: the file did not end with a newline, so the
        // appended block does not either.
        out.extend_from_slice(eol.as_bytes());
        let last = lines.pop().unwrap_or("");
        for l in lines {
            out.extend_from_slice(l.as_bytes());
            out.extend_from_slice(eol.as_bytes());
        }
        out.extend_from_slice(last.as_bytes());
    } else {
        for l in lines {
            out.extend_from_slice(l.as_bytes());
            out.extend_from_slice(eol.as_bytes());
        }
    }
    out
}

/// Byte range covered by an inclusive line range, widened to swallow the preceding
/// terminator when the range ends the file without one.
fn removal_range(a: &Analysis<'_>, (b0, b1): (usize, usize)) -> (usize, usize) {
    let first = a.lines[b0];
    let last = a.lines[b1];
    if !last.has_terminator() && b0 > 0 {
        // The block was appended to a file that had no final newline, using the file's
        // own line ending: swallow that ending too ("\r\n" when the marker line has it).
        let mut from = a.lines[b0 - 1].end;
        let crlf = first.end > first.start && a.bytes[first.end - 1] == b'\r';
        if crlf && from > a.lines[b0 - 1].start && a.bytes[from - 1] == b'\r' {
            from -= 1;
        }
        (from, last.end)
    } else {
        (first.start, last.next)
    }
}

fn delete_blocks(a: &Analysis<'_>) -> Vec<u8> {
    let mut out = Vec::with_capacity(a.bytes.len());
    let mut cursor = 0;
    for &blk in &a.blocks {
        let (from, to) = removal_range(a, blk);
        out.extend_from_slice(&a.bytes[cursor..from]);
        cursor = to;
    }
    out.extend_from_slice(&a.bytes[cursor..]);
    out
}

fn replace_blocks(a: &Analysis<'_>, block: &str) -> Vec<u8> {
    let eol = detect_eol(a.bytes);
    let (b0, b1) = a.blocks[0];
    let first = a.lines[b0];
    let last = a.lines[b1];
    let mut out = Vec::with_capacity(a.bytes.len() + block.len());
    let mut cursor = 0;
    for (n, &(c0, c1)) in a.blocks.iter().enumerate() {
        if n == 0 {
            out.extend_from_slice(&a.bytes[cursor..first.start]);
            let new_lines: Vec<&str> = block.lines().collect();
            for (i, l) in new_lines.iter().enumerate() {
                out.extend_from_slice(l.as_bytes());
                if i + 1 < new_lines.len() || last.has_terminator() {
                    out.extend_from_slice(eol.as_bytes());
                }
            }
            cursor = last.next;
        } else {
            let (from, to) = removal_range(a, (c0, c1));
            out.extend_from_slice(&a.bytes[cursor..from]);
            cursor = to;
        }
    }
    out.extend_from_slice(&a.bytes[cursor..]);
    out
}

fn comment_regions(a: &Analysis<'_>, regions: &[(usize, usize)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(a.bytes.len() + regions.len() * 40);
    let mut cursor = 0;
    for &(r0, r1) in regions {
        out.extend_from_slice(&a.bytes[cursor..a.lines[r0].start]);
        for i in r0..=r1 {
            let line = a.lines[i];
            let raw = &a.bytes[line.start..line.next];
            let indent = raw.iter().take_while(|&&b| b == b' ' || b == b'\t').count();
            let blank = raw[indent..].iter().all(|&b| b == b'\r' || b == b'\n');
            if blank {
                out.extend_from_slice(raw);
                continue;
            }
            out.extend_from_slice(&raw[..indent]);
            if i == r0 && indent > 0 && a.dialect == Dialect::Posix {
                // Indented: probably inside if/then/fi. A bare comment would leave an empty
                // compound body, which is a syntax error; the null command keeps it valid.
                out.extend_from_slice(b": ");
            }
            out.extend_from_slice(DISABLED_PREFIX.as_bytes());
            out.extend_from_slice(&raw[indent..]);
        }
        cursor = a.lines[r1].next;
    }
    out.extend_from_slice(&a.bytes[cursor..]);
    out
}

// ---------------------------------------------------------------------------------------
// Foreign kclaude detection
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForeignKind {
    Source,
    Alias,
    Function,
}

fn find_foreign(
    bytes: &[u8],
    lines: &[Line],
    blocks: &[(usize, usize)],
    dialect: Dialect,
    command: &str,
) -> Result<Vec<(usize, usize)>, ()> {
    let in_block = |i: usize| blocks.iter().any(|&(a, b)| i >= a && i <= b);
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if in_block(i) {
            i += 1;
            continue;
        }
        let text = line_text(bytes, &lines[i]);
        let Some(kind) = classify_line(&text, dialect, command) else {
            i += 1;
            continue;
        };
        let end = match kind {
            ForeignKind::Function => function_end(bytes, lines, i, dialect)?,
            ForeignKind::Source | ForeignKind::Alias => continuation_end(bytes, lines, i, dialect),
        };
        regions.push((i, end));
        i = end + 1;
    }
    Ok(regions)
}

fn continuation_end(bytes: &[u8], lines: &[Line], from: usize, dialect: Dialect) -> usize {
    let cont = match dialect {
        Dialect::Posix => '\\',
        Dialect::PowerShell => '`',
    };
    let mut i = from;
    while i + 1 < lines.len() && line_text(bytes, &lines[i]).trim_end().ends_with(cont) {
        i += 1;
    }
    i
}

/// Find the last line of a function definition starting at `from` by balancing braces
/// (quotes and comments are skipped). `Err` when the braces never balance.
fn function_end(bytes: &[u8], lines: &[Line], from: usize, dialect: Dialect) -> Result<usize, ()> {
    let mut depth: i32 = 0;
    let mut seen_open = false;
    for (i, line) in lines.iter().enumerate().skip(from) {
        if !seen_open && i > from + 3 {
            return Err(());
        }
        let text = line_text(bytes, line);
        let mut quote: Option<char> = None;
        let mut prev: Option<char> = None;
        for ch in text.chars() {
            match quote {
                Some(q) => {
                    if ch == q && !(dialect == Dialect::Posix && q == '"' && prev == Some('\\')) {
                        quote = None;
                    }
                }
                None => match ch {
                    '\'' | '"' => quote = Some(ch),
                    '#' if prev.map_or(true, char::is_whitespace) => break,
                    '{' => {
                        depth += 1;
                        seen_open = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                },
            }
            prev = Some(ch);
        }
        if seen_open && depth <= 0 {
            return Ok(i);
        }
    }
    Err(())
}

fn classify_line(text: &str, dialect: Dialect, command: &str) -> Option<ForeignKind> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    // Look at each simple command on the line: `a; b && c || d`, `if x; then source y; fi`.
    for raw in trimmed.split([';', '|', '&']) {
        let mut seg = raw.trim();
        loop {
            let stripped = ["then ", "else ", "do ", "{ "]
                .iter()
                .find_map(|kw| seg.strip_prefix(kw))
                .map(str::trim_start);
            match stripped {
                Some(s) => seg = s,
                None => break,
            }
        }
        if let Some(kind) = classify_segment(seg, dialect, command) {
            return Some(kind);
        }
    }
    None
}

fn is_name_end(rest: &str) -> bool {
    rest.chars().next().map_or(true, |c| c.is_whitespace() || matches!(c, '(' | '{'))
}

/// True for characters that can be part of a command name.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

/// True when `name` appears in `haystack` as a whole word: not preceded or followed by
/// another name character. `kclaude` is in `~/x/kclaude.zsh` but not in `~/x/kclaude-old.zsh`
/// or `~/x/xkclaude.zsh`.
fn mentions_word(haystack: &str, name: &str) -> bool {
    haystack.match_indices(name).any(|(at, _)| {
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + name.len()..].chars().next();
        !before.is_some_and(is_name_char) && !after.is_some_and(is_name_char)
    })
}

fn classify_segment(seg: &str, dialect: Dialect, command: &str) -> Option<ForeignKind> {
    let lower = seg.to_ascii_lowercase();
    let lower_name = command.to_ascii_lowercase();
    match dialect {
        Dialect::Posix => {
            for kw in ["source", "."] {
                if let Some(rest) = seg.strip_prefix(kw) {
                    if rest.starts_with([' ', '\t']) && mentions_word(rest, command) {
                        return Some(ForeignKind::Source);
                    }
                }
            }
            if let Some(rest) = seg.strip_prefix("alias") {
                if rest.starts_with([' ', '\t']) {
                    let name = skip_alias_flags(rest.trim_start());
                    if name.strip_prefix(command).is_some_and(|after| after.starts_with('=')) {
                        return Some(ForeignKind::Alias);
                    }
                }
            }
            if let Some(rest) = seg.strip_prefix("function") {
                if rest.starts_with([' ', '\t']) {
                    if let Some(after) = rest.trim_start().strip_prefix(command) {
                        if is_name_end(after) {
                            return Some(ForeignKind::Function);
                        }
                    }
                }
            }
            if let Some(after) = seg.strip_prefix(command) {
                if after.trim_start().starts_with('(') {
                    return Some(ForeignKind::Function);
                }
            }
            None
        }
        Dialect::PowerShell => {
            if let Some(rest) = lower.strip_prefix('.') {
                if rest.starts_with([' ', '\t']) && mentions_word(rest, &lower_name) {
                    return Some(ForeignKind::Source);
                }
            }
            if let Some(rest) = lower.strip_prefix("function") {
                if rest.starts_with([' ', '\t']) {
                    if let Some(after) = rest.trim_start().strip_prefix(lower_name.as_str()) {
                        if is_name_end(after) {
                            return Some(ForeignKind::Function);
                        }
                    }
                }
            }
            for kw in ["set-alias", "new-alias", "sal", "nal"] {
                if let Some(rest) = lower.strip_prefix(kw) {
                    if rest.starts_with([' ', '\t']) {
                        let mut words = rest.split_whitespace().filter(|w| *w != "-name");
                        if words.next() == Some(lower_name.as_str()) {
                            return Some(ForeignKind::Alias);
                        }
                    }
                }
            }
            None
        }
    }
}

fn skip_alias_flags(mut s: &str) -> &str {
    while s.starts_with('-') {
        s = s.trim_start_matches(|c: char| !c.is_whitespace()).trim_start();
    }
    s
}

// ---------------------------------------------------------------------------------------
// Which file to use
// ---------------------------------------------------------------------------------------

/// Map `$SHELL` to a shell family. Unknown or empty values fall back to zsh, the macOS
/// default.
pub fn shell_from_env(shell_var: Option<&str>) -> ShellKind {
    let name = shell_var
        .and_then(|s| s.rsplit(['/', '\\']).next())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.starts_with("bash") {
        ShellKind::Bash
    } else {
        ShellKind::Zsh
    }
}

/// Startup file to manage for `shell`.
///
/// zsh: `~/.zshrc`. bash: `~/.bash_profile` when it exists and does not source `.bashrc`
/// (login shells never read `.bashrc` then); `~/.bashrc` otherwise. On macOS, where
/// Terminal starts login shells, a missing `.bash_profile` with no `.bash_login` /
/// `.profile` also goes to `~/.bash_profile`, because `.bashrc` would never be read.
///
/// Args:
///     shell: Zsh or Bash (PowerShell is handled by `ps_profile`).
///     home: Home directory.
///     is_macos: Whether the platform starts login shells for new terminals.
pub fn choose_rc_file(shell: ShellKind, home: &Path, is_macos: bool) -> PathBuf {
    match shell {
        ShellKind::Zsh | ShellKind::Powershell => home.join(".zshrc"),
        ShellKind::Bash => {
            let bash_profile = home.join(".bash_profile");
            match fs::read(&bash_profile) {
                Ok(content) => {
                    if String::from_utf8_lossy(&content).contains(".bashrc") {
                        home.join(".bashrc")
                    } else {
                        bash_profile
                    }
                }
                Err(_) => {
                    let login_alternatives =
                        home.join(".bash_login").exists() || home.join(".profile").exists();
                    if is_macos && !login_alternatives {
                        bash_profile
                    } else {
                        home.join(".bashrc")
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// IO
// ---------------------------------------------------------------------------------------

/// Read a startup file; a missing file is empty content.
pub fn read_rc(path: &Path) -> io::Result<Vec<u8>> {
    match fs::read(path) {
        Ok(b) => Ok(b),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Back up `path` (when it exists) and replace its content with `new_bytes`.
///
/// The backup is `<name>.bak-kiro-gateway-<stamp>` next to the file. If it cannot be
/// created, nothing is written.
///
/// Args:
///     path: Startup file.
///     new_bytes: Complete new content.
///     stamp: `YYYYMMDD-HHMMSS`.
///
/// Returns:
///     `BackupFailed` or `WriteFailed` errors with user-facing messages.
pub fn apply(path: &Path, new_bytes: &[u8], stamp: &str) -> Result<(), KclaudeError> {
    if path.exists() {
        let backup = backup_path(path, stamp);
        if fs::copy(path, &backup).is_err() {
            let _ = fs::remove_file(&backup);
            return Err(KclaudeError::new(
                ErrorCode::BackupFailed,
                "备份启动文件失败，没有改动它。检查文件权限和磁盘空间后重试",
            ));
        }
    }
    write_atomic(path, new_bytes).map_err(|_| {
        KclaudeError::new(
            ErrorCode::WriteFailed,
            "写入启动文件失败。检查文件权限和磁盘空间后重试",
        )
    })
}

#[cfg(test)]
mod tests;
