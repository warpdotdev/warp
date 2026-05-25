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

use crate::model::{grid::CellType, Point};

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
        let capacity = if columns > 0 && old_index.columns > 0 {
            old_index.len().saturating_mul(old_index.columns) / columns + 1
        } else {
            old_index.len()
        };
        let mut index = Self::new(columns, Some(capacity));
        index.content_len = old_index
            .rows
            .front()
            .map(|entry| entry.content_offset)
            .unwrap_or_default()
            .as_usize();

        let mut entry_builder = EntryBuilder::new();

        // entries_with_runs() merges the row VecDeque and grapheme_sizing BTreeMap
        // in a single O(n) scan, avoiding a per-row O(log n) BTreeMap lookup.
        for (entry, row_runs) in old_index.entries_with_runs() {
            if entry_builder.is_empty() {
                if entry.has_trailing_newline {
                    // Fast path A: narrowing uniform — arithmetic split avoids
                    // per-grapheme work when the row must be split across many
                    // output rows.
                    if let GraphemeSizing::Uniform(run) = &entry.grapheme_sizing {
                        let cell_width = run.info.cell_width as usize;
                        if run.cols() > columns && cell_width > 0 && columns >= cell_width {
                            emit_narrowed_uniform(run, &mut index);
                            continue;
                        }
                    }
                    // Fast path B: row fits in the new width — memcpy the entry.
                    if try_emit_row_with_newline(entry, row_runs, &mut index) {
                        continue;
                    }
                } else {
                    // Fast path C: soft-wrapped row that fits — absorb into the
                    // builder so it merges with subsequent soft-wrap rows.
                    if try_accumulate_softwrap(entry, row_runs, &mut entry_builder, columns) {
                        continue;
                    }
                }
            }

            // Fast path D: uniform run (cell_width 1 or 2) with carry-over
            // from a previous soft-wrapped row — arithmetic split without
            // per-grapheme processing.
            if let GraphemeSizing::Uniform(run) = &entry.grapheme_sizing {
                if try_emit_carryover_uniform(
                    run,
                    entry.has_trailing_newline,
                    &mut entry_builder,
                    &mut index,
                ) {
                    continue;
                }
            }

            // Medium path: bulk-accumulate runs that fit whole; delegate only
            // the boundary-straddling run to process_graphemes_batch.
            emit_runs(
                row_runs,
                entry.has_trailing_newline,
                &mut entry_builder,
                &mut index,
            );
        }

        entry_builder.append_to_index_if_nonempty(&mut index);

        if index.content_len > old_index.content_len {
            log::error!("somehow ended up with too much flat storage content!");
        }

        index
    }

    /// BASELINE: Simple, obviously-correct rebuild algorithm used as the
    /// reference implementation in differential tests.
    ///
    /// Iterates over each source row's grapheme runs in order and feeds them
    /// into [`EntryBuilder::process_graphemes_batch`] one run at a time.  This
    /// naturally handles soft-wrapping at the new column width without any of
    /// the arithmetic fast-paths found in [`Self::rebuild`], making it easy to
    /// audit for correctness.
    #[cfg(any(test, feature = "test-util"))]
    pub fn rebuild_baseline(old_index: &Index, columns: usize) -> Self {
        let mut index = Self::new(columns, Some(old_index.len()));
        index.content_len = old_index
            .rows
            .front()
            .map(|entry| entry.content_offset)
            .unwrap_or_default()
            .as_usize();

        let mut entry_builder = EntryBuilder::new();

        for (entry, runs) in old_index.entries_with_runs() {
            for run in runs {
                entry_builder.process_graphemes_batch(run.info, run.count.get(), &mut index);
            }
            if entry.has_trailing_newline {
                entry_builder.add_trailing_newline();
                entry_builder.flush_to_index(&mut index);
            }
        }

        entry_builder.append_to_index_if_nonempty(&mut index);
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
        let bounded_count = count.min(self.rows.len());
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

        self.rows.drain(..bounded_count);
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

    /// Returns an iterator over every row's `(Entry, runs)` pair in order.
    ///
    /// Unlike repeated `grapheme_runs_for_row` calls (O(log n) per row due to
    /// BTreeMap lookup), this merges the row VecDeque and the grapheme_sizing
    /// BTreeMap in a single O(n) scan, relying on both being ordered by
    /// content_offset.
    fn entries_with_runs(&self) -> impl Iterator<Item = (&Entry, &[GraphemeRun])> + '_ {
        let mut sizing_iter = self.grapheme_sizing.iter();
        self.rows.iter().map(move |entry| {
            let runs: &[GraphemeRun] = match &entry.grapheme_sizing {
                GraphemeSizing::Uniform(run) => std::slice::from_ref(run),
                GraphemeSizing::NonUniform => sizing_iter.next().map_or(&[], |(_, v)| v.as_slice()),
                GraphemeSizing::EmptyRow => &[],
            };
            (entry, runs)
        })
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
                checked_add_run_count(&mut last_run.count, 1);
            }
            _ => {
                self.grapheme_runs.push(GraphemeRun {
                    count: unsafe { NonZeroU16::new_unchecked(1) },
                    info,
                });
            }
        }
    }

    /// Batch processes multiple graphemes from the same run in one call.
    ///
    /// Uses arithmetic chunk processing (O(rows) not O(graphemes)) and
    /// correctly accounts for bytes per flushed segment — the original
    /// per-grapheme loop accumulated all bytes at the end, which caused
    /// content-offset miscalculation whenever a flush occurred mid-batch.
    fn process_graphemes_batch(&mut self, info: GraphemeInfo, count: u16, index: &mut Index) {
        let grapheme_len = info.utf8_bytes.get() as usize;
        let cell_width = info.cell_width as usize;
        let mut remaining = count as usize;

        while remaining > 0 {
            let space = index.columns.saturating_sub(self.num_cells);
            let fits = if cell_width > 0 {
                space / cell_width
            } else {
                0
            };

            if fits == 0 {
                // No room for even one grapheme — flush and retry.
                if cell_width > 1 && self.num_cells != index.columns {
                    self.add_leading_wide_char_spacer();
                }
                self.flush_to_index(index);

                // If the grapheme is wider than the entire column width it
                // can never fit via normal packing. Emit one grapheme per row
                // (matching process_grapheme_info's behaviour) and move on.
                if cell_width > index.columns {
                    // The initial flush above already emitted the preceding row
                    // (with ends_with_leading_wide_char_spacer set). Now emit
                    // each wide char on its own row; all but the last get the
                    // spacer flag (because the next wide char will "overflow"
                    // onto the following row, mirroring process_grapheme_info).
                    while remaining > 0 {
                        self.num_cells += cell_width;
                        self.incr_content_offset += grapheme_len;
                        self.grapheme_runs.push(GraphemeRun {
                            count: unsafe { NonZeroU16::new_unchecked(1) },
                            info,
                        });
                        if remaining > 1 {
                            self.add_leading_wide_char_spacer();
                        }
                        self.flush_to_index(index);
                        remaining -= 1;
                    }
                    return;
                }
                continue;
            }

            let take = remaining.min(fits);
            // SAFETY: take > 0 because fits > 0 and remaining > 0.
            let take_u16 = unsafe { NonZeroU16::new_unchecked(take as u16) };

            self.num_cells += take * cell_width;
            self.incr_content_offset += take * grapheme_len;

            match self.grapheme_runs.last_mut() {
                Some(last) if last.info == info => {
                    checked_add_run_count(&mut last.count, take_u16.get());
                }
                _ => {
                    self.grapheme_runs.push(GraphemeRun {
                        count: take_u16,
                        info,
                    });
                }
            }

            remaining -= take;

            // If the row is now full and there are more graphemes, flush.
            if self.num_cells >= index.columns && remaining > 0 {
                self.flush_to_index(index);
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

    /// Flushes the current entry to the index and resets in-place,
    /// preserving Vec capacity to avoid reallocation.
    fn flush_to_index(&mut self, index: &mut Index) {
        #[cfg(debug_assertions)]
        {
            self.was_processed = true;
        }

        let content_offset = index.content_len.into();

        let grapheme_sizing = if self.grapheme_runs.len() == 1 {
            // pop() leaves the Vec with capacity intact.
            GraphemeSizing::Uniform(unsafe { self.grapheme_runs.pop().unwrap_unchecked() })
        } else if self.grapheme_runs.is_empty() {
            GraphemeSizing::EmptyRow
        } else {
            // Clone the runs into a right-sized vec for storage, then clear
            // self in-place so the next row reuses the existing allocation
            // and avoids repeated Vec reallocations.
            let runs = self.grapheme_runs.clone();
            self.grapheme_runs.clear();
            index.grapheme_sizing.insert(content_offset, runs);
            GraphemeSizing::NonUniform
        };

        index.content_len += self.incr_content_offset.as_usize();
        index.rows.push_back(Entry {
            content_offset,
            grapheme_sizing,
            has_trailing_newline: self.has_trailing_newline,
            ends_with_leading_wide_char_spacer: self.ends_with_leading_wide_char_spacer,
        });

        // Reset in-place, preserving grapheme_runs capacity.
        self.num_cells = 0;
        self.incr_content_offset = ByteOffset::zero();
        self.has_trailing_newline = false;
        self.ends_with_leading_wide_char_spacer = false;
        #[cfg(debug_assertions)]
        {
            self.was_processed = false;
        }
    }
}

impl Drop for EntryBuilder {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        if !std::thread::panicking() {
            debug_assert!(
                self.was_processed,
                "EntryBuilder must be processed before it is dropped"
            );
        }
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

// ── Rebuild helper functions ──────────────────────────────────────────────────
//
// Each function handles one fast path in `Index::rebuild`.  They are free
// functions (not methods) so they can borrow `index` and `entry_builder`
// independently without fighting the borrow checker.

/// Emits a narrowing uniform run as a sequence of full rows followed by one
/// partial row carrying the trailing newline.
///
/// Preconditions (caller-enforced):
///  - `entry_builder` is empty
///  - the source entry has a trailing newline
///  - `run.cols() > index.columns`
///  - `run.info.cell_width > 0 && index.columns >= run.info.cell_width as usize`
fn emit_narrowed_uniform(run: &GraphemeRun, index: &mut Index) {
    let cell_width = run.info.cell_width as usize;
    let graphemes_per_row = index.columns / cell_width;
    let byte_len = run.info.utf8_bytes.get() as usize;
    let mut rem = run.count.get() as usize;
    // A full row of wide chars leaves a 1-cell gap when columns is odd.
    // The last cell becomes a leading-wide-char-spacer placeholder.
    let full_row_spacer = cell_width == 2 && index.columns % 2 == 1;

    while rem > graphemes_per_row {
        let content_offset: ByteOffset = index.content_len.into();
        index.content_len += graphemes_per_row * byte_len;
        index.rows.push_back(Entry {
            content_offset,
            grapheme_sizing: GraphemeSizing::Uniform(GraphemeRun {
                count: NonZeroU16::new(graphemes_per_row as u16).unwrap(),
                info: run.info,
            }),
            has_trailing_newline: false,
            ends_with_leading_wide_char_spacer: full_row_spacer,
        });
        rem -= graphemes_per_row;
    }
    // The loop condition `rem > graphemes_per_row` guarantees rem >= 1.
    debug_assert!(rem > 0);
    // The final row ends with a trailing newline, not a wide-char overflow,
    // so ends_with_leading_wide_char_spacer is always false here.
    let content_offset: ByteOffset = index.content_len.into();
    index.content_len += rem * byte_len + 1; // +1 for newline
    index.rows.push_back(Entry {
        content_offset,
        grapheme_sizing: GraphemeSizing::Uniform(GraphemeRun {
            count: NonZeroU16::new(rem as u16).unwrap(),
            info: run.info,
        }),
        has_trailing_newline: true,
        ends_with_leading_wide_char_spacer: false,
    });
}

/// Tries to emit a source row that fits entirely within `index.columns`,
/// including its trailing newline.
///
/// Returns `true` if emitted; `false` if the row is wider than the new column
/// count (caller should fall through to the next path).
///
/// Preconditions: `entry_builder` is empty; entry has a trailing newline.
fn try_emit_row_with_newline(entry: &Entry, row_runs: &[GraphemeRun], index: &mut Index) -> bool {
    let (cells, byte_len) = match &entry.grapheme_sizing {
        GraphemeSizing::Uniform(run) => (
            run.cols(),
            run.count.get() as usize * run.info.utf8_bytes.get() as usize,
        ),
        GraphemeSizing::EmptyRow => (0, 0),
        GraphemeSizing::NonUniform => {
            let cells = row_runs.iter().map(GraphemeRun::cols).sum();
            let byte_len = row_runs
                .iter()
                .map(|r| r.count.get() as usize * r.info.utf8_bytes.get() as usize)
                .sum();
            (cells, byte_len)
        }
    };
    if cells > index.columns {
        return false;
    }
    let content_offset: ByteOffset = index.content_len.into();
    index.content_len += byte_len + 1; // +1 for newline
    if matches!(entry.grapheme_sizing, GraphemeSizing::NonUniform) {
        index
            .grapheme_sizing
            .insert(content_offset, row_runs.to_vec());
    }
    index.rows.push_back(Entry {
        content_offset,
        grapheme_sizing: entry.grapheme_sizing,
        has_trailing_newline: true,
        ends_with_leading_wide_char_spacer: false,
    });
    true
}

/// Tries to absorb a soft-wrapped source row into `entry_builder` when the
/// row's cells fit entirely within `columns`.
///
/// Returns `true` if absorbed (caller should `continue` to the next source
/// row); `false` if the row is too wide and needs splitting (fall through).
///
/// Preconditions: `entry_builder` is empty; entry has no trailing newline.
fn try_accumulate_softwrap(
    entry: &Entry,
    row_runs: &[GraphemeRun],
    entry_builder: &mut EntryBuilder,
    columns: usize,
) -> bool {
    match &entry.grapheme_sizing {
        GraphemeSizing::Uniform(run) if run.cols() <= columns => {
            entry_builder.num_cells += run.cols();
            entry_builder.incr_content_offset +=
                run.count.get() as usize * run.info.utf8_bytes.get() as usize;
            entry_builder.grapheme_runs.push(*run);
            true
        }
        GraphemeSizing::EmptyRow => true, // zero-width row — nothing to absorb
        GraphemeSizing::NonUniform => {
            let cells: usize = row_runs.iter().map(GraphemeRun::cols).sum();
            if cells > columns {
                return false;
            }
            for run in row_runs {
                entry_builder.num_cells += run.cols();
                entry_builder.incr_content_offset +=
                    run.count.get() as usize * run.info.utf8_bytes.get() as usize;
                match entry_builder.grapheme_runs.last_mut() {
                    Some(last) if last.info == run.info => {
                        checked_add_run_count(&mut last.count, run.count.get());
                    }
                    _ => entry_builder.grapheme_runs.push(*run),
                }
            }
            true
        }
        GraphemeSizing::Uniform(_) => false, // cols() > columns — needs splitting
    }
}

/// Tries to handle a uniform run using arithmetic carry-over, accounting for
/// any graphemes already accumulated in `entry_builder` from previous
/// soft-wrapped source rows.
///
/// Handles `cell_width == 1` (ASCII/single-width) and `cell_width == 2`
/// (wide chars).  Other cell widths fall through to the medium path.
///
/// Returns `true` if handled; `false` to fall through to the medium path.
fn try_emit_carryover_uniform(
    run: &GraphemeRun,
    has_trailing_newline: bool,
    entry_builder: &mut EntryBuilder,
    index: &mut Index,
) -> bool {
    let cell_width = run.info.cell_width as usize;
    if cell_width == 0 || cell_width > 2 {
        return false;
    }
    let count = run.count.get() as usize;
    let byte_len = run.info.utf8_bytes.get() as usize;
    let columns = index.columns;
    // graphemes_per_row: how many graphemes of this width fit in a complete row.
    // For wide chars (cell_width == 2) with an odd column count, there is a
    // 1-cell remainder that cannot hold another wide char — rows filled to
    // that boundary get ends_with_leading_wide_char_spacer set.
    let graphemes_per_row = columns / cell_width;
    let full_row_spacer = cell_width == 2 && columns % 2 == 1;
    let remaining_cells = columns.saturating_sub(entry_builder.num_cells);
    let remaining_graphemes = remaining_cells / cell_width;

    // Sub-case A: builder is exactly full — flush it, then process the run
    // as if starting from an empty builder.
    if entry_builder.num_cells >= columns && graphemes_per_row > 0 {
        entry_builder.flush_to_index(index);
        if count > graphemes_per_row {
            // Fill and flush one full row via the builder, then use direct
            // arithmetic for the remainder.
            entry_builder.num_cells = graphemes_per_row * cell_width;
            entry_builder.incr_content_offset += graphemes_per_row * byte_len;
            entry_builder.grapheme_runs.push(GraphemeRun {
                count: NonZeroU16::new(graphemes_per_row as u16).unwrap(),
                info: run.info,
            });
            if full_row_spacer {
                entry_builder.add_leading_wide_char_spacer();
            }
            entry_builder.flush_to_index(index);
            let mut rem = count - graphemes_per_row;
            // Strict `>` ensures rem stays in [1, graphemes_per_row] after the
            // loop — with `>=` rem could reach 0, causing `has_trailing_newline`
            // to emit an empty row instead of annotating the last content row.
            while rem > graphemes_per_row {
                let content_offset: ByteOffset = index.content_len.into();
                index.content_len += graphemes_per_row * byte_len;
                index.rows.push_back(Entry {
                    content_offset,
                    grapheme_sizing: GraphemeSizing::Uniform(GraphemeRun {
                        count: NonZeroU16::new(graphemes_per_row as u16).unwrap(),
                        info: run.info,
                    }),
                    has_trailing_newline: false,
                    ends_with_leading_wide_char_spacer: full_row_spacer,
                });
                rem -= graphemes_per_row;
            }
            // rem is in [1, graphemes_per_row] — never zero (see loop comment).
            debug_assert!(rem > 0);
            entry_builder.num_cells = rem * cell_width;
            entry_builder.incr_content_offset += rem * byte_len;
            entry_builder.grapheme_runs.push(GraphemeRun {
                count: NonZeroU16::new(rem as u16).unwrap(),
                info: run.info,
            });
        } else {
            // Entire run fits in a fresh row.
            entry_builder.num_cells = count * cell_width;
            entry_builder.incr_content_offset += count * byte_len;
            match entry_builder.grapheme_runs.last_mut() {
                Some(last) if last.info == run.info => {
                    checked_add_run_count(&mut last.count, count as u16);
                }
                _ => entry_builder.grapheme_runs.push(*run),
            }
        }
        if has_trailing_newline {
            entry_builder.add_trailing_newline();
            entry_builder.flush_to_index(index);
        }
        return true;
    }

    // Sub-case B: run straddles the row boundary — fill the current partial
    // row, flush, then use arithmetic for the remainder.
    if remaining_graphemes > 0 && remaining_graphemes < count && graphemes_per_row > 0 {
        // For wide chars with an odd column count the partial row may have a
        // 1-cell gap that cannot fit another wide char.
        let partial_row_spacer = cell_width == 2 && remaining_cells % 2 == 1;
        entry_builder.num_cells += remaining_graphemes * cell_width;
        entry_builder.incr_content_offset += remaining_graphemes * byte_len;
        match entry_builder.grapheme_runs.last_mut() {
            Some(last) if last.info == run.info => {
                checked_add_run_count(&mut last.count, remaining_graphemes as u16);
            }
            _ => entry_builder.grapheme_runs.push(GraphemeRun {
                count: NonZeroU16::new(remaining_graphemes as u16).unwrap(),
                info: run.info,
            }),
        }
        if partial_row_spacer {
            entry_builder.add_leading_wide_char_spacer();
        }
        entry_builder.flush_to_index(index);

        let mut rem = count - remaining_graphemes;
        // Same strict `>` invariant as Sub-case A.
        while rem > graphemes_per_row {
            let content_offset: ByteOffset = index.content_len.into();
            index.content_len += graphemes_per_row * byte_len;
            index.rows.push_back(Entry {
                content_offset,
                grapheme_sizing: GraphemeSizing::Uniform(GraphemeRun {
                    count: NonZeroU16::new(graphemes_per_row as u16).unwrap(),
                    info: run.info,
                }),
                has_trailing_newline: false,
                ends_with_leading_wide_char_spacer: full_row_spacer,
            });
            rem -= graphemes_per_row;
        }
        if rem > 0 {
            entry_builder.num_cells = rem * cell_width;
            entry_builder.incr_content_offset += rem * byte_len;
            entry_builder.grapheme_runs.push(GraphemeRun {
                count: NonZeroU16::new(rem as u16).unwrap(),
                info: run.info,
            });
        }
        if has_trailing_newline {
            entry_builder.add_trailing_newline();
            entry_builder.flush_to_index(index);
        }
        return true;
    }

    // Sub-case C: entire run fits in the remaining space of the current row.
    if count <= remaining_graphemes && entry_builder.num_cells + count * cell_width <= columns {
        entry_builder.num_cells += count * cell_width;
        entry_builder.incr_content_offset += count * byte_len;
        match entry_builder.grapheme_runs.last_mut() {
            Some(last) if last.info == run.info => {
                checked_add_run_count(&mut last.count, count as u16);
            }
            _ => entry_builder.grapheme_runs.push(*run),
        }
        if has_trailing_newline {
            entry_builder.add_trailing_newline();
            entry_builder.flush_to_index(index);
        }
        return true;
    }

    false // fall through to the medium path
}

/// Processes a slice of grapheme runs into output index rows, splitting at
/// column boundaries.
///
/// Runs that fit entirely within the remaining row space are bulk-accumulated
/// via `extend_from_slice` — no per-run branches, cache-friendly, and
/// vectorizable by the compiler. Only the single run that straddles a row
/// boundary requires per-grapheme arithmetic via `process_graphemes_batch`.
///
/// This is used by the medium path of `Index::rebuild` to replace the old
/// per-run fit-check loop, trading O(runs) branch-heavy operations for
/// O(output_rows) boundary events plus O(runs) branch-free arithmetic.
fn emit_runs(
    runs: &[GraphemeRun],
    has_trailing_newline: bool,
    entry_builder: &mut EntryBuilder,
    index: &mut Index,
) {
    let mut run_idx = 0;

    while run_idx < runs.len() {
        // Scan forward to find how many consecutive runs fit entirely in the
        // remaining row space.  This is pure arithmetic — no Vec mutations.
        let scan_start = run_idx;
        let mut col = entry_builder.num_cells;
        while run_idx < runs.len() {
            let run_cols = runs[run_idx].cols();
            if col + run_cols > index.columns {
                break;
            }
            col += run_cols;
            run_idx += 1;
        }

        // Bulk-accumulate all fitting runs with a single extend_from_slice.
        let fitting = &runs[scan_start..run_idx];
        if !fitting.is_empty() {
            // Sum bytes in a branch-free pass (auto-vectorizable).
            let bytes: usize = fitting
                .iter()
                .map(|r| r.count.get() as usize * r.info.utf8_bytes.get() as usize)
                .sum();
            entry_builder.incr_content_offset += bytes;
            entry_builder.num_cells = col;

            // Merge with the last existing run if the info matches, then
            // extend the rest in one slice copy.
            let skip_first = if let Some(last) = entry_builder.grapheme_runs.last_mut() {
                if last.info == fitting[0].info {
                    checked_add_run_count(&mut last.count, fitting[0].count.get());
                    true
                } else {
                    false
                }
            } else {
                false
            };
            entry_builder
                .grapheme_runs
                .extend_from_slice(if skip_first { &fitting[1..] } else { fitting });
        }

        // Handle the one run that straddles the row boundary (if any).
        if run_idx < runs.len() {
            let br = &runs[run_idx];
            entry_builder.process_graphemes_batch(br.info, br.count.get(), index);
            run_idx += 1;
        }
    }

    if has_trailing_newline {
        entry_builder.add_trailing_newline();
        entry_builder.flush_to_index(index);
    }
}

fn checked_add_run_count(count: &mut NonZeroU16, add: u16) {
    *count = count
        .checked_add(add)
        .expect("should not have more than 2^16 graphemes in a single row");
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
