use super::{Match, MatchStrategy};
use crate::completer::matchers::match_type_for_case_insensitive;

#[test]
fn test_match_type_for_case_insensitive() {
    assert_eq!(
        match_type_for_case_insensitive("git", "git"),
        Some(Match::Exact {
            is_case_sensitive: true
        })
    );
    assert_eq!(
        match_type_for_case_insensitive("gIt", "git"),
        Some(Match::Exact {
            is_case_sensitive: false
        })
    );
    assert_eq!(
        match_type_for_case_insensitive("abc", "abcdef"),
        Some(Match::Prefix {
            is_case_sensitive: true
        })
    );
    assert_eq!(
        match_type_for_case_insensitive("aBc", "abcdef"),
        Some(Match::Prefix {
            is_case_sensitive: false
        })
    );
    assert_eq!(match_type_for_case_insensitive("abc", "def"), None);
}

#[test]
fn test_get_match_type_case_sensitive() {
    let matcher = MatchStrategy::CaseSensitive;

    assert_eq!(matcher.get_match_type("git", "GIT"), None);
    assert_eq!(
        matcher.get_match_type("git", "git"),
        Some(Match::Exact {
            is_case_sensitive: true
        })
    );
    assert_eq!(
        matcher.get_match_type("AsDs", "AsDss"),
        Some(Match::Prefix {
            is_case_sensitive: true
        })
    );
    assert_eq!(matcher.get_match_type("Asds", "asds"), None);
}

#[test]
fn test_get_match_type_case_insensitive() {
    let matcher = MatchStrategy::CaseInsensitive;

    assert_eq!(
        matcher.get_match_type("git", "GIT"),
        Some(Match::Exact {
            is_case_sensitive: false
        })
    );
    assert_eq!(
        matcher.get_match_type("AsDs", "asdss"),
        Some(Match::Prefix {
            is_case_sensitive: false
        })
    );
    assert_eq!(matcher.get_match_type("Asd", "ads"), None);
}

#[test]
fn test_get_match_type_fuzzy() {
    let matcher = MatchStrategy::Fuzzy;

    assert_eq!(
        matcher.get_match_type("git", "GIT"),
        Some(Match::Exact {
            is_case_sensitive: false
        })
    );
    assert_eq!(
        matcher.get_match_type("AsDs", "asdss"),
        Some(Match::Prefix {
            is_case_sensitive: false
        })
    );
    assert!(matches!(
        matcher.get_match_type("abc", "aabac"),
        Some(Match::Fuzzy { .. })
    ));
    assert_eq!(matcher.get_match_type("abc", "xyz"), None);
}

#[test]
fn test_get_match_type_fuzzy_rejects_punctuation_only_queries() {
    let matcher = MatchStrategy::Fuzzy;

    // A query with no alphanumeric character must not subsequence-match a candidate that merely
    // contains those characters (e.g. the "." in "vim.basic"), which previously surfaced spurious
    // command completions for shell input like "." or "echo hi;.".
    assert_eq!(matcher.get_match_type(".", "vim.basic"), None);
    assert_eq!(matcher.get_match_type(".", "date"), None);
    assert_eq!(matcher.get_match_type("$(date).", "vim.basic"), None);
    assert_eq!(matcher.get_match_type("echo hi;.", "vim.basic"), None);
}

#[test]
fn test_get_match_type_fuzzy_keeps_prefix_and_exact_for_punctuation_queries() {
    let matcher = MatchStrategy::Fuzzy;

    // The alphanumeric gate only guards the subsequence fallback; prefix and exact matches -- even
    // for queries that are all punctuation or start with punctuation -- are untouched.
    assert_eq!(
        matcher.get_match_type("../", "../foo"),
        Some(Match::Prefix {
            is_case_sensitive: true
        })
    );
    assert_eq!(
        matcher.get_match_type(".bash", ".bashrc"),
        Some(Match::Prefix {
            is_case_sensitive: true
        })
    );
    assert_eq!(
        matcher.get_match_type(".bashrc", ".bashrc"),
        Some(Match::Exact {
            is_case_sensitive: true
        })
    );
    assert_eq!(
        matcher.get_match_type("git -", "git --version"),
        Some(Match::Prefix {
            is_case_sensitive: true
        })
    );
}
