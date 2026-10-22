use super::*;

const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn r(line: &str) -> String {
    redact_line(line, SECRET)
}

#[test]
fn strips_color_codes_from_real_loguru_output() {
    // Copied from a real gateway run: loguru is configured with colorize=True.
    let raw = "\x1b[32m2026-10-22 22:02:36\x1b[0m | \x1b[1mINFO    \x1b[0m | \x1b[36mmain\x1b[0m:\x1b[36mlifespan\x1b[0m:\x1b[36m510\x1b[0m - \x1b[1mAccount system initialized successfully\x1b[0m";
    assert_eq!(
        strip_ansi(raw),
        "2026-10-22 22:02:36 | INFO     | main:lifespan:510 - Account system initialized successfully"
    );
}

#[test]
fn strip_ansi_handles_odd_sequences_without_eating_text() {
    assert_eq!(strip_ansi("plain"), "plain");
    assert_eq!(strip_ansi(""), "");
    assert_eq!(strip_ansi("\x1b[31m\x1b[1mERROR\x1b[0m"), "ERROR");
    assert_eq!(strip_ansi("a\x1b[2Kb"), "ab");
    assert_eq!(strip_ansi("a\x1b[38;5;196mb\x1b[0m"), "ab");
    // A lone ESC at the end or a truncated sequence must not panic or loop.
    assert_eq!(strip_ansi("tail\x1b"), "tail");
    assert_eq!(strip_ansi("tail\x1b[31"), "tail");
    // OSC title sequence terminated by BEL.
    assert_eq!(strip_ansi("\x1b]0;title\x07after"), "after");
    // Multi-byte text next to escapes stays intact.
    assert_eq!(strip_ansi("\x1b[1m网关\x1b[0m 已启动"), "网关 已启动");
}

#[test]
fn carriage_returns_are_dropped() {
    assert_eq!(strip_ansi("line\r"), "line");
}

#[test]
fn ordinary_log_lines_pass_through_unchanged() {
    for line in [
        "2026-10-22 22:02:36 | INFO     | kiro.cache:update:76 - Updating model cache. Found 21 models.",
        "2026-10-22 22:02:36 | ERROR    | logging:callHandlers:1737 - [Errno 48] error while attempting to bind on address ('127.0.0.1', 8150): address already in use",
        "2026-10-22 22:02:36 | INFO     | kiro.auth:_load_enterprise_device_registration:493 - Enterprise device registration loaded",
        "",
        "Traceback (most recent call last):",
    ] {
        assert_eq!(r(line), line);
    }
}

#[test]
fn long_hex_and_base64_runs_are_masked() {
    let hex = "9b5ad71b2ce5302211f9c61530b329a4922fc6a4";
    assert_eq!(r(&format!("sha1 {hex} done")), "sha1 *** done");
    let b64 = "dGhpcyBpcyBhIHZlcnkgbG9uZyBzZWNyZXQgdG9rZW4=";
    assert_eq!(r(&format!("token={b64}")), "token=***");
    let jwt_part = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9";
    assert_eq!(r(&format!("Authorization: Bearer {jwt_part}.abc")), "Authorization: Bearer ***.abc");
}

#[test]
fn run_of_exactly_32_is_masked_and_31_is_not() {
    let t32 = "a1".repeat(16);
    let t31 = format!("{}b", "a1".repeat(15));
    assert_eq!(t32.len(), 32);
    assert_eq!(t31.len(), 31);
    assert_eq!(r(&t32), "***");
    assert_eq!(r(&t31), t31);
}

#[test]
fn base64url_alphabet_with_dash_and_underscore_counts_as_one_run() {
    let token = "AbCd_Ef-Gh1JkLm_Nop-Qr2StUv_Wx-Yz3AbCd";
    assert!(token.len() >= 32);
    assert_eq!(r(&format!("t={token};")), "t=***;");
}

#[test]
fn standard_base64_with_slash_and_plus_is_masked_whole() {
    // AWS-style secret access key: 40 chars with + and /.
    let key = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
    assert_eq!(r(&format!("secret {key} end")), "secret *** end");
}

#[test]
fn slash_separated_base64_without_plus_is_masked_when_it_does_not_start_with_slash() {
    let token = "Ab3dEf6hIj9lMn2pQr5tUv8xYz1Bc4De/Fg7Hi0Jk";
    assert_eq!(r(token), "***");
}

#[test]
fn file_system_paths_are_not_treated_as_tokens() {
    for path in [
        "/var/folders/r9/j4988b2n0219vhfbzp5k12ch0000gn/T/kgw-smoke",
        "/Users/me/Library/Application Support/dev.kirogateway.desktop/logs/gateway.log",
        "/Users/me/.aws/sso/cache/kiro-auth-token.json",
    ] {
        assert_eq!(r(&format!("opened {path} ok")), format!("opened {path} ok"), "{path}");
    }
}

#[test]
fn long_identifiers_without_digits_are_kept() {
    // "_load_enterprise_device_registration" is 36 characters and appears in real log lines.
    let line = "kiro.auth:_load_enterprise_device_registration:493 - loaded";
    assert_eq!(r(line), line);
    let camel = "ThisIsAVeryLongPythonClassNameThatKeepsGoing";
    assert_eq!(r(camel), camel);
}

#[test]
fn the_access_key_is_masked_even_when_embedded_or_split_by_color_codes() {
    assert_eq!(r(&format!("x-api-key: {SECRET}")), "x-api-key: ***");
    assert_eq!(r(&format!("a{SECRET}b")), "a***b");
    assert_eq!(r(&format!("{SECRET} and {SECRET}")), "*** and ***");
    let (head, tail) = SECRET.split_at(20);
    assert_eq!(r(&format!("key=\x1b[31m{head}\x1b[0m{tail}")), "key=***");
}

#[test]
fn a_short_hand_set_key_is_masked_by_exact_match() {
    let key = "my-hand-set-key-123";
    assert_eq!(redact_line(&format!("Authorization: Bearer {key}"), key), "Authorization: Bearer ***");
}

#[test]
fn an_empty_secret_does_not_blank_the_line() {
    assert_eq!(redact_line("hello world", ""), "hello world");
}

#[test]
fn several_tokens_in_one_line_are_all_masked() {
    let a = "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6";
    let b = "ffeeddccbbaa99887766554433221100aa";
    assert_eq!(r(&format!("{a} {b}")), "*** ***");
}

#[test]
fn unicode_text_is_untouched() {
    assert_eq!(r("网关已启动，端口 8000"), "网关已启动，端口 8000");
}

#[test]
fn tail_keeps_the_last_n_lines_redacted_and_in_order() {
    let lines: Vec<String> = (1..=25).map(|n| format!("line {n}")).collect();
    let tail = redact_tail(&lines, 20, SECRET);
    assert_eq!(tail.len(), 20);
    assert_eq!(tail.first().unwrap(), "line 6");
    assert_eq!(tail.last().unwrap(), "line 25");
}

#[test]
fn tail_with_fewer_lines_returns_them_all() {
    let lines = vec!["a".to_string(), "b".to_string()];
    assert_eq!(redact_tail(&lines, 20, SECRET), vec!["a", "b"]);
    assert!(redact_tail(&[], 20, SECRET).is_empty());
}

#[test]
fn tail_masks_secrets_in_every_line_and_drops_blank_ones() {
    let lines = vec![
        format!("\x1b[31mERROR\x1b[0m using key {SECRET}"),
        "   ".to_string(),
        String::new(),
        "token 9b5ad71b2ce5302211f9c61530b329a4922fc6a4".to_string(),
    ];
    let tail = redact_tail(&lines, 20, SECRET);
    assert_eq!(tail, vec!["ERROR using key ***", "token ***"]);
}

#[test]
fn blank_lines_do_not_use_up_the_window() {
    let mut lines: Vec<String> = (1..=20).map(|n| format!("keep {n}")).collect();
    lines.splice(5..5, std::iter::repeat(String::new()).take(10));
    let tail = redact_tail(&lines, 20, SECRET);
    assert_eq!(tail.len(), 20);
    assert_eq!(tail[0], "keep 1");
}

#[test]
fn a_very_long_line_is_shortened() {
    let long = "x ".repeat(2000);
    let tail = redact_tail(&[long], 20, SECRET);
    assert_eq!(tail.len(), 1);
    assert!(tail[0].chars().count() <= MAX_EXCERPT_LINE_CHARS + 1);
    assert!(tail[0].ends_with('…'));
}

#[test]
fn shortening_never_splits_a_multibyte_character() {
    let long = "网".repeat(MAX_EXCERPT_LINE_CHARS + 50);
    let tail = redact_tail(&[long], 20, SECRET);
    assert!(tail[0].ends_with('…'));
}
