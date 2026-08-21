use super::*;
use crate::terminal::model::secrets::regexes::FIREBASE_AUTH_DOMAIN;
use crate::terminal::shell::ShellType;

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

#[test]
fn build_git_grep_command_single_quotes_shell_substitution() {
    let queries = vec!["$(touch /tmp/warp-poc); `id`".to_string()];

    let command = build_git_grep_command(&queries, "/tmp/repo path", ShellType::Bash);

    assert_eq!(
        command,
        "git --no-pager grep --color=never --untracked -nIE -e '$(touch /tmp/warp-poc); `id`' '/tmp/repo path'"
    );
}

#[test]
fn build_grep_command_escapes_single_quotes() {
    let queries = vec!["owner's code".to_string()];

    let command = build_grep_command(&queries, "/tmp/repo", ShellType::Bash);

    assert_eq!(
        command,
        r#"grep --color=never -nrIHE --devices=skip -e 'owner'"'"'s code' '/tmp/repo'"#
    );
}

#[test]
fn build_select_string_command_single_quotes_powershell_substitution() {
    let queries = vec![r#"$(New-Item C:\pwn); 'literal'"#.to_string()];

    let command = build_select_string_command(&queries, r#"C:\repo path"#);

    assert_eq!(
        command,
        r#"Get-ChildItem -Path 'C:\repo path' -Recurse -File | Select-String -NoEmphasis -CaseSensitive -Pattern '$(New-Item C:\pwn); ''literal'''"#
    );
}

#[test]
fn parse_grep_output_handles_colon_in_windows_path() {
    let output = r#"C:\repo\file.rs:42:some content"#;

    let matched_files = parse_grep_output(output, None, None).expect("Should parse successfully");

    assert_eq!(matched_files.len(), 1);
    assert_eq!(matched_files[0].file_path, r#"C:\repo\file.rs"#);
    assert_eq!(
        matched_files[0].matched_lines,
        vec![GrepLineMatch { line_number: 42 }]
    );
}

#[test]
fn parse_grep_output_handles_colon_in_relative_path() {
    let output = "path/with:colon/file.go:7:content";

    let matched_files = parse_grep_output(output, None, None).expect("Should parse successfully");

    assert_eq!(matched_files.len(), 1);
    assert_eq!(matched_files[0].file_path, "path/with:colon/file.go");
    assert_eq!(
        matched_files[0].matched_lines,
        vec![GrepLineMatch { line_number: 7 }]
    );
}

#[test]
fn parse_grep_output_handles_colon_in_go_module_path() {
    let output = "vendor/github.com/foo/bar:v1/pkg/x.go:42:return nil";

    let matched_files = parse_grep_output(output, None, None).expect("Should parse successfully");

    assert_eq!(matched_files.len(), 1);
    assert_eq!(
        matched_files[0].file_path,
        "vendor/github.com/foo/bar:v1/pkg/x.go"
    );
    assert_eq!(
        matched_files[0].matched_lines,
        vec![GrepLineMatch { line_number: 42 }]
    );
}

#[test]
fn parse_grep_output_skips_unparseable_lines_but_keeps_valid_matches() {
    let output = "src/main.rs:10:foo\nthis line has no line number\nsrc/lib.rs:20:bar";

    let mut matched_files =
        parse_grep_output(output, None, None).expect("Should parse successfully");
    matched_files.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    assert_eq!(
        matched_files,
        vec![
            GrepFileMatch {
                file_path: "src/lib.rs".to_string(),
                matched_lines: vec![GrepLineMatch { line_number: 20 }],
            },
            GrepFileMatch {
                file_path: "src/main.rs".to_string(),
                matched_lines: vec![GrepLineMatch { line_number: 10 }],
            },
        ]
    );
}

#[test]
fn parse_grep_output_errors_when_every_line_is_unparseable() {
    let output = "not a grep line\nneither is this one";

    let result = parse_grep_output(output, None, None);

    assert!(result.is_err());
}

#[test]
fn parse_grep_output_returns_empty_for_empty_output() {
    let matched_files = parse_grep_output("", None, None).expect("Should parse successfully");

    assert!(matched_files.is_empty());
}

#[test]
fn split_grep_line_ignores_leading_colon() {
    assert_eq!(split_grep_line(":10:content"), None);
}

#[test]
fn split_grep_line_finds_boundary_after_colon_in_path() {
    assert_eq!(
        split_grep_line("path/with:colon/file.go:7:content"),
        Some(("path/with:colon/file.go", 7))
    );
}
