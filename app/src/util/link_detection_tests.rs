use itertools::Itertools;

use super::*;

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
fn test_detect_urls_stops_at_fullwidth_punctuation() {
    assert_eq!(detect_urls("go https://example.com，next"), vec![3..22]);
    assert_eq!(detect_urls("go https://example.com。"), vec![3..22]);
}

#[cfg(feature = "local_fs")]
#[test]
fn test_detect_file_paths_stops_at_fullwidth_punctuation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("warp-rich-content.md");
    std::fs::write(&file, "# Hello\n").unwrap();

    let text = "see warp-rich-content.md， and warp-rich-content.md。";
    let detected_paths = detect_file_paths(dir.path().to_str().unwrap(), text, None);

    let link_ranges = detected_paths.keys().cloned().collect_vec();
    assert!(link_ranges.contains(&(4..24)));
    assert!(link_ranges.contains(&(30..50)));
    assert!(!link_ranges.contains(&(4..25)));
    assert!(!link_ranges.contains(&(30..51)));
}

#[cfg(feature = "local_fs")]
#[test]
fn test_detect_file_paths_keeps_fullwidth_punctuation_when_it_is_the_filename() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("warp-rich-content.md，");
    std::fs::write(&file, "# Hello\n").unwrap();

    let text = "see warp-rich-content.md，";
    let detected_paths = detect_file_paths(dir.path().to_str().unwrap(), text, None);

    assert!(detected_paths.contains_key(&(4..25)));
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
fn test_possible_file_paths_in_tree_output() {
    let word = "│└──alpha.md";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert!(possible_paths.contains(&"alpha.md"));
}

#[test]
fn test_possible_file_paths_in_tree_output_multibyte_filename() {
    let word = "│└──音楽.md";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert!(possible_paths.contains(&"音楽.md"));
}

#[test]
fn test_possible_file_paths_in_tree_output_absolute_path_leaf() {
    let word = "│└──/tmp/foo.md";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert!(possible_paths.contains(&"/tmp/foo.md"));
}

#[test]
fn test_possible_file_paths_in_word_cjk_punctuation() {
    // Fullwidth colon (U+FF1A) directly touching a path — common in CJK prose
    // such as `路径：/path/to/file`.
    let word = "路径：/path/to/file.md";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert!(possible_paths.contains(&"/path/to/file.md"));
    assert!(possible_paths.contains(&"路径"));

    // Fullwidth parentheses (U+FF08 / U+FF09) wrapping a path.
    let word = "（/path/to/file）";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert!(possible_paths.contains(&"/path/to/file"));

    // CJK corner brackets (U+300C / U+300D) wrapping a path.
    let word = "「/path/to/file」";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert!(possible_paths.contains(&"/path/to/file"));

    // Ideographic full stop (U+3002) following a path.
    let word = "/path/to/file。";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert!(possible_paths.contains(&"/path/to/file"));

    // Fullwidth comma (U+FF0C) between paths.
    let word = "/a/b，/c/d";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert!(possible_paths.contains(&"/a/b"));
    assert!(possible_paths.contains(&"/c/d"));

    // CJK letters (general category Lo) must NOT split a token, otherwise paths
    // legitimately containing CJK characters would be fragmented.
    let word = "/path/音楽/テスト.txt";
    let possible_paths = possible_file_paths_in_word(word).collect_vec();
    assert!(possible_paths.contains(&"/path/音楽/テスト.txt"));
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
    assert!(
        possible_file_paths_in_word(&too_many_separators)
            .next()
            .is_none()
    );
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

/// Regression guard for link tooltips not appearing in multi-block Agent Mode conversations.
///
/// The bug: every AI block anchored its link tooltip overlay to a single shared, global
/// save-position id. With 2+ rich-content blocks in a conversation, those anchors collided, so
/// the overlay could not position itself on the clicked link and no tooltip appeared. The fix
/// gives each block a per-view-unique anchor id (`tooltip_position_id`), so distinct blocks must
/// resolve to distinct anchor ids.
#[test]
fn link_tooltip_anchor_ids_are_unique_per_block() {
    // Two AI blocks each set their own per-view anchor id (as `show_link_tooltip` does using the
    // block's view id).
    let mut block_a = DetectedLinksState::default();
    let mut block_b = DetectedLinksState::default();
    block_a.tooltip_position_id = format!("{RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID}_1");
    block_b.tooltip_position_id = format!("{RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID}_2");

    assert_ne!(
        block_a.resolved_tooltip_position_id(),
        block_b.resolved_tooltip_position_id(),
        "distinct AI blocks must resolve to distinct link tooltip anchor ids so their tooltip \
         overlays don't collide in a multi-block conversation"
    );
    assert_eq!(
        block_a.resolved_tooltip_position_id(),
        block_a.tooltip_position_id,
        "a block with an assigned anchor id must resolve to exactly that id"
    );

    // A block that has never opened a tooltip falls back to the shared constant. This is harmless
    // because registration of the anchor only happens alongside an open tooltip, which always
    // assigns a per-view id first.
    let unset = DetectedLinksState::default();
    assert_eq!(
        unset.resolved_tooltip_position_id(),
        RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID
    );
}

/// Baseline: a plain-text `foo.rs:59` reference resolves to a line-aware `FilePath`.
#[cfg(feature = "local_fs")]
#[test]
fn plain_text_file_reference_is_line_aware() {
    use crate::ai::blocklist::block::TextLocation;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("foo.rs"), "// x\n").unwrap();
    let cwd = dir.path().to_str().unwrap().to_owned();
    let location = TextLocation::Output {
        section_index: 0,
        line_index: 0,
    };

    let texts = vec![("foo.rs:59".to_string(), location)];
    let all_links = detect_all_links(&texts, Vec::new(), Some(&cwd), None);

    let link = all_links
        .get(&location)
        .and_then(|links| links.get(&(0..9)))
        .expect("a file link should be detected for the plain-text reference");
    assert!(
        matches!(
            link,
            DetectedLinkType::FilePath {
                line_and_column_num: Some(lc),
                ..
            } if lc.line_num == 59
        ),
        "plain-text `foo.rs:59` must resolve to a FilePath carrying line 59, got {link:?}"
    );
}

/// Regression for the reported bug: a markdown link `[foo.rs:59](foo.rs)` must
/// stay a line-aware `FilePath` rather than being clobbered by a line-less `Url`
/// (which opened the file at line 1).
#[cfg(feature = "local_fs")]
#[test]
fn markdown_link_preserves_line_aware_file_path() {
    use crate::ai::blocklist::block::TextLocation;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("foo.rs"), "// x\n").unwrap();
    let cwd = dir.path().to_str().unwrap().to_owned();
    let location = TextLocation::Output {
        section_index: 0,
        line_index: 0,
    };

    // Display text `foo.rs:59`; markdown URL `foo.rs` (no line) over range 0..9.
    let texts = vec![("foo.rs:59".to_string(), location)];
    let md_hyperlinks: HyperlinksByLocation = vec![(location, vec![(0..9, "foo.rs".to_string())])];

    let all_links = detect_all_links(&texts, md_hyperlinks, Some(&cwd), None);

    let link = all_links
        .get(&location)
        .and_then(|links| links.get(&(0..9)))
        .expect("a link should be detected for the markdown file reference");
    assert!(
        matches!(
            link,
            DetectedLinkType::FilePath {
                line_and_column_num: Some(lc),
                ..
            } if lc.line_num == 59
        ),
        "the markdown link must resolve to a line-aware FilePath (line 59), not a \
         line-less URL that opens the file at line 1, got {link:?}"
    );
}

/// Guard: a genuine external markdown hyperlink stays a `Url`.
#[cfg(feature = "local_fs")]
#[test]
fn markdown_external_link_stays_url() {
    use crate::ai::blocklist::block::TextLocation;

    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap().to_owned();
    let location = TextLocation::Output {
        section_index: 0,
        line_index: 0,
    };

    let texts = vec![("docs".to_string(), location)];
    let md_hyperlinks: HyperlinksByLocation =
        vec![(location, vec![(0..4, "https://example.com".to_string())])];

    let all_links = detect_all_links(&texts, md_hyperlinks, Some(&cwd), None);

    let link = all_links
        .get(&location)
        .and_then(|links| links.get(&(0..4)))
        .expect("the external hyperlink should be detected");
    assert!(
        matches!(link, DetectedLinkType::Url(url) if url == "https://example.com"),
        "an external markdown hyperlink must remain a Url, got {link:?}"
    );
}

/// Guard: a file-like label with an explicit external href stays a `Url` (the
/// guard is line-aware only, so a line-less local-file label doesn't hijack it).
#[cfg(feature = "local_fs")]
#[test]
fn markdown_external_link_with_file_like_label_stays_url() {
    use crate::ai::blocklist::block::TextLocation;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
    let cwd = dir.path().to_str().unwrap().to_owned();
    let location = TextLocation::Output {
        section_index: 0,
        line_index: 0,
    };

    // `[Cargo.toml](https://example.com)`: label is a real local file (no line).
    let texts = vec![("Cargo.toml".to_string(), location)];
    let md_hyperlinks: HyperlinksByLocation =
        vec![(location, vec![(0..10, "https://example.com".to_string())])];

    let all_links = detect_all_links(&texts, md_hyperlinks, Some(&cwd), None);

    let link = all_links
        .get(&location)
        .and_then(|links| links.get(&(0..10)))
        .expect("the hyperlink should be detected");
    assert!(
        matches!(link, DetectedLinkType::Url(url) if url == "https://example.com"),
        "a file-like label with an explicit external href must stay a Url (not open \
         the local file), got {link:?}"
    );
}

/// Regression for the reported case: the line lives in the hyperlink *URL* while the on-screen
/// label has no line. The URL must be resolved to a line-aware file link so clicking opens the
/// file at that line instead of dropping it.
#[cfg(feature = "local_fs")]
#[test]
fn markdown_link_with_line_in_url_is_line_aware() {
    use crate::ai::blocklist::block::TextLocation;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("server.go"), "package main\n").unwrap();
    let cwd = dir.path().to_str().unwrap().to_owned();
    let location = TextLocation::Output {
        section_index: 0,
        line_index: 0,
    };

    // On-screen label `server.go` carries no line; the hyperlink URL `server.go:164` does.
    let texts = vec![("server.go".to_string(), location)];
    let md_hyperlinks: HyperlinksByLocation =
        vec![(location, vec![(0..9, "server.go:164".to_string())])];

    let all_links = detect_all_links(&texts, md_hyperlinks, Some(&cwd), None);

    let link = all_links
        .get(&location)
        .and_then(|links| links.get(&(0..9)))
        .expect("a link should be detected for the markdown file reference");
    assert!(
        matches!(
            link,
            DetectedLinkType::FilePath {
                line_and_column_num: Some(lc),
                ..
            } if lc.line_num == 164
        ),
        "a markdown link whose URL carries `:164` must resolve to a line-aware FilePath \
         (line 164), got {link:?}"
    );
}
