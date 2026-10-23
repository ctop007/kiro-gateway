use super::*;

#[test]
fn default_name_is_valid_and_not_reserved() {
    assert_eq!(DEFAULT_COMMAND_NAME, "cclaude");
    assert_eq!(validate(DEFAULT_COMMAND_NAME), Ok(()));
}

#[test]
fn accepts_names_in_the_documented_shape() {
    for ok in ["cclaude", "kclaude", "kclaude-test", "my_claude", "ab", "A1", "a-b_c9", &"x".repeat(32), "Cclaude"] {
        assert_eq!(validate(ok), Ok(()), "{ok}");
    }
}

#[test]
fn rejects_bad_format() {
    let too_long = "x".repeat(33);
    for bad in [
        "", "a", "9abc", "-abc", "_abc", "a b", " abc", "abc ", "a\tb", "a\nb", "a.b", "a/b", "a;b", "a$b", "a`b",
        "a'b", "a\"b", "a&b", "a|b", "a\\b", "a#b", "a=b", "a(b", "名字", "cclaudé", &too_long,
    ] {
        assert_eq!(validate(bad), Err(NameProblem::Format), "{bad:?}");
    }
}

#[test]
fn rejects_claude_and_the_required_system_commands_in_any_case() {
    for name in [
        "claude", "kiro-cli", "ls", "cd", "cat", "cp", "mv", "rm", "git", "node", "npm", "python", "python3", "sh",
        "bash", "zsh", "sudo", "open", "kill", "test", "which", "type", "echo",
    ] {
        assert_eq!(validate(name), Err(NameProblem::Reserved), "{name}");
        assert_eq!(validate(&name.to_ascii_uppercase()), Err(NameProblem::Reserved), "{name} upper");
    }
}

#[test]
fn single_letter_reserved_style_names_fail_on_format_first() {
    assert_eq!(validate("c"), Err(NameProblem::Format));
}

#[test]
fn messages_are_the_documented_chinese_texts() {
    assert_eq!(NameProblem::Format.message(), "命令名只能用字母、数字、- 和 _，以字母开头，2 到 32 个字符");
    assert_eq!(NameProblem::Reserved.message(), "这个名字是系统命令，换一个");
    assert_eq!(NameProblem::Taken.message(), "这个名字已经被占用");
}

#[test]
fn every_reserved_name_is_itself_well_formed_or_irrelevant() {
    // A reserved name that is not even valid would be dead weight; keep the list honest.
    for name in RESERVED_NAMES {
        assert!(is_valid_format(name), "{name} can never be chosen anyway");
    }
}
