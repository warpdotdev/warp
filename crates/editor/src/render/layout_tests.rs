//! Tests for the per-line shaping cap applied before text is handed to the layout cache.
//!
//! These cover the cap's logic directly. Whether the shaped frame actually shrinks is not
//! assertable here: the test platform's `layout_text` ignores its input and returns an empty
//! frame, so any end-to-end assertion about shaped length would pass with the cap removed.

use warpui_core::fonts::{FamilyId, Properties};
use warpui_core::text_layout::{StyleAndFont, TextStyle};

use super::{MAX_LAYOUT_LINE_CHARS, clamp_style_runs_for_layout, truncate_text_for_layout};

fn test_style() -> StyleAndFont {
    StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::new())
}

#[test]
fn test_truncate_text_for_layout_leaves_text_at_the_cap_untouched() {
    let at_cap = "a".repeat(MAX_LAYOUT_LINE_CHARS);
    assert_eq!(truncate_text_for_layout(&at_cap).len(), at_cap.len());

    let under_cap = "a".repeat(MAX_LAYOUT_LINE_CHARS - 1);
    assert_eq!(truncate_text_for_layout(&under_cap).len(), under_cap.len());
}

#[test]
fn test_truncate_text_for_layout_caps_text_one_char_over() {
    let over_cap = "a".repeat(MAX_LAYOUT_LINE_CHARS + 1);
    let truncated = truncate_text_for_layout(&over_cap);

    assert_eq!(truncated.chars().count(), MAX_LAYOUT_LINE_CHARS);
    assert!(over_cap.starts_with(truncated));
}

#[test]
fn test_truncate_text_for_layout_slices_on_a_char_boundary() {
    // Multi-byte chars make the byte pre-check pessimistic: this text is over the cap in bytes but
    // well under it in chars, so it must come back untouched rather than being sliced mid-char.
    let multi_byte = "é".repeat(MAX_LAYOUT_LINE_CHARS);
    assert!(multi_byte.len() > MAX_LAYOUT_LINE_CHARS);
    assert_eq!(
        truncate_text_for_layout(&multi_byte).len(),
        multi_byte.len()
    );

    // Genuinely over the cap in chars: slicing must land on a char boundary, which is implied by
    // the fact that this returns a valid `&str` at all, and must keep exactly the cap in chars.
    let over_cap = "é".repeat(MAX_LAYOUT_LINE_CHARS + 10);
    let truncated = truncate_text_for_layout(&over_cap);
    assert_eq!(truncated.chars().count(), MAX_LAYOUT_LINE_CHARS);
    assert!(truncated.chars().all(|c| c == 'é'));
}

#[test]
fn test_truncate_text_for_layout_keeps_a_grapheme_cluster_boundary_char_intact() {
    // A multi-char grapheme straddling the cap is split between its component chars. That is
    // acceptable (the cap is a memory bound, not a text-segmentation guarantee), but it must still
    // produce valid UTF-8 rather than slicing inside a single char.
    let mut text = "a".repeat(MAX_LAYOUT_LINE_CHARS - 1);
    text.push('e');
    text.push('\u{0301}'); // Combining acute accent: a second char in the same grapheme.
    text.push('z');

    let truncated = truncate_text_for_layout(&text);
    assert_eq!(truncated.chars().count(), MAX_LAYOUT_LINE_CHARS);
    assert!(truncated.ends_with('e'));
}

#[test]
fn test_clamp_style_runs_for_layout_clamps_a_run_straddling_the_cap() {
    let style = test_style();
    let runs = vec![
        (0..10, style),
        (10..MAX_LAYOUT_LINE_CHARS + 50, style),
        (
            MAX_LAYOUT_LINE_CHARS + 50..MAX_LAYOUT_LINE_CHARS + 90,
            style,
        ),
    ];

    let clamped = clamp_style_runs_for_layout(&runs);

    // The run starting past the cap is dropped; the straddling run keeps its start and is cut to
    // the cap, so the runs stay in bounds of the truncated text.
    assert_eq!(clamped.len(), 2);
    assert_eq!(clamped[0].0, 0..10);
    assert_eq!(clamped[1].0, 10..MAX_LAYOUT_LINE_CHARS);
}

#[test]
fn test_clamp_style_runs_for_layout_leaves_in_bounds_runs_alone() {
    let style = test_style();
    let runs = vec![(0..10, style), (10..20, style)];

    let clamped = clamp_style_runs_for_layout(&runs);

    assert_eq!(clamped.len(), 2);
    assert_eq!(clamped[0].0, 0..10);
    assert_eq!(clamped[1].0, 10..20);
}

#[test]
fn test_clamp_style_runs_for_layout_drops_a_run_starting_exactly_at_the_cap() {
    let style = test_style();
    let runs = vec![
        (0..MAX_LAYOUT_LINE_CHARS, style),
        (MAX_LAYOUT_LINE_CHARS..MAX_LAYOUT_LINE_CHARS + 1, style),
    ];

    let clamped = clamp_style_runs_for_layout(&runs);

    // The cap is exclusive as an index, so a run starting at it covers no shaped text.
    assert_eq!(clamped.len(), 1);
    assert_eq!(clamped[0].0, 0..MAX_LAYOUT_LINE_CHARS);
}
