//! Rules for the kclaude command name (`commandName`, `docs/ipc-contract.md` section 4).
//!
//! The name becomes a shell function name, a script file name and a word inside a
//! startup file, so it is restricted to `[A-Za-z][A-Za-z0-9_-]{1,31}` and may not shadow
//! `claude` or a common system command. This module is the pure part (format and reserved
//! names); whether the name is already taken on this machine (PATH, startup files) is
//! decided by the kclaude service.

/// Command name used when the user has not chosen one.
pub const DEFAULT_COMMAND_NAME: &str = "cclaude";
/// Shortest accepted name.
pub const MIN_NAME_LEN: usize = 2;
/// Longest accepted name.
pub const MAX_NAME_LEN: usize = 32;

/// Names that must never be taken over: `claude` itself, the Kiro CLI and the commands
/// people type all day. Compared case-insensitively.
pub const RESERVED_NAMES: &[&str] = &[
    "claude", "kiro-cli", "ls", "cd", "cat", "cp", "mv", "rm", "git", "node", "npm", "python", "python3", "sh",
    "bash", "zsh", "sudo", "open", "kill", "test", "which", "type", "echo", // required minimum
    "pwd", "mkdir", "rmdir", "touch", "grep", "sed", "awk", "find", "tar", "curl", "ssh", "scp", "make", "man",
    "env", "export", "alias", "unalias", "source", "exit", "eval", "exec", "set", "unset", "if", "for", "while",
    "do", "then", "fi", "case", "esac", "function", "pip", "pip3", "brew", "code", "vim", "vi", "ps", "top",
    "true", "false", "kiro", "pwsh", "powershell", "cmd", "dir", "del", "copy", "move",
    // Shell builtins and keywords that a function of the same name would shadow.
    "history", "jobs", "fg", "bg", "read", "wait", "trap", "ulimit", "umask", "pushd", "popd", "dirs", "fc", "let",
    "local", "return", "break", "continue", "shift", "times", "typeset", "declare", "readonly", "printf",
    "command", "builtin", "hash", "help", "logout", "time", "select", "until", "in", "nohup",
    // Everyday tools.
    "chmod", "chown", "ln", "head", "tail", "less", "more", "sort", "uniq", "diff", "date", "sleep", "clear", "tee",
    "xargs", "docker", "go", "java", "ruby", "perl", "cargo", "rustc", "pnpm", "yarn", "npx", "uv",
];

/// Why a name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    /// Wrong characters, wrong first character or wrong length.
    Format,
    /// `claude` or a common system command.
    Reserved,
    /// Already a command, function or alias on this machine.
    Taken,
}

impl NameProblem {
    /// One-sentence Chinese reason shown next to the input.
    pub fn message(self) -> &'static str {
        match self {
            NameProblem::Format => "命令名只能用字母、数字、- 和 _，以字母开头，2 到 32 个字符",
            NameProblem::Reserved => "这个名字是系统命令，换一个",
            NameProblem::Taken => "这个名字已经被占用",
        }
    }
}

/// True when `name` matches `^[A-Za-z][A-Za-z0-9_-]{1,31}$`.
pub fn is_valid_format(name: &str) -> bool {
    let len = name.len();
    if !name.is_ascii() || !(MIN_NAME_LEN..=MAX_NAME_LEN).contains(&len) {
        return false;
    }
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// True when `name` is on the reserved list (case-insensitive).
pub fn is_reserved(name: &str) -> bool {
    RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(name))
}

/// Check the parts of a command name that need no look at the machine.
///
/// Args:
///     name: Candidate name, exactly as typed (surrounding spaces are an error, not trimmed).
///
/// Returns:
///     `Ok(())`, or `Format` / `Reserved`.
pub fn validate(name: &str) -> Result<(), NameProblem> {
    if !is_valid_format(name) {
        return Err(NameProblem::Format);
    }
    if is_reserved(name) {
        return Err(NameProblem::Reserved);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
