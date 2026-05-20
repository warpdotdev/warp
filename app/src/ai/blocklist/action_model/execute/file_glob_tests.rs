/// Tests for the pure command builders behind [`run_find_command`] and
/// [`run_powershell_get_childitem_command`]. These pin the shell-quoting
/// guarantee for #11132: an agent-supplied `target_path` containing shell
/// metacharacters must survive verbatim into the underlying `find` /
/// `Get-ChildItem` invocation.
use super::{build_find_command, build_get_childitem_command};

#[test]
fn find_posix_escapes_target_path_spaces() {
    let cmd = build_find_command(&["*.rs".to_string()], "/tmp/my code");
    assert_eq!(cmd, r#"find /tmp/my\ code -type f  -name '*.rs'"#);
}

#[test]
fn find_posix_escapes_command_substitution() {
    let cmd = build_find_command(&["*.txt".to_string()], "/tmp/innocent$(touch ~/PROBE_RAN)");
    assert!(
        !cmd.contains("$(touch"),
        "unescaped command substitution survived: {cmd}"
    );
    assert!(cmd.contains(r"\$\(touch"));
}

#[test]
fn find_posix_escapes_backticks() {
    let cmd = build_find_command(&["*.md".to_string()], "/tmp/`rm`/here");
    assert!(
        !cmd.contains("/`rm`/"),
        "unescaped backtick run survived: {cmd}"
    );
    assert!(cmd.contains(r"\`rm\`"));
}

#[test]
fn find_handles_multiple_patterns() {
    // Pattern args are joined with ` -o ` and must survive without the
    // path-quoting changes affecting them.
    let cmd = build_find_command(&["*.rs".to_string(), "*.toml".to_string()], "/tmp/repo");
    assert_eq!(
        cmd,
        "find /tmp/repo -type f  -name '*.rs' -o -name '*.toml'"
    );
}

#[test]
fn get_childitem_powershell_escapes_target_path() {
    let cmd = build_get_childitem_command(&["*.rs".to_string()], "C:\\Users\\me\\My Code");
    assert!(
        cmd.contains("-Path C:\\Users\\me\\My`\u{20}Code "),
        "expected escaped path; got: {cmd}"
    );
    assert!(cmd.starts_with("Get-ChildItem -File -Recurse -Include '*.rs' -Path "));
}

#[test]
fn get_childitem_powershell_escapes_metacharacters() {
    // PowerShell expands `$x` and `$env:USERPROFILE` inside `-Path "..."`;
    // we need every metacharacter shell-escaped via backtick.
    let cmd =
        build_get_childitem_command(&["*.txt".to_string()], "C:\\Users\\$env:USERPROFILE\\repo");
    let mut iter = cmd.match_indices("$env:");
    if let Some((idx, _)) = iter.next() {
        assert!(
            idx > 0 && &cmd[idx - 1..idx] == "`",
            "unescaped $env: at byte {idx}: {cmd}"
        );
    }
}

#[test]
fn empty_path_is_handled_safely() {
    // `ShellFamily::shell_escape("")` returns `''` (the POSIX literal
    // empty string).
    let cmd = build_find_command(&["*.rs".to_string()], "");
    assert!(cmd.starts_with("find '' -type f"));
}
