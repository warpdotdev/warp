use super::*;

#[test]
fn normalize_verb_trims_whitespace() {
    assert_eq!(
        normalize_warping_verb("   cooking up a plan   "),
        Some("cooking up a plan".to_owned())
    );
}

#[test]
fn normalize_verb_preserves_single_word_casing() {
    assert_eq!(
        normalize_warping_verb("warping"),
        Some("warping".to_owned())
    );
}

#[test]
fn normalize_verb_preserves_phrase_casing() {
    assert_eq!(
        normalize_warping_verb("going to infinity"),
        Some("going to infinity".to_owned())
    );
}

#[test]
fn normalize_verb_preserves_existing_acronym_casing() {
    assert_eq!(
        normalize_warping_verb("checking NASA docs"),
        Some("checking NASA docs".to_owned())
    );
}

#[test]
fn normalize_verb_drops_empty_and_whitespace_only() {
    assert_eq!(normalize_warping_verb(""), None);
    assert_eq!(normalize_warping_verb("    "), None);
    assert_eq!(normalize_warping_verb("\t\n"), None);
}

#[test]
fn normalize_verb_strips_trailing_dots_and_ellipsis() {
    assert_eq!(
        normalize_warping_verb("Thinking..."),
        Some("Thinking".to_owned())
    );
    assert_eq!(
        normalize_warping_verb("Thinking…"),
        Some("Thinking".to_owned())
    );
    assert_eq!(
        normalize_warping_verb("Thinking....."),
        Some("Thinking".to_owned())
    );
}

#[test]
fn normalize_verb_returns_none_when_only_dots() {
    assert_eq!(normalize_warping_verb("..."), None);
    assert_eq!(normalize_warping_verb("…"), None);
}

#[test]
fn normalize_verb_truncates_long_entries_at_char_boundary() {
    // 60 chars, should be truncated to 40.
    let long = "Abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwx";
    let result = normalize_warping_verb(long).unwrap();
    assert_eq!(result.chars().count(), MAX_WARPING_VERB_CHARS);
    assert!(long.starts_with(&result));
}

#[test]
fn normalize_verb_respects_multibyte_chars() {
    // Many multi-byte chars (é, …) — should not panic or split a codepoint.
    let verb: String = "é".repeat(50);
    let result = normalize_warping_verb(&verb).unwrap();
    assert_eq!(result.chars().count(), MAX_WARPING_VERB_CHARS);
}

#[test]
fn normalize_verbs_caps_list_length_and_drops_empties() {
    let input: Vec<String> = (0..(MAX_CUSTOM_WARPING_VERBS + 10))
        .map(|i| format!("Verb {i}"))
        .chain(std::iter::once("   ".to_owned()))
        .chain(std::iter::once(String::new()))
        .collect();
    let out = normalize_warping_verbs(input);
    assert_eq!(out.len(), MAX_CUSTOM_WARPING_VERBS);
    // The first entry should match our untrimmed `Verb 0`.
    assert_eq!(out[0], "Verb 0");
}

#[test]
fn format_for_display_appends_ellipsis() {
    assert_eq!(format_for_display("thinking"), "thinking...");
    assert_eq!(format_for_display("cooking"), "cooking...");
    assert_eq!(
        format_for_display("going to infinity"),
        "going to infinity..."
    );
}

#[test]
fn format_for_display_preserves_existing_punctuation() {
    assert_eq!(format_for_display("are you sure?"), "are you sure?");
    assert_eq!(format_for_display("working hard!"), "working hard!");
    assert_eq!(format_for_display("thinking..."), "thinking...");
    assert_eq!(format_for_display("thinking…"), "thinking…");
}

#[test]
fn format_for_display_falls_back_for_empty_input() {
    assert_eq!(format_for_display(""), "Warping...");
    assert_eq!(format_for_display("    "), "Warping...");
}

#[test]
fn pick_verb_with_single_entry_returns_that_entry() {
    let verbs = vec!["only".to_owned()];
    // Even when `previous` matches, a single-entry list can only return that entry.
    assert_eq!(pick_verb(&verbs, None), "only");
    assert_eq!(pick_verb(&verbs, Some("only")), "only");
}

#[test]
fn pick_verb_avoids_previous_when_alternatives_exist() {
    let verbs = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    // Run many times to catch any bias; should never return the previous value
    // while alternatives exist.
    for _ in 0..100 {
        let picked = pick_verb(&verbs, Some("a"));
        assert_ne!(picked, "a");
        assert!(verbs.contains(&picked));
    }
}

#[test]
fn pick_verb_returns_a_valid_entry_when_previous_absent() {
    let verbs = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    for _ in 0..20 {
        let picked = pick_verb(&verbs, None);
        assert!(verbs.contains(&picked));
    }
}

#[test]
fn selector_keeps_same_display_for_same_session() {
    let selector = WarpingVerbSelector::new();
    let verbs = vec!["a".to_owned(), "b".to_owned()];

    let first = selector.resolve_from_verbs("stream-1", &verbs);

    for _ in 0..20 {
        assert_eq!(selector.resolve_from_verbs("stream-1", &verbs), first);
    }
}

#[test]
fn selector_keeps_same_display_when_verbs_change_during_session() {
    let selector = WarpingVerbSelector::new();
    let first_verbs = vec!["a".to_owned(), "b".to_owned()];
    let changed_verbs = vec!["c".to_owned(), "d".to_owned()];

    let first = selector.resolve_from_verbs("stream-1", &first_verbs);

    assert_eq!(
        selector.resolve_from_verbs("stream-1", &changed_verbs),
        first
    );
}

#[test]
fn selector_picks_new_display_for_new_session_when_alternatives_exist() {
    let selector = WarpingVerbSelector::new();
    let verbs = vec!["a".to_owned(), "b".to_owned()];

    let first = selector.resolve_from_verbs("stream-1", &verbs);
    let second = selector.resolve_from_verbs("stream-2", &verbs);

    assert_ne!(second, first);
}

#[test]
fn selector_uses_updated_verbs_for_new_session() {
    let selector = WarpingVerbSelector::new();
    let first_verbs = vec!["a".to_owned(), "b".to_owned()];
    let changed_verbs = vec!["c".to_owned(), "d".to_owned()];

    let first = selector.resolve_from_verbs("stream-1", &first_verbs);

    assert_eq!(
        selector.resolve_from_verbs("stream-1", &changed_verbs),
        first
    );
    assert!(["c...", "d..."].contains(
        &selector
            .resolve_from_verbs("stream-2", &changed_verbs)
            .as_str()
    ));
}

#[test]
fn selector_normalizes_raw_setting_values_before_display() {
    let selector = WarpingVerbSelector::new();
    let raw = format!("  {}  ", "a".repeat(MAX_WARPING_VERB_CHARS + 10));

    let display = selector.resolve_from_verbs("stream-1", &[raw]);
    let raw_display = display.trim_end_matches("...");

    assert_eq!(raw_display.chars().count(), MAX_WARPING_VERB_CHARS);
    assert!(raw_display.starts_with('a'));
}

#[test]
fn selector_drops_blank_raw_setting_values_before_display() {
    let selector = WarpingVerbSelector::new();
    let verbs = vec!["   ".to_owned(), "...".to_owned()];

    assert_eq!(
        selector.resolve_from_verbs("stream-1", &verbs),
        DEFAULT_WARPING_VERB
    );
}

#[test]
fn selector_invalidates_normalized_cache_when_raw_verbs_change_after_empty_list() {
    let selector = WarpingVerbSelector::new();
    let empty_verbs = vec!["   ".to_owned(), "...".to_owned()];
    let changed_verbs = vec!["  Working...  ".to_owned()];

    assert_eq!(
        selector.resolve_from_verbs("stream-1", &empty_verbs),
        DEFAULT_WARPING_VERB
    );
    assert_eq!(
        selector.resolve_from_verbs("stream-2", &changed_verbs),
        "Working..."
    );
}
