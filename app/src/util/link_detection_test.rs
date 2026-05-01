use super::*;

use itertools::Itertools;

#[test]
fn test_possible_file_paths_in_word() {
    let word = "/path/to/file:16:hello";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert_eq!(
        possible_paths,
        vec![
            "/path/to/file:16:hello",
            "/path/to/file:16",
            "/path/to/file",
            "16:hello",
            "hello",
            "16"
        ]
    );

    let word = "/path/to/file:162:47.";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert_eq!(
        possible_paths,
        vec![
            "/path/to/file:162:47.",
            "/path/to/file:162:47",
            "/path/to/file:162",
            "/path/to/file",
            "162:47.",
            "162:47",
            "162",
            "47.",
            "47"
        ]
    );

    let word = "<Cargo.toml:16:4>";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert_eq!(
        possible_paths,
        vec![
            "<Cargo.toml:16:4>",
            "<Cargo.toml:16:4",
            "Cargo.toml:16:4>",
            "Cargo.toml:16:4",
            "<Cargo.toml:16",
            "Cargo.toml:16",
            "<Cargo.toml",
            "Cargo.toml",
            "16:4>",
            "16:4",
            "16",
            "4>",
            "4"
        ]
    );
}

#[test]
fn test_possible_file_paths_in_word_multibyte() {
    let word = "/path/音楽/テストファイル.txt:16:ḧeĹḹo";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert_eq!(
        possible_paths,
        vec![
            "/path/音楽/テストファイル.txt:16:ḧeĹḹo",
            "/path/音楽/テストファイル.txt:16",
            "/path/音楽/テストファイル.txt",
            "16:ḧeĹḹo",
            "ḧeĹḹo",
            "16"
        ]
    );
}

#[test]
fn test_possible_file_paths_in_word_skips_oversized_token() {
    let oversized = "a".repeat(MAX_WORD_LEN_FOR_FILE_PATH + 1);
    assert!(possible_file_paths_in_word(&oversized).next().is_none());
}

#[test]
fn test_possible_file_paths_in_word_accepts_token_at_word_length_cap() {
    let at_cap = "a".repeat(MAX_WORD_LEN_FOR_FILE_PATH);
    let possible_paths = possible_file_paths_in_word(&at_cap).collect_vec();
    assert_eq!(possible_paths, vec![at_cap.as_str()]);
}

#[test]
fn test_possible_file_paths_in_word_skips_token_with_too_many_separators() {
    let too_many_separators = ":".repeat(MAX_SEPARATORS_PER_WORD + 1);
    assert!(possible_file_paths_in_word(&too_many_separators)
        .next()
        .is_none());
}

#[test]
fn test_possible_file_paths_in_word_accepts_token_at_separator_count_cap() {
    // A token with separators interleaved between letters: e.g. "a:a:a:...:a".
    // Has exactly MAX_SEPARATORS_PER_WORD ':' characters and is non-empty
    // between them, so we expect at least one candidate (e.g. "a").
    let mut at_cap = String::with_capacity(MAX_SEPARATORS_PER_WORD * 2 + 1);
    at_cap.push('a');
    for _ in 0..MAX_SEPARATORS_PER_WORD {
        at_cap.push(':');
        at_cap.push('a');
    }
    assert!(possible_file_paths_in_word(&at_cap).next().is_some());
}
