use ai::agent::action::CommentSide;

use super::{DiffHunkParseError, parse_diff_hunk};

#[test]
fn test_parse_preserves_whitespace() {
    let diff_hunk = "@@ -1,2 +1,3 @@
 first line
+    indented line
 last line";

    // Diff line index 2 is the added line with indentation
    let result = parse_diff_hunk(diff_hunk, 2, None);
    assert!(result.is_ok());
    let (_, content) = result.unwrap();
    assert_eq!(content.content, "+    indented line");
}

#[test]
fn test_parse_empty_hunk() {
    let result = parse_diff_hunk("", 1, None);
    assert!(matches!(result, Err(DiffHunkParseError::EmptyHunk)));
}

#[test]
fn test_parse_malformed_header() {
    // Header doesn't have valid format
    let diff_hunk = "@@ invalid @@";
    let result = parse_diff_hunk(diff_hunk, 1, None);
    assert!(matches!(result, Err(DiffHunkParseError::InvalidHeader(_))));
}

#[test]
fn test_parse_missing_header() {
    // Content without a valid header line
    let diff_hunk = " line1\n+line2\n line3";
    let result = parse_diff_hunk(diff_hunk, 1, None);
    assert!(matches!(result, Err(DiffHunkParseError::InvalidHeader(_))));
}

#[test]
fn test_parse_hunk_with_two_headers() {
    // Diff with an unexpected second header line
    let diff_hunk = "@@ -1,3 +1,4 @@\n line1\n@@ -5,2 +5,2 @@\n line2";
    let result = parse_diff_hunk(diff_hunk, 2, None);
    assert!(matches!(
        result,
        Err(DiffHunkParseError::UnexpectedHunkHeader { line_index: 2 })
    ));
}

// ---------------------------------------------------------------------------
// Regression: markdown list items in diff hunks (PR #23626)
// ---------------------------------------------------------------------------

/// Regression test for diff hunks from PR #23626 where the commented line is a
/// markdown list item (`+- ...`). The leading `-` is part of the content, not a
/// diff deletion marker.
#[test]
fn test_parse_markdown_list_item_in_pure_addition_hunk() {
    // Trimmed hunk shape from PR #23626 comment 2997753460
    let diff_hunk = "@@ -0,0 +16,1 @@\n+- `specs/<issue-number>/TECH.md`";

    let result = parse_diff_hunk(diff_hunk, 16, Some(CommentSide::Right));
    assert!(result.is_ok());
    let (_, content) = result.unwrap();
    assert_eq!(content.content, "+- `specs/<issue-number>/TECH.md`");
    assert_eq!(content.original_text(), "- `specs/<issue-number>/TECH.md`");
}

/// Same scenario but with more surrounding context, matching the full diff hunk
/// shape from the GitHub API for PR #23626.
#[test]
fn test_parse_markdown_list_item_in_full_addition_hunk() {
    let diff_hunk = "@@ -0,0 +1,116 @@\n\
+---\n\
+name: write-tech-spec\n\
+description: desc\n\
+---\n\
+\n\
+# write-tech-spec\n\
+\n\
+Write a spec.\n\
+\n\
+## Overview\n\
+\n\
+The tech spec overview.\n\
+\n\
+Write specs into source control under:\n\
+\n\
+- `specs/<issue-number>/TECH.md`";

    let result = parse_diff_hunk(diff_hunk, 16, Some(CommentSide::Right));
    assert!(result.is_ok());
    let (_, content) = result.unwrap();
    assert_eq!(content.content, "+- `specs/<issue-number>/TECH.md`");
    assert_eq!(content.original_text(), "- `specs/<issue-number>/TECH.md`");
}

/// Diff line whose content starts with `+` after the diff `+` prefix.
#[test]
fn test_parse_addition_with_leading_plus_in_content() {
    let diff_hunk = "@@ -0,0 +1,1 @@\n++positive value";

    let result = parse_diff_hunk(diff_hunk, 1, Some(CommentSide::Right));
    assert!(result.is_ok());
    let (_, content) = result.unwrap();
    assert_eq!(content.content, "++positive value");
    assert_eq!(content.original_text(), "+positive value");
}

/// Diff line whose content starts with `-` after the diff `-` prefix (deletion
/// of a markdown list item).
#[test]
fn test_parse_deletion_with_leading_dash_in_content() {
    let diff_hunk = "@@ -1,1 +1,0 @@\n-- list item";

    let result = parse_diff_hunk(diff_hunk, 1, Some(CommentSide::Left));
    assert!(result.is_ok());
    let (_, content) = result.unwrap();
    assert_eq!(content.content, "-- list item");
    assert_eq!(content.original_text(), "- list item");
}

/// Context line with content that starts with `-`.
#[test]
fn test_parse_context_line_starting_with_dash() {
    let diff_hunk = "@@ -1,3 +1,4 @@\n - list item\n+added\n second\n third";

    let result = parse_diff_hunk(diff_hunk, 1, Some(CommentSide::Right));
    assert!(result.is_ok());
    let (_, content) = result.unwrap();
    assert_eq!(content.content, " - list item");
}

/// Context line with content that starts with `@`.
#[test]
fn test_parse_context_line_starting_with_at_sign() {
    let diff_hunk = "@@ -1,2 +1,3 @@\n @decorator\n+added\n next";

    let result = parse_diff_hunk(diff_hunk, 1, Some(CommentSide::Right));
    assert!(result.is_ok());
    let (_, content) = result.unwrap();
    assert_eq!(content.content, " @decorator");
}
