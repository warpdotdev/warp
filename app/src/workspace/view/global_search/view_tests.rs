use string_offset::ByteOffset;
use warp_ripgrep::search::Submatch;

use super::{GlobalSearchView, MAX_STORED_LINE_TEXT_BYTES};

fn submatch(byte_start: usize, byte_end: usize) -> Submatch {
    Submatch {
        byte_start: ByteOffset::from(byte_start),
        byte_end: ByteOffset::from(byte_end),
    }
}

#[test]
fn short_line_text_is_stored_unchanged() {
    let line = "fn main() {}";
    let submatches = vec![submatch(3, 7)];

    let (stored, stored_submatches) =
        GlobalSearchView::truncate_line_text_for_storage(line, &submatches);

    assert_eq!(stored, line);
    assert_eq!(stored_submatches[0].byte_start.as_usize(), 3);
    assert_eq!(stored_submatches[0].byte_end.as_usize(), 7);
}

/// Reproduces the reported memory blowup: a single extremely long line (e.g. a
/// minified/bundled file) must not be stored in full, regardless of where in the
/// line the match sits.
#[test]
fn extremely_long_single_line_is_truncated_to_a_bounded_size() {
    let prefix = "x".repeat(5_000_000);
    let suffix = "y".repeat(5_000_000);
    let line = format!("{prefix}TARGET{suffix}");
    let match_start = prefix.len();
    let match_end = match_start + "TARGET".len();
    let submatches = vec![submatch(match_start, match_end)];

    let (stored, stored_submatches) =
        GlobalSearchView::truncate_line_text_for_storage(&line, &submatches);

    // The stored snippet must be tiny compared to the original ~10 MB line, not just
    // "less than the full multi-megabyte string".
    assert!(stored.len() <= MAX_STORED_LINE_TEXT_BYTES + 8);
    assert!(stored.len() < line.len() / 1000);

    // The submatch offsets must still point at "TARGET" within the truncated text.
    assert_eq!(stored_submatches.len(), 1);
    let sub = &stored_submatches[0];
    assert_eq!(
        &stored[sub.byte_start.as_usize()..sub.byte_end.as_usize()],
        "TARGET"
    );
}

#[test]
fn truncation_omits_prefix_ellipsis_when_match_is_near_the_start() {
    let line = format!("TARGET{}", "z".repeat(MAX_STORED_LINE_TEXT_BYTES * 4));
    let submatches = vec![submatch(0, 6)];

    let (stored, stored_submatches) =
        GlobalSearchView::truncate_line_text_for_storage(&line, &submatches);

    assert!(!stored.starts_with('…'), "unexpected prefix ellipsis");
    assert!(stored.ends_with('…'), "expected a suffix ellipsis");
    let sub = &stored_submatches[0];
    assert_eq!(
        &stored[sub.byte_start.as_usize()..sub.byte_end.as_usize()],
        "TARGET"
    );
}

#[test]
fn truncation_omits_suffix_ellipsis_when_match_is_near_the_end() {
    let filler = "z".repeat(MAX_STORED_LINE_TEXT_BYTES * 4);
    let line = format!("{filler}TARGET");
    let match_start = filler.len();
    let match_end = match_start + "TARGET".len();
    let submatches = vec![submatch(match_start, match_end)];

    let (stored, stored_submatches) =
        GlobalSearchView::truncate_line_text_for_storage(&line, &submatches);

    assert!(stored.starts_with('…'), "expected a prefix ellipsis");
    assert!(!stored.ends_with('…'), "unexpected suffix ellipsis");
    let sub = &stored_submatches[0];
    assert_eq!(
        &stored[sub.byte_start.as_usize()..sub.byte_end.as_usize()],
        "TARGET"
    );
}

#[test]
fn submatches_far_outside_the_retained_window_are_dropped() {
    let filler = "z".repeat(MAX_STORED_LINE_TEXT_BYTES * 4);
    let line = format!("TARGET_NEAR{filler}TARGET_FAR");
    let near_end = "TARGET_NEAR".len();
    let far_start = "TARGET_NEAR".len() + filler.len();
    let far_end = far_start + "TARGET_FAR".len();
    let submatches = vec![submatch(0, near_end), submatch(far_start, far_end)];

    let (stored, stored_submatches) =
        GlobalSearchView::truncate_line_text_for_storage(&line, &submatches);

    // Only the submatch inside the retained window survives.
    assert_eq!(stored_submatches.len(), 1);
    let sub = &stored_submatches[0];
    assert_eq!(
        &stored[sub.byte_start.as_usize()..sub.byte_end.as_usize()],
        "TARGET_NEAR"
    );
}

#[test]
fn truncation_never_splits_a_multi_byte_character() {
    // '€' is 3 bytes wide, and 3 does not evenly divide `MAX_STORED_LINE_TEXT_BYTES` or
    // half of it, so the naive (unsnapped) window bounds are guaranteed to land
    // mid-character here. Slicing at those bounds would panic if the boundary-snapping
    // logic were wrong or missing.
    let filler: String = "€".repeat(MAX_STORED_LINE_TEXT_BYTES);
    let line = format!("{filler}TARGET{filler}");
    let match_start = filler.len();
    let match_end = match_start + "TARGET".len();
    let submatches = vec![submatch(match_start, match_end)];

    let (stored, stored_submatches) =
        GlobalSearchView::truncate_line_text_for_storage(&line, &submatches);

    let sub = &stored_submatches[0];
    assert_eq!(
        &stored[sub.byte_start.as_usize()..sub.byte_end.as_usize()],
        "TARGET"
    );
}

#[test]
fn truncation_with_no_submatches_anchors_on_the_start_of_the_line() {
    let line = "x".repeat(MAX_STORED_LINE_TEXT_BYTES * 4);

    let (stored, stored_submatches) = GlobalSearchView::truncate_line_text_for_storage(&line, &[]);

    assert!(stored_submatches.is_empty());
    assert!(!stored.starts_with('…'));
    assert!(stored.ends_with('…'));
    assert!(stored.len() <= MAX_STORED_LINE_TEXT_BYTES + 8);
}

/// End-to-end check that a line truncated at ingestion still highlights the correct
/// substring at render time, matching the "highlighting ... stay accurate" requirement.
#[test]
fn highlighting_is_still_accurate_after_ingestion_time_truncation() {
    let prefix = "a".repeat(MAX_STORED_LINE_TEXT_BYTES * 8);
    let line = format!("{prefix}NEEDLE");
    let match_start = prefix.len();
    let match_end = match_start + "NEEDLE".len();
    let submatches = vec![submatch(match_start, match_end)];

    let (stored, stored_submatches) =
        GlobalSearchView::truncate_line_text_for_storage(&line, &submatches);

    let highlight_indices =
        GlobalSearchView::highlight_indices_from_submatches(&stored, &stored_submatches);

    let highlighted: String = stored
        .chars()
        .enumerate()
        .filter(|(idx, _)| highlight_indices.contains(idx))
        .map(|(_, ch)| ch)
        .collect();
    assert_eq!(highlighted, "NEEDLE");
}
