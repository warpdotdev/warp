use std::ops::Range;

use super::super::{CharCellState, CharCellTemporaryBlock, LineCount};
use super::{DisplayRow, DisplayRowKind};

/// A `CharCellState` with wrap tables built for `text`, the public entry
/// point for everything under test.
fn state(text: &str, terminal_width: u16) -> CharCellState {
    let state = CharCellState::new(terminal_width);
    state.update_text(text);
    state
}

fn ghost(content: &str, insert_before: usize) -> CharCellTemporaryBlock {
    CharCellTemporaryBlock {
        content: content.to_string(),
        insert_before: LineCount::from(insert_before),
        line_decoration: None,
        inline_decorations: Vec::new(),
    }
}

/// `(kind, char_range, is_continuation)` triples for compact assertions.
fn summarize(rows: &[DisplayRow]) -> Vec<(DisplayRowKind, Range<usize>, bool)> {
    rows.iter()
        .map(|row| {
            (
                row.kind.clone(),
                row.char_range.clone(),
                row.is_continuation,
            )
        })
        .collect()
}

fn buffer(line_index: usize) -> DisplayRowKind {
    DisplayRowKind::Buffer { line_index }
}

#[test]
fn plain_text_wraps_with_char_ranges() {
    // Width 4: "abcdef" wraps into chars 0..4 + 4..6; "gh" starts at char 7.
    let state = state("abcdef\ngh", 4);
    assert_eq!(
        summarize(&state.display_rows(&[])),
        vec![
            (buffer(0), 0..4, false),
            (buffer(0), 4..6, true),
            (buffer(1), 7..9, false),
        ]
    );
}

#[test]
fn empty_lines_keep_one_row() {
    let state = state("a\n\nb", 10);
    assert_eq!(
        summarize(&state.display_rows(&[])),
        vec![
            (buffer(0), 0..1, false),
            (buffer(1), 2..2, false),
            (buffer(2), 3..4, false),
        ]
    );
}

#[test]
fn wide_chars_wrap_by_display_width() {
    // Width 4 fits two wide chars per row.
    let state = state("你好你好", 4);
    assert_eq!(
        summarize(&state.display_rows(&[])),
        vec![(buffer(0), 0..2, false), (buffer(0), 2..4, true)]
    );
}

#[test]
fn ghosts_interleave_before_their_line_and_wrap() {
    let state = state("line0\nline1", 9);
    state.set_temporary_blocks(vec![ghost("removed a", 1), ghost("removed b!!", 1)]);
    assert_eq!(
        summarize(&state.display_rows(&[])),
        vec![
            (buffer(0), 0..5, false),
            (DisplayRowKind::Ghost { ghost_index: 0 }, 0..9, false),
            // The second ghost is 11 chars: wraps at width 9.
            (DisplayRowKind::Ghost { ghost_index: 1 }, 0..9, false),
            (DisplayRowKind::Ghost { ghost_index: 1 }, 9..11, true),
            (buffer(1), 6..11, false),
        ]
    );
}

#[test]
fn ghost_trailing_newline_is_excluded_from_rows() {
    // Removed-line blocks conventionally end with '\n'; it must not add a
    // column or an extra wrapped row.
    let state = state("kept", 20);
    state.set_temporary_blocks(vec![ghost("old\n", 0)]);
    assert_eq!(
        summarize(&state.display_rows(&[])),
        vec![
            (DisplayRowKind::Ghost { ghost_index: 0 }, 0..3, false),
            (buffer(0), 0..4, false),
        ]
    );
}

#[test]
fn ghost_at_end_of_buffer_renders_after_last_line() {
    let state = state("line0", 20);
    state.set_temporary_blocks(vec![ghost("deleted at eof", 1)]);
    assert_eq!(
        summarize(&state.display_rows(&[])),
        vec![
            (buffer(0), 0..5, false),
            (DisplayRowKind::Ghost { ghost_index: 0 }, 0..14, false),
        ]
    );
}

#[test]
fn interior_hidden_ranges_become_gaps_edges_render_nothing() {
    // Lines 0-1 hidden (leading), 3-5 hidden (interior), 7 hidden (trailing).
    let state = state("l0\nl1\nl2\nl3\nl4\nl5\nl6\nl7", 20);
    assert_eq!(
        summarize(&state.display_rows(&[0..2, 3..6, 7..8])),
        vec![
            (buffer(2), 6..8, false),
            (DisplayRowKind::Gap { line_range: 3..6 }, 0..0, false),
            (buffer(6), 18..20, false),
        ]
    );
}

#[test]
fn ghost_inside_hidden_region_still_renders_and_splits_the_gap() {
    // Lines 1-4 hidden; a ghost inserts before line 3 (inside the hidden run).
    let state = state("l0\nl1\nl2\nl3\nl4\nl5", 20);
    state.set_temporary_blocks(vec![ghost("removed", 3)]);
    // One hidden *range*, not a range of values.
    #[allow(clippy::single_range_in_vec_init)]
    let hidden = [1..5];
    assert_eq!(
        summarize(&state.display_rows(&hidden)),
        vec![
            (buffer(0), 0..2, false),
            (DisplayRowKind::Gap { line_range: 1..3 }, 0..0, false),
            (DisplayRowKind::Ghost { ghost_index: 0 }, 0..7, false),
            (DisplayRowKind::Gap { line_range: 3..5 }, 0..0, false),
            (buffer(5), 15..17, false),
        ]
    );
}

#[test]
fn zero_terminal_width_disables_wrapping() {
    let state = state("abcdef", 0);
    assert_eq!(
        summarize(&state.display_rows(&[])),
        vec![(buffer(0), 0..6, false)]
    );
}

mod geometry {
    use super::*;

    #[test]
    fn offset_round_trips_through_display_point_with_overlays() {
        // Rows: line0 | ghost | gap(1..3) | line3.
        let state = state("l0\nl1\nl2\nl3", 20);
        state.set_temporary_blocks(vec![ghost("removed", 1)]);
        // One hidden *range*, not a range of values.
        #[allow(clippy::single_range_in_vec_init)]
        let hidden = [1..3];

        // Char 9 = 'l' of line3 (chars: l0\n=0..3, l1\n=3..6, l2\n=6..9, l3=9..11).
        // Display rows: 0=line0, 1=ghost, 2=gap, 3=line3.
        assert_eq!(state.offset_to_display_point(9, &hidden), (3, 0));
        assert_eq!(state.offset_to_display_point(10, &hidden), (3, 1));
        assert_eq!(state.display_point_to_offset(3, 0, &hidden), 9);
        assert_eq!(state.display_point_to_offset(3, 1, &hidden), 10);

        // Line 0 is unaffected by overlays below it.
        assert_eq!(state.offset_to_display_point(0, &hidden), (0, 0));
        assert_eq!(state.display_point_to_offset(0, 1, &hidden), 1);
    }

    #[test]
    fn hidden_offsets_resolve_to_their_gap_row() {
        let state = state("l0\nl1\nl2\nl3", 20);
        // One hidden *range*, not a range of values.
        #[allow(clippy::single_range_in_vec_init)]
        let hidden = [1..3];
        // Char 4 is inside hidden line 1; the gap is display row 1.
        assert_eq!(state.offset_to_display_point(4, &hidden), (1, 0));
        // Clicking the gap resolves to the start of its first hidden line.
        assert_eq!(state.display_point_to_offset(1, 0, &hidden), 3);
    }

    #[test]
    fn ghost_rows_resolve_to_their_insert_position() {
        let state = state("l0\nl1", 20);
        state.set_temporary_blocks(vec![ghost("removed", 1)]);
        // Display row 1 is the ghost; nearest buffer offset = start of line 1.
        assert_eq!(state.display_point_to_offset(1, 4, &[]), 3);
    }

    #[test]
    fn points_past_the_display_resolve_to_buffer_end() {
        let state = state("ab", 20);
        assert_eq!(state.display_point_to_offset(5, 0, &[]), 2);
    }

    #[test]
    fn deferred_wrap_cursor_lands_on_phantom_row() {
        // "abcd" at width 4: end-of-buffer cursor wraps to a phantom row 1.
        let state = state("abcd", 4);
        assert_eq!(state.display_rows(&[]).len(), 1);
        assert_eq!(state.offset_to_display_point(4, &[]), (1, 0));
    }

    #[test]
    fn visual_row_char_range_follows_softwrap_rows() {
        // Width 4: "abcdef" wraps into 0..4 + 4..6; "gh" is 7..9.
        let state = state("abcdef\ngh", 4);
        assert_eq!(state.visual_row_char_range(0), 0..4);
        assert_eq!(state.visual_row_char_range(3), 0..4);
        assert_eq!(state.visual_row_char_range(4), 4..6);
        // The trailing newline is excluded from the row's range.
        assert_eq!(state.visual_row_char_range(5), 4..6);
        assert_eq!(state.visual_row_char_range(7), 7..9);
    }
}
