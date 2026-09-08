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
        "git --no-pager grep --color=never --untracked -nIEz -e '$(touch /tmp/warp-poc); `id`' '/tmp/repo path'"
    );
}

#[test]
fn build_grep_command_escapes_single_quotes() {
    let queries = vec!["owner's code".to_string()];

    let command = build_grep_command(&queries, "/tmp/repo", ShellType::Bash);

    assert_eq!(
        command,
        r#"grep --color=never -nrIHE --devices=skip --null -e 'owner'"'"'s code' '/tmp/repo'"#
    );
}

#[test]
fn build_grep_command_uses_long_null_option_not_short_z() {
    // `-Z` means `--decompress` (run as zgrep) on BSD/macOS grep, not NUL
    // delimiting -- and is accepted silently there, with ordinary
    // colon-delimited output. The long `--null` option is the only
    // portable spelling; never "simplify" this back to `-Z`.
    let queries = vec!["needle".to_string()];

    let command = build_grep_command(&queries, "/tmp/repo", ShellType::Bash);

    assert!(command.contains("--null"));
    assert!(!command.split_whitespace().any(|arg| arg == "-Z"));
}

#[test]
fn build_grep_list_files_command_lists_recursively() {
    let queries = vec!["needle".to_string()];

    let command = build_grep_list_files_command(&queries, "/tmp/repo", ShellType::Bash);

    assert_eq!(
        command,
        "grep --color=never -rlIE --devices=skip -e 'needle' '/tmp/repo'"
    );
}

#[test]
fn build_grep_content_scan_command_wraps_script_in_sh_c() {
    let queries = vec!["needle".to_string()];

    let command = build_grep_content_scan_command(&queries, "/tmp/repo", ShellType::Bash);

    let expected_script = r#"files=$(grep --color=never -rlIE --devices=skip -e 'needle' '/tmp/repo'); status=$?; if [ "$status" -gt 1 ]; then exit "$status"; fi; if [ -n "$files" ]; then printf '%s\n' "$files" | while IFS= read -r f; do printf '\000%s\000' "$f"; grep --color=never -nIE --devices=skip -e 'needle' -- "$f"; done; exit 0; fi; exit 1"#;
    assert_eq!(
        command,
        format!(
            "sh -c {}",
            shell_quote_arg(expected_script, ShellType::Bash)
        )
    );
}

#[test]
fn build_grep_content_scan_command_escapes_single_quote_through_both_quoting_layers() {
    let queries = vec!["owner's code".to_string()];

    let command = build_grep_content_scan_command(&queries, "/tmp/repo", ShellType::Bash);

    let expected_script = r#"files=$(grep --color=never -rlIE --devices=skip -e 'owner'"'"'s code' '/tmp/repo'); status=$?; if [ "$status" -gt 1 ]; then exit "$status"; fi; if [ -n "$files" ]; then printf '%s\n' "$files" | while IFS= read -r f; do printf '\000%s\000' "$f"; grep --color=never -nIE --devices=skip -e 'owner'"'"'s code' -- "$f"; done; exit 0; fi; exit 1"#;
    assert_eq!(
        command,
        format!(
            "sh -c {}",
            shell_quote_arg(expected_script, ShellType::Bash)
        )
    );
}

#[test]
fn build_grep_content_scan_command_keeps_adversarial_query_inert_through_both_quoting_layers() {
    let queries = vec!["$(touch /tmp/warp-poc); `id`".to_string()];

    let command = build_grep_content_scan_command(&queries, "/tmp/repo path", ShellType::Bash);

    let expected_script = r#"files=$(grep --color=never -rlIE --devices=skip -e '$(touch /tmp/warp-poc); `id`' '/tmp/repo path'); status=$?; if [ "$status" -gt 1 ]; then exit "$status"; fi; if [ -n "$files" ]; then printf '%s\n' "$files" | while IFS= read -r f; do printf '\000%s\000' "$f"; grep --color=never -nIE --devices=skip -e '$(touch /tmp/warp-poc); `id`' -- "$f"; done; exit 0; fi; exit 1"#;
    assert_eq!(
        command,
        format!(
            "sh -c {}",
            shell_quote_arg(expected_script, ShellType::Bash)
        )
    );
}

#[test]
fn build_grep_content_scan_command_uses_bash_style_quoting_inside_the_script_even_for_fish_sessions()
 {
    // The inner script is parsed by `sh`, not by the session's own shell,
    // so it must always use POSIX/bash-style single-quote escaping
    // internally even when the session itself is fish (which escapes `'`
    // differently). Only the outer wrapping (the argument to `sh -c`) uses
    // the session's actual shell_type.
    let queries = vec!["owner's code".to_string()];

    let command = build_grep_content_scan_command(&queries, "/tmp/repo", ShellType::Fish);

    let expected_script = r#"files=$(grep --color=never -rlIE --devices=skip -e 'owner'"'"'s code' '/tmp/repo'); status=$?; if [ "$status" -gt 1 ]; then exit "$status"; fi; if [ -n "$files" ]; then printf '%s\n' "$files" | while IFS= read -r f; do printf '\000%s\000' "$f"; grep --color=never -nIE --devices=skip -e 'owner'"'"'s code' -- "$f"; done; exit 0; fi; exit 1"#;
    assert_eq!(
        command,
        format!(
            "sh -c {}",
            shell_quote_arg(expected_script, ShellType::Fish)
        )
    );
}

#[test]
fn build_select_string_command_single_quotes_powershell_substitution() {
    let queries = vec![r#"$(New-Item C:\pwn); 'literal'"#.to_string()];

    let command = build_select_string_command(&queries, r#"C:\repo path"#);

    assert_eq!(
        command,
        r#"Get-ChildItem -Path 'C:\repo path' -Recurse -File | Select-String -NoEmphasis -CaseSensitive -Pattern '$(New-Item C:\pwn); ''literal''' | ForEach-Object { "$($_.Path)`0$($_.LineNumber)`0" }"#
    );
}

#[test]
fn parse_null_delimited_grep_output_handles_colon_in_windows_path() {
    // git-grep-`-z`-style record: both separators are NUL.
    let output = "C:\\repo\\file.rs\x0042\0some content\n";

    let matched_files =
        parse_null_delimited_grep_output(output, None, None).expect("Should parse successfully");

    assert_eq!(matched_files.len(), 1);
    assert_eq!(matched_files[0].file_path, r#"C:\repo\file.rs"#);
    assert_eq!(
        matched_files[0].matched_lines,
        vec![GrepLineMatch { line_number: 42 }]
    );
}

#[test]
fn parse_null_delimited_grep_output_handles_gnu_grep_null_style() {
    // GNU/BSD `grep --null` only replaces the path separator with NUL; the
    // line-number separator stays `:`.
    let output = "path/with:colon/file.go\x007:content\n";

    let matched_files =
        parse_null_delimited_grep_output(output, None, None).expect("Should parse successfully");

    assert_eq!(matched_files.len(), 1);
    assert_eq!(matched_files[0].file_path, "path/with:colon/file.go");
    assert_eq!(
        matched_files[0].matched_lines,
        vec![GrepLineMatch { line_number: 7 }]
    );
}

#[test]
fn parse_null_delimited_grep_output_handles_path_that_looks_like_a_record_boundary() {
    // Regression test: a naive `:<digits>:` heuristic would misparse this
    // path, since the path itself contains that exact sequence. The
    // NUL-delimited format has no such ambiguity.
    let output = "src/a:123:part.rs\x007\0needle\n";

    let matched_files =
        parse_null_delimited_grep_output(output, None, None).expect("Should parse successfully");

    assert_eq!(matched_files.len(), 1);
    assert_eq!(matched_files[0].file_path, "src/a:123:part.rs");
    assert_eq!(
        matched_files[0].matched_lines,
        vec![GrepLineMatch { line_number: 7 }]
    );
}

#[test]
fn parse_null_delimited_grep_output_handles_newline_embedded_in_path() {
    // A path containing a raw newline is safe too: the path is delimited by
    // the first NUL byte regardless of what bytes precede it.
    let output = "weird\nname.rs\x0042\0content\n";

    let matched_files =
        parse_null_delimited_grep_output(output, None, None).expect("Should parse successfully");

    assert_eq!(matched_files.len(), 1);
    assert_eq!(matched_files[0].file_path, "weird\nname.rs");
    assert_eq!(
        matched_files[0].matched_lines,
        vec![GrepLineMatch { line_number: 42 }]
    );
}

#[test]
fn parse_null_delimited_grep_output_handles_multiple_records() {
    // Real `git grep -z -n` output for two matches in one file and one in
    // another.
    let output = "colon:file.txt\x001\0needle one\ncolon:file.txt\x002\0second line needle\nnormal.txt\x001\0needle two\n";

    let mut matched_files =
        parse_null_delimited_grep_output(output, None, None).expect("Should parse successfully");
    matched_files.sort_by(|a, b| a.file_path.cmp(&b.file_path));

    assert_eq!(
        matched_files,
        vec![
            GrepFileMatch {
                file_path: "colon:file.txt".to_string(),
                matched_lines: vec![
                    GrepLineMatch { line_number: 1 },
                    GrepLineMatch { line_number: 2 },
                ],
            },
            GrepFileMatch {
                file_path: "normal.txt".to_string(),
                matched_lines: vec![GrepLineMatch { line_number: 1 }],
            },
        ]
    );
}

#[test]
fn parse_null_delimited_grep_output_skips_unparseable_records_but_keeps_valid_matches() {
    // The middle record has a NUL but no digits after it, so it's
    // unparseable; parsing should resync on the following newline and keep
    // going instead of misattributing it to a neighboring record.
    let output = "src/main.rs\x0010\0foo\nbad\0not-a-number\nsrc/lib.rs\x0020\0bar\n";

    let mut matched_files =
        parse_null_delimited_grep_output(output, None, None).expect("Should parse successfully");
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
fn parse_null_delimited_grep_output_errors_when_every_record_is_unparseable() {
    let output = "not a grep record\nneither is this one";

    let result = parse_null_delimited_grep_output(output, None, None);

    assert!(result.is_err());
}

#[test]
fn parse_null_delimited_grep_output_returns_empty_for_empty_output() {
    let matched_files =
        parse_null_delimited_grep_output("", None, None).expect("Should parse successfully");

    assert!(matched_files.is_empty());
}

#[test]
fn take_null_delimited_record_rejects_empty_path() {
    assert_eq!(take_null_delimited_record("\x0010\0content\n"), None);
}

#[test]
fn take_null_delimited_record_rejects_missing_line_number() {
    assert_eq!(take_null_delimited_record("path.rs\0not-a-number\n"), None);
}

#[test]
fn parse_single_file_grep_output_handles_line_with_colon_in_content() {
    // The caller already knows the path (see build_grep_content_scan_command),
    // so a colon-bearing path like `src/a:123:part.rs` never has to appear
    // in this output at all -- there's nothing here for it to be confused
    // with.
    let output = "7:needle: found here\n";

    let line_numbers = parse_single_file_grep_output(output);

    assert_eq!(line_numbers, vec![7]);
}

#[test]
fn parse_single_file_grep_output_skips_lines_without_a_leading_line_number() {
    let output = "10:foo\nno line number here\n20:bar\n";

    let line_numbers = parse_single_file_grep_output(output);

    assert_eq!(line_numbers, vec![10, 20]);
}

#[test]
fn parse_single_file_grep_output_returns_empty_for_empty_output() {
    assert_eq!(parse_single_file_grep_output(""), Vec::<usize>::new());
}

#[test]
fn parse_grep_content_scan_output_handles_multiple_files() {
    let output = "\x00src/main.rs\x0010:foo\n\x00src/lib.rs\x0020:bar\n25:baz\n";

    let matched_files = parse_grep_content_scan_output(output, &None, &None);

    assert_eq!(
        matched_files,
        vec![
            GrepFileMatch {
                file_path: "src/main.rs".to_string(),
                matched_lines: vec![GrepLineMatch { line_number: 10 }],
            },
            GrepFileMatch {
                file_path: "src/lib.rs".to_string(),
                matched_lines: vec![
                    GrepLineMatch { line_number: 20 },
                    GrepLineMatch { line_number: 25 },
                ],
            },
        ]
    );
}

#[test]
fn parse_grep_content_scan_output_handles_colon_in_path() {
    let output = "\x00src/a:123:part.rs\x007:needle\n";

    let matched_files = parse_grep_content_scan_output(output, &None, &None);

    assert_eq!(
        matched_files,
        vec![GrepFileMatch {
            file_path: "src/a:123:part.rs".to_string(),
            matched_lines: vec![GrepLineMatch { line_number: 7 }],
        }]
    );
}

#[test]
fn parse_grep_content_scan_output_handles_a_marker_containing_a_raw_newline() {
    // The parser itself is unambiguous no matter what bytes a `(path,
    // content)` pair holds, including a raw newline in the path -- markers
    // are found by their NUL bytes, never by splitting on newlines. This
    // does not mean the fallback as a whole is newline-safe: see
    // `parse_grep_content_scan_output_skips_fragments_of_a_newline_bearing_path`
    // for what `build_grep_content_scan_command`'s listing loop actually
    // produces for a path like this in practice.
    let output = "\x00weird\nname.rs\x0042:content\n";

    let matched_files = parse_grep_content_scan_output(output, &None, &None);

    assert_eq!(
        matched_files,
        vec![GrepFileMatch {
            file_path: "weird\nname.rs".to_string(),
            matched_lines: vec![GrepLineMatch { line_number: 42 }],
        }]
    );
}

#[test]
fn parse_grep_content_scan_output_skips_fragments_of_a_newline_bearing_path() {
    // Pins the real, verified behavior for a path containing a raw newline
    // byte (e.g. "weird\nname.rs"): `build_grep_content_scan_command`'s
    // `while read -r f` loop still enumerates `grep -l`'s newline-terminated
    // output one line at a time, so it reads that single file as two
    // fragments ("weird" and "name.rs"). Neither fragment names a real
    // file, so both come back with empty content and are skipped here --
    // a missed match, not a misattribution to some other file.
    let output = "\x00weird\x00\x00name.rs\x00";

    let matched_files = parse_grep_content_scan_output(output, &None, &None);

    assert_eq!(matched_files, Vec::<GrepFileMatch>::new());
}

#[test]
fn parse_grep_content_scan_output_skips_a_file_whose_content_came_back_empty() {
    // Pins the skip-not-fail policy: a listed file whose re-grep came back
    // empty (e.g. removed between listing and this command) is dropped
    // rather than reported with zero matches.
    let output = "\x00src/main.rs\x00\x00src/lib.rs\x0010:foo\n";

    let matched_files = parse_grep_content_scan_output(output, &None, &None);

    assert_eq!(
        matched_files,
        vec![GrepFileMatch {
            file_path: "src/lib.rs".to_string(),
            matched_lines: vec![GrepLineMatch { line_number: 10 }],
        }]
    );
}

#[test]
fn parse_grep_content_scan_output_returns_empty_for_empty_output() {
    assert_eq!(
        parse_grep_content_scan_output("", &None, &None),
        Vec::<GrepFileMatch>::new()
    );
}
