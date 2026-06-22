use std::cmp::Ordering;
use std::ops::{Range, RangeInclusive};

use bimap::BiMap;
use itertools::Itertools;

use crate::terminal::model::index::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplaySource {
    CursorLine,
    FilterMatch,
    FilterContext,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayedRows {
    pub range: RangeInclusive<usize>,
    pub source: DisplaySource,
}

impl DisplayedRows {
    #[cfg(test)]
    pub fn new(range: RangeInclusive<usize>, source: DisplaySource) -> Self {
        Self { range, source }
    }
}

/// Whether or not to respect the displayed output/filter when
/// retrieving the grid contents.
#[derive(Copy, Clone)]
pub enum RespectDisplayedOutput {
    /// Points are assumed to represent the displayed location in the grid.
    Yes,
    /// Points are assumed to represent the original location in the grid.
    No,
}

/// Structure to represent a subset of rows from the grid that we want to
/// make visible when rendering.
#[derive(Clone, Default, Debug)]
pub struct DisplayedOutput {
    /// The rows that we want to display in the grid, represented by their
    /// row indices in the grid. The ranges must be non-overlapping and in
    /// sorted order, from lowest to highest.
    displayed_rows: Vec<DisplayedRows>,
    /// Height of the displayed rows in the grid. This is equivalent to the
    /// number of displayed rows.
    height: usize,
    row_translation_map: RowTranslationMap,
}

/// A mapping to translate locations in the grid between the original row and the offset row after filtering.
/// NOTE: `Left` = original row and `Right` = offset/translated row
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowTranslationMap {
    pub(crate) inner: BiMap<usize, usize>,
}

impl From<BiMap<usize, usize>> for RowTranslationMap {
    fn from(value: BiMap<usize, usize>) -> Self {
        Self { inner: value }
    }
}

impl RowTranslationMap {
    /// Translates a point from its original location to the displayed point's location (the offset with the filter applied).
    ///
    /// NOTE: If the original location is not found in the row translation map,
    /// then it will return the given point.
    pub fn maybe_translate_point_from_original_to_displayed(&self, original_point: Point) -> Point {
        self.inner
            .get_by_left(&original_point.row)
            .map(|translated_row| Point {
                row: *translated_row,
                col: original_point.col,
            })
            .unwrap_or_else(|| {
                log::warn!("Could not translate point {original_point:?} to its displayed location, returning given point instead");
                original_point
            })
    }

    /// Translates a point from its displayed location (the offset with the filter applied) to its original location.
    ///
    /// NOTE: If the displayed location is not found in the row translation map,
    /// then it will return the given point.
    pub fn maybe_translate_point_from_displayed_to_original(
        &self,
        displayed_point: Point,
    ) -> Point {
        self.inner
            .get_by_right(&displayed_point.row)
            .map(|original_row| Point {
                row: *original_row,
                col: displayed_point.col,
            })
            .unwrap_or_else(|| {
                log::warn!("Could not translate point {displayed_point:?} to its original location, returning given point instead");
                displayed_point
            })
    }

    /// Translates a row from its displayed location (the offset with the filter applied) to its original location.
    ///
    /// NOTE: If the displayed location is not found in the row translation map,
    /// then it will return the given row.
    pub fn maybe_translate_row_from_displayed_to_original(&self, displayed_row: usize) -> usize {
        *self.inner
            .get_by_right(&displayed_row)
            .unwrap_or_else(|| {
                log::warn!("Could not translate row {displayed_row:?} to its original location, returning given row instead");
                &displayed_row
            })
    }

    /// Returns true iff the `row` is a displayed row.
    pub fn is_displayed_row(&self, row: usize) -> bool {
        self.inner.contains_left(&row)
    }
}

impl DisplayedOutput {
    /// Constructs a new instance of DisplayedOutput from the given displayed
    /// rows.
    ///
    /// This is faster than creating a new instance via DisplayedOutput::default()
    /// and calling extend_displayed_lines.
    ///
    /// Caller must ensure that the row ranges in displayed_rows are:
    ///   - In ascending order
    ///   - Representing one or more whole logical lines. The row ranges cannot
    ///     only have part of a logical line.
    pub fn new_from_displayed_lines(displayed_lines: Vec<DisplayedRows>) -> Self {
        // Assert that the displayed rows are in ascending order.
        debug_assert!(displayed_lines
            .windows(2)
            .all(|w| w[0].range.end() < w[1].range.start()));

        let mut height = 0;
        for rows in displayed_lines.iter() {
            height += (rows.range.end() - rows.range.start()) + 1;
        }

        let mut original_to_offset_rows = BiMap::with_capacity(height);
        for (offset, row) in displayed_lines
            .iter()
            .map(|rows| &rows.range)
            .cloned()
            .flatten()
            .enumerate()
        {
            original_to_offset_rows.insert(row, offset);
        }

        Self {
            displayed_rows: displayed_lines,
            height,
            row_translation_map: RowTranslationMap {
                inner: original_to_offset_rows,
            },
        }
    }

    /// Get the height of the displayed rows.
    pub fn height(&self) -> usize {
        self.height
    }

    pub fn displayed_rows(&self) -> &[DisplayedRows] {
        &self.displayed_rows
    }

    /// Returns an iterator over the indices in the grid of the displayed rows.
    pub fn rows(&self) -> impl DoubleEndedIterator<Item = usize> + '_ {
        self.displayed_rows
            .iter()
            .map(|rows| &rows.range)
            .cloned()
            .flatten()
    }

    /// Marks a set of row ranges representing logical lines in the grid as
    /// visible at the end of the currently displayed lines.
    ///
    /// Caller must ensure that the given lines are in ascending order and
    /// greater than all existing lines.
    pub fn extend_displayed_lines(&mut self, displayed_lines: Vec<DisplayedRows>) {
        for rows in displayed_lines.iter() {
            self.height += (rows.range.end() - rows.range.start()) + 1;
        }
        self.append_to_row_translation(displayed_lines.iter());
        self.displayed_rows.extend(displayed_lines);

        // Assert that displayed_rows is still in ascending order.
        debug_assert!(self
            .displayed_rows
            .windows(2)
            .all(|w| w[0].range.end() < w[1].range.start()));
    }

    /// Marks a set of row ranges representing logical lines in the grid as
    /// visible at the beginning of the currently displayed lines.
    ///
    /// Caller must ensure that the given lines are in ascending order and
    /// less than all existing lines.
    pub fn prepend_displayed_lines(&mut self, displayed_lines: Vec<DisplayedRows>) {
        for rows in displayed_lines.iter() {
            self.height += (rows.range.end() - rows.range.start()) + 1;
        }
        self.displayed_rows.splice(0..0, displayed_lines);
        self.prepend_to_row_translation();

        // Assert that displayed_rows is still in ascending order.
        debug_assert!(self
            .displayed_rows
            .windows(2)
            .all(|w| w[0].range.end() < w[1].range.start()));
    }

    /// Replaces the specified range in the existing displayed lines with new
    /// displayed lines. The new displayed lines need not be the same length
    /// as the specified range.
    ///
    /// Caller must ensure that the specified range is valid and the new lines
    /// are inserted in ascending order.
    pub fn splice_displayed_lines(
        &mut self,
        replace_range: Range<usize>,
        displayed_lines: Vec<DisplayedRows>,
    ) -> Vec<DisplayedRows> {
        for rows in displayed_lines.iter() {
            self.height += (rows.range.end() - rows.range.start()) + 1;
        }

        let removed = self
            .displayed_rows
            .splice(replace_range.clone(), displayed_lines)
            .collect_vec();
        self.replace_rows_from_row_translation(replace_range);

        for rows in removed.iter() {
            self.height = self
                .height
                .saturating_sub((rows.range.end() - rows.range.start()) + 1);
        }

        // Assert that displayed_rows is still in ascending order.
        debug_assert!(self
            .displayed_rows
            .windows(2)
            .all(|w| w[0].range.end() < w[1].range.start()));

        removed
    }

    /// Truncates the displayed rows so only rows up to and including the provided
    /// row are kept.
    ///
    /// e.g. If we are displaying rows [2..=4, 6..=10, 16..=17], truncating to
    /// to row 8 would yield [2..=4, 6..=8].
    ///
    /// Mainly used when finishing a block, as the grid gets truncated to the
    /// current cursor position.
    pub fn truncate_to_row(&mut self, row: usize) {
        if let Some(first) = self.displayed_rows.first() {
            // If we are truncating before the first rows, just clear it all.
            if row < *first.range.start() {
                self.displayed_rows.clear();
                self.height = 0;
                return;
            }
        } else if let Some(last) = self.displayed_rows.last() {
            // If we are truncating after the last rows, do nothing.
            if *last.range.end() < row {
                return;
            }
        }

        // Find the index of the range that we want to truncate from. Searches
        // from right-to-left assuming that most of the time, the row to
        // truncate to will be near the end of displayed_rows.
        let mut truncate_idx = None;
        for (i, rows) in self.displayed_rows.iter().enumerate().rev() {
            if row > *rows.range.end() {
                break;
            }
            truncate_idx = Some(i);
        }

        let Some(truncate_idx) = truncate_idx else {
            return;
        };
        let mut truncated = self.displayed_rows.drain(truncate_idx..);

        let mut partial_rows = None;
        // We need to handle the first truncated range specially. We may be
        // truncating in the middle of the range, in which case we want to
        // keep the partial row range before the truncate point.
        if let Some(first_truncated) = truncated.next() {
            self.height = self
                .height
                .saturating_sub((first_truncated.range.end() - first_truncated.range.start()) + 1);

            if first_truncated.range.contains(&row) {
                partial_rows = Some(DisplayedRows {
                    range: *first_truncated.range.start()..=row,
                    source: first_truncated.source,
                });
            }
        }

        // Subtract the remaining truncated rows from the height.
        for rows in truncated {
            self.height = self
                .height
                .saturating_sub((rows.range.end() - rows.range.start()) + 1);
        }

        if let Some(partial_rows) = partial_rows {
            self.height += (partial_rows.range.end() - partial_rows.range.start()) + 1;
            self.displayed_rows.push(partial_rows);
        }
    }

    /// Updates the row translation map assuming that we only appended displayed rows.
    /// NOTE: Must be called before updating `self.displayed_rows`.
    fn append_to_row_translation<'a, I>(&mut self, new_line_ranges: I)
    where
        I: IntoIterator<Item = &'a DisplayedRows>,
    {
        let last_displayed_row = self
            .displayed_rows
            .last()
            .map(|last_displayed_row| *last_displayed_row.range.end())
            .unwrap_or(0);

        let mut offset_row = if let Some(last_row_offset) = self
            .row_translation_map
            .inner
            .get_by_left(&last_displayed_row)
        {
            last_row_offset + 1
        } else {
            0
        };
        for rows in new_line_ranges {
            for row in rows.range.clone() {
                self.row_translation_map.inner.insert(row, offset_row);
                offset_row += 1;
            }
        }
    }

    /// Updates the row translation map assuming that we only prepended displayed rows.
    /// NOTE: must be called after updating `self.displayed_rows`.
    fn prepend_to_row_translation(&mut self) {
        // None of the current entries are valid.
        self.row_translation_map.inner.clear();

        for (offset, row) in self
            .displayed_rows
            .iter()
            .map(|row_range| &row_range.range)
            .cloned()
            .flatten()
            .enumerate()
        {
            self.row_translation_map.inner.insert(row, offset);
        }
    }

    /// Updates the row translation map assuming that we replaced some of the displayed rows.
    /// `replace_range` represents the range of indices in displayed rows that has been updated.
    /// NOTE: Must be called after updating `self.displayed_rows`.
    fn replace_rows_from_row_translation(&mut self, replace_range: Range<usize>) {
        let first_row_before_replace_range = if replace_range.start == 0 {
            // We are replacing the beginning row(s).
            None
        } else {
            self.displayed_rows
                .get(replace_range.start - 1)
                .map(|first_replaced| first_replaced.range.end())
        };

        let mut offset_row =
            first_row_before_replace_range.map_or(0, |first_row_before_replace_range| {
                if let Some(row) = self
                    .row_translation_map
                    .inner
                    .get_by_left(first_row_before_replace_range)
                {
                    row + 1
                } else {
                    // There is no offset which means we are at the beginning.
                    0
                }
            });

        // Clear all rows from our start offset row to the last offset row.
        let last_offset_row = self.row_translation_map.inner.len();
        for row in offset_row..=last_offset_row {
            self.row_translation_map.inner.remove_by_right(&row);
        }
        // Insert entries for all entries in self.displayed_rows from the start of the replace range.
        for rows in &self.displayed_rows[replace_range.start..] {
            for row in rows.range.clone() {
                self.row_translation_map.inner.insert(row, offset_row);
                offset_row += 1;
            }
        }
    }

    /// Translates a point from its original location to the displayed point's location (the offset with the filter applied).
    ///
    /// NOTE: If the original location is not found in the row translation map,
    /// then it will return the given point.
    pub fn maybe_translate_point_from_original_to_displayed(&self, original_point: Point) -> Point {
        self.row_translation_map
            .maybe_translate_point_from_original_to_displayed(original_point)
    }

    /// Translates a point from its displayed location (the offset with the filter applied) to its original location.
    ///
    /// NOTE: If the displayed location is not found in the row translation map,
    /// then it will return the given point.
    pub fn maybe_translate_point_from_displayed_to_original(
        &self,
        displayed_point: Point,
    ) -> Point {
        self.row_translation_map
            .maybe_translate_point_from_displayed_to_original(displayed_point)
    }

    /// Translates a row from its displayed location (the offset with the filter applied) to its original location.
    ///
    /// NOTE: If the displayed location is not found in the row translation map,
    /// then it will return the given row.
    pub fn maybe_translate_row_from_displayed_to_original(&self, displayed_row: usize) -> usize {
        self.row_translation_map
            .maybe_translate_row_from_displayed_to_original(displayed_row)
    }

    /// Returns true if `row` is a displayed row.
    pub fn is_displayed_row(&self, row: usize) -> bool {
        self.row_translation_map.is_displayed_row(row)
    }

    /// If the given original row is being displayed, returns the row it is
    /// displayed at. Otherwise, searches for the next closest original row that
    /// is greater than the given original row and is being displayed, and
    /// returns the row that is displayed at. If there is no next closest row,
    /// returns None.
    ///
    /// e.g. If our displayed output structure looked like this:
    /// original row | displayed row
    /// 2            | 0
    /// 3            | 1
    /// 7            | 2
    /// get_exact_or_next_displayed_row(2) == 0
    /// get_exact_or_next_displayed_row(4) == 2
    /// get_exact_or_next_displayed_row(8) == None
    pub fn get_exact_or_next_displayed_row(&self, target_original_row: usize) -> Option<usize> {
        if self.row_translation_map.inner.is_empty() {
            return None;
        }

        if let Some(displayed_row) = self
            .row_translation_map
            .inner
            .get_by_left(&target_original_row)
        {
            return Some(*displayed_row);
        }

        // Perform binary search for displayed row of next closest original row.
        let mut low = 0;
        let mut high = self.row_translation_map.inner.len() - 1;
        let mut candidate = None;

        while low <= high {
            let displayed_row = (high + low) / 2;
            let original_row = self
                .row_translation_map
                .inner
                .get_by_right(&displayed_row)?;

            match original_row.cmp(&target_original_row) {
                Ordering::Greater => {
                    candidate = Some(displayed_row);
                    if displayed_row == 0 {
                        // Break early to avoid underflowing.
                        break;
                    }
                    high = displayed_row - 1;
                }
                Ordering::Less => {
                    low = displayed_row + 1;
                }
                Ordering::Equal => {
                    // We should never reach this case because of the early exit
                    // condition at the beginning of the method, but it is included
                    // here for completeness.
                    return Some(displayed_row);
                }
            }
        }
        candidate
    }

    /// Finds the first displayed rows that are fully or partially greater than
    /// or equal to the given row index.
    ///
    /// Returns the index and a reference to the DisplayedRows object.
    ///
    /// This is mainly used for updating the dirty lines when filtering an active
    /// block. The common case has the dirty lines at the end of the block (i.e.
    /// adding new lines to the output), so this method performs a reverse linear
    /// search to optimize for this case.
    pub fn first_rows_greater_than_or_contained_in(
        &self,
        row: usize,
    ) -> Option<(usize, &DisplayedRows)> {
        let mut start_idx = None;
        let mut start_rows = None;
        for (idx, rows) in self.displayed_rows.iter().enumerate().rev() {
            if row > *rows.range.end() {
                break;
            }
            start_idx = Some(idx);
            start_rows = Some(rows);
        }
        start_idx.zip(start_rows)
    }

    /// Finds the last displayed rows that are fully or partially less than or
    /// equal to the given row index.
    ///
    /// Returns the index and a reference to the DisplayedRows object.
    pub fn last_rows_less_than_or_contained_in(
        &self,
        row: usize,
    ) -> Option<(usize, &DisplayedRows)> {
        self.displayed_rows
            .iter()
            .enumerate()
            .rev()
            .find(|(_, rows)| *rows.range.start() <= row)
    }

    #[cfg(test)]
    pub fn new_for_test(displayed_rows: Vec<RangeInclusive<usize>>) -> Self {
        DisplayedOutput::new_from_displayed_lines(
            displayed_rows
                .into_iter()
                .map(|rows| DisplayedRows {
                    range: rows,
                    source: DisplaySource::FilterMatch,
                })
                .collect_vec(),
        )
    }

    pub fn reset(&mut self) {
        self.displayed_rows.clear();
        self.height = 0;
        self.row_translation_map.inner.clear();
    }
}

#[cfg(test)]
#[path = "displayed_output_tests.rs"]
mod tests;
