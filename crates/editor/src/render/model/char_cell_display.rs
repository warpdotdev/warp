//! Char-cell display-row functions: the single implementation of "which
//! terminal rows does a char-cell editor occupy, and where is everything on
//! them" once structural overlays are applied.
//!
//! A *display row* is one terminal row of the rendered editor, produced from:
//!
//! - soft-wrapped buffer rows (the same width-aware wrap math as the rest of
//!   the char-cell path),
//! - ghost rows ([`CharCellTemporaryBlock`]s — e.g. removed diff lines)
//!   interleaved before their `insert_before` buffer line, themselves wrapped
//!   at the same width,
//! - hidden line ranges elided into single gap rows (interior gaps only;
//!   leading/trailing hidden runs produce no rows).
//!
//! Rows are style- and text-free: they carry char *ranges* (into the buffer
//! text or a ghost's content), never strings or colors. Both consumers — the
//! TUI editor element's painting and interaction geometry (cursor placement,
//! mouse hit-testing) — are projections of this one computation, so what is
//! painted on row N and what a click on row N resolves to can never disagree.
//!
//! Display-row space vs buffer visual-row space: the softwrap functions
//! ([`char_cell_offset_to_softwrap_point`](super::char_cell_offset_to_softwrap_point)
//! and friends) describe soft-wrapped *buffer* rows only and are what cursor
//! navigation uses. With no ghosts and no hidden ranges the two spaces are
//! identical.
//!
//! Like the softwrap functions, these are free functions over the wrap-table
//! slices; the public entry points are the thin borrowing delegates on
//! [`CharCellState`](super::CharCellState).

use std::ops::Range;

use super::{
    CharCellTemporaryBlock, char_cell_display_width, char_cell_line_gap_position,
    char_cell_line_row_starts, char_cell_logical_line,
};

/// What a display row was projected from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayRowKind {
    /// A (wrapped row of a) logical buffer line.
    Buffer {
        /// 0-based logical line index.
        line_index: usize,
    },
    /// A (wrapped row of a) ghost line — content not present in the buffer.
    Ghost {
        /// Index into the ghost slice (`CharCellState::temporary_blocks`).
        ghost_index: usize,
    },
    /// A run of elided buffer lines between visible content. Carries no
    /// content; consumers render their own separator (e.g. `… N lines`).
    Gap {
        /// The 0-based logical lines this gap elides.
        line_range: Range<usize>,
    },
}

/// One terminal row of the display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayRow {
    pub kind: DisplayRowKind,
    /// 0-based char range of this row's content: into the buffer text for
    /// `Buffer` rows, into the ghost's `content` for `Ghost` rows, empty for
    /// `Gap` rows.
    pub char_range: Range<usize>,
    /// Whether this is a soft-wrap continuation of the previous row.
    pub is_continuation: bool,
}

/// Projects the wrap tables + overlays into the flat display-row list
/// described in the module docs. Ghosts always render, even when their insert
/// position falls inside a hidden range (they represent changed content),
/// splitting the gap.
pub(super) fn display_rows(
    line_starts: &[usize],
    char_widths: &[u8],
    terminal_width: u16,
    ghosts: &[CharCellTemporaryBlock],
    hidden_line_ranges: &[Range<usize>],
) -> Vec<DisplayRow> {
    let mut rows = Vec::new();
    let mut pending_ghosts = ghosts.iter().enumerate().peekable();
    // Hidden lines accumulated since the last visible row; materialized as a
    // Gap row only when more visible content follows (interior gaps).
    let mut pending_hidden: Option<Range<usize>> = None;
    let mut emitted_visible = false;

    let flush_gap =
        |rows: &mut Vec<DisplayRow>, pending: &mut Option<Range<usize>>, emitted: bool| {
            // `take` runs unconditionally: a leading (nothing-emitted) hidden
            // run is dropped, not deferred.
            if let Some(line_range) = pending.take()
                && emitted
            {
                rows.push(DisplayRow {
                    kind: DisplayRowKind::Gap { line_range },
                    char_range: 0..0,
                    is_continuation: false,
                });
            }
        };

    for line_index in 0..line_starts.len() {
        let hidden = hidden_line_ranges
            .iter()
            .any(|range| range.contains(&line_index));
        let has_ghosts_here = pending_ghosts
            .peek()
            .is_some_and(|(_, ghost)| (ghost.insert_before.as_u32() as usize) <= line_index);

        if has_ghosts_here || !hidden {
            flush_gap(&mut rows, &mut pending_hidden, emitted_visible);
        }

        while let Some((ghost_index, ghost)) = pending_ghosts.peek() {
            if (ghost.insert_before.as_u32() as usize) > line_index {
                break;
            }
            push_ghost_rows(&mut rows, *ghost_index, ghost, terminal_width);
            emitted_visible = true;
            pending_ghosts.next();
        }

        if hidden {
            match &mut pending_hidden {
                Some(range) => range.end = line_index + 1,
                None => pending_hidden = Some(line_index..line_index + 1),
            }
        } else {
            push_buffer_line_rows(
                &mut rows,
                line_index,
                line_starts,
                char_widths,
                terminal_width,
            );
            emitted_visible = true;
        }
    }

    // Ghosts positioned at/after the end of the buffer (e.g. a deletion at
    // EOF) still render; a preceding hidden run becomes an interior gap.
    if pending_ghosts.peek().is_some() {
        flush_gap(&mut rows, &mut pending_hidden, emitted_visible);
        for (ghost_index, ghost) in pending_ghosts {
            push_ghost_rows(&mut rows, ghost_index, ghost, terminal_width);
        }
    }

    rows
}

/// The `(display_row, display_col)` of the gap before 0-based char index
/// `char_idx`, in display-row space.
///
/// Offsets inside hidden lines resolve to their gap row (or clamp to the
/// nearest display row when the hidden run produced no gap). A deferred-wrap
/// cursor at the end of a row that exactly fills the width lands one row past
/// the line's last display row, mirroring [`char_cell_line_gap_position`];
/// callers sizing a viewport must accommodate that phantom row.
pub(super) fn offset_to_display_point(
    char_idx: usize,
    line_starts: &[usize],
    char_widths: &[u8],
    terminal_width: u16,
    ghosts: &[CharCellTemporaryBlock],
    hidden_line_ranges: &[Range<usize>],
) -> (u32, u16) {
    let line_index = line_starts
        .partition_point(|&start| start <= char_idx)
        .saturating_sub(1);
    let rows = display_rows(
        line_starts,
        char_widths,
        terminal_width,
        ghosts,
        hidden_line_ranges,
    );

    if hidden_line_ranges
        .iter()
        .any(|range| range.contains(&line_index))
    {
        let gap_row = rows.iter().position(|row| {
            matches!(&row.kind, DisplayRowKind::Gap { line_range } if line_range.contains(&line_index))
        });
        let row = gap_row.unwrap_or_else(|| rows.len().saturating_sub(1));
        return (row as u32, 0);
    }

    let line_start = line_starts.get(line_index).copied().unwrap_or(0);
    let line = char_cell_logical_line(line_starts, char_widths, line_index);
    let (row_within_line, col) =
        char_cell_line_gap_position(line, terminal_width, char_idx - line_start);

    let first_row_of_line = rows
        .iter()
        .position(
            |row| matches!(row.kind, DisplayRowKind::Buffer { line_index: l } if l == line_index),
        )
        .unwrap_or(rows.len());
    (first_row_of_line as u32 + row_within_line, col)
}

/// The 0-based char index of the gap at `(display_row, display_col)` — the
/// inverse of [`offset_to_display_point`] for buffer rows.
///
/// Non-buffer rows resolve to the *nearest buffer offset* so mouse drags have
/// sensible semantics: ghost rows map to their insert position's line start,
/// gap rows to the start of their first hidden line, and rows past the end of
/// the display to the end of the buffer.
pub(super) fn display_point_to_offset(
    display_row: u32,
    display_col: usize,
    line_starts: &[usize],
    char_widths: &[u8],
    terminal_width: u16,
    ghosts: &[CharCellTemporaryBlock],
    hidden_line_ranges: &[Range<usize>],
) -> usize {
    let rows = display_rows(
        line_starts,
        char_widths,
        terminal_width,
        ghosts,
        hidden_line_ranges,
    );
    let Some(row) = rows.get(display_row as usize) else {
        return char_widths.len();
    };
    match &row.kind {
        DisplayRowKind::Buffer { .. } => {
            // Walk the row's per-char widths to the gap at or just before
            // `display_col`, clamped to the row's end.
            let mut col = 0usize;
            let mut idx = row.char_range.start;
            while idx < row.char_range.end {
                let width = char_widths[idx] as usize;
                if col + width > display_col {
                    break;
                }
                col += width;
                idx += 1;
            }
            idx
        }
        DisplayRowKind::Ghost { ghost_index } => {
            let insert_before = ghosts[*ghost_index].insert_before.as_u32() as usize;
            line_starts
                .get(insert_before)
                .copied()
                .unwrap_or(char_widths.len())
        }
        DisplayRowKind::Gap { line_range } => line_starts
            .get(line_range.start)
            .copied()
            .unwrap_or(char_widths.len()),
    }
}

/// Appends the wrapped rows of buffer line `line_index`.
fn push_buffer_line_rows(
    rows: &mut Vec<DisplayRow>,
    line_index: usize,
    line_starts: &[usize],
    char_widths: &[u8],
    terminal_width: u16,
) {
    let line_start = line_starts[line_index].min(char_widths.len());
    let line = char_cell_logical_line(line_starts, char_widths, line_index);
    let row_starts = char_cell_line_row_starts(line, terminal_width);
    for (row, &start) in row_starts.iter().enumerate() {
        let end = row_starts.get(row + 1).copied().unwrap_or(line.len());
        rows.push(DisplayRow {
            kind: DisplayRowKind::Buffer { line_index },
            char_range: (line_start + start)..(line_start + end),
            is_continuation: row > 0,
        });
    }
}

/// Appends the wrapped rows of a ghost line, wrapped at the same width and
/// with the same wide-char rules as buffer rows. A trailing newline in the
/// ghost's content is a line separator, not content (diff removed-line blocks
/// conventionally carry one), so it is excluded like buffer lines exclude
/// theirs.
fn push_ghost_rows(
    rows: &mut Vec<DisplayRow>,
    ghost_index: usize,
    ghost: &CharCellTemporaryBlock,
    terminal_width: u16,
) {
    let content = ghost.content.strip_suffix('\n').unwrap_or(&ghost.content);
    let widths: Vec<u8> = content
        .chars()
        .map(|c| char_cell_display_width(c) as u8)
        .collect();
    let row_starts = char_cell_line_row_starts(&widths, terminal_width);
    for (row, &start) in row_starts.iter().enumerate() {
        let end = row_starts.get(row + 1).copied().unwrap_or(widths.len());
        rows.push(DisplayRow {
            kind: DisplayRowKind::Ghost { ghost_index },
            char_range: start..end,
            is_continuation: row > 0,
        });
    }
}

#[cfg(test)]
#[path = "char_cell_display_tests.rs"]
mod tests;
