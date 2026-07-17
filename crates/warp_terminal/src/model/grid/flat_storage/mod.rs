//! A space-efficient grid storage implementation optimized for scrollback
//! buffer use-cases.
//!
//! This set of data structures is designed to provide space- and CPU-efficient
//! support for the following operations:
//!
//! * `Index`
//! * `Scan`/`Iterate`
//! * `Push`
//! * `Pop`
//!
//! Notably, `Insert`` is not in the above list, as inserting something in the
//! middle of a flat array is relatively expensive (requires shifting all
//! data after the insertion point).  That said, for grids that are immutable,
//! or for the portion of a grid that cannot be accessed via the cursor, this
//! structure provides great performance without compromising on space
//! efficiency.

mod attribute_map;
mod content;
mod grapheme;
mod hyperlink;
mod index;
mod row_iterator;
mod style;
#[cfg(test)]
mod testing;

use std::rc::Rc;

use attribute_map::AttributeMap;
use content::Content;
use get_size::GetSize;
use grapheme::Grapheme;
use hyperlink::HyperlinkIdMap;
use index::Index;
use itertools::Itertools;
use string_offset::ByteOffset;
use style::BgAndStyle;

use super::row::Row;
use super::{cell, CellType};
use crate::model::grid::HyperlinkId;
use crate::model::{ansi, Point};

const DEFAULT_FG_COLOR: ansi::Color = ansi::Color::Named(ansi::NamedColor::Foreground);

/// A grid storage implementation that stores content in a flat buffer.
#[derive(Debug, Clone, GetSize)]
pub struct FlatStorage {
    /// The grid content.
    content: Content,

    /// A helper structure for mapping a row index to an offset into the
    /// content buffer.
    index: Index,

    /// The width of the grid.
    columns: usize,

    /// An interval map storing information about cell fg color.
    fg_color_map: style::FgColorMap,

    /// An interval map storing additional styling information.
    bg_and_style_map: style::BgAndStyleMap,

    /// An interval map storing each cell's OSC 8 hyperlink id (or `None`
    /// for cells that aren't part of a hyperlink span). RLE deduplicates
    /// long runs of cells sharing the same id into a single entry. The
    /// id resolves to a `Hyperlink` via the owning grid's
    /// `HyperlinkRegistry`.
    hyperlink_id_map: HyperlinkIdMap,

    /// The content offset with the end of prompt marker, if any.
    end_of_prompt_marker: Option<EndOfPromptMarker>,

    /// The maximum number of rows that can be stored.
    max_rows: Option<usize>,

    /// The number of rows that were truncated due to the `max_rows` limit.
    num_truncated_rows: u64,
}

impl FlatStorage {
    /// Constructs a new [`FlatStorage`].
    ///
    /// `initial_capacity` can be provided to minimize heap allocations
    /// performed while building backing data structures.
    pub fn new(columns: usize, max_rows: Option<usize>, initial_capacity: Option<usize>) -> Self {
        let index = Index::new(columns, initial_capacity);
        Self {
            content: Content::new(),
            index,
            columns,
            fg_color_map: AttributeMap::new(DEFAULT_FG_COLOR),
            bg_and_style_map: AttributeMap::new(BgAndStyle::default()),
            hyperlink_id_map: AttributeMap::new(None),
            end_of_prompt_marker: None,
            max_rows,
            num_truncated_rows: 0,
        }
    }

    /// Pushes new rows into storage.
    pub fn push_rows<'a>(&mut self, rows: impl IntoIterator<Item = &'a Row>) {
        self.push_rows_internal(&mut rows.into_iter());

        // If we've exceeded the maximum number of rows, drop the excess.
        self.apply_max_rows();
    }

    /// Pushes new rows into storage without applying max row limits.
    ///
    /// This should be used for cases where we may temporarily exceed the
    /// maximum number of rows, such as hybrid grid resizing.
    pub fn push_rows_without_truncation<'a>(&mut self, rows: impl IntoIterator<Item = &'a Row>) {
        self.push_rows_internal(&mut rows.into_iter());
    }

    /// Applies the maximum row limit to the grid.
    ///
    /// This should be called after running logic that uses
    /// `push_rows_without_truncation` to ensure that we end up in a state
    /// where the maximum row limit is applied.
    pub fn apply_max_rows(&mut self) {
        if let Some(num_excess_rows) = self
            .index
            .len()
            .checked_sub(self.max_rows.unwrap_or(usize::MAX))
        {
            self.truncate_rows_front(num_excess_rows);
        }
    }

    /// Pops the last `count` rows off of storage and returns them.
    ///
    /// May return fewer than `count` rows if there are fewer rows stored.
    pub fn pop_rows(&mut self, count: usize) -> Vec<Row> {
        let start_row = self.total_rows().saturating_sub(count);

        // Materialize the rows that we're popping off.
        let rows = self
            .rows_from(start_row)
            .map(Rc::unwrap_or_clone)
            .collect_vec();

        // Truncate internal data structures to exclude the rows we're
        // popping off.
        self.truncate(start_row);

        rows
    }

    /// Truncates internal data structures based on the length of the grid (in
    /// rows).
    fn truncate(&mut self, new_len: usize) {
        // Truncate the index.
        let new_content_len = self.index.truncate(new_len);
        // Using the new content length, truncate other internal structures.
        self.content.truncate(new_content_len);
        self.fg_color_map.truncate(new_content_len);
        self.bg_and_style_map.truncate(new_content_len);
        self.hyperlink_id_map.truncate(new_content_len);

        // Clear out the end-of-prompt marker if it was in a row we just
        // popped.
        match self.end_of_prompt_marker {
            Some(EndOfPromptMarker { offset, .. }) if offset >= new_content_len => {
                self.end_of_prompt_marker = None;
            }
            _ => {}
        }
    }

    /// Drops the first `count` rows from storage.
    pub fn truncate_rows_front(&mut self, count: usize) {
        if count == 0 {
            return;
        }

        // Make sure we don't truncate more rows than we have.
        let count = count.min(self.total_rows());

        let new_start_offset = self.index.truncate_front(count);

        self.content.truncate_front(new_start_offset);
        self.fg_color_map.truncate_front(new_start_offset);
        self.bg_and_style_map.truncate_front(new_start_offset);
        self.hyperlink_id_map.truncate_front(new_start_offset);

        self.num_truncated_rows += count as u64;
    }

    /// Pushes new rows into storage.
    ///
    /// This contains the actual logic, taking in a non-generic [`Iterator`] to
    /// avoid creating copies of all of the code in this function.
    fn push_rows_internal(&mut self, rows: &mut dyn Iterator<Item = &Row>) {
        let mut fg_color = self.fg_color_map.tail();
        let mut bg_and_style = self.bg_and_style_map.tail();
        let mut hyperlink_id: Option<HyperlinkId> = self.hyperlink_id_map.tail();

        for row in rows {
            let start_offset = ByteOffset::from(self.content().end_offset());
            let mut entry_builder = self.index.start_row();

            let mut last_cell: isize = -1;

            // Use an empty but pre-allocated buffer to collect characters from
            // the cells.
            let mut offset = start_offset;

            // We track index manually here instead of creating an iterator and
            // using enumerate as this is slightly more performant.
            let mut idx: isize = -1;
            for cell in row.dirty_cells() {
                idx += 1;

                // Skip over cells that don't contain any actual content.
                if cell.flags().intersects(
                    cell::Flags::WIDE_CHAR_SPACER | cell::Flags::LEADING_WIDE_CHAR_SPACER,
                ) {
                    if cell
                        .flags()
                        .intersects(cell::Flags::LEADING_WIDE_CHAR_SPACER)
                    {
                        entry_builder.add_leading_wide_char_spacer();
                    }
                    last_cell = idx;
                    continue;
                }

                let mut needs_processing = !cell.is_empty();
                if cell.fg != fg_color {
                    needs_processing = true;
                    fg_color = cell.fg;
                    self.fg_color_map.push_attribute_change(offset.., fg_color);
                }
                if bg_and_style != cell {
                    needs_processing = true;
                    bg_and_style = cell.into();
                    self.bg_and_style_map
                        .push_attribute_change(offset.., bg_and_style);
                }
                if hyperlink_id != cell.hyperlink_id() {
                    needs_processing = true;
                    hyperlink_id = cell.hyperlink_id();
                    self.hyperlink_id_map
                        .push_attribute_change(offset.., hyperlink_id);
                }
                if let Some(marker) = cell.end_of_prompt_marker() {
                    needs_processing = true;
                    self.end_of_prompt_marker = Some(EndOfPromptMarker {
                        offset,
                        has_extra_trailing_newline: marker.has_extra_trailing_newline,
                    });
                }

                let grapheme = Grapheme::new_from_cell(cell);
                offset += grapheme.len().as_usize();

                if needs_processing {
                    for _ in last_cell..(idx - 1) {
                        // We skipped a bunch of empty cells, but having hit a
                        // content-ful cell, we need to add them back in.
                        entry_builder
                            .process_grapheme_info_unchecked(Grapheme::EMPTY_CELL.sizing_info());
                        self.content.push_grapheme(&Grapheme::EMPTY_CELL);
                    }
                    last_cell = idx;
                    entry_builder.process_grapheme_info_unchecked(grapheme.sizing_info());
                    self.content.push_grapheme(&grapheme);
                }
            }

            // If the grid row soft wraps, the last cell will be marked
            // with the WRAPLINE flag.
            let row_soft_wraps = row.occ == self.columns
                && row[self.columns - 1]
                    .flags()
                    .intersects(cell::Flags::WRAPLINE);
            if !row_soft_wraps {
                entry_builder.add_trailing_newline();
                self.content.push_grapheme(&Grapheme::NEWLINE);
            }

            entry_builder.append_to_index(&mut self.index);
        }
    }

    /// Clears out the contents of flat storage.
    pub fn clear(&mut self) {
        // Drop all rows from the index.
        self.truncate(0);
    }

    /// Updates the width of the grid.
    pub fn set_columns(&mut self, new_columns: usize) {
        if self.columns == new_columns {
            return;
        }

        self.columns = new_columns;
        // Rebuild the index to account for the updated width.
        self.index = Index::rebuild(&self.index, new_columns);
    }

    /// Returns the total number of rows in the grid.
    pub fn total_rows(&self) -> usize {
        self.index.len()
    }

    /// Returns the maximum number of rows that can be stored.
    pub fn max_rows(&self) -> Option<usize> {
        self.max_rows
    }

    /// Sets the maximum number of rows that can be stored.
    #[cfg(any(test, feature = "test-util"))]
    pub fn set_max_rows(&mut self, max_rows: Option<usize>) {
        self.max_rows = max_rows;
    }

    /// Returns the number of rows that were truncated due to exceeding the
    /// `max_rows` limit.
    pub fn num_truncated_rows(&self) -> u64 {
        self.num_truncated_rows
    }

    /// Returns an iterator over rows in the grid, starting at the given row.
    pub fn rows_from(&self, start_row: usize) -> impl Iterator<Item = Rc<Row>> + '_ {
        row_iterator::RowIterator::new(self, start_row)
    }

    /// Returns the structure holding all of the grid's string content.
    fn content(&self) -> &Content {
        &self.content
    }

    /// Returns an estimate of the structure's total memory usage, in bytes.
    pub fn estimated_memory_usage_bytes(&self) -> usize {
        self.get_size()
    }

    /// Returns the content [`ByteOffset`] for the given point.
    ///
    /// Returns an error if the point is outside the bounds of the structure or
    /// points at an empty cell after the end of a hard-wrapped line.
    pub fn content_offset_at_point(
        &self,
        point: Point,
    ) -> Result<ByteOffset, index::ContentOffsetToPointError> {
        self.index.content_offset_at_point(point)
    }

    /// Returns the grid [`Point`] where the content at a given offset is
    /// located.
    ///
    /// Returns an error if:
    /// 1. The content offset is smaller or larger than the stored content, or
    /// 2. The content offset points at something that doesn't map to a
    ///    particular cell, such as a newline character.
    pub fn content_offset_to_point(
        &self,
        offset: ByteOffset,
    ) -> Result<Point, index::PointFromContentOffsetError> {
        self.index.content_offset_to_point(offset)
    }

    /// Returns the type of the cell at (row, col).
    pub fn cell_type(&self, row: usize, col: usize) -> Option<CellType> {
        self.index.cell_type(row, col)
    }

    pub fn row_wraps(&self, row: usize) -> bool {
        self.index
            .get_entry(row)
            .is_some_and(|entry| !entry.has_trailing_newline)
    }
}

/// Information about the end-of-prompt marker.
#[derive(Debug, Copy, Clone)]
struct EndOfPromptMarker {
    /// The content offset of the marker.
    offset: ByteOffset,
    /// Whether or not the prompt has an extra newline after its content.
    has_extra_trailing_newline: bool,
}

/// [`GetSize`] is entirely stack-allocated, so the default impl is
/// sufficient.
impl GetSize for EndOfPromptMarker {}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
