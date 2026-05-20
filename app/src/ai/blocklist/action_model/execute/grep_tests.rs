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
        assert_eq!(
            cmd,
            r#"git --no-pager grep --color=never --untracked -nIE -e "TODO" /tmp/my\ repo"#
        );
    }

    #[test]
    fn git_grep_posix_escapes_command_substitution() {
        // Path-level metacharacters get backslash-escaped; query-level
        // double-quotes are kept (the query is regex literal, not a path).
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
    fn git_grep_powershell_path_uses_powershell_escape() {
        // For PowerShell sessions, the path is escaped with backticks
        // (`ShellFamily::PowerShell`) — POSIX backslash escapes are not
        // valid in PowerShell.
        let cmd = build_git_grep_command(
            &["pattern".to_string()],
            "C:\\Users\\me\\repo",
            ShellType::PowerShell,
        );
        // The drive-letter colon stays literal (PowerShell escape doesn't
        // need to escape `:`); the path lands as a single argument.
        assert!(cmd.ends_with(" C:\\Users\\me\\repo"));
        // PowerShell-escaped queries still use the existing
        // backtick-double-quote helper.
        assert!(cmd.contains(r#"-e "pattern""#));
    }

    #[test]
    fn grep_posix_escapes_target_path() {
        let cmd = build_grep_command(&["TODO".to_string()], "/tmp/has space");
        assert_eq!(
            cmd,
            r#"grep --color=never -nrIHE --devices=skip -e "TODO" /tmp/has\ space"#
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
    fn select_string_powershell_escapes_target_path() {
        let cmd = build_select_string_command(&["TODO".to_string()], "C:\\Users\\me\\My Stuff");
        // The path appears once, after `-Path `, with PowerShell-style
        // backtick escapes for the space.
        assert!(
            cmd.contains("-Path C:\\Users\\me\\My`\u{20}Stuff "),
            "expected escaped path; got: {cmd}"
        );
        // Pattern is kept as a backtick-double-quoted PowerShell string.
        assert!(cmd.ends_with(r#"-Pattern "TODO""#));
    }

    #[test]
    fn select_string_powershell_escapes_command_substitution() {
        // PowerShell expands `$x` and `$env:USER` inside double quotes; we
        // need every metacharacter shell-escaped via backtick.
        let cmd =
            build_select_string_command(&["q".to_string()], "C:\\Users\\$env:USERPROFILE\\repo");
        // No bare `$env:` survives in the rendered command — it's always
        // preceded by a backtick.
        let mut iter = cmd.match_indices("$env:");
        if let Some((idx, _)) = iter.next() {
            assert!(
                idx > 0 && &cmd[idx - 1..idx] == "`",
                "unescaped $env: at byte {idx}: {cmd}"
            );
        }
    }
}
