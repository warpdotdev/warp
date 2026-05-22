//! Logic related to indexing a grid's content by (soft-wrapped) row.
//!
//! The index provides an efficient way to map from a point in the grid to the
//! content offset at which that point's content begins.  Its design allows for
//! efficient reconstruction with a different number of columns, without any
//! need to re-parse the grid contents.
//!
//! ## Content offsets
//!
//! A content offset is the byte offset of a character in the overall set of
//! content that the grid has _ever_ seen.  When content is removed from the
//! front of the grid, the offset of all remaining content is left unchanged,
//! allowing us to avoid modifying any of the data structures that are keyed
//! on content offsets.
//!
//! Content offsets are used throughout the flat storage implementation as the
//! primary key for looking up metadata, as they are stable even if rows are
//! dropped from the front or back of the grid.  To this end, the only thing
//! in the entire flat storage implementation that should be keyed on anything
//! other than content offsets is the [`rows`](Index::rows) field of the index.

use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroU16;
use std::ops::Range;

use cfg_if::cfg_if;
use get_size::GetSize;
use string_offset::ByteOffset;
use thiserror::Error;

use super::grapheme::Grapheme;
use crate::model::grid::CellType;
use crate::model::Point;

#[derive(Debug, Clone, GetSize)]
/// A structure to help index into a grid's content by (soft-wrapped) row.
pub struct Index {
    /// A "mapping" from row index to metadata about that row.
    rows: VecDeque<Entry>,
    /// The number of columns in the grid.
    columns: usize,
    /// The total length of the underlying content.
    content_len: usize,
    /// Holds grapheme sizing information for runs with non-uniform sizing.
    ///
    /// Each entry in the map is a row, keyed by its start offset (so that the
    /// map is stable even if rows are dropped from the front).
    grapheme_sizing: BTreeMap<ByteOffset, GraphemeRuns>,
}

/// An entry in the row index.
#[derive(Debug, Clone, Copy, GetSize)]
pub struct Entry {
    /// The offset into the content at which this row's data begins.
    ///
    /// TODO(vorporeal): ByteOffset should probably store a u64, not a usize?
    content_offset: ByteOffset,
    /// Information about the sizing of graphemes in this row.
    grapheme_sizing: GraphemeSizing,
    /// Whether or not the row's backing content includes a trailing newline.
    pub has_trailing_newline: bool,
    /// Whether or not the row ends with a leading wide character spacer (i.e.:
    /// the next row starts with a wide char that there wasn't room for in this
    /// row).
    pub ends_with_leading_wide_char_spacer: bool,
}

// Assert that an `Entry` has the size we expect.
//
// If `Entry` grows in size, it will significantly impact perforamnce due
// to fitting fewer instances in a single 64-byte cache line.
//
// This is smaller on wasm due to it using a 32-bit usize (other platforms
// have a 64-bit usize).
cfg_if! {
    if #[cfg(target_family = "wasm")] {
        static_assertions::assert_eq_size!(Entry, [u8; 16]);
    } else {
        static_assertions::assert_eq_size!(Entry, [u8; 24]);
    }
}

impl Index {
    /// Creates a new empty index for a grid with the given number of columns.
    ///
    /// `initial_capacity` can be provided in order to reduce the likelihood
    /// that additional heap allocations will be necessary as content gets
    /// added to the index.
    pub fn new(columns: usize, initial_capacity: Option<usize>) -> Self {
        Self {
            rows: VecDeque::with_capacity(initial_capacity.unwrap_or_default()),
            columns,
            content_len: 0,
            grapheme_sizing: Default::default(),
        }
    }

    /// Rebuilds an [`Index`] to wrap lines at a different number of columns.
    pub fn rebuild(old_index: &Index, columns: usize) -> Self {
        let mut index = Self::new(columns, Some(old_index.len()));
        // Update the content length to be the start offset of the first row,
        // to ensure we properly handle resizing after truncation.
        index.content_len = old_index
            .rows
            .front()
            .map(|entry| entry.content_offset)
            .unwrap_or_default()
            .as_usize();

        let mut entry_builder = EntryBuilder::new();

        // Loop over rows in the old index, processing each grapheme in order
        // and adding newlines where appropriate.
        //
        // TODO(vorporeal): This can be significantly optimized - processing
        // each grapheme individually is a clearly poor choice in the (common)
        // case of a grid that contains only ASCII text.  We could take more
        // advantage of the run-length encoded `GraphemeRun` structure here.
        for row_idx in 0..old_index.len() {
            if let Some(grapheme_infos) = old_index.grapheme_infos_for_row(row_idx) {
                for info in grapheme_infos {
                    entry_builder.process_grapheme_info(info, &mut index);
                }
            }
            if old_index
                .get_entry(row_idx)
                .expect("row should have an entry")
                .has_trailing_newline
            {
                entry_builder.process_grapheme(&Grapheme::NEWLINE, &mut index);
            }
        }

        // Add the final entry to the index.
        //
        // If reflowing the content led to some trailing empty cells being
        // pushed onto a new row, don't add that empty row to the index.
        entry_builder.append_to_index_if_nonempty(&mut index);

        if index.content_len > old_index.content_len {
            log::error!("somehow ended up with too much flat storage content!");
        }

        index
    }

    /// Truncates the index to the given number of rows, returning the new
    /// content length.
    pub fn truncate(&mut self, new_len: usize) -> ByteOffset {
        // Update our content length to be the start of the first row we're truncating.
        let Some(new_content_len) = self.content_offset_for_row(new_len) else {
            // If the new length is longer than our current length, we have no work to do.
            return ByteOffset::from(self.content_len);
        };

        // Truncate the index to the new length.
        self.rows.truncate(new_len);
        // Drop any grapheme sizing metadata for the truncated rows.
        let _ = self.grapheme_sizing.split_off(&new_content_len);

        self.content_len = new_content_len.as_usize();

        new_content_len
    }

    /// Removes the first `count` rows from the index, returning the new start
    /// offset for the remaining content.
    pub fn truncate_front(&mut self, count: usize) -> ByteOffset {
        let new_start_offset = self.content_offset_for_row(count).unwrap_or_else(|| {
            if count > self.rows.len() {
                log::error!(
                    "should not attempt to truncate more rows than exist in flat storage; \
                     have {} rows, trying to truncate {}",
                    self.rows.len(),
                    count
                );
            }
            self.content_len.into()
        });

        for _ in 0..count {
            self.rows.pop_front();
        }
        self.grapheme_sizing = self.grapheme_sizing.split_off(&new_start_offset);

        new_start_offset
    }

    pub fn start_row(&mut self) -> EntryBuilder {
        EntryBuilder::new()
    }

    /// Returns the total number of rows in the index.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns the content [`ByteOffset`] for the given point.
    ///
    /// Returns an error if:
    /// 1. The point is outside the bounds of the structure, or
    /// 2. Points at an empty cell after the end of a hard-wrapped line.
    ///
    /// TODO(vorporeal): Write tests to cover the following cases:
    ///  * Points at valid content
    ///  * Points at content after the end of a hard-wrapped line
    ///  * Points at a WIDE_CHAR_SPACER cell
    ///  * Points at a LEADING_WIDE_CHAR_SPACER cell
    ///  * Points at column 0
    pub fn content_offset_at_point(
        &self,
        point: Point,
    ) -> Result<ByteOffset, ContentOffsetToPointError> {
        let entry =
            self.rows
                .get(point.row)
                .ok_or_else(|| ContentOffsetToPointError::RowOutOfBounds {
                    row: point.row,
                    max_row: self.rows.len().saturating_sub(1),
                })?;

        let runs = match &entry.grapheme_sizing {
            GraphemeSizing::Uniform(grapheme_run) => std::slice::from_ref(grapheme_run),
            GraphemeSizing::NonUniform => self
                .grapheme_sizing
                .get(&entry.content_offset)
                .ok_or(ContentOffsetToPointError::MissingGraphemeSizing {
                    content_offset: entry.content_offset,
                })?
                .as_slice(),
            GraphemeSizing::EmptyRow => {
                if point.col == 0 {
                    return Ok(entry.content_offset);
                } else {
                    return Err(ContentOffsetToPointError::NonZeroColumnInEmptyRow {
                        row: point.row,
                        col: point.col,
                    });
                }
            }
        };

        let mut offset = entry.content_offset;
        let mut cols_remaining = point.col;

        for run in runs {
            if cols_remaining == 0 {
                break;
            }

            let cols_from_run = run.cols().min(cols_remaining);
            let graphemes_from_run = cols_from_run / run.info.cell_width as usize;

            offset += graphemes_from_run * run.info.utf8_bytes.get() as usize;
            cols_remaining -= cols_from_run;
        }

        if cols_remaining == 0 {
            return Ok(offset);
        }

        // If we get to this point, the provided column index exceeded the
        // number of content-ful cells in this row.
        Err(ContentOffsetToPointError::ColumnExceedsContent {
            row: point.row,
            col: point.col,
        })
    }

    pub fn content_offset_to_point(
        &self,
        offset: ByteOffset,
    ) -> Result<Point, PointFromContentOffsetError> {
        let partition = self
            .rows
            .partition_point(|entry| entry.content_offset <= offset);
        let row = match partition.checked_sub(1) {
            Some(r) => r,
            None => {
                let first_row_offset = self
                    .rows
                    .front()
                    .map(|e| e.content_offset)
                    .unwrap_or_default();
                return Err(PointFromContentOffsetError::OffsetBeforeFirstRow {
                    offset,
                    first_row_offset,
                });
            }
        };

        let entry = self
            .get_entry(row)
            .ok_or(PointFromContentOffsetError::RowOutOfBounds { row })?;

        let runs = match &entry.grapheme_sizing {
            GraphemeSizing::Uniform(grapheme_run) => std::slice::from_ref(grapheme_run),
            GraphemeSizing::NonUniform => self
                .grapheme_sizing
                .get(&entry.content_offset)
                .ok_or(PointFromContentOffsetError::MissingGraphemeSizing {
                    content_offset: entry.content_offset,
                })?
                .as_slice(),
            GraphemeSizing::EmptyRow => {
                // The only valid content offset for an empty row is the offset
                // of the start of the row.
                assert_eq!(offset, entry.content_offset);
                return Ok(Point { row, col: 0 });
            }
        };

        let mut column = 0;
        let mut remaining_offset = offset - entry.content_offset;

        for run in runs {
            let graphemes_in_run = run.cols() / run.info.cell_width as usize;
            let content_in_run =
                ByteOffset::from(graphemes_in_run * run.info.utf8_bytes.get() as usize);

            let remaining_offset_in_run = remaining_offset.min(content_in_run);
            let remaining_graphemes_in_run =
                remaining_offset_in_run.as_usize() / run.info.utf8_bytes.get() as usize;
            let remaining_cells_in_run = remaining_graphemes_in_run * run.info.cell_width as usize;

            column += remaining_cells_in_run;
            remaining_offset -= remaining_offset_in_run;

            if remaining_offset == ByteOffset::zero() {
                return Ok(Point { row, col: column });
            }
        }

        #[cfg(debug_assertions)]
        log::warn!(
            "tried to convert content offset to point but was past the end of the content in a row"
        );
        Err(PointFromContentOffsetError::OffsetDoesNotMapToCellInRow { row, offset })
    }

    /// Returns the range of content that represents this row.
    pub fn content_range_for_row(&self, row: usize) -> Option<Range<ByteOffset>> {
        let start = self.content_offset_for_row(row)?;
        let end = self
            .content_offset_for_row(row + 1)
            .unwrap_or(ByteOffset::from(self.content_len));
        Some(start..end)
    }

    /// Returns the byte offset at which the given row's content begins.
    fn content_offset_for_row(&self, row: usize) -> Option<ByteOffset> {
        Some(self.rows.get(row)?.content_offset)
    }

    pub fn get_entry(&self, row: usize) -> Option<&Entry> {
        self.rows.get(row)
    }

    /// Returns the [`CellType`] for the cell at the given (row, col), or
    /// [`None`] if that point is outside of the grid bounds.
    pub fn cell_type(&self, row: usize, col: usize) -> Option<CellType> {
        let entry = self.get_entry(row)?;

        if entry.ends_with_leading_wide_char_spacer && col == self.columns - 1 {
            return Some(CellType::LeadingWideCharSpacer);
        }

        let Some(grapheme_runs) = (match &entry.grapheme_sizing {
            GraphemeSizing::Uniform(run) => {
                // For a row with only wide characters, make sure blank
                // space at the end of the line isn't counted as a wide
                // character.
                if col >= run.cols() {
                    return Some(CellType::RegularChar);
                }
                return run.cell_type_at_offset(col);
            }
            GraphemeSizing::NonUniform => self.grapheme_sizing.get(&entry.content_offset),
            GraphemeSizing::EmptyRow => return Some(CellType::RegularChar),
        }) else {
            log::error!(
                "Found entry with non-uniform grapheme sizing and no grapheme run information!"
            );
            return None;
        };

        let mut start_col: usize = 0;
        for run in grapheme_runs.iter() {
            let run_end_col = start_col + run.cols();
            if run_end_col > col {
                return run.cell_type_at_offset(col - start_col);
            }
            start_col = run_end_col;
        }

        // If the column is part of the blank space at the end of a
        // hard-wrapped line, we should treat it as a narrow char.
        Some(CellType::RegularChar)
    }

    /// Returns a slice of grapheme runs for the given row.
    ///
    /// Returns [`None`] if the provided row index is out-of-bounds.
    pub(super) fn grapheme_runs_for_row(&self, row_idx: usize) -> Option<&[GraphemeRun]> {
        let entry = self.get_entry(row_idx)?;

        let runs = match &entry.grapheme_sizing {
            GraphemeSizing::Uniform(grapheme_run) => std::slice::from_ref(grapheme_run),
            GraphemeSizing::NonUniform => {
                self.grapheme_sizing.get(&entry.content_offset)?.as_slice()
            }
            GraphemeSizing::EmptyRow => &[],
        };

        Some(runs)
    }

    /// Returns an iterator over the sizing information for each individual
    /// grapheme in the given row.
    ///
    /// Returns [`None`] if the provided row index is out-of-bounds.
    pub fn grapheme_infos_for_row(
        &self,
        row_idx: usize,
    ) -> Option<impl Iterator<Item = GraphemeInfo> + '_> {
        let runs = self.grapheme_runs_for_row(row_idx)?;

        Some(
            runs.iter()
                .flat_map(|run| std::iter::repeat_n(run.info, run.count.get() as usize)),
        )
    }
}

/// Errors that can occur when converting a point to a content offset.
#[derive(Debug, Error)]
pub enum ContentOffsetToPointError {
    /// The point's row is outside the bounds of the index.
    #[error("Point row {row} is outside the bounds of the index (max: {max_row})")]
    RowOutOfBounds { row: usize, max_row: usize },
    /// Missing grapheme sizing data for a non-uniform row.
    #[error("Missing grapheme sizing data for non-uniform row at content offset {content_offset}")]
    MissingGraphemeSizing { content_offset: ByteOffset },
    /// Point column is not 0 for an empty row.
    #[error("Point column {col} is not 0 for empty row {row}")]
    NonZeroColumnInEmptyRow { row: usize, col: usize },
    /// Point column exceeds the number of content cells in the row.
    #[error("Point column {col} exceeds the number of content cells in row {row}")]
    ColumnExceedsContent { row: usize, col: usize },
}

/// Errors that can occur when converting a content offset to a point.
#[derive(Debug, Error)]
pub enum PointFromContentOffsetError {
    /// The provided offset is before the start of the first row.
    #[error("Offset {offset} is before the start of the first row (first row starts at {first_row_offset})")]
    OffsetBeforeFirstRow {
        offset: ByteOffset,
        first_row_offset: ByteOffset,
    },
    /// The computed row index was out of bounds.
    #[error("Computed row index {row} is out of bounds")]
    RowOutOfBounds { row: usize },
    /// Missing grapheme sizing data for a non-uniform row.
    #[error("Missing grapheme sizing data for non-uniform row at content offset {content_offset}")]
    MissingGraphemeSizing { content_offset: ByteOffset },
    /// The provided offset does not map to a cell in the computed row.
    #[error("Content offset {offset} does not map to a cell in row {row}")]
    OffsetDoesNotMapToCellInRow { row: usize, offset: ByteOffset },
}

/// A helper structure for building up an [`Entry`] while iterating through a
/// list of `Cell`s in a `Row`.
#[derive(Default)]
pub struct EntryBuilder {
    num_cells: usize,
    incr_content_offset: ByteOffset,
    has_trailing_newline: bool,
    ends_with_leading_wide_char_spacer: bool,
    #[cfg(debug_assertions)]
    was_processed: bool,
    grapheme_runs: GraphemeRuns,
}

impl EntryBuilder {
    fn new() -> Self {
        Default::default()
    }

    /// Processes the next [`Grapheme`] in the row.
    pub fn process_grapheme(&mut self, grapheme: &Grapheme, index: &mut Index) {
        if grapheme.starts_new_row() {
            self.add_trailing_newline();
            std::mem::take(self).append_to_index(index);
            return;
        }

        self.process_grapheme_info(grapheme.sizing_info(), index);
    }

    /// Processes the next grapheme in the row, based only on its sizing
    /// information (and not its content).
    fn process_grapheme_info(&mut self, info: GraphemeInfo, index: &mut Index) {
        let grapheme_len = info.utf8_bytes.get() as usize;
        debug_assert!(
            grapheme_len > 0,
            "should not process an empty string as a grapheme"
        );

        if info.cell_width == 0 {
            #[cfg(debug_assertions)]
            log::error!("encountered unexpected grapheme with a computed cell width of zero!");
            return;
        }
        debug_assert!(
            info.cell_width <= 2,
            "graphemes should not be more than two cells wide, but encountered one with width {}",
            info.cell_width
        );

        // If there isn't enough room in the row for this grapheme, cut off
        // the row here, starting the new row with the _current_ grapheme.
        if self.num_cells + info.cell_width as usize > index.columns {
            // If this is a non-full row and we've got a wide char, mark
            // the fact that we have a leading wide char spacer.
            if info.cell_width > 1 && self.num_cells != index.columns {
                self.add_leading_wide_char_spacer();
            }
            std::mem::take(self).append_to_index(index);
            debug_assert_eq!(self.incr_content_offset, ByteOffset::zero());
        }

        self.num_cells += info.cell_width as usize;

        self.process_grapheme_info_unchecked(info);
    }

    /// Processes the next grapheme in the row, without performing any checks
    /// around whether or not the row is full.
    ///
    /// This is intended to be used when building up an [`Entry`] from an
    /// existing [`Row`], as the row can't have more cells than fit in it.
    ///
    /// Callers will need to invoke [`Self::add_leading_wide_char_spacer`] and
    /// [`Self::append_to_index`] as appropriate.
    pub fn process_grapheme_info_unchecked(&mut self, info: GraphemeInfo) {
        let grapheme_len = info.utf8_bytes.get() as usize;

        self.incr_content_offset += grapheme_len;

        // Store information about this grapheme's cell width and UTF-8 length.
        match self.grapheme_runs.last_mut() {
            Some(last_run) if last_run.info == info => {
                // TODO(vorporeal): might be able to eke out some extra performance
                // if we remove the error checking here.
                last_run.count = last_run
                    .count
                    .checked_add(1)
                    .expect("should not have more than 2^16 graphemes in a single row");
            }
            _ => {
                self.grapheme_runs.push(GraphemeRun {
                    count: unsafe { NonZeroU16::new_unchecked(1) },
                    info,
                });
            }
        }
    }

    /// Marks the [`Entry`]'s row as containing a trailing newline.
    pub fn add_trailing_newline(&mut self) {
        self.incr_content_offset += '\n'.len_utf8();
        self.has_trailing_newline = true;
    }

    /// Marks the [`Entry`]'s row as ending with a leading wide-char spacer
    /// (i.e.: a wide char was wrapped to the next line due to there only being
    /// one cell of space).
    pub fn add_leading_wide_char_spacer(&mut self) {
        self.ends_with_leading_wide_char_spacer = true;
    }

    /// Builds an [`Entry`] and appends it to the provided index, or simply
    /// drops `self` if the [`Entry`] would be empty.
    pub fn append_to_index_if_nonempty(mut self, index: &mut Index) {
        #[cfg(debug_assertions)]
        {
            self.was_processed = true;
        }

        if !self.is_empty() {
            self.append_to_index(index);
        }
    }

    /// Builds an [`Entry`] and appends it to the provided index.
    pub fn append_to_index(mut self, index: &mut Index) {
        #[cfg(debug_assertions)]
        {
            self.was_processed = true;
        }

        let content_offset = index.content_len.into();

        let grapheme_sizing = if self.grapheme_runs.len() == 1 {
            GraphemeSizing::Uniform(
                // SAFETY: Checked the length of self.grapheme_runs above.
                unsafe { self.grapheme_runs.pop().unwrap_unchecked() },
            )
        } else if self.grapheme_runs.is_empty() {
            GraphemeSizing::EmptyRow
        } else {
            index
                .grapheme_sizing
                .insert(content_offset, std::mem::take(&mut self.grapheme_runs));
            GraphemeSizing::NonUniform
        };

        index.content_len += self.incr_content_offset.as_usize();
        index.rows.push_back(Entry {
            content_offset,
            grapheme_sizing,
            has_trailing_newline: self.has_trailing_newline,
            ends_with_leading_wide_char_spacer: self.ends_with_leading_wide_char_spacer,
        });
    }

    fn is_empty(&self) -> bool {
        self.incr_content_offset == ByteOffset::zero()
            && !self.has_trailing_newline
            && !self.ends_with_leading_wide_char_spacer
            && self.grapheme_runs.is_empty()
    }
}

impl Drop for EntryBuilder {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.was_processed,
            "EntryBuilder must be processed before it is dropped"
        );
    }
}

/// Run-length encoded information about grapheme sizes.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(super) struct GraphemeRun {
    /// The number of consecutive graphemes for which `info` is accurate.
    count: NonZeroU16,
    /// Metadata that applies to each grapheme in this run.
    info: GraphemeInfo,
}

impl GraphemeRun {
    fn cols(&self) -> usize {
        self.count.get() as usize * self.info.cell_width as usize
    }

    fn cell_type_at_offset(&self, offset: usize) -> Option<CellType> {
        if self.info.cell_width == 1 {
            Some(CellType::RegularChar)
        } else {
            assert!(
                offset < self.cols(),
                "cannot compute cell type for offset {offset} in run that spans {} columns",
                self.cols()
            );
            if offset.is_multiple_of(2) {
                Some(CellType::WideChar)
            } else {
                Some(CellType::WideCharSpacer)
            }
        }
    }
}

/// [`GraphemeRun`] is entirely stack-allocated, so the default impl is
/// sufficient.
impl GetSize for GraphemeRun {}

/// Type alias for a list of grapheme runs.
type GraphemeRuns = Vec<GraphemeRun>;

/// Information about sizing of graphemes in a single grid row.
#[derive(Debug, Copy, Clone, PartialEq)]
enum GraphemeSizing {
    /// All graphemes in the row have the same sizing information.
    Uniform(GraphemeRun),
    /// Grapheme sizing is non-uniform, with the details stored in the index's
    /// `grapheme_sizing` map.
    NonUniform,
    /// The row contains no graphemes.
    EmptyRow,
}

/// [`GraphemeSizing`] is entirely stack-allocated, so the default impl is
/// sufficient.
impl GetSize for GraphemeSizing {}

/// Metadata about a grapheme.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct GraphemeInfo {
    /// The width of the grapheme, in cells.
    pub cell_width: u8,
    /// The length, in bytes, of this grapheme using a UTF-8 encoding.
    pub utf8_bytes: NonZeroU16,
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
