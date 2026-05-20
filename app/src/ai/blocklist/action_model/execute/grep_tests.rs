use super::*;
use crate::terminal::{model::secrets::regexes::FIREBASE_AUTH_DOMAIN, shell::ShellType};

#[test]
fn test_create_redacted_grep_error_event() {
    crate::terminal::model::set_user_and_enterprise_secret_regexes(
        [&regex::Regex::new(FIREBASE_AUTH_DOMAIN).expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    // Create input with a known secret pattern (Firebase domain)
    let queries = vec![
        "normal query".to_string(),
        "query with warp-server-staging.firebaseapp.com secret".to_string(),
    ];
    let path = "path/to/file/with/warp-server-staging.firebaseapp.com/secret".to_string();
    let shell_type = Some(ShellType::Bash);
    let working_directory = Some("/users/test/warp-server-staging.firebaseapp.com".to_string());
    let absolute_path =
        "/absolute/path/with/warp-server-staging.firebaseapp.com/secret".to_string();
    let error = GrepError::new("Error message".to_string())
        .with_command("grep warp-server-staging.firebaseapp.com".to_string())
        .with_output("Output with warp-server-staging.firebaseapp.com".to_string());

    // Call the function with the test inputs
    let event = create_redacted_grep_error_event(
        true,
        None,
        queries.clone(),
        path.clone(),
        shell_type,
        working_directory.clone(),
        absolute_path.clone(),
        error,
    );

    // Verify the telemetry event has redacted secrets
    if let TelemetryEvent::GrepToolFailed {
        queries: Some(redacted_queries),
        path: Some(redacted_path),
        shell_type: _,
        working_directory: Some(redacted_working_directory),
        absolute_path: Some(redacted_absolute_path),
        command: Some(redacted_command),
        output: Some(redacted_output),
        error: _,
        server_output_id: _,
    } = event
    {
        // Verify secrets are redacted from all relevant fields
        assert_eq!(redacted_queries.len(), 2);
        assert_eq!(redacted_queries[0], "normal query");
        assert!(!redacted_queries[1].contains("warp-server-staging.firebaseapp.com"));
        assert!(redacted_queries[1].contains("*****"));

        assert!(!redacted_path.contains("warp-server-staging.firebaseapp.com"));
        assert!(redacted_path.contains("*****"));

        assert!(!redacted_working_directory.contains("warp-server-staging.firebaseapp.com"));
        assert!(redacted_working_directory.contains("*****"));

        assert!(!redacted_absolute_path.contains("warp-server-staging.firebaseapp.com"));
        assert!(redacted_absolute_path.contains("*****"));

        assert!(!redacted_command.contains("warp-server-staging.firebaseapp.com"));
        assert!(redacted_command.contains("*****"));

        assert!(!redacted_output.contains("warp-server-staging.firebaseapp.com"));
        assert!(redacted_output.contains("*****"));
    } else {
        panic!("Expected GrepToolFailed event");
    }
}

/// Tests for the pure command builders behind [`run_git_grep_command`],
/// [`run_grep_command`], and [`run_select_string_command`]. These pin the
/// shell-quoting guarantee for #11132: an agent-supplied `target_path`
/// containing shell metacharacters must survive verbatim into the
/// underlying `git grep` / `grep` / `Select-String` invocation.
mod path_quoting {
    use super::super::{build_git_grep_command, build_grep_command, build_select_string_command};
    use crate::terminal::shell::ShellType;

    #[test]
    fn git_grep_posix_escapes_target_path_spaces() {
        let cmd = build_git_grep_command(&["TODO".to_string()], "/tmp/my repo", ShellType::Bash);
        // Query is wrapped in single-quote literal form (safe for embedded
        // newlines, metacharacters, etc.); path uses backslash-escape.
        assert_eq!(
            cmd,
            r#"git --no-pager grep --color=never --untracked -nIE -e 'TODO' /tmp/my\ repo"#
        );
    }

    #[test]
    fn git_grep_posix_escapes_command_substitution_in_path() {
        // Path-level metacharacters get backslash-escaped.
        let cmd = build_git_grep_command(
            &["pattern".to_string()],
            "/tmp/$(touch ~/PROBE_RAN)",
            ShellType::Bash,
        );
        assert!(
            !cmd.contains("$(touch"),
            "unescaped command substitution survived: {cmd}"
        );
        assert!(cmd.contains(r"\$\(touch"));
    }

    #[test]
    fn git_grep_posix_command_substitution_in_query_stays_literal() {
        // Single-quote literal form neutralizes `$(...)` without per-char
        // escaping — POSIX shells don't expand anything inside `'...'`.
        let cmd = build_git_grep_command(
            &["match$(rm -rf ~)me".to_string()],
            "/tmp/repo",
            ShellType::Bash,
        );
        assert!(cmd.contains(" -e 'match$(rm -rf ~)me' "));
    }

    #[test]
    fn git_grep_posix_backticks_in_query_stay_literal() {
        // Same shape — inside POSIX single quotes, backticks are literal.
        let cmd =
            build_git_grep_command(&["a`rm -rf ~`b".to_string()], "/tmp/repo", ShellType::Bash);
        assert!(cmd.contains(" -e 'a`rm -rf ~`b' "));
    }

    #[test]
    fn git_grep_posix_embedded_newline_in_query_is_preserved() {
        // The reason we switched away from `shell_escape` for queries:
        // shell_escape would emit `\<newline>` which bash/zsh parse as line
        // continuation, breaking the command. Single-quote wrapping keeps
        // the literal newline as part of the argument the grep tool sees.
        let cmd = build_git_grep_command(&["foo\nbar".to_string()], "/tmp/repo", ShellType::Bash);
        // The query has a literal newline inside the wrapping quotes, NOT
        // a `\<newline>` line-continuation sequence.
        assert!(cmd.contains(" -e 'foo\nbar' "));
        assert!(
            !cmd.contains("\\\n"),
            "line-continuation slipped in: {cmd:?}"
        );
    }

    #[test]
    fn git_grep_posix_query_embedded_single_quote_is_escaped() {
        // Inside POSIX single-quote literal, an embedded `'` must be
        // expressed as `'\''` (close, escaped quote, reopen).
        let cmd = build_git_grep_command(&["a'b".to_string()], "/tmp/repo", ShellType::Bash);
        assert!(cmd.contains(r"'a'\''b'"), "got: {cmd}");
    }

    #[test]
    fn git_grep_powershell_query_uses_single_quote_literal() {
        let cmd = build_git_grep_command(
            &["pattern".to_string()],
            "C:\\Users\\me\\repo",
            ShellType::PowerShell,
        );
        assert!(cmd.ends_with(" C:\\Users\\me\\repo"));
        // Query is single-quote-wrapped (PowerShell literal form).
        assert!(cmd.contains(" -e 'pattern' "));
    }

    #[test]
    fn git_grep_powershell_env_var_in_query_stays_literal() {
        // PowerShell single-quoted strings are literal — `$env:VAR` is NOT
        // expanded inside them.
        let cmd = build_git_grep_command(
            &["leak$env:USERPROFILE".to_string()],
            "C:\\repo",
            ShellType::PowerShell,
        );
        assert!(cmd.contains(" -e 'leak$env:USERPROFILE' "));
    }

    #[test]
    fn git_grep_powershell_query_embedded_single_quote_doubles() {
        // PowerShell single-quote literal escapes `'` by doubling it.
        let cmd = build_git_grep_command(&["a'b".to_string()], "C:\\repo", ShellType::PowerShell);
        assert!(cmd.contains(" -e 'a''b' "), "got: {cmd}");
    }

    #[test]
    fn grep_posix_escapes_target_path() {
        let cmd = build_grep_command(&["TODO".to_string()], "/tmp/has space");
        assert_eq!(
            cmd,
            r#"grep --color=never -nrIHE --devices=skip -e 'TODO' /tmp/has\ space"#
        );
    }

    #[test]
    fn grep_posix_escapes_metacharacters_in_path() {
        let cmd = build_grep_command(&["a".to_string()], "/tmp/innocent$(rm -rf ~)");
        assert!(
            !cmd.contains("$(rm"),
            "unescaped command substitution survived: {cmd}"
        );
    }

    #[test]
    fn grep_posix_command_substitution_in_query_stays_literal() {
        let cmd = build_grep_command(&["a$(rm)b".to_string()], "/tmp/repo");
        assert!(cmd.contains(" -e 'a$(rm)b' "));
    }

    #[test]
    fn select_string_powershell_uses_literal_path() {
        // `-LiteralPath` suppresses PowerShell wildcard interpretation on
        // the search directory — without it a path containing `*` / `?` /
        // `[...]` would be expanded by Get-ChildItem itself.
        let cmd = build_select_string_command(&["TODO".to_string()], "C:\\Users\\me\\My Stuff");
        assert!(
            cmd.contains("-LiteralPath C:\\Users\\me\\My`\u{20}Stuff "),
            "expected -LiteralPath escaped path; got: {cmd}"
        );
        assert!(
            !cmd.contains("-Path "),
            "wildcard-interpretable -Path leaked: {cmd}"
        );
        assert!(cmd.ends_with("-Pattern 'TODO'"));
    }

    #[test]
    fn select_string_powershell_escapes_command_substitution_in_path() {
        let cmd =
            build_select_string_command(&["q".to_string()], "C:\\Users\\$env:USERPROFILE\\repo");
        let mut iter = cmd.match_indices("$env:");
        if let Some((idx, _)) = iter.next() {
            assert!(
                idx > 0 && &cmd[idx - 1..idx] == "`",
                "unescaped $env: at byte {idx}: {cmd}"
            );
        }
    }

    #[test]
    fn select_string_powershell_env_var_in_query_stays_literal() {
        let cmd = build_select_string_command(&["leak$env:USERPROFILE".to_string()], "C:\\repo");
        assert!(cmd.contains("-Pattern 'leak$env:USERPROFILE'"));
    }
}
