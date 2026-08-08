use super::is_valid_search_query;

#[test]
fn navigation_heuristic_counts_chars_not_bytes() {
    let prev_query = "→";
    let query = "→  ls";

    assert!(!is_valid_search_query(true, prev_query, query));
}
