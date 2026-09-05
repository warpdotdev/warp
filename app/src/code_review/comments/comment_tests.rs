use super::LineDiffContent;

#[test]
fn original_text_strips_addition_prefix() {
    let content = LineDiffContent::from_content("+added line");
    assert_eq!(content.original_text(), "added line");
}

#[test]
fn original_text_strips_deletion_prefix() {
    let content = LineDiffContent::from_content("-deleted line");
    assert_eq!(content.original_text(), "deleted line");
}

#[test]
fn original_text_preserves_markdown_list_dash_in_addition() {
    let content = LineDiffContent::from_content("+- list item");
    assert_eq!(content.original_text(), "- list item");
}

#[test]
fn original_text_preserves_dash_only_content_in_addition() {
    let content = LineDiffContent::from_content("+-");
    assert_eq!(content.original_text(), "-");
}

#[test]
fn original_text_strips_only_one_leading_plus() {
    let content = LineDiffContent::from_content("++text");
    assert_eq!(content.original_text(), "+text");
}

#[test]
fn original_text_strips_only_one_leading_minus() {
    let content = LineDiffContent::from_content("--text");
    assert_eq!(content.original_text(), "-text");
}

#[test]
fn original_text_preserves_space_prefixed_content() {
    let content = LineDiffContent {
        content: " - context list item".to_string(),
        ..Default::default()
    };
    assert_eq!(content.original_text(), " - context list item");
}

#[test]
fn original_text_strips_trailing_newline() {
    let content = LineDiffContent::from_content("+added line\n");
    assert_eq!(content.original_text(), "added line");
}

#[test]
fn original_text_handles_empty_content() {
    let content = LineDiffContent::from_content("");
    assert_eq!(content.original_text(), "");
}

#[test]
fn original_text_handles_plain_text_without_prefix() {
    let content = LineDiffContent {
        content: "no prefix".to_string(),
        ..Default::default()
    };
    assert_eq!(content.original_text(), "no prefix");
}

#[test]
fn imported_original_text_strips_context_space_prefix() {
    let content = LineDiffContent {
        content: " line 2".to_string(),
        ..Default::default()
    };
    assert_eq!(content.imported_original_text(), "line 2");
}

#[test]
fn imported_original_text_strips_only_one_leading_space() {
    let content = LineDiffContent {
        content: "  indented".to_string(),
        ..Default::default()
    };
    assert_eq!(content.imported_original_text(), " indented");
}

#[test]
fn imported_original_text_strips_addition_and_deletion_markers() {
    assert_eq!(
        LineDiffContent::from_content("+add").imported_original_text(),
        "add"
    );
    assert_eq!(
        LineDiffContent::from_content("-del").imported_original_text(),
        "del"
    );
}

#[test]
fn imported_original_text_handles_blank_context_line() {
    let content = LineDiffContent {
        content: " ".to_string(),
        ..Default::default()
    };
    assert_eq!(content.imported_original_text(), "");
}

#[test]
fn imported_original_text_strips_only_one_marker_for_markdown_list() {
    // Addition of a markdown list item: `+- list`. Only the diff `+` is stripped.
    let content = LineDiffContent::from_content("+- list");
    assert_eq!(content.imported_original_text(), "- list");
}

#[test]
fn test_api_review_comment_line_range_deleted_file() {
    use std::path::PathBuf;

    use chrono::Local;

    use super::*;

    // Simulated comment on a removed line in a deleted file (line_range is empty 0..0)
    let comment = AttachedReviewComment {
        id: CommentId::new(),
        content: "Test".to_string(),
        target: AttachedReviewCommentTarget::Line {
            absolute_file_path: LocalOrRemotePath::Local(PathBuf::from("/repo/file.txt")),
            content: LineDiffContent {
                content: "-deleted line".to_string(),
                lines_added: LineCount::from(0),
                lines_removed: LineCount::from(1),
            },
            line: EditorLineLocation::Removed {
                line_number: LineCount::from(0),
                line_range: LineCount::from(0)..LineCount::from(0),
                index: 0,
            },
        },
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    };

    let api_comment = api::ReviewComment::from(comment);
    let target = api_comment.comment_target.unwrap();
    if let api::review_comment::CommentTarget::CommentedLine(hunk) = target {
        let range = hunk.line_range.unwrap();
        // For a deleted file, start and end must both be 0 (0-length range)
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 0);
    } else {
        panic!("Expected CommentedLine target");
    }
}

#[test]
fn test_api_review_comment_line_range_normal_file() {
    use std::path::PathBuf;

    use chrono::Local;

    use super::*;

    // Simulated comment on a removed line in a normal file (hunk has line range 5..10)
    let comment = AttachedReviewComment {
        id: CommentId::new(),
        content: "Test".to_string(),
        target: AttachedReviewCommentTarget::Line {
            absolute_file_path: LocalOrRemotePath::Local(PathBuf::from("/repo/file.txt")),
            content: LineDiffContent {
                content: "-deleted line".to_string(),
                lines_added: LineCount::from(0),
                lines_removed: LineCount::from(1),
            },
            line: EditorLineLocation::Removed {
                line_number: LineCount::from(5),
                line_range: LineCount::from(5)..LineCount::from(10),
                index: 0,
            },
        },
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    };

    let api_comment = api::ReviewComment::from(comment);
    let target = api_comment.comment_target.unwrap();
    if let api::review_comment::CommentTarget::CommentedLine(hunk) = target {
        let range = hunk.line_range.unwrap();
        // For normal files, we ensure a minimum span of 1 line
        assert_eq!(range.start, 5);
        assert_eq!(range.end, 6);
    } else {
        panic!("Expected CommentedLine target");
    }
}

#[test]
fn test_api_review_comment_line_range_multiline() {
    use std::path::PathBuf;

    use chrono::Local;

    use super::*;

    // Simulated comment on a multi-line hunk (lines_added: 3)
    let comment = AttachedReviewComment {
        id: CommentId::new(),
        content: "Test".to_string(),
        target: AttachedReviewCommentTarget::Line {
            absolute_file_path: LocalOrRemotePath::Local(PathBuf::from("/repo/file.txt")),
            content: LineDiffContent {
                content: "+added line 1\n+added line 2\n+added line 3".to_string(),
                lines_added: LineCount::from(3),
                lines_removed: LineCount::from(0),
            },
            line: EditorLineLocation::Current {
                line_number: LineCount::from(10),
                line_range: LineCount::from(10)..LineCount::from(13),
            },
        },
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    };

    let api_comment = api::ReviewComment::from(comment);
    let target = api_comment.comment_target.unwrap();
    if let api::review_comment::CommentTarget::CommentedLine(hunk) = target {
        let range = hunk.line_range.unwrap();
        assert_eq!(range.start, 10);
        assert_eq!(range.end, 13);
    } else {
        panic!("Expected CommentedLine target");
    }
}

#[test]
fn test_api_review_comment_line_range_pure_deletion() {
    use std::path::PathBuf;

    use chrono::Local;

    use super::*;

    // Simulated comment on a pure deletion (line_range is empty 5..5, but line_number is 5)
    let comment = AttachedReviewComment {
        id: CommentId::new(),
        content: "Test".to_string(),
        target: AttachedReviewCommentTarget::Line {
            absolute_file_path: LocalOrRemotePath::Local(PathBuf::from("/repo/file.txt")),
            content: LineDiffContent {
                content: "-deleted line".to_string(),
                lines_added: LineCount::from(0),
                lines_removed: LineCount::from(1),
            },
            line: EditorLineLocation::Removed {
                line_number: LineCount::from(5),
                line_range: LineCount::from(5)..LineCount::from(5),
                index: 0,
            },
        },
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    };

    let api_comment = api::ReviewComment::from(comment);
    let target = api_comment.comment_target.unwrap();
    if let api::review_comment::CommentTarget::CommentedLine(hunk) = target {
        let range = hunk.line_range.unwrap();
        // Since it's a normal file (line_number != 0), we ensure a minimum span of 1 line
        assert_eq!(range.start, 5);
        assert_eq!(range.end, 6);
    } else {
        panic!("Expected CommentedLine target");
    }
}
