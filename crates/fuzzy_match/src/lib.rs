//! # Fuzzy Match
//!
//! This crate provides fuzzy matching capabilities with support for both traditional
//! fuzzy search and glob-style wildcard patterns optimized for file path searching.
//!
//! ## Features
//!
//! ### Traditional Fuzzy Matching
//! - [`match_indices`] - Smart case fuzzy matching
//! - [`match_indices_case_insensitive`] - Case-insensitive fuzzy matching
//!
//! ### Wildcard Pattern Matching
//! - [`match_wildcard_pattern`] - Glob-style patterns with `*` and `?` support
//! - [`match_wildcard_pattern_case_insensitive`] - Case-insensitive wildcard matching
//! - [`contains_wildcards`] - Check if a query contains wildcard characters
//!
//! ## Wildcard Pattern Examples
//!
//! ```rust
//! use fuzzy_match::{match_wildcard_pattern, match_wildcard_pattern_case_insensitive};
//!
//! // File extension matching
//! assert!(match_wildcard_pattern("button.rs", "*.rs").is_some());
//! assert!(match_wildcard_pattern("component.tsx", "*.tsx").is_some());
//!
//! // Progressive typing support
//! assert!(match_wildcard_pattern("button.rs", "*.r").is_some()); // Matches .rs, .rb, etc.
//! assert!(match_wildcard_pattern("test.js", "*.").is_some());   // Matches any extension start
//!
//! // Directory patterns
//! assert!(match_wildcard_pattern("/src/ui/button.rs", "ui/*").is_some());
//! assert!(match_wildcard_pattern("/src/components/button.rs", "src/*").is_some());
//!
//! // Complex patterns
//! assert!(match_wildcard_pattern("/src/ui/button.rs", "*/ui/*.rs").is_some());
//!
//! // Case insensitive matching
//! assert!(match_wildcard_pattern_case_insensitive("Button.RS", "*.rs").is_some());
//! assert!(match_wildcard_pattern_case_insensitive("/src/UI/button.rs", "ui/*").is_some());
//! ```
//!
//! ## Performance
//!
//! The wildcard matching is optimized for file search use cases:
//! - **Fast paths** for common patterns like `*.rs` and `src/*`
//! - **Substring matching** for file paths (patterns match anywhere in the path)
//! - **No regex compilation** - uses efficient recursive algorithms
//! - **Progressive typing** support for better user experience
//!
//! ## Wildcard Characters
//!
//! - `*` - Matches zero or more characters
//! - `?` - Matches exactly one character

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct FuzzyMatchResult {
    pub score: i64,
    /// matched_indices is a vector of char indices
    pub matched_indices: Vec<usize>,
}

impl FuzzyMatchResult {
    /// Constructs a dummy [`FuzzyMatchResult`] that represents an item that is unmatched. The item
    /// will have no matching indices and a score of 0.
    pub fn no_match() -> Self {
        Self {
            score: 0,
            matched_indices: vec![],
        }
    }
}

/// Performs a fuzzy matching algorithm on text and query strings.
/// If query contains no uppercase letters, then it will be insensitive to casing.
/// If query does contain uppercase letters, then it will be case sensitive.
/// Returns struct that contains the score of the match and a vector
/// of matching byte indices.
pub fn match_indices(text: &str, query: &str) -> Option<FuzzyMatchResult> {
    match_internal(text, query, SkimMatcherV2::default())
}

/// Performs a case insensitive fuzzy matching algorithm on text and query strings.
/// Returns struct that contains the score of the match and a vector
/// of matching byte indices.
pub fn match_indices_case_insensitive(text: &str, query: &str) -> Option<FuzzyMatchResult> {
    match_internal(text, query, SkimMatcherV2::default().ignore_case())
}

/// Performs a case insensitive fuzzy matching algorithm on text and query strings,
/// ignoring spaces in the query. This is useful for symbol name matching where
/// spaces in the search query should be ignored.
///
/// The function removes all spaces from the query before performing the match,
/// but returns indices that correspond to the original text.
///
/// # Examples
///
/// ```
/// use fuzzy_match::match_indices_case_insensitive_ignore_spaces;
///
/// // Query "my func" matches "myFunc" by ignoring spaces
/// let result = match_indices_case_insensitive_ignore_spaces("myFunction", "my func");
/// assert!(result.is_some());
/// ```
pub fn match_indices_case_insensitive_ignore_spaces(
    text: &str,
    query: &str,
) -> Option<FuzzyMatchResult> {
    // Remove all spaces from the query
    let query_no_spaces: String = query.chars().filter(|c| !c.is_whitespace()).collect();

    // If the query becomes empty after removing spaces, return no match
    if query_no_spaces.is_empty() {
        return None;
    }

    // Perform the fuzzy match with the space-stripped query
    match_internal(
        text,
        &query_no_spaces,
        SkimMatcherV2::default().ignore_case(),
    )
}

fn match_internal(text: &str, query: &str, matcher: SkimMatcherV2) -> Option<FuzzyMatchResult> {
    matcher
        // The fuzzy_indices API returns char indices, so we don't need to manually convert.
        .fuzzy_indices(text, query)
        .map(|(score, indices)| FuzzyMatchResult {
            score,
            matched_indices: indices,
        })
}

/// Checks if a query contains wildcard characters (* or ?).
///
/// Wildcard characters are:
/// - `*` matches zero or more characters
/// - `?` matches exactly one character
///
/// # Examples
///
/// ```
/// use fuzzy_match::contains_wildcards;
///
/// assert!(contains_wildcards("*.rs"));
/// assert!(contains_wildcards("file?.txt"));
/// assert!(contains_wildcards("*test*"));
/// assert!(!contains_wildcards("normal_file.rs"));
/// assert!(!contains_wildcards("test"));
/// ```
pub fn contains_wildcards(query: &str) -> bool {
    query.contains('*') || query.contains('?')
}

/// Performs pattern matching with wildcard support (* and ?) on text and query strings.
///
/// This function supports glob-style patterns and is optimized for file path matching.
/// It uses fast paths for common patterns and supports progressive typing.
///
/// # Wildcard characters
///
/// - `*` matches zero or more characters
/// - `?` matches exactly one character
///
/// # Features
///
/// - **Fast paths**: Optimized for common patterns like `*.rs`, `src/*`
/// - **Substring matching**: Patterns like `ui/*` match anywhere in the path
/// - **Progressive typing**: Partial patterns like `*.r` match `.rs`, `.rb`, etc.
/// - **Case sensitive**: Use [`match_wildcard_pattern_case_insensitive`] for case-insensitive matching
///
/// # Examples
///
/// ```
/// use fuzzy_match::match_wildcard_pattern;
///
/// // Suffix patterns
/// assert!(match_wildcard_pattern("button.rs", "*.rs").is_some());
/// assert!(match_wildcard_pattern("component.tsx", "*.tsx").is_some());
///
/// // Progressive typing support
/// assert!(match_wildcard_pattern("button.rs", "*.r").is_some());
/// assert!(match_wildcard_pattern("test.js", "*.").is_some());
///
/// // Prefix patterns
/// assert!(match_wildcard_pattern("src/components/button.rs", "src/*").is_some());
///
/// // Complex patterns
/// assert!(match_wildcard_pattern("/src/ui/button.rs", "*/ui/*.rs").is_some());
///
/// // Question mark patterns
/// assert!(match_wildcard_pattern("test1.rs", "test?.rs").is_some());
/// assert!(match_wildcard_pattern("test12.rs", "test?.rs").is_none());
///
/// // No match
/// assert!(match_wildcard_pattern("button.rs", "*.py").is_none());
/// ```
///
/// # Performance
///
/// This function is optimized for common file search patterns:
/// - Simple suffix patterns like `*.rs` use O(1) string operations
/// - Simple prefix patterns like `src/*` use O(1) string operations  
/// - Complex patterns use efficient recursive matching without regex
///
/// # Returns
///
/// Returns `Some(FuzzyMatchResult)` with:
/// - `score`: Higher scores (1000+) for exact matches, lower scores (800+) for partial matches
/// - `matched_indices`: Character indices of the matched portions in the original text
///
/// Returns `None` if the pattern doesn't match.
pub fn match_wildcard_pattern(text: &str, pattern: &str) -> Option<FuzzyMatchResult> {
    if pattern.is_empty() {
        return Some(FuzzyMatchResult::no_match());
    }

    // Fast path for simple suffix patterns like "*.rs" or partial suffixes like "*.r"
    if pattern.starts_with('*') && !pattern[1..].contains('*') && !pattern[1..].contains('?') {
        let suffix = &pattern[1..];

        // First try exact suffix match
        if text.ends_with(suffix) {
            let text_char_count = text.chars().count();
            let suffix_char_count = suffix.chars().count();
            let start_char_idx = text_char_count - suffix_char_count;
            let matched_indices: Vec<usize> = (start_char_idx..text_char_count).collect();
            return Some(FuzzyMatchResult {
                score: 1000,
                matched_indices,
            });
        }

        // If no exact match, try partial suffix match (for progressive typing)
        if let Some(match_info) = find_partial_suffix_match(text, suffix) {
            return Some(FuzzyMatchResult {
                score: 800, // Lower score than exact match but still good
                matched_indices: match_info,
            });
        }

        return None;
    }

    // Fast path for simple prefix patterns like "src/*" - but only if it starts at the beginning
    if pattern.ends_with('*')
        && !pattern[..pattern.len() - 1].contains('*')
        && !pattern[..pattern.len() - 1].contains('?')
    {
        let prefix = &pattern[..pattern.len() - 1];
        if text.starts_with(prefix) {
            let prefix_char_count = prefix.chars().count();
            let matched_indices: Vec<usize> = (0..prefix_char_count).collect();
            return Some(FuzzyMatchResult {
                score: 1000,
                matched_indices,
            });
        }
        // Don't return None here - fall through to substring matching
    }

    // For file paths, we want to match patterns anywhere in the path, not just at the beginning
    // Try to find the pattern as a substring match first
    if let Some(match_info) = find_substring_glob_match(text, pattern) {
        return Some(FuzzyMatchResult {
            score: 1000,
            matched_indices: match_info,
        });
    }

    // Fallback to exact glob matching for complex patterns
    if is_glob_match(text, pattern) {
        // For complex patterns, we'll mark the entire text as matched
        // This is a simplification but much faster than tracking exact indices
        let matched_indices: Vec<usize> = (0..text.chars().count()).collect();

        let score = if pattern.contains('*') || pattern.contains('?') {
            1000 // Good score for wildcard matches
        } else {
            2000 // Higher score for exact matches
        };

        Some(FuzzyMatchResult {
            score,
            matched_indices,
        })
    } else {
        None
    }
}

/// Finds partial suffix matches for progressive typing support.
///
/// This function enables progressive typing by matching partial file extensions.
/// For example, when a user types `*.r`, it should match files ending in `.rs`, `.rb`, etc.
///
/// # Arguments
///
/// * `text` - The text to search in (e.g., "button.rs")
/// * `partial_suffix` - The partial suffix to match (e.g., ".r")
///
/// # Returns
///
/// Returns `Some(Vec<usize>)` with character indices of the matched portion,
/// or `None` if no match is found.
///
/// # Examples
///
/// ```
/// // This would be called internally when matching "*.r" against "button.rs"
/// // find_partial_suffix_match("button.rs", ".r") -> Some([6, 7])
/// ```
fn find_partial_suffix_match(text: &str, partial_suffix: &str) -> Option<Vec<usize>> {
    if partial_suffix.is_empty() {
        return None;
    }

    // Look for any suffix in the text that starts with our partial suffix
    let text_chars: Vec<char> = text.chars().collect();
    let partial_chars: Vec<char> = partial_suffix.chars().collect();

    // Start from the end of the text and work backwards to find potential suffix matches
    for start_pos in (0..text_chars.len()).rev() {
        // Check if the remaining text starts with our partial suffix
        let remaining_text = &text_chars[start_pos..];

        if remaining_text.len() >= partial_chars.len() {
            let matches = remaining_text
                .iter()
                .zip(partial_chars.iter())
                .all(|(t, p)| t.eq_ignore_ascii_case(p));

            if matches {
                // Found a match! Return the indices of the matched part
                let matched_indices: Vec<usize> =
                    (start_pos..start_pos + partial_chars.len()).collect();
                return Some(matched_indices);
            }
        }
    }

    None
}

/// Fast glob pattern matching without regex.
///
/// Implements a simple recursive algorithm for glob patterns that avoids
/// the overhead of regex compilation and matching.
///
/// # Arguments
///
/// * `text` - The text to match against
/// * `pattern` - The glob pattern (supports * and ?)
///
/// # Returns
///
/// Returns `true` if the pattern matches the entire text from start to end.
///
/// # Note
///
/// This function requires the pattern to match the entire text, unlike
/// [`find_substring_glob_match`] which finds patterns anywhere in the text.
fn is_glob_match(text: &str, pattern: &str) -> bool {
    is_glob_match_recursive(text.as_bytes(), pattern.as_bytes(), 0, 0)
}

/// Finds a glob pattern as a substring anywhere in the text.
///
/// This is the core function that enables substring matching for file paths.
/// Unlike standard glob matching that requires patterns to match from the start,
/// this function finds patterns anywhere within the text.
///
/// # Key Features
///
/// - **Substring matching**: `ui/*` matches `/src/ui/button.rs`
/// - **Complex patterns**: `ui/*.r` matches `/src/ui/button.rs` (progressive typing)
/// - **Performance optimized**: Uses fast paths for common patterns
/// - **Recursion safe**: Prevents infinite recursion for complex patterns
///
/// # Arguments
///
/// * `text` - The text to search in (e.g., "/src/ui/button.rs")
/// * `pattern` - The glob pattern to find (e.g., "ui/*.r")
///
/// # Returns
///
/// Returns `Some(Vec<usize>)` with character indices of all matched portions,
/// or `None` if the pattern is not found anywhere in the text.
///
/// # Examples
///
/// ```
/// // Pattern "ui/*.rs" in text "/src/ui/button.rs"
/// // Would return indices for "ui/" and ".rs" portions
/// // Some([5, 6, 7, 14, 15, 16])
/// ```
fn find_substring_glob_match(text: &str, pattern: &str) -> Option<Vec<usize>> {
    // Handle patterns like "ui/*.r" - need to find the prefix and match the suffix pattern
    if let Some(star_pos) = pattern.find('*') {
        let prefix = &pattern[..star_pos];
        let suffix_pattern = &pattern[star_pos..];

        // Find the prefix in the text
        if let Some(prefix_start) = text.find(prefix) {
            let prefix_end = prefix_start + prefix.len();
            let remaining_text = &text[prefix_end..];

            // Try to match the suffix pattern against the remaining text
            // Use a non-recursive approach for simple suffix patterns to avoid infinite recursion
            let suffix_result = if suffix_pattern.starts_with('*')
                && !suffix_pattern[1..].contains('*')
                && !suffix_pattern[1..].contains('?')
            {
                // Handle simple suffix patterns directly
                let suffix_part = &suffix_pattern[1..];
                if remaining_text.ends_with(suffix_part) {
                    let remaining_char_count = remaining_text.chars().count();
                    let suffix_char_count = suffix_part.chars().count();
                    let start_idx = remaining_char_count - suffix_char_count;
                    Some(FuzzyMatchResult {
                        score: 1000,
                        matched_indices: (start_idx..remaining_char_count).collect(),
                    })
                } else {
                    find_partial_suffix_match(remaining_text, suffix_part).map(|partial_match| {
                        FuzzyMatchResult {
                            score: 800,
                            matched_indices: partial_match,
                        }
                    })
                }
            } else {
                // For complex patterns, use the full matching but prevent recursion
                if is_glob_match(remaining_text, suffix_pattern) {
                    Some(FuzzyMatchResult {
                        score: 1000,
                        matched_indices: (0..remaining_text.chars().count()).collect(),
                    })
                } else {
                    None
                }
            };

            if let Some(suffix_result) = suffix_result {
                // Combine the matched indices
                let prefix_start_char = text[..prefix_start].chars().count();
                let prefix_char_count = prefix.chars().count();
                let remaining_start_char = text[..prefix_end].chars().count();

                let mut combined_indices: Vec<usize> =
                    (prefix_start_char..prefix_start_char + prefix_char_count).collect();

                // Add suffix match indices, adjusted for position
                for idx in suffix_result.matched_indices {
                    combined_indices.push(remaining_start_char + idx);
                }

                return Some(combined_indices);
            }
        }
    }

    // For simple patterns like "ui/*", try to find "ui/" substring and match the rest
    if pattern.ends_with('*')
        && !pattern[..pattern.len() - 1].contains('*')
        && !pattern[..pattern.len() - 1].contains('?')
    {
        let prefix = &pattern[..pattern.len() - 1];
        if let Some(start_pos) = text.find(prefix) {
            // Found the prefix as a substring, create indices for the matched part
            let start_char_idx = text[..start_pos].chars().count();
            let prefix_char_count = prefix.chars().count();
            let matched_indices: Vec<usize> =
                (start_char_idx..start_char_idx + prefix_char_count).collect();
            return Some(matched_indices);
        }
    }

    // For more complex patterns, try to find any substring that matches
    let text_chars: Vec<char> = text.chars().collect();
    let pattern_chars: Vec<char> = pattern.chars().collect();

    // Try matching the pattern starting at each position in the text
    for start_idx in 0..text_chars.len() {
        if is_glob_match_at_position(&text_chars, &pattern_chars, start_idx) {
            // Found a match starting at start_idx
            // For simplicity, return indices from start_idx to end of match
            // In a more sophisticated implementation, we'd track exact matched characters
            let end_idx = find_glob_match_end(&text_chars, &pattern_chars, start_idx)
                .unwrap_or(text_chars.len());
            let matched_indices: Vec<usize> = (start_idx..end_idx.min(text_chars.len())).collect();
            return Some(matched_indices);
        }
    }

    None
}

/// Check if a glob pattern matches starting at a specific position
fn is_glob_match_at_position(
    text_chars: &[char],
    pattern_chars: &[char],
    start_idx: usize,
) -> bool {
    is_glob_match_chars(&text_chars[start_idx..], pattern_chars)
}

/// Find the end position of a glob match
fn find_glob_match_end(
    text_chars: &[char],
    pattern_chars: &[char],
    start_idx: usize,
) -> Option<usize> {
    // This is a simplified version - for complex patterns we'd need more sophisticated tracking
    // For now, assume the entire remaining text after start_idx
    if is_glob_match_at_position(text_chars, pattern_chars, start_idx) {
        Some(text_chars.len())
    } else {
        None
    }
}

/// Glob matching for character slices
fn is_glob_match_chars(text: &[char], pattern: &[char]) -> bool {
    is_glob_match_chars_recursive(text, pattern, 0, 0)
}

fn is_glob_match_chars_recursive(
    text: &[char],
    pattern: &[char],
    text_idx: usize,
    pattern_idx: usize,
) -> bool {
    // If we've consumed the entire pattern
    if pattern_idx >= pattern.len() {
        return text_idx >= text.len();
    }

    // If we've consumed the entire text but pattern remains
    if text_idx >= text.len() {
        // Only match if the remaining pattern is all '*'
        return pattern[pattern_idx..].iter().all(|&c| c == '*');
    }

    match pattern[pattern_idx] {
        '*' => {
            // Try matching zero characters (skip the *)
            if is_glob_match_chars_recursive(text, pattern, text_idx, pattern_idx + 1) {
                return true;
            }
            // Try matching one or more characters
            for i in text_idx..text.len() {
                if is_glob_match_chars_recursive(text, pattern, i + 1, pattern_idx + 1) {
                    return true;
                }
            }
            false
        }
        '?' => {
            // ? matches exactly one character
            is_glob_match_chars_recursive(text, pattern, text_idx + 1, pattern_idx + 1)
        }
        c => {
            // Regular character - must match exactly (case insensitive)
            if text[text_idx].eq_ignore_ascii_case(&c) {
                is_glob_match_chars_recursive(text, pattern, text_idx + 1, pattern_idx + 1)
            } else {
                false
            }
        }
    }
}

fn is_glob_match_recursive(
    text: &[u8],
    pattern: &[u8],
    text_idx: usize,
    pattern_idx: usize,
) -> bool {
    // If we've consumed the entire pattern
    if pattern_idx >= pattern.len() {
        return text_idx >= text.len();
    }

    // If we've consumed the entire text but pattern remains
    if text_idx >= text.len() {
        // Only match if the remaining pattern is all '*'
        return pattern[pattern_idx..].iter().all(|&c| c == b'*');
    }

    match pattern[pattern_idx] {
        b'*' => {
            // Try matching zero characters (skip the *)
            if is_glob_match_recursive(text, pattern, text_idx, pattern_idx + 1) {
                return true;
            }
            // Try matching one or more characters
            for i in text_idx..text.len() {
                if is_glob_match_recursive(text, pattern, i + 1, pattern_idx + 1) {
                    return true;
                }
            }
            false
        }
        b'?' => {
            // ? matches exactly one character
            is_glob_match_recursive(text, pattern, text_idx + 1, pattern_idx + 1)
        }
        c => {
            // Regular character - must match exactly (case insensitive)
            if text[text_idx].eq_ignore_ascii_case(&c) {
                is_glob_match_recursive(text, pattern, text_idx + 1, pattern_idx + 1)
            } else {
                false
            }
        }
    }
}

/// Performs case insensitive pattern matching with wildcard support.
///
/// This function works identically to [`match_wildcard_pattern`] but ignores case differences
/// in both the text and pattern.
///
/// # Examples
///
/// ```
/// use fuzzy_match::match_wildcard_pattern_case_insensitive;
///
/// // Case insensitive matching
/// assert!(match_wildcard_pattern_case_insensitive("Button.RS", "*.rs").is_some());
/// assert!(match_wildcard_pattern_case_insensitive("/src/UI/button.rs", "ui/*").is_some());
/// assert!(match_wildcard_pattern_case_insensitive("TEST.JS", "test.*").is_some());
///
/// // Mixed case patterns
/// assert!(match_wildcard_pattern_case_insensitive("/src/Components/Button.tsx", "*/components/*.TSX").is_some());
/// ```
///
/// # Note
///
/// The returned `matched_indices` correspond to character positions in the original text,
/// not the lowercased version used internally for matching.
pub fn match_wildcard_pattern_case_insensitive(
    text: &str,
    pattern: &str,
) -> Option<FuzzyMatchResult> {
    if pattern.is_empty() {
        return Some(FuzzyMatchResult::no_match());
    }

    // Convert both text and pattern to lowercase for case insensitive matching
    let text_lower = text.to_lowercase();
    let pattern_lower = pattern.to_lowercase();

    match_wildcard_pattern(&text_lower, &pattern_lower).map(|mut result| {
        // Map the matched indices back to the original text
        // Since we're doing character-level matching and both strings should have
        // the same character positions, the indices should remain valid
        result
            .matched_indices
            .retain(|&idx| idx < text.chars().count());
        result
    })
}

#[cfg(test)]
#[path = "fuzzy_tests.rs"]
mod tests;
