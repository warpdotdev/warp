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
