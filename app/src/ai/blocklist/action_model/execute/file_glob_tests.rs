/// Tests for the pure command builders behind [`run_find_command`] and
/// [`run_powershell_get_childitem_command`]. These pin the shell-quoting
/// guarantee for #11132: an agent-supplied `target_path` containing shell
/// metacharacters must survive verbatim into the underlying `find` /
/// `Get-ChildItem` invocation.
use super::{build_find_command, build_get_childitem_command, build_git_ls_files_command};
use crate::terminal::shell::ShellType;

#[test]
fn find_posix_escapes_target_path_spaces() {
    let cmd = build_find_command(&["*.rs".to_string()], "/tmp/my code");
    // Both the path AND the pattern are shell-escaped. The `*` in the pattern
    // is backslash-escaped at the shell level; the shell consumes the escape
    // and hands `find` the literal `*` for its own glob matching.
    assert_eq!(cmd, r"find /tmp/my\ code -type f  -name \*.rs");
}

#[test]
fn find_posix_escapes_command_substitution_in_path() {
    let cmd = build_find_command(&["*.txt".to_string()], "/tmp/innocent$(touch ~/PROBE_RAN)");
    assert!(
        !cmd.contains("$(touch"),
        "unescaped command substitution survived: {cmd}"
    );
    assert!(cmd.contains(r"\$\(touch"));
}

#[test]
fn find_posix_escapes_backticks_in_path() {
    let cmd = build_find_command(&["*.md".to_string()], "/tmp/`rm`/here");
    assert!(
        !cmd.contains("/`rm`/"),
        "unescaped backtick run survived: {cmd}"
    );
    assert!(cmd.contains(r"\`rm\`"));
}

#[test]
fn find_posix_escapes_single_quote_in_pattern() {
    // The previous implementation wrapped each pattern in `'...'`. An agent
    // glob containing a single quote closed the wrapper and let everything
    // after it execute as shell input. After shell-escape, the single quote
    // is backslash-escaped and the shell sees one token.
    let cmd = build_find_command(&["evil'$(rm)".to_string()], "/tmp/repo");
    assert!(
        !cmd.contains("$(rm"),
        "unescaped command substitution survived in pattern: {cmd}"
    );
    assert!(cmd.contains(r"\'\$\(rm\)"));
}

#[test]
fn find_handles_multiple_patterns() {
    let cmd = build_find_command(&["*.rs".to_string(), "*.toml".to_string()], "/tmp/repo");
    // Each pattern's `*` is backslash-escaped at the shell level; `find`
    // still receives literal `*` characters for its glob matching.
    assert_eq!(cmd, r"find /tmp/repo -type f  -name \*.rs -o -name \*.toml");
}

#[test]
fn get_childitem_powershell_uses_literal_path() {
    let cmd = build_get_childitem_command(&["*.rs".to_string()], "C:\\Users\\me\\My Code");
    // `-LiteralPath` suppresses PowerShell wildcard interpretation on the
    // search directory — a path containing `*` / `?` / `[...]` is treated
    // as literal rather than as a glob pattern (which would otherwise
    // change which directory we recurse into). Glob semantics for filtering
    // are still applied via `-Include`.
    assert!(
        cmd.contains("-LiteralPath C:\\Users\\me\\My`\u{20}Code"),
        "expected -LiteralPath escaped path; got: {cmd}"
    );
    assert!(
        !cmd.contains("-Path "),
        "wildcard-interpretable -Path leaked: {cmd}"
    );
    // Pattern is single-quote-wrapped (PowerShell literal form).
    assert!(
        cmd.starts_with("Get-ChildItem -File -Recurse -Include '*.rs' -LiteralPath "),
        "expected literal-wrapped pattern; got: {cmd}"
    );
}

#[test]
fn get_childitem_powershell_escapes_metacharacters_in_path() {
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
fn get_childitem_powershell_env_var_in_pattern_stays_literal() {
    // PowerShell single-quoted literals don't expand `$env:`, so the pattern
    // reaches `Get-ChildItem -Include` verbatim.
    let cmd = build_get_childitem_command(&["leak$env:USERPROFILE".to_string()], "C:\\repo");
    assert!(cmd.contains("-Include 'leak$env:USERPROFILE'"));
}

#[test]
fn empty_path_is_handled_safely() {
    // `ShellFamily::shell_escape("")` returns `''` (the POSIX literal
    // empty string).
    let cmd = build_find_command(&["*.rs".to_string()], "");
    assert!(cmd.starts_with("find '' -type f"));
}

// --- git ls-files (the in-git-repo branch) -----------------------------------

#[test]
fn git_ls_files_posix_escapes_target_path_spaces() {
    let cmd =
        build_git_ls_files_command(&["*.rs".to_string()], "/tmp/my code", ShellType::Bash, None);
    assert!(
        cmd.contains(r"/tmp/my\ code/\*.rs"),
        "expected escaped path + glob; got: {cmd}"
    );
    assert!(cmd.starts_with("git ls-files -c -o --exclude-standard -- "));
}

#[test]
fn git_ls_files_posix_escapes_single_quote_in_target_path() {
    // The original implementation wrapped each joined path in literal single
    // quotes (`'...'`), so a single quote inside `target_path` closed the
    // wrapper and let everything after it parse as shell input — a real
    // injection vector once the shell saw a following `$(...)`.
    let cmd = build_git_ls_files_command(
        &["*.rs".to_string()],
        "/tmp/innocent'$(touch ~/PROBE_RAN)",
        ShellType::Bash,
        None,
    );
    assert!(
        !cmd.contains("$(touch"),
        "unescaped command substitution survived: {cmd}"
    );
    assert!(
        !cmd.contains("/tmp/innocent'$"),
        "raw single quote followed by `$` survived: {cmd}"
    );
    assert!(cmd.contains(r"\'\$\(touch\ \~/PROBE_RAN\)"));
}

#[test]
fn git_ls_files_posix_escapes_command_substitution() {
    let cmd = build_git_ls_files_command(
        &["*.txt".to_string()],
        "/tmp/innocent$(rm -rf ~)",
        ShellType::Bash,
        None,
    );
    assert!(
        !cmd.contains("$(rm"),
        "unescaped command substitution survived: {cmd}"
    );
    assert!(cmd.contains(r"\$\(rm\ -rf\ \~\)"));
}

#[test]
fn git_ls_files_posix_escapes_backticks() {
    let cmd = build_git_ls_files_command(
        &["*.md".to_string()],
        "/tmp/`rm`/here",
        ShellType::Bash,
        None,
    );
    assert!(
        !cmd.contains("/`rm`/"),
        "unescaped backtick run survived: {cmd}"
    );
    // Each backtick must be backslash-escaped.
    for (i, c) in cmd.char_indices() {
        if c == '`' {
            assert!(
                i > 0 && cmd.as_bytes()[i - 1] == b'\\',
                "unescaped backtick at byte {i}: {cmd}"
            );
        }
    }
}

#[test]
fn git_ls_files_powershell_escapes_target_path() {
    // PowerShell variant: backtick is the escape character. The single-quote
    // wrapping was already POSIX-only, so the previous code shipped broken
    // PowerShell quoting; with `ShellFamily::from(ShellType::PowerShell)`
    // the same call path now produces a shell-safe arg on Windows.
    let cmd = build_git_ls_files_command(
        &["*.rs".to_string()],
        "C:\\Users\\me\\My Code",
        ShellType::PowerShell,
        None,
    );
    // PowerShell uses backtick-escape; `join_paths` uses `/` as the separator
    // regardless of shell type, so the literal substring we care about is the
    // escaped path component plus the escaped glob.
    assert!(
        cmd.contains("C:\\Users\\me\\My` Code/`*.rs"),
        "expected backtick-escaped path + glob; got: {cmd}"
    );
    // Sanity: the previous POSIX-only single-quote wrapping must not be present.
    assert!(
        !cmd.contains("'C:\\Users"),
        "unsafe POSIX single-quoting leaked into PowerShell output: {cmd}"
    );
}

#[test]
fn git_ls_files_emits_both_top_level_and_subdir_pattern_args() {
    // The builder doubles every pattern: once joined as `<path>/<pattern>` and
    // once as `<path>/*/<pattern>` so git matches files at the top of the
    // target directory AND in any subdirectory.
    let cmd = build_git_ls_files_command(&["*.rs".to_string()], "/tmp/repo", ShellType::Bash, None);
    assert!(
        cmd.contains(r"/tmp/repo/\*.rs"),
        "missing top-level arg: {cmd}"
    );
    assert!(
        cmd.contains(r"/tmp/repo/\*/\*.rs"),
        "missing subdirectory arg: {cmd}"
    );
}
