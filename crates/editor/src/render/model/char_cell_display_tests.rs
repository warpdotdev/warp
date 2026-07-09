use std::ops::Range;

use string_offset::CharOffset;

use super::super::{CharCellState, CharCellTemporaryBlock, LineCount};
use super::{DisplayPoint, DisplayRow, DisplayRowKind};

/// A `CharCellState` with wrap tables built for `text`, the public entry
/// point for everything under test.
fn state(text: &str, terminal_width: u16) -> CharCellState {
    let state = CharCellState::new(terminal_width, None);
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

/// The lattice's rows at `hidden`, for row-structure assertions.
fn rows(state: &CharCellState, hidden: &[Range<usize>]) -> Vec<DisplayRow> {
    state.display_lattice(hidden).rows().to_vec()
}

/// `offset_to_display_point` as a bare `(row, col)` pair.
fn point(state: &CharCellState, char_idx: usize, hidden: &[Range<usize>]) -> Option<(u32, u16)> {
    state
        .display_lattice(hidden)
        .offset_to_display_point(CharOffset::from(char_idx))
        .map(|point| (point.row, point.col))
}

/// `display_point_to_offset` from a bare `(row, col)` pair.
fn offset(
    state: &CharCellState,
    row: u32,
    col: u16,
    hidden: &[Range<usize>],
) -> Option<CharOffset> {
    state
        .display_lattice(hidden)
        .display_point_to_offset(DisplayPoint { row, col })
}

/// `(kind, char_range, is_continuation)` triples for compact assertions.
fn summarize(rows: &[DisplayRow]) -> Vec<(DisplayRowKind, Range<CharOffset>, bool)> {
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
fn char_range(range: Range<usize>) -> Range<CharOffset> {
    CharOffset::range(range)
}

#[test]
fn plain_text_wraps_with_char_ranges() {
    // Width 4: "abcdef" wraps into chars 0..4 + 4..6; "gh" starts at char 7.
    let state = state("abcdef\ngh", 4);
    assert_eq!(
        summarize(&rows(&state, &[])),
        vec![
            (buffer(0), char_range(0..4), false),
            (buffer(0), char_range(4..6), true),
            (buffer(1), char_range(7..9), false),
        ]
    );
}

#[test]
fn ghosts_interleave_before_their_line_and_wrap() {
    let state = state("line0\nline1", 9);
    // The first ghost's trailing '\n' is a line separator (removed-line
    // blocks conventionally carry one), not content: it must not add a
    // column or an extra wrapped row.
    state.set_temporary_blocks(vec![ghost("removed a\n", 1), ghost("removed b!!", 1)]);
    assert_eq!(
        summarize(&rows(&state, &[])),
        vec![
            (buffer(0), char_range(0..5), false),
            (
                DisplayRowKind::Ghost { ghost_index: 0 },
                char_range(0..9),
                false,
            ),
            // The second ghost is 11 chars: wraps at width 9.
            (
                DisplayRowKind::Ghost { ghost_index: 1 },
                char_range(0..9),
                false,
            ),
            (
                DisplayRowKind::Ghost { ghost_index: 1 },
                char_range(9..11),
                true,
            ),
            (buffer(1), char_range(6..11), false),
        ]
    );
}

#[test]
fn interior_hidden_ranges_become_gaps_edges_render_nothing() {
    // Lines 0-1 hidden (leading), 3-5 hidden (interior), 7 hidden (trailing).
    let state = state("l0\nl1\nl2\nl3\nl4\nl5\nl6\nl7", 20);
    assert_eq!(
        summarize(&rows(&state, &[0..2, 3..6, 7..8])),
        vec![
            (buffer(2), char_range(6..8), false),
            (
                DisplayRowKind::Gap { line_range: 3..6 },
                CharOffset::zero().empty_range(),
                false,
            ),
            (buffer(6), char_range(18..20), false),
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
        summarize(&rows(&state, &hidden)),
        vec![
            (buffer(0), char_range(0..2), false),
            (
                DisplayRowKind::Gap { line_range: 1..3 },
                CharOffset::zero().empty_range(),
                false,
            ),
            (
                DisplayRowKind::Ghost { ghost_index: 0 },
                char_range(0..7),
                false,
            ),
            (
                DisplayRowKind::Gap { line_range: 3..5 },
                CharOffset::zero().empty_range(),
                false,
            ),
            (buffer(5), char_range(15..17), false),
        ]
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
        assert_eq!(point(&state, 9, &hidden), Some((3, 0)));
        assert_eq!(point(&state, 10, &hidden), Some((3, 1)));
        assert_eq!(offset(&state, 3, 0, &hidden), Some(CharOffset::from(9)));
        assert_eq!(offset(&state, 3, 1, &hidden), Some(CharOffset::from(10)));

        // Line 0 is unaffected by overlays below it.
        assert_eq!(point(&state, 0, &hidden), Some((0, 0)));
        assert_eq!(offset(&state, 0, 1, &hidden), Some(CharOffset::from(1)));

        // Hidden offsets and synthetic display rows have no exact inverse.
        assert_eq!(point(&state, 4, &hidden), None);
        assert_eq!(offset(&state, 2, 0, &hidden), None);

        // Ghost rows do not correspond to buffer offsets.
        assert_eq!(offset(&state, 1, 4, &hidden), None);

        // Points past the display have no corresponding buffer offset.
        assert_eq!(offset(&state, 99, 0, &hidden), None);
    }

    #[test]
    fn deferred_wrap_cursor_skips_non_buffer_rows() {
        // "abcd" at width 4: the end-of-buffer cursor wraps to a phantom row
        // one past the single text row.
        let state = state("abcd", 4);
        assert_eq!(rows(&state, &[]).len(), 1);
        assert_eq!(point(&state, 4, &[]), Some((1, 0)));

        // With a ghost at EOF the cursor cannot sit on the ghost row; it
        // lands one past the entire display (which also pins that EOF ghosts
        // render at all — the post-loop flush).
        state.set_temporary_blocks(vec![ghost("rm", 1)]);
        assert_eq!(rows(&state, &[]).len(), 2);
        assert_eq!(point(&state, 4, &[]), Some((2, 0)));

        // On an interior line that exactly fills the width, the cursor skips
        // the interleaved ghost and lands on the next buffer row.
        state.update_text("abcd\nef");
        // Rows: 0=line0, 1=ghost, 2=line1.
        assert_eq!(point(&state, 4, &[]), Some((2, 0)));
    }

    #[test]
    fn visual_row_char_range_follows_softwrap_rows() {
        // Width 4: "abcdef" wraps into 0..4 + 4..6; "gh" is 7..9.
        let state = state("abcdef\ngh", 4);
        assert_eq!(
            state.visual_row_char_range(CharOffset::from(0)),
            char_range(0..4)
        );
        assert_eq!(
            state.visual_row_char_range(CharOffset::from(3)),
            char_range(0..4)
        );
        assert_eq!(
            state.visual_row_char_range(CharOffset::from(4)),
            char_range(4..6)
        );
        // The trailing newline is excluded from the row's range.
        assert_eq!(
            state.visual_row_char_range(CharOffset::from(5)),
            char_range(4..6)
        );
        assert_eq!(
            state.visual_row_char_range(CharOffset::from(7)),
            char_range(7..9)
        );
    }
}
