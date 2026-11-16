use super::*;

#[test]
fn test_contains_wildcards() {
    assert!(contains_wildcards("*.rs"));
    assert!(contains_wildcards("file?.txt"));
    assert!(contains_wildcards("*test*"));
    assert!(!contains_wildcards("normal_file.rs"));
    assert!(!contains_wildcards("test"));
}

#[test]
fn test_debug_ui_pattern() {
    // Debug the ui/* pattern step by step
    let text = "/src/ui/button.rs";
    let pattern = "ui/*";

    // Test if it should hit the prefix fast path
    let is_prefix_pattern = pattern.ends_with('*')
        && !pattern[..pattern.len() - 1].contains('*')
        && !pattern[..pattern.len() - 1].contains('?');
    assert!(is_prefix_pattern);

    let prefix = &pattern[..pattern.len() - 1];
    assert_eq!(prefix, "ui/");
    assert!(text.contains(prefix)); // Changed from starts_with to contains since text starts with "/src"

    let result = match_wildcard_pattern(text, pattern);
    assert!(result.is_some());
}

#[test]
fn test_wildcard_pattern_star() {
    let result = match_wildcard_pattern("button.rs", "*.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_pattern_question_mark() {
    let result = match_wildcard_pattern("button.rs", "butto?.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_pattern_no_match() {
    let result = match_wildcard_pattern("button.rs", "*.py");
    assert!(result.is_none());
}

#[test]
fn test_wildcard_pattern_case_insensitive() {
    let result = match_wildcard_pattern_case_insensitive("Button.RS", "*.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_pattern_complex() {
    let result = match_wildcard_pattern("/src/ui/button.rs", "*/ui/*.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_pattern_exact_match() {
    let result = match_wildcard_pattern("/src/components/button.rs", "/src/components/button.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_escape_special_chars() {
    // Test that regex special characters are properly escaped
    let result = match_wildcard_pattern("test.file", "test.file");
    assert!(result.is_some());

    let result = match_wildcard_pattern("testXfile", "test.file");
    assert!(result.is_none()); // Should not match because . is literal, not wildcard
}

#[test]
fn test_fuzzy_match_still_works() {
    // Ensure regular fuzzy matching still works
    let result = match_indices_case_insensitive("button.rs", "btn");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_ui_star_pattern() {
    // Test the specific ui/* pattern that's not working
    let result = match_wildcard_pattern_case_insensitive("/src/ui/button.rs", "ui/*");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_prefix_star_patterns() {
    // Test various prefix/* patterns
    let result1 = match_wildcard_pattern("src/ui/button.rs", "src/*");
    assert!(result1.is_some());

    let result2 = match_wildcard_pattern("ui/button.rs", "ui/*");
    assert!(result2.is_some());

    let result3 = match_wildcard_pattern("/src/ui/button.rs", "ui/*");
    // This should now match because we do substring matching
    assert!(result3.is_some());
}

#[test]
fn test_partial_suffix_patterns() {
    // Test partial suffix patterns for progressive typing
    let result1 = match_wildcard_pattern("button.rs", "*.r");
    assert!(result1.is_some());

    let result2 = match_wildcard_pattern("component.tsx", "*.t");
    assert!(result2.is_some());

    let result3 = match_wildcard_pattern("test.js", "*.");
    assert!(result3.is_some());

    // Should not match if there's no matching suffix
    let result4 = match_wildcard_pattern("button.rs", "*.py");
    assert!(result4.is_none());
}

#[test]
fn test_ui_star_partial_patterns() {
    // Test the specific cases mentioned in the issue
    let path = "/src/ui/button.rs";

    let result1 = match_wildcard_pattern_case_insensitive(path, "ui/*.rs");
    assert!(result1.is_some());

    let result2 = match_wildcard_pattern_case_insensitive(path, "ui/*.r");
    assert!(result2.is_some());

    let result3 = match_wildcard_pattern_case_insensitive(path, "ui/*.");
    assert!(result3.is_some());
}

use crate::{
    match_indices, match_indices_case_insensitive, match_indices_case_insensitive_ignore_spaces,
};

#[test]
fn test_simple_fuzzy_match_indices() {
    let text = "axbycz";
    let query = "abc";

    let result = match_indices(text, query).unwrap();
    assert_eq!(result.matched_indices, vec![0, 2, 4]);
}

#[test]
fn test_multibyte_char_fuzzy_match_indices() {
    let text = "ay東cz";
    let query = "a東c";

    let result = match_indices(text, query).unwrap();
    assert_eq!(result.matched_indices, vec![0, 2, 3]);
}

#[test]
fn test_empty_query_fuzzy_match_search() {
    let text = "abcdef";
    let query = "";

    let result = match_indices(text, query).unwrap();
    assert_eq!(result.matched_indices.len(), 0);
}

#[test]
fn test_no_match_query_fuzzy_match_search() {
    let text = "abcdef";
    let query = "ghijk";

    let result = match_indices(text, query);
    assert!(result.is_none());
}

#[test]
fn test_case_insensitive_fuzzy_match_search() {
    let text = "AXBYcz";
    let query = "abC";

    let result = match_indices_case_insensitive(text, query).unwrap();
    assert_eq!(result.matched_indices, vec![0, 2, 4]);
}

// Tests for match_indices_case_insensitive_ignore_spaces

#[test]
fn test_ignore_spaces_basic_functionality() {
    let text = "myFunction";
    let query_with_spaces = "my func";
    let query_without_spaces = "myfunc";

    // Both queries should produce the same result
    let result_with_spaces = match_indices_case_insensitive_ignore_spaces(text, query_with_spaces);
    let result_without_spaces = match_indices_case_insensitive(text, query_without_spaces);

    assert!(result_with_spaces.is_some());
    assert!(result_without_spaces.is_some());

    let with_spaces = result_with_spaces.unwrap();
    let without_spaces = result_without_spaces.unwrap();

    assert_eq!(with_spaces.matched_indices, without_spaces.matched_indices);
    assert_eq!(with_spaces.score, without_spaces.score);
}

#[test]
fn test_ignore_spaces_symbol_matching() {
    let text = "myFunction";
    let query = "my func";

    let result = match_indices_case_insensitive_ignore_spaces(text, query).unwrap();
    // Should match "my" + "func" in "myFunction"
    assert_eq!(result.matched_indices, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn test_ignore_spaces_case_insensitive() {
    let text = "MyFunction";
    let query = "my FUNC";

    let result = match_indices_case_insensitive_ignore_spaces(text, query).unwrap();
    // Should match "My" + "Func" in "MyFunction" (case insensitive)
    assert_eq!(result.matched_indices, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn test_ignore_spaces_multiple_spaces() {
    let text = "calculateSum";
    let query = "calc   sum"; // Multiple spaces

    let result = match_indices_case_insensitive_ignore_spaces(text, query).unwrap();
    // Should match "calc" + "Sum" in "calculateSum"
    assert_eq!(result.matched_indices, vec![0, 1, 2, 3, 9, 10, 11]);
}

#[test]
fn test_ignore_spaces_leading_trailing_spaces() {
    let text = "myFunction";
    let query = "  my func  "; // Leading and trailing spaces

    let result = match_indices_case_insensitive_ignore_spaces(text, query).unwrap();
    // Should still match "my" + "func" in "myFunction"
    assert_eq!(result.matched_indices, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn test_ignore_spaces_tabs_and_newlines() {
    let text = "myFunction";
    let query = "my\tfunc\n"; // Tab and newline

    let result = match_indices_case_insensitive_ignore_spaces(text, query).unwrap();
    // Should match "my" + "func" in "myFunction"
    assert_eq!(result.matched_indices, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn test_ignore_spaces_only_spaces_query() {
    let text = "myFunction";
    let query = "   "; // Only spaces

    let result = match_indices_case_insensitive_ignore_spaces(text, query);
    // Should return None because query becomes empty after removing spaces
    assert!(result.is_none());
}

#[test]
fn test_ignore_spaces_empty_query() {
    let text = "myFunction";
    let query = "";

    let result = match_indices_case_insensitive_ignore_spaces(text, query);
    // Should return None because query is empty
    assert!(result.is_none());
}

#[test]
fn test_ignore_spaces_no_match() {
    let text = "myFunction";
    let query = "hello world";

    let result = match_indices_case_insensitive_ignore_spaces(text, query);
    // Should return None because "helloworld" doesn't match "myFunction"
    assert!(result.is_none());
}

#[test]
fn test_ignore_spaces_partial_match() {
    let text = "getUserName";
    let query = "get user";

    let result = match_indices_case_insensitive_ignore_spaces(text, query).unwrap();
    // Should match "get" + "User" in "getUserName"
    assert_eq!(result.matched_indices, vec![0, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn test_ignore_spaces_scattered_match() {
    let text = "myAwesomeFunction";
    let query = "my func";

    let result = match_indices_case_insensitive_ignore_spaces(text, query).unwrap();
    // Should match "my" + scattered "func" letters
    // This will depend on the fuzzy matching algorithm's behavior
    assert!(!result.matched_indices.is_empty());
    assert!(result.score > 0);
}

#[test]
fn test_ignore_spaces_exact_match_with_spaces() {
    let text = "myfunction";
    let query = "my function";

    // This won't be an exact match because the text doesn't contain spaces
    // But it should still match the characters
    let result = match_indices_case_insensitive_ignore_spaces(text, query).unwrap();
    assert!(!result.matched_indices.is_empty());
    assert!(result.score > 0);
}

#[test]
fn test_ignore_spaces_vs_regular_matching() {
    let text = "myFunction";
    let query_with_spaces = "my func";

    // Regular matching should fail
    let regular_result = match_indices_case_insensitive(text, query_with_spaces);
    assert!(regular_result.is_none());

    // Space-ignoring matching should succeed
    let space_ignoring_result =
        match_indices_case_insensitive_ignore_spaces(text, query_with_spaces);
    assert!(space_ignoring_result.is_some());
}

#[test]
fn test_ignore_spaces_single_word_query() {
    let text = "myFunction";
    let query = "function";

    // Single word query should work the same as regular matching
    let regular_result = match_indices_case_insensitive(text, query).unwrap();
    let space_ignoring_result = match_indices_case_insensitive_ignore_spaces(text, query).unwrap();

    assert_eq!(
        regular_result.matched_indices,
        space_ignoring_result.matched_indices
    );
    assert_eq!(regular_result.score, space_ignoring_result.score);
}
