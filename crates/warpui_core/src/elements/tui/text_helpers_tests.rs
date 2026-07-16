use super::{text_width, truncate_with_ellipsis};

#[test]
fn truncates_by_display_columns_without_splitting_graphemes() {
    assert_eq!(truncate_with_ellipsis("infrastructure", 8), "infra...");
    assert_eq!(truncate_with_ellipsis("abcdef", 2), "..");
    assert_eq!(truncate_with_ellipsis("界界界界", 7), "界界...");
    assert_eq!(truncate_with_ellipsis("e\u{301}clair", 5), "e\u{301}c...");
    assert_eq!(text_width("界界..."), 7);
}
