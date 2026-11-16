const MAX_NEW_SPACES: usize = 2;

/// If this is ever false, we close the AI context menu.
pub fn is_valid_search_query(is_navigation: bool, prev_query: &str, query: &str) -> bool {
    if query.contains('\n') || query.contains("  ") {
        return false;
    }

    if is_navigation {
        // We need a simple heuristic to handle when somebody jumps to the end
        // of the line. Since spaces are valid characters, we only count
        // how many spaces the users likely jumped over between queries
        let new_chars = query.chars().skip(prev_query.len());
        return new_chars.filter(|c| *c == ' ').count() < MAX_NEW_SPACES;
    }
    true
}
