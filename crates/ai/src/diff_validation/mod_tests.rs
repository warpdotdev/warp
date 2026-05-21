use std::vec;

use super::*;

fn deltas(diff: &AIRequestedCodeDiff) -> &[DiffDelta] {
    match &diff.diff_type {
        DiffType::Update { deltas, .. } => deltas,
        other => panic!("Expected Update diff_type, got {other:?}"),
    }
}

const CONTENT: &str = "I'd just like to interject
                        for a moment. What you're refering to as
                        Linux, is in fact, GNU/Linux, or as I've
                        recently taken to calling it, GNU plus
                        Linux. Linux is not an operating system
                        unto itself, but rather another free
                        component of a fully functioning GNU
                        system made useful by the GNU corelibs,
                        shell utilities and vital system
                        components comprising a full OS as
                        defined by POSIX.";

#[test]
fn test_simple() {
    let input_diffs = vec![
        SearchAndReplace {
            search: "2|hey".to_string(),
            replace: "what".to_string(),
        },
        SearchAndReplace {
            search: "4|world\n5|of".to_string(),
            replace: "hey".to_string(),
        },
    ];

    let diff = fuzzy_match_diffs("test.rs", &input_diffs, "what\nhey\nthere\nworld\nof\n");
    assert_eq!(diff.file_name, "test.rs");
    assert_eq!(
        deltas(&diff),
        &[
            DiffDelta {
                replacement_line_range: 2..3,
                insertion: "what".to_string(),
            },
            DiffDelta {
                replacement_line_range: 4..6,
                insertion: "hey".to_string(),
            }
        ]
    );
}

#[test]
fn test_incorrect_line_numbers() {
    let input_diffs = vec![SearchAndReplace {
        search: "4|world\n5|of".to_string(),
        replace: "hey".to_string(),
    }];

    let diff = fuzzy_match_diffs("test.rs", &input_diffs, "what\nthere\nworld\nof");
    assert_eq!(diff.file_name, "test.rs");
    assert_eq!(
        deltas(&diff),
        &[DiffDelta {
            replacement_line_range: 3..5,
            insertion: "hey".to_string(),
        }]
    );
}

#[test]
fn test_missing_line_numbers() {
    let input_diffs = vec![SearchAndReplace {
        search: "hey\nthere".to_string(),
        replace: "world".to_string(),
    }];

    let diff = fuzzy_match_diffs("test.rs", &input_diffs, "what\nhey\nthere\nworld\nof\n");
    assert_eq!(diff.file_name, "test.rs");
    assert_eq!(
        deltas(&diff),
        &[DiffDelta {
            replacement_line_range: 2..4,
            insertion: "world".to_string(),
        }]
    );

    let failures = diff.failures.expect("Expected failures to be tracked");
    assert_eq!(failures.missing_line_numbers, 1);
    assert_eq!(failures.fuzzy_match_failures, 0);
    assert_eq!(failures.noop_deltas, 0);
}

#[test]
fn test_blank_search() {
    let input_diffs = vec![SearchAndReplace {
        search: "".to_string(),
        replace: "hey".to_string(),
    }];

    let diff = fuzzy_match_diffs("test.rs", &input_diffs, "what\nhey\nthere\nworld\nof\n");
    assert_eq!(diff.file_name, "test.rs");
    assert_eq!(
        deltas(&diff),
        &[DiffDelta {
            replacement_line_range: 0..0,
            insertion: "hey".to_string(),
        }]
    );
}

#[test]
fn test_closest() {
    let input_diffs = vec![SearchAndReplace {
        search: "4|world\n5|of".to_string(),
        replace: "hey".to_string(),
    }];

    let diff = fuzzy_match_diffs(
        "test.rs",
        &input_diffs,
        "what\nhey\nworld\nof\nthe\nworld\nof\n",
    );
    assert_eq!(diff.file_name, "test.rs");
    assert_eq!(
        deltas(&diff),
        &[DiffDelta {
            replacement_line_range: 3..5,
            insertion: "hey".to_string(),
        }]
    );
}

#[test]
fn test_line_numbers_off_by_one() {
    let insertion = "                        Linux, is in fact, GNU/Linux, or as I've
                        recently taken to calling it, GNU plus
                        Linux. Linux is not an operating system
                        unto itself, but rather another free
                        component of a fully functioning GNU
                        system made useful by the GNU corelibs,
                        hello, world!"
        .to_string();
    let input_diffs = vec![SearchAndReplace {
        search: "2|                        Linux, is in fact, GNU/Linux, or as I've\n\
                 3|                        recently taken to calling it, GNU plus\n\
                 4|                        Linux. Linux is not an operating system\n\
                 5|                        unto itself, but rather another free\n\
                 6|                        component of a fully functioning GNU\n\
                 7|                        system made useful by the GNU corelibs,"
            .to_string(),
        replace: insertion.clone(),
    }];
    let diff = fuzzy_match_diffs("test.rs", &input_diffs, CONTENT);
    assert_eq!(
        deltas(&diff),
        &[DiffDelta {
            replacement_line_range: 3..9,
            insertion,
        }]
    );
}

#[test]
fn test_append_to_end_of_file() {
    let input_diffs = vec![SearchAndReplace {
        search: "3|".to_string(),
        replace: "foo".to_string(),
    }];
    let diff = fuzzy_match_diffs("test.rs", &input_diffs, "\n\n\n");
    assert_eq!(
        deltas(&diff),
        &[DiffDelta {
            replacement_line_range: 3..4,
            insertion: "foo".to_string(),
        }]
    )
}

#[test]
fn test_totally_unrelated_search() {
    let input_diffs = vec![SearchAndReplace {
        search: "4|foo bar baz".to_string(),
        replace: "hello, world!".to_string(),
    }];
    let diff = fuzzy_match_diffs("test.rs", &input_diffs, CONTENT);
    assert!(deltas(&diff).is_empty());
    assert!(diff.failures.is_some());
}

/// The agent sometimes emits a search whose final line is a prefix of the actual file line.
/// Before `PrefixTailMatch`, the Jaro-Winkler scorer landed just under the 0.9 threshold for
/// long lines and the diff failed with `Could not apply all diffs to <file>`.  With
/// `PrefixTailMatch` in the cascade, the rescue succeeds and the existing suffix-preservation
/// fixup splices the unmatched tail into the insertion.
#[test]
fn test_prefix_tail_rescue_with_line_number_hint() {
    let actual_line = "if the stripping tool encounters any error (nesting, unmatched markers, UTF-8 decode failure), the sync workflow **fails** and does **not** update the watermark.  the next run will retry from the same commit.  this is correct fail-closed behavior \u{2014} a stripping error might indicate a condition that could cause private code to leak.";
    let file_content = format!("(preamble)\n\n### error handling\n\n{actual_line}\n\n(trailer)\n");

    // Search is a prefix of line 5, with the `5|` line-number hint.
    let search = "5|if the stripping tool encounters any error (nesting, unmatched markers, UTF-8 decode failure), the sync workflow **fails** and does **not** update the watermark.";
    let replace = "if the stripping tool encounters any error (nesting, unmatched markers, UTF-8 decode failure, symlinks), the sync workflow **fails** and does **not** update the watermark.";

    let input_diffs = vec![SearchAndReplace {
        search: search.to_string(),
        replace: replace.to_string(),
    }];

    let diff = fuzzy_match_diffs("TECH-DESIGN.md", &input_diffs, &file_content);

    // The rescue should produce a single delta replacing line 5 with the replacement
    // plus the unmatched suffix of the original line appended by the existing fixup.
    let unmatched_suffix = &actual_line[search.strip_prefix("5|").unwrap().len()..];
    let expected_insertion = format!("{replace}{unmatched_suffix}");
    assert_eq!(
        deltas(&diff),
        &[DiffDelta {
            replacement_line_range: 5..6,
            insertion: expected_insertion,
        }]
    );

    // The rescue succeeds cleanly — no failure signals should be surfaced.
    assert!(diff.failures.is_none());
    assert!(!diff.warrants_failure());
}

#[test]
fn test_parse_line_numbers() {
    let search = "1|hey\n2|there\n3|world";
    let (line_range, line) = parse_line_numbers(search);
    assert_eq!(line_range, Some(1..4));
    assert_eq!(line, "hey\nthere\nworld");

    let search = "hey\nthere";
    let (line_range, line) = parse_line_numbers(search);
    assert_eq!(line_range, None);
    assert_eq!(line, "hey\nthere");

    let search = "";
    let (line_range, line) = parse_line_numbers(search);
    assert_eq!(line_range, Some(0..0));
    assert_eq!(line, "");
}

#[test]
fn test_remove_extra_line_num_prefix() {
    // Test with line numbers.
    let input = "1|first line\n2|second line\n3|third line".to_string();
    assert_eq!(
        remove_extra_line_num_prefix(input),
        "first line\nsecond line\nthird line"
    );

    // Test with no line numbers.
    let input = "first line\nsecond line".to_string();
    assert_eq!(
        remove_extra_line_num_prefix(input),
        "first line\nsecond line"
    );

    // Test empty string.
    assert_eq!(remove_extra_line_num_prefix("".to_string()), "");

    // Test single line with number.
    assert_eq!(
        remove_extra_line_num_prefix("1|only line".to_string()),
        "only line"
    );

    // Test with line numbers with mixed prefixes.
    let input = "first line\n2|second line\n3|third line".to_string();
    assert_eq!(
        remove_extra_line_num_prefix(input),
        "first line\nsecond line\nthird line"
    );

    // Test single line without number.
    let input = "no number line".to_string();
    assert_eq!(remove_extra_line_num_prefix(input.clone()), input);
}

#[test]
fn test_find_similar_sections_out_of_bounds() {
    let matches = find_similar_sections("hey\nthere\nyou", &[], 0.9);
    assert!(matches.is_empty());

    let matches = find_similar_sections("hey\nthere\nyou", &["hey", "there", "you"], 0.9);
    assert_eq!(
        matches,
        vec![Match {
            start_line: 1,
            end_line: 4,
            similarity: 1.0
        }]
    );

    let matches = find_similar_sections("hey\nthere\nyou", &["hey", "there"], 0.9);
    assert!(matches.is_empty());

    let matches = find_similar_sections("", &[], 0.9);
    assert!(matches.is_empty());
}

#[test]
fn test_v4a_exact_match() {
    let hunks = vec![V4AHunk {
        change_context: vec![],
        pre_context: "fn main() {".to_string(),
        old: "    println!(\"Hello\");".to_string(),
        new: "    println!(\"Hello, World!\");".to_string(),
        post_context: "}".to_string(),
    }];

    let file_content = "fn main() {\n    println!(\"Hello\");\n}";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert_eq!(diff.file_name, "test.rs");
    assert_eq!(deltas(&diff).len(), 1);
    assert_eq!(
        deltas(&diff)[0],
        DiffDelta {
            replacement_line_range: 2..3,
            insertion: "    println!(\"Hello, World!\");".to_string(),
        }
    );
}

#[test]
fn test_v4a_with_change_context() {
    let hunks = vec![V4AHunk {
        change_context: vec!["impl MyStruct {".to_string()],
        pre_context: "    fn method1() {\n        // comment".to_string(),
        old: "        let x = 1;".to_string(),
        new: "        let x = 2;".to_string(),
        post_context: "    }\n}".to_string(),
    }];

    let file_content = "struct MyStruct {}\n\nimpl MyStruct {\n    fn method1() {\n        // comment\n        let x = 1;\n    }\n}";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert_eq!(deltas(&diff).len(), 1);
    assert_eq!(
        deltas(&diff)[0],
        DiffDelta {
            replacement_line_range: 6..7,
            insertion: "        let x = 2;".to_string(),
        }
    );
}

#[test]
fn test_v4a_indentation_agnostic_match() {
    // Hunk has different indentation than the actual file
    let hunks = vec![V4AHunk {
        change_context: vec![],
        pre_context: "def hello():".to_string(),
        old: "print(\"hello\")".to_string(), // No indentation
        new: "    print(\"hello world\")".to_string(),
        post_context: "".to_string(),
    }];

    let file_content = "def hello():\n    print(\"hello\")"; // Has indentation
    let diff = fuzzy_match_v4a_diffs("test.py", &hunks, None, file_content);

    assert_eq!(deltas(&diff).len(), 1);
    assert_eq!(
        deltas(&diff)[0],
        DiffDelta {
            replacement_line_range: 2..3,
            insertion: "    print(\"hello world\")".to_string(),
        }
    );
}

#[test]
fn test_v4a_fuzzy_match() {
    // Hunk has slightly different content (typo)
    let hunks = vec![V4AHunk {
        change_context: vec![],
        pre_context: "function greet() {".to_string(),
        old: "    console.log(\"helo\");".to_string(), // Typo: "helo" instead of "hello"
        new: "    console.log(\"hello world\");".to_string(),
        post_context: "}".to_string(),
    }];

    let file_content = "function greet() {\n    console.log(\"hello\");\n}"; // Correct spelling
    let diff = fuzzy_match_v4a_diffs("test.js", &hunks, None, file_content);

    // Should match due to high similarity (> 0.9)
    assert_eq!(deltas(&diff).len(), 1);
    assert_eq!(deltas(&diff)[0].replacement_line_range, 2..3);
}

#[test]
fn test_v4a_no_match() {
    let hunks = vec![V4AHunk {
        change_context: vec![],
        pre_context: "fn does_not_exist() {".to_string(),
        old: "    unrelated_code();".to_string(),
        new: "    new_code();".to_string(),
        post_context: "}".to_string(),
    }];

    let file_content = "fn main() {\n    println!(\"Hello\");\n}";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert!(deltas(&diff).is_empty());
    assert!(diff.failures.is_some());
    let failures = diff.failures.unwrap();
    assert_eq!(failures.fuzzy_match_failures, 1);
}

#[test]
fn test_v4a_noop_diff() {
    let hunks = vec![V4AHunk {
        change_context: vec![],
        pre_context: "fn main() {".to_string(),
        old: "    println!(\"Hello\");".to_string(),
        new: "    println!(\"Hello\");".to_string(), // Same as old
        post_context: "}".to_string(),
    }];

    let file_content = "fn main() {\n    println!(\"Hello\");\n}";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert!(deltas(&diff).is_empty());
    assert!(diff.failures.is_some());
    let failures = diff.failures.unwrap();
    assert_eq!(failures.noop_deltas, 1);
}

#[test]
fn test_v4a_empty_context() {
    // Test with no pre or post context
    let hunks = vec![V4AHunk {
        change_context: vec![],
        pre_context: String::new(),
        old: "let x = 1;".to_string(),
        new: "let x = 2;".to_string(),
        post_context: String::new(),
    }];

    let file_content = "let x = 1;";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert_eq!(deltas(&diff).len(), 1);
    assert_eq!(
        deltas(&diff)[0],
        DiffDelta {
            replacement_line_range: 1..2,
            insertion: "let x = 2;".to_string(),
        }
    );
}

#[test]
fn test_v4a_multiline_old_content() {
    let hunks = vec![V4AHunk {
        change_context: vec![],
        pre_context: "fn calculate() {".to_string(),
        old: "    let a = 1;\n    let b = 2;\n    let sum = a + b;".to_string(),
        new: "    let sum = 3;".to_string(),
        post_context: "    println!(\"{}\", sum);\n}".to_string(),
    }];

    let file_content = "fn calculate() {\n    let a = 1;\n    let b = 2;\n    let sum = a + b;\n    println!(\"{}\", sum);\n}";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert_eq!(deltas(&diff).len(), 1);
    assert_eq!(
        deltas(&diff)[0],
        DiffDelta {
            replacement_line_range: 2..5,
            insertion: "    let sum = 3;".to_string(),
        }
    );
}

#[test]
fn test_v4a_multiple_hunks() {
    let hunks = vec![
        V4AHunk {
            change_context: vec![],
            pre_context: "fn first() {".to_string(),
            old: "    let x = 1;".to_string(),
            new: "    let x = 10;".to_string(),
            post_context: "}".to_string(),
        },
        V4AHunk {
            change_context: vec![],
            pre_context: "fn second() {".to_string(),
            old: "    let y = 2;".to_string(),
            new: "    let y = 20;".to_string(),
            post_context: "}".to_string(),
        },
    ];

    let file_content = "fn first() {\n    let x = 1;\n}\n\nfn second() {\n    let y = 2;\n}";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert_eq!(deltas(&diff).len(), 2);
    assert_eq!(deltas(&diff)[0].replacement_line_range, 2..3);
    assert_eq!(deltas(&diff)[0].insertion, "    let x = 10;");
    assert_eq!(deltas(&diff)[1].replacement_line_range, 6..7);
    assert_eq!(deltas(&diff)[1].insertion, "    let y = 20;");
}

#[test]
fn test_v4a_add_line_with_change_context_no_old() {
    // Test adding a new line using only change_context to locate position, without old content or pre-context
    let hunks = vec![V4AHunk {
        change_context: vec!["class MyClass {".to_string()],
        pre_context: "".to_string(),
        old: "".to_string(),
        new: "    fn new_method() {\n        return 2;\n    }".to_string(),
        post_context: "    fn existing_method() {".to_string(),
    }];

    let file_content = "class MyClass {\n    fn existing_method() {\n        return 1;\n    }\n}";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert_eq!(deltas(&diff).len(), 1);
    // The insertion should happen after the change_context line (line 1)
    assert_eq!(deltas(&diff)[0].replacement_line_range, 2..2);
    assert_eq!(
        deltas(&diff)[0].insertion,
        "    fn new_method() {\n        return 2;\n    }"
    );
}

#[test]
fn test_v4a_add_line_at_start_of_file() {
    // Test adding a line at the very start of a file
    let hunks = vec![V4AHunk {
        change_context: vec![],
        pre_context: "".to_string(), // No pre-context - start of file
        old: "".to_string(),         // No old content
        new: "// New header comment".to_string(),
        post_context: "fn main() {".to_string(),
    }];

    let file_content = "fn main() {\n    println!(\"Hello\");\n}";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert_eq!(deltas(&diff).len(), 1);
    // Should insert at the beginning (line range 1..1 means before line 1)
    assert_eq!(deltas(&diff)[0].replacement_line_range, 1..1);
    assert_eq!(deltas(&diff)[0].insertion, "// New header comment");
}

#[test]
fn test_v4a_add_line_at_end_of_file() {
    // Test adding a line at the very end of a file
    let hunks = vec![V4AHunk {
        change_context: vec![],
        pre_context: "fn main() {\n    println!(\"Hello\");\n}".to_string(),
        old: "".to_string(), // No old content
        new: "\n// Footer comment".to_string(),
        post_context: "".to_string(), // No post-context - end of file
    }];

    let file_content = "fn main() {\n    println!(\"Hello\");\n}";
    let diff = fuzzy_match_v4a_diffs("test.rs", &hunks, None, file_content);

    assert_eq!(deltas(&diff).len(), 1);
    // Should insert after the last line (line 3), so insertion point is 4..4
    assert_eq!(deltas(&diff)[0].replacement_line_range, 4..4);
    assert_eq!(deltas(&diff)[0].insertion, "\n// Footer comment");
}

#[test]
fn test_partial_last_line_in_search_preserves_suffix() {
    // When a search string ends with a partial line (e.g. "let x = 1;\nlet x" where
    // "let x" is only a prefix of the actual file line "let x = 2;"), the Jaro-Winkler
    // fuzzy matcher matches via whole-line windows. The unmatched suffix (" = 2;") from
    // the file's last matched line must be preserved in the insertion.
    let file_content = "func foo() {\nlet x = 1;\nlet x = 2;\n}";

    let diffs = [SearchAndReplace {
        search: "let x = 1;\nlet x".to_string(),
        replace: "let y = 1;\nlet x".to_string(),
    }];

    let (deltas, _failures) = fuzzy_match_file_diffs(&diffs, file_content);

    assert_eq!(deltas.len(), 1, "Expected one matched delta");
    assert_eq!(deltas[0].replacement_line_range, 2..4);
    // The insertion has the unmatched suffix " = 2;" appended to the last line.
    assert_eq!(deltas[0].insertion, "let y = 1;\nlet x = 2;");

    // Verify applying the delta produces correct output (no data loss).
    let file_lines: Vec<&str> = file_content.lines().collect();
    let range = &deltas[0].replacement_line_range;
    let mut result = String::new();
    for line in &file_lines[..range.start - 1] {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str(&deltas[0].insertion);
    result.push('\n');
    for line in &file_lines[range.end - 1..] {
        result.push_str(line);
        result.push('\n');
    }
    assert_eq!(result, "func foo() {\nlet y = 1;\nlet x = 2;\n}\n");
}

#[test]
fn test_partial_last_line_in_multiline_replacement_preserves_suffix() {
    // This mirrors a model edit that deletes middle lines while leaving the final line as partial
    // trailing context. The final line should remain a no-op after suffix preservation.
    let file_content = "\
mod proxy;
pub fn run_daemon() -> anyhow::Result<()> {
    // Logging is now handled by init_common (log_destination: File).

    // socket_path: ~/.warp[-channel]/remote-server/server.sock
    //   The Unix domain socket the daemon binds on.
}
";

    let diffs = [SearchAndReplace {
        search: "\
2|pub fn run_daemon() -> anyhow::Result<()> {
3|    // Logging is now handled by init_common (log_destination: File).
4|
5|    // socket_path:"
            .to_string(),
        replace: "\
pub fn run_daemon() -> anyhow::Result<()> {
    // socket_path:"
            .to_string(),
    }];

    let (deltas, _failures) = fuzzy_match_file_diffs(&diffs, file_content);

    assert_eq!(deltas.len(), 1, "Expected one matched delta");
    assert_eq!(deltas[0].replacement_line_range, 2..6);
    assert_eq!(
        deltas[0].insertion,
        "pub fn run_daemon() -> anyhow::Result<()> {\n    // socket_path: ~/.warp[-channel]/remote-server/server.sock"
    );

    let file_lines: Vec<&str> = file_content.lines().collect();
    let range = &deltas[0].replacement_line_range;
    let mut result = String::new();
    for line in &file_lines[..range.start - 1] {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str(&deltas[0].insertion);
    result.push('\n');
    for line in &file_lines[range.end - 1..] {
        result.push_str(line);
        result.push('\n');
    }
    assert_eq!(
        result,
        "\
mod proxy;
pub fn run_daemon() -> anyhow::Result<()> {
    // socket_path: ~/.warp[-channel]/remote-server/server.sock
    //   The Unix domain socket the daemon binds on.
}
"
    );
}
#[test]
fn test_search_and_replace_accommodates_none() {
    let parsed_diff = ParsedDiff::StrReplaceEdit {
        file: None,
        search: None,
        replace: None,
    };
    let search_and_replace: Result<SearchAndReplace, ()> = parsed_diff.try_into();
    assert_eq!(Err(()), search_and_replace);

    let parsed_diff = ParsedDiff::StrReplaceEdit {
        file: None,
        search: Some("search".into()),
        replace: None,
    };
    assert_eq!(
        Ok(SearchAndReplace {
            search: "search".into(),
            replace: String::new()
        }),
        parsed_diff.try_into()
    );

    let parsed_diff = ParsedDiff::StrReplaceEdit {
        file: None,
        search: None,
        replace: Some("replace".into()),
    };
    assert_eq!(
        Ok(SearchAndReplace {
            search: String::new(),
            replace: "replace".into()
        }),
        parsed_diff.try_into()
    );
}

/// Test that if a search/replace pair is not a noop, but the overall effect is a noop when applied
/// to the file contents, we skip the diff.
#[test]
fn test_replace_matches_file_content() {
    let diffs = [SearchAndReplace {
        search: "1|Hey, there".to_string(),
        replace: "Hi, there".to_string(),
    }];
    let (deltas, errors) = fuzzy_match_file_diffs(&diffs, "Hi, there\nGoodbye, world");
    assert!(deltas.is_empty());
    assert_eq!(errors.noop_deltas, 1);
}

#[test]
fn test_search_range_greater_than_file_length() {
    // This should not panic!
    let r = match_diff(
        "hey\nthere",
        Some(14..15),
        &["hey", "there"],
        1f64,
        MakeExactMatch,
    );

    assert_eq!(r, Some(1..3));
}

#[test]
fn test_custom_lines() {
    assert_eq!(lines("").collect_vec(), vec![""]);
    assert_eq!(lines("foobar").collect_vec(), vec!["foobar"]);
    assert_eq!(lines("foo\nbar").collect_vec(), vec!["foo", "bar"]);
    assert_eq!(lines("foo\nbar\n").collect_vec(), vec!["foo", "bar"]);
}

/// Regression test for WARP-CLIENT-DEV-NYY: panic "Invalid edit range 4042..3982".
///
/// Reproduces the crash from MAA conversation d71bf84b (request b621adb3).
/// Two V4A hunks target the same region: a large deletion whose matched range
/// subsumes a nearby single-line edit. Without `deduplicate_overlapping_deltas`,
/// both deltas survive and `Buffer::edit` panics on the overlapping ranges.
#[test]
fn test_v4a_maa_crash_d71bf84b_no_overlapping_deltas() {
    // File content where hunk A (deletion) and hunk B (delegate tweak) both
    // match, and hunk A's matched range fully contains hunk B's.
    // The `ActiveMicButtonTheme.background` line that hunk B targets sits
    // inside `DefaultWeightAgentInputButtonTheme`'s impl, so hunk A's
    // deletion (which covers the whole impl) subsumes hunk B.
    let file_content = "\
        }\n\
    }\n\
}\n\
\n\
struct DefaultWeightAgentInputButtonTheme;\n\
\n\
impl ActionButtonTheme for DefaultWeightAgentInputButtonTheme {\n\
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {\n\
        AgentInputButtonTheme.background(hovered, appearance)\n\
    }\n\
\n\
    fn text_color(\n\
        &self,\n\
        hovered: bool,\n\
        background: Option<Fill>,\n\
        appearance: &Appearance,\n\
    ) -> ColorU {\n\
        AgentInputButtonTheme.text_color(hovered, background, appearance)\n\
    }\n\
\n\
    fn border(&self, appearance: &Appearance) -> Option<ColorU> {\n\
        AgentInputButtonTheme.border(appearance)\n\
    }\n\
\n\
    fn should_opt_out_of_contrast_adjustment(&self) -> bool {\n\
        true\n\
    }\n\
}";

    let hunks = vec![
        // Hunk A: delete the entire DefaultWeightAgentInputButtonTheme block.
        V4AHunk {
            change_context: vec![],
            pre_context: "        }\n    }\n}".to_string(),
            old: "\nstruct DefaultWeightAgentInputButtonTheme;\n\nimpl ActionButtonTheme for DefaultWeightAgentInputButtonTheme {\n    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {\n        AgentInputButtonTheme.background(hovered, appearance)\n    }\n\n    fn text_color(\n        &self,\n        hovered: bool,\n        background: Option<Fill>,\n        appearance: &Appearance,\n    ) -> ColorU {\n        AgentInputButtonTheme.text_color(hovered, background, appearance)\n    }\n\n    fn border(&self, appearance: &Appearance) -> Option<ColorU> {\n        AgentInputButtonTheme.border(appearance)\n    }\n\n    fn should_opt_out_of_contrast_adjustment(&self) -> bool {\n        true\n    }\n}".to_string(),
            new: String::new(),
            post_context: String::new(),
        },
        // Hunk B: tweak a delegate call inside the same region hunk A deletes.
        // Its preContext + old match a line inside hunk A's range, so it
        // produces a delta whose range overlaps with hunk A's.
        V4AHunk {
            change_context: vec![],
            pre_context: "impl ActionButtonTheme for DefaultWeightAgentInputButtonTheme {\n    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {".to_string(),
            old: "        AgentInputButtonTheme.background(hovered, appearance)".to_string(),
            new: "        AgentInputButtonTheme::default().background(hovered, appearance)".to_string(),
            post_context: "    }".to_string(),
        },
    ];

    let diff = fuzzy_match_v4a_diffs("mod.rs", &hunks, None, file_content);
    let deltas = deltas(&diff);

    // Hunk B's matched range is inside hunk A's, so deduplication must drop it.
    // Only hunk A's delta (the deletion) should survive.
    assert_eq!(
        deltas.len(),
        1,
        "Expected 1 delta (subsumed hunk should be dropped), got {}: {:?}",
        deltas.len(),
        deltas
            .iter()
            .map(|d| &d.replacement_line_range)
            .collect::<Vec<_>>(),
    );
    assert!(
        deltas[0].insertion.is_empty(),
        "The surviving delta should be the deletion"
    );
}
