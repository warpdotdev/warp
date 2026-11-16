/// Truncate text from the end with ellipsis if it exceeds max_length.
/// Properly handles UTF-8 character boundaries to avoid panics.
pub fn truncate_from_end(text: &str, max_length: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_length {
        text.to_string()
    } else {
        let chars_to_take = max_length.saturating_sub(1);
        let truncated: String = text.chars().take(chars_to_take).collect();
        format!("{truncated}…")
    }
}

/// Truncate text from the beginning with ellipsis if it exceeds max_length.
/// Properly handles UTF-8 character boundaries to avoid panics.
pub fn truncate_from_beginning(text: &str, max_length: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_length {
        text.to_string()
    } else {
        let chars_to_take = max_length.saturating_sub(1);
        let truncated: String = text.chars().skip(char_count - chars_to_take).collect();
        format!("…{truncated}")
    }
}
