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
fn original_text_preserves_leading_dash_for_unmodified_line() {
    let content = LineDiffContent {
        content: "- context list item".to_string(),
        ..Default::default()
    };
    assert_eq!(content.original_text(), "- context list item");
}

#[test]
fn original_text_preserves_leading_plus_for_unmodified_line() {
    let content = LineDiffContent {
        content: "+ context value".to_string(),
        ..Default::default()
    };
    assert_eq!(content.original_text(), "+ context value");
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
