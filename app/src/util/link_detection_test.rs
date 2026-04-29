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
