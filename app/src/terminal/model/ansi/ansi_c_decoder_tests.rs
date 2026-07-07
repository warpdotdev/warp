use super::parse_ansi_c_quoted_string;

#[test]
fn test_unescape_ps1() {
    // This a "real-life" example of a PS1 sent from Git Bash with `printf %q`.
    let escaped_ps1 = "$'\\001\\E]0;MINGW64:/c/Users/abhis\\a\\002\\r\\n\\001\\E[32m\\002abhis@abhi-linux \\001\\E[35m\\002MINGW64 \\001\\E[33m\\002~\\001\\E[36m\\002\\001\\E[0m\\002\\r\\n$ '";
    assert_eq!(parse_ansi_c_quoted_string(escaped_ps1.to_string()), "\x1b]0;MINGW64:/c/Users/abhis\x07\r\n\x1b[32mabhis@abhi-linux \x1b[35mMINGW64 \x1b[33m~\x1b[36m\x1b[0m\r\n$ ");
}

#[test]
fn test_unescape_ps1_cjk() {
    let escaped_ps1 = r#"$'\001\E]0;MINGW64:/c/Users/abhis/dev/你\a\002\r\n\001\E[32m\002abhis@abhi-linux \001\E[35m\002MINGW64 \001\E[33m\002~/dev/你\001\E[36m\002\001\E[0m\002\r\n$ '"#;
    assert_eq!(parse_ansi_c_quoted_string(escaped_ps1.to_string()), "\x1b]0;MINGW64:/c/Users/abhis/dev/你\x07\r\n\x1b[32mabhis@abhi-linux \x1b[35mMINGW64 \x1b[33m~/dev/你\x1b[36m\x1b[0m\r\n$ ");
}

#[test]
fn test_unescape_ps1_empty() {
    let escaped_ps1 = "";
    assert_eq!(parse_ansi_c_quoted_string(escaped_ps1.to_string()), "");
    let escaped_ps1 = "$''";
    assert_eq!(parse_ansi_c_quoted_string(escaped_ps1.to_string()), "");
}

#[test]
fn test_unescape_ps1_common_characters() {
    let escaped_ps1 = "$'\\a \\b \\E \\e \\f \\n \\r \\t \\v \\\\ \\\' \\\" \\?'";
    assert_eq!(
        parse_ansi_c_quoted_string(escaped_ps1.to_string()),
        "\x07 \x08 \x1b \x1b \x0c \n \r \t \x0b \\ \' \" ?"
    );
}

#[test]
fn test_unescape_ps1_octal_sequences() {
    let escaped_ps1 = "$'\\101\\177'"; // Octal sequences
    assert_eq!(parse_ansi_c_quoted_string(escaped_ps1.to_string()), "A\x7f");
}

#[test]
fn test_unescape_ps1_hex_sequences() {
    let escaped_ps1 = "$'\\x1b\\x7f'"; // Hex sequences
    assert_eq!(
        parse_ansi_c_quoted_string(escaped_ps1.to_string()),
        "\x1b\x7f"
    );
}

#[test]
fn test_unescape_ps1_unicode_4() {
    let escaped_ps1 = "$'\\u597D\\u4E0D\\u597D'";
    assert_eq!(
        parse_ansi_c_quoted_string(escaped_ps1.to_string()),
        "好不好"
    );
}

#[test]
fn test_unescape_ps1_unicode_8() {
    let escaped_ps1 = "$'\\U0000597D'"; // Hex sequences
    assert_eq!(parse_ansi_c_quoted_string(escaped_ps1.to_string()), "好");
}

#[test]
fn test_unescape_ps1_mixed_content() {
    let escaped_ps1 = "$'Hello\\nWorld\\x21'"; // Mixed normal text and escapes
    assert_eq!(
        parse_ansi_c_quoted_string(escaped_ps1.to_string()),
        "Hello\nWorld!"
    );
}

#[test]
fn test_unescape_ps1_control_sequences() {
    let escaped_ps1 = "$'\\cA\\cZ'"; // Control characters
    assert_eq!(
        parse_ansi_c_quoted_string(escaped_ps1.to_string()),
        "\x01\x1A"
    );
}

#[test]
fn test_unescape_ps1_invalid_sequences() {
    let escaped_ps1 = "$'\\z\\xZZ\\U3D\\'";
    // Instead of preserving the invalid escape sequences, we want to ensure that the algorithm
    // doesn't get stuck (i.e. the test finishes).
    let _ = parse_ansi_c_quoted_string(escaped_ps1.to_string());
}
