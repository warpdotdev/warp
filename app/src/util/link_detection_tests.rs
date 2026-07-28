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

// Cross-line URL reconstruction (GH9385).
//
// CLI agents such as Claude Code hard-wrap their TUI output, so a long URL arrives as
// several formatted lines. Detecting each line on its own found only the prefix that
// happened to parse as a URL, and Cmd+click opened that truncated target.

fn output_line(line_index: usize) -> TextLocation {
    TextLocation::Output {
        section_index: 0,
        line_index,
    }
}

/// Builds the `(text, location)` pairs that `collect_output_data_for_link_detection`
/// produces for one plain-text section. `raw_text()` terminates every formatted line
/// with a newline, so the fixtures do too.
fn output_lines(lines: &[&str]) -> Vec<(String, TextLocation)> {
    lines
        .iter()
        .enumerate()
        .map(|(line_index, line)| (format!("{line}\n"), output_line(line_index)))
        .collect_vec()
}

fn urls_in(
    links: &HashMap<TextLocation, HashMap<Range<usize>, DetectedLinkType>>,
    location: TextLocation,
) -> Vec<(Range<usize>, String)> {
    links
        .get(&location)
        .map(|detected| {
            detected
                .iter()
                .filter_map(|(range, link)| match link {
                    DetectedLinkType::Url(url) => Some((range.clone(), url.clone())),
                    #[cfg(feature = "local_fs")]
                    DetectedLinkType::FilePath { .. } => None,
                })
                .sorted_by_key(|(range, _)| range.start)
                .collect_vec()
        })
        .unwrap_or_default()
}

fn detected_urls_by_line(lines: &[&str]) -> Vec<Vec<(Range<usize>, String)>> {
    let texts = output_lines(lines);
    let links = detect_all_links(&texts, vec![], None, None);
    (0..lines.len())
        .map(|line_index| urls_in(&links, output_line(line_index)))
        .collect_vec()
}

#[test]
fn test_reconstructs_url_hard_wrapped_across_two_lines() {
    const FULL_URL: &str =
        "https://example.com/some/very/long/path?param1=value1&param2=value2&param3=value3";
    let detected = detected_urls_by_line(&[
        "https://example.com/some/very/long/path?param1=value1&param",
        "2=value2&param3=value3",
    ]);

    // Both visual lines stay individually clickable, and both open the whole URL.
    assert_eq!(detected[0], vec![(0..59, FULL_URL.to_owned())]);
    assert_eq!(detected[1], vec![(0..22, FULL_URL.to_owned())]);
}

#[test]
fn test_reconstructs_url_hard_wrapped_across_three_lines() {
    const FULL_URL: &str =
        "https://example.com/a/very/long/path/that/keeps/going/and/going?q=1&r=2";
    let detected = detected_urls_by_line(&[
        "Docs: https://example.com/a/very/long",
        "/path/that/keeps/going/and/going",
        "?q=1&r=2",
    ]);

    assert_eq!(detected[0], vec![(6..37, FULL_URL.to_owned())]);
    assert_eq!(detected[1], vec![(0..32, FULL_URL.to_owned())]);
    assert_eq!(detected[2], vec![(0..8, FULL_URL.to_owned())]);
}

#[test]
fn test_single_line_url_detection_is_unchanged() {
    let detected = detected_urls_by_line(&["see https://example.com/foo and more", "next line"]);

    assert_eq!(
        detected[0],
        vec![(4..27, "https://example.com/foo".to_owned())]
    );
    assert!(detected[1].is_empty());
}

#[test]
fn test_url_followed_by_prose_is_not_joined() {
    let detected = detected_urls_by_line(&["Visit https://example.com", "This is a sentence."]);

    // Joining would have produced `https://example.comThis`.
    assert_eq!(detected[0], vec![(6..25, "https://example.com".to_owned())]);
    assert!(detected[1].is_empty());
}

#[test]
fn test_adjacent_independent_urls_stay_separate() {
    let detected = detected_urls_by_line(&["https://example.com", "https://other.example.org"]);

    assert_eq!(detected[0], vec![(0..19, "https://example.com".to_owned())]);
    assert_eq!(
        detected[1],
        vec![(0..25, "https://other.example.org".to_owned())]
    );
}

#[test]
fn test_layout_rows_are_not_joined_onto_a_url() {
    // A URL can never resume after a list marker, table delimiter or indentation, so each
    // of these must leave the first line's URL exactly as detected on its own.
    for continuation in [
        "- another item",
        "* another item",
        "2. another item",
        "│ cell │",
        "|cell|",
        "    indented/continuation",
        "",
    ] {
        let detected = detected_urls_by_line(&["https://example.com/foo", continuation]);
        assert_eq!(
            detected[0],
            vec![(0..23, "https://example.com/foo".to_owned())],
            "unexpectedly joined continuation {continuation:?}"
        );
    }
}

#[test]
fn test_reconstructed_ranges_are_char_based_not_byte_based() {
    const FULL_URL: &str = "https://example.com/very/long/tail?x=1";
    let detected = detected_urls_by_line(&["日本語 https://example.com/very/long", "/tail?x=1"]);

    // "日本語 " is 4 chars but 10 bytes; the range must start at char 4.
    assert_eq!(detected[0], vec![(4..33, FULL_URL.to_owned())]);
    assert_eq!(detected[1], vec![(0..9, FULL_URL.to_owned())]);
}

#[test]
fn test_urls_are_not_reconstructed_across_output_sections() {
    let texts = vec![
        (
            "https://example.com/some/very/long/path?param1=value1&param\n".to_owned(),
            TextLocation::Output {
                section_index: 0,
                line_index: 0,
            },
        ),
        (
            "2=value2&param3=value3\n".to_owned(),
            TextLocation::Output {
                section_index: 1,
                line_index: 0,
            },
        ),
    ];
    let links = detect_all_links(&texts, vec![], None, None);

    assert_eq!(
        urls_in(
            &links,
            TextLocation::Output {
                section_index: 0,
                line_index: 0
            }
        ),
        vec![(
            0..59,
            "https://example.com/some/very/long/path?param1=value1&param".to_owned()
        )]
    );
    assert!(
        urls_in(
            &links,
            TextLocation::Output {
                section_index: 1,
                line_index: 0
            }
        )
        .is_empty()
    );
}

#[test]
fn test_markdown_hyperlinks_take_precedence_over_reconstructed_urls() {
    let texts = output_lines(&[
        "https://example.com/some/very/long/path?param1=value1&param",
        "2=value2&param3=value3",
    ]);
    let md_hyperlinks = vec![(
        output_line(0),
        vec![(0..59, "https://override.example".to_owned())],
    )];
    let links = detect_all_links(&texts, md_hyperlinks, None, None);

    assert_eq!(
        urls_in(&links, output_line(0)),
        vec![(0..59, "https://override.example".to_owned())]
    );
}
