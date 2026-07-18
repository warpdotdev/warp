//! Char-cell display-row projection: the single implementation of "which
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
//! The public entry point is
//! [`CharCellState::display_lattice`](super::CharCellState::display_lattice):
//! it projects the wrap tables and overlays once into a [`DisplayLattice`],
//! which owns the row list and answers every query against those same rows.

use std::cell::Ref;
use std::ops::Range;

use string_offset::CharOffset;
use warpui_core::text::TuiGridPoint;

use super::{CharCellTemporaryBlock, CharCellTextIndex};

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
    pub char_range: Range<CharOffset>,
    /// Whether this is a soft-wrap continuation of the previous row.
    pub is_continuation: bool,
}

/// The display-row projection at one snapshot of wrap tables, ghosts, and
/// hidden line ranges: the row list plus the inputs it was computed from.
///
/// Callers can mix row iteration and point queries freely without
/// re-projecting. The lattice owns the immutable borrow guards for the
/// char-cell state it was projected from, so every query describes the same
/// rows.
pub struct DisplayLattice<'a> {
    rows: Vec<DisplayRow>,
    text_index: Ref<'a, CharCellTextIndex>,
    terminal_width: u16,
    ghosts: Ref<'a, Vec<CharCellTemporaryBlock>>,
    hidden_line_ranges: Vec<Range<usize>>,
}

impl<'a> DisplayLattice<'a> {
    pub(super) fn new(
        text_index: Ref<'a, CharCellTextIndex>,
        terminal_width: u16,
        ghosts: Ref<'a, Vec<CharCellTemporaryBlock>>,
        hidden_line_ranges: &[Range<usize>],
    ) -> Self {
        let hidden_line_ranges = normalize_hidden_line_ranges(hidden_line_ranges);
        let rows = display_rows(&text_index, terminal_width, &ghosts, &hidden_line_ranges);
        Self {
            rows,
            text_index,
            terminal_width,
            ghosts,
            hidden_line_ranges,
        }
    }

    /// The projected display rows, one per terminal row.
    pub fn rows(&self) -> &[DisplayRow] {
        &self.rows
    }

    /// The ghost blocks that `Ghost` rows' `ghost_index` values index into.
    pub fn ghosts(&self) -> &[CharCellTemporaryBlock] {
        &self.ghosts
    }

    /// The display columns occupied by the clamped buffer character `range`.
    pub fn display_width(&self, range: Range<CharOffset>) -> u16 {
        let start = range
            .start
            .as_usize()
            .min(self.text_index.char_widths.len());
        let end = range
            .end
            .as_usize()
            .clamp(start, self.text_index.char_widths.len());
        self.text_index.char_widths[start..end]
            .iter()
            .fold(0u16, |width, &next| width.saturating_add(u16::from(next)))
    }

    /// The [`TuiGridPoint`] of the gap before 0-based `char_offset`.
    ///
    /// Returns `None` when the offset is inside a hidden line. A deferred-wrap
    /// cursor at the end of a line that exactly fills the width lands on the
    /// next *buffer* row, or one past the entire display when no buffer row
    /// follows — never on an interleaved ghost or gap row, which holds no
    /// buffer gap for a cursor. Callers sizing a viewport must accommodate
    /// that phantom row.
    pub fn offset_to_display_point(&self, char_offset: CharOffset) -> Option<TuiGridPoint> {
        let line_index = self
            .text_index
            .line_starts
            .partition_point(|&start| start <= char_offset)
            .saturating_sub(1);
        if line_is_hidden(&self.hidden_line_ranges, line_index) {
            return None;
        }

        let line_range = self.text_index.logical_line_char_range(line_index);
        let char_index = char_offset.as_usize().min(line_range.end);
        let visual_row = self
            .text_index
            .visual_row_for_offset(line_index, char_offset);
        let row_range = self
            .text_index
            .visual_row_char_range(line_index, visual_row);
        let col = self.text_index.char_widths[row_range.start..char_index]
            .iter()
            .map(|&width| width as usize)
            .sum::<usize>();
        let mut row_within_line = visual_row - self.text_index.line_visual_row_starts[line_index];
        let mut display_col = col as u16;
        if char_index == line_range.end
            && self.terminal_width > 0
            && col == self.terminal_width as usize
        {
            row_within_line += 1;
            display_col = 0;
        }

        // The line's display rows are contiguous by construction.
        let mut line_rows = self.rows.iter().enumerate().filter(|(_, row)| {
            matches!(row.kind, DisplayRowKind::Buffer { line_index: l } if l == line_index)
        });
        let (first_row, _) = line_rows.next()?;
        let last_row = line_rows.next_back().map_or(first_row, |(index, _)| index);
        if row_within_line <= last_row - first_row {
            return Some(TuiGridPoint {
                row: first_row + row_within_line,
                col: display_col,
            });
        }

        // Deferred wrap: the cursor sits one row past the line's last row.
        // Skip past any interleaved ghost/gap rows to the next buffer row, or
        // one past the entire display when none follows.
        let row = self.rows[last_row + 1..]
            .iter()
            .position(|row| matches!(row.kind, DisplayRowKind::Buffer { .. }))
            .map_or(self.rows.len(), |offset| last_row + 1 + offset);
        Some(TuiGridPoint {
            row,
            col: display_col,
        })
    }

    /// The 0-based character offset of the gap at `point`.
    ///
    /// Returns `None` for ghost, gap, and out-of-range rows because they have
    /// no corresponding buffer offset.
    pub fn display_point_to_offset(&self, point: TuiGridPoint) -> Option<CharOffset> {
        let row = self.rows.get(point.row)?;
        match &row.kind {
            DisplayRowKind::Buffer { .. } => {
                // Walk the row's per-char widths to the gap at or just before
                // `point.col`, clamped to the row's end.
                let target_col = point.col as usize;
                let mut col = 0usize;
                let mut offset = row.char_range.start;
                while offset < row.char_range.end {
                    let width = self.text_index.char_widths[offset.as_usize()] as usize;
                    if col + width > target_col {
                        break;
                    }
                    col += width;
                    offset += 1;
                }
                Some(offset)
            }
            DisplayRowKind::Ghost { .. } | DisplayRowKind::Gap { .. } => None,
        }
    }
}

fn normalize_hidden_line_ranges(ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut ranges: Vec<_> = ranges
        .iter()
        .filter(|range| range.start < range.end)
        .cloned()
        .collect();
    ranges.sort_by_key(|range| range.start);

    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn line_is_hidden(ranges: &[Range<usize>], line_index: usize) -> bool {
    let candidate = ranges.partition_point(|range| range.end <= line_index);
    ranges
        .get(candidate)
        .is_some_and(|range| range.start <= line_index)
}

/// Projects the wrap tables + overlays into the flat display-row list
/// described in the module docs. Ghosts always render, even when their insert
/// position falls inside a hidden range (they represent changed content),
/// splitting the gap.
fn display_rows(
    text_index: &CharCellTextIndex,
    terminal_width: u16,
    ghosts: &[CharCellTemporaryBlock],
    hidden_line_ranges: &[Range<usize>],
) -> Vec<DisplayRow> {
    let mut rows = Vec::new();
    let mut pending_ghosts = ghosts.iter().enumerate().peekable();
    let mut hidden_ranges = hidden_line_ranges.iter().peekable();
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
                    char_range: CharOffset::zero().empty_range(),
                    is_continuation: false,
                });
            }
        };

    for line_index in 0..text_index.line_starts.len() {
        while hidden_ranges
            .peek()
            .is_some_and(|range| range.end <= line_index)
        {
            hidden_ranges.next();
        }
        let hidden = hidden_ranges
            .peek()
            .is_some_and(|range| range.start <= line_index);
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
            push_buffer_line_rows(&mut rows, line_index, text_index);
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

/// Appends the wrapped rows of buffer line `line_index`.
fn push_buffer_line_rows(
    rows: &mut Vec<DisplayRow>,
    line_index: usize,
    text_index: &CharCellTextIndex,
) {
    let line_rows = text_index.logical_line_visual_rows(line_index);
    for (row, visual_row) in line_rows.enumerate() {
        let range = text_index.visual_row_char_range(line_index, visual_row);
        rows.push(DisplayRow {
            kind: DisplayRowKind::Buffer { line_index },
            char_range: CharOffset::range(range),
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
    let mut cached_rows = ghost.wrapped_row_starts.borrow_mut();
    if cached_rows
        .as_ref()
        .is_none_or(|(width, _)| *width != terminal_width)
    {
        let mut row_starts = cached_rows
            .take()
            .map(|(_, row_starts)| row_starts)
            .unwrap_or_default();
        super::char_cell_line_row_starts_into(
            &ghost.line_breaks,
            &ghost.char_widths,
            terminal_width,
            &mut row_starts,
        );
        *cached_rows = Some((terminal_width, row_starts));
    }
    let row_starts = &cached_rows.as_ref().unwrap().1;
    for (row, &start) in row_starts.iter().enumerate() {
        let end = row_starts
            .get(row + 1)
            .copied()
            .unwrap_or(ghost.char_widths.len());
        rows.push(DisplayRow {
            kind: DisplayRowKind::Ghost { ghost_index },
            char_range: CharOffset::range(start..end),
            is_continuation: row > 0,
        });
    }
}

#[cfg(test)]
#[path = "char_cell_display_tests.rs"]
mod tests;
