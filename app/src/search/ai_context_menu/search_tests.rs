use super::is_valid_search_query;

#[test]
fn test_query_with_newline_or_double_space_is_invalid() {
    assert!(!is_valid_search_query(
        /*is_navigation=*/ false, /*prev_query=*/ "", /*query=*/ "foo\nbar"
    ));
    assert!(!is_valid_search_query(
        /*is_navigation=*/ false, /*prev_query=*/ "", /*query=*/ "foo  bar"
    ));
    assert!(is_valid_search_query(
        /*is_navigation=*/ false, /*prev_query=*/ "", /*query=*/ "foo bar"
    ));
}

#[test]
fn test_navigation_counts_newly_skipped_spaces() {
    // Jumping over a single space keeps the menu open.
    assert!(is_valid_search_query(
        /*is_navigation=*/ true, /*prev_query=*/ "foo", /*query=*/ "foo bar"
    ));
    // Jumping over two spaces means the cursor moved too far, so the menu closes.
    assert!(!is_valid_search_query(
        /*is_navigation=*/ true,
        /*prev_query=*/ "foo",
        /*query=*/ "foo bar baz"
    ));
}

#[test]
fn test_navigation_skips_by_char_count_not_byte_length() {
    // The previous query is 2 chars but 6 bytes. Skipping by byte length would swallow the rest
    // of the query and never see the newly skipped spaces.
    assert!(!is_valid_search_query(
        /*is_navigation=*/ true,
        /*prev_query=*/ "你好",
        /*query=*/ "你好 bar baz",
    ));
    assert!(is_valid_search_query(
        /*is_navigation=*/ true,
        /*prev_query=*/ "你好",
        /*query=*/ "你好 bar",
    ));
}
