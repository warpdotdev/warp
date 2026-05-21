use std::{
    cmp::{max, min},
    ops::{Range, RangeInclusive},
    sync::Arc,
};

use itertools::Itertools as _;

use crate::terminal::model::{
    find::RegexDFAs,
    grid::displayed_output::{DisplaySource, DisplayedOutput, DisplayedRows},
    index::Point,
};

use super::GridHandler;

/// Structure that represents a filter applied to a block's output.
#[derive(Clone, Debug)]
pub struct FilterState {
    /// The dfas created from the filter term.
    pub dfas: Arc<RegexDFAs>,
    /// The matches corresponding to the filter term in the current output grid,
    /// in descending order.
    pub matches: Vec<RangeInclusive<Point>>,
    /// The number of logical lines in the grid that contain a match.
    pub num_matched_lines: usize,
    /// The number of context lines to include above/below each matched line.
    pub num_context_lines: usize,
    /// True if the user has requested lines matching the filter term to be
    /// _excluded_ from the output.
    pub invert_filter: bool,
}

impl FilterState {
    /// Updates the matches after a range of lines have been dirtied and new
    /// matches have been found. Specifically, this will remove all existing
    /// matches that have any part in the dirty range and add the new matches
    /// in their place.
    ///
    /// Invariant: Caller must ensure that the new matches lie within the dirty
    /// range. New matches should be in descending order.
    fn update_dirty_matches(
        &mut self,
        dirty_range: RangeInclusive<usize>,
        new_matches: Vec<RangeInclusive<Point>>,
    ) {
        // If there are no current matches or the dirty range is lower than the
        // lowest match, insert the new matches at the end.
        if self.matches.is_empty()
            || self
                .matches
                .last()
                .is_some_and(|lowest_match| *dirty_range.end() < lowest_match.start().row)
        {
            self.matches.extend(new_matches);
            return;
        }

        // If the dirty range is higher than the highest match, insert the new
        // line ranges at the start.
        if self
            .matches
            .first()
            .is_some_and(|highest_match| highest_match.end().row < *dirty_range.start())
        {
            self.matches.splice(0..0, new_matches);
            return;
        }

        // Find the start/end indices of the matches that are in the dirty range.
        let Some(replace_start) = self
            .matches
            .iter()
            .position(|m| *dirty_range.end() >= m.start().row)
        else {
            log::error!("Could not find replacement start when updating dirty matches");
            return;
        };

        let Some(replace_end) = self
            .matches
            .iter()
            .rposition(|m| m.end().row >= *dirty_range.start())
        else {
            log::error!("Could not find replacement end when updating dirty matches");
            return;
        };

        let replace_range = if replace_start <= replace_end {
            replace_start..(replace_end + 1)
        } else {
            // We should only get replace_start > replace_end if the entire dirty
            // range lies between two adjacent matches, in which case we want to
            // insert between those two matches without replacing anything.
            replace_start..replace_start
        };
        self.matches.splice(replace_range, new_matches);

        // Assert that the matches are still in descending order.
        debug_assert!(self.matches.windows(2).all(|w| w[0].end() > w[1].start()));
    }

    /// Returns a FilterState with a dummy DFA and the provided matches.
    #[cfg(test)]
    fn new_with_matches_for_test(matches: Vec<RangeInclusive<Point>>) -> FilterState {
        use crate::terminal::block_filter::DEFAULT_CONTEXT_LINES_VALUE;

        Self {
            dfas: Arc::new(RegexDFAs::new("").unwrap()),
            matches,
            num_matched_lines: 0,
            num_context_lines: DEFAULT_CONTEXT_LINES_VALUE as usize,
            invert_filter: false,
        }
    }

    fn clear_matches(&mut self) {
        self.matches.clear();
    }
}

impl GridHandler {
    /// Clear the grid's displayed output and filter matches.
    pub(super) fn clear_displayed_rows_and_filter_matches(&mut self) {
        if let Some(filter_state) = self.filter_state.as_mut() {
            filter_state.clear_matches();
        }
        if let Some(displayed_output) = self.displayed_output.as_mut() {
            displayed_output.reset();
        }
    }

    /// Returns the length of the grid containing only displayed rows, if a
    /// constraint (e.g. filter) has been placed on the displayed rows. Otherwise
    /// returns None.
    pub fn len_displayed(&self) -> Option<usize> {
        self.displayed_output.as_ref().map(|output| output.height())
    }

    /// Returns an iterator over the indices of the rows to display in the grid.
    /// Will return None if no filtering has been applied and all the rows should
    /// be displayed.
    pub fn displayed_output_rows(&self) -> Option<impl DoubleEndedIterator<Item = usize> + '_> {
        self.displayed_output
            .as_ref()
            .map(|displayed_output| displayed_output.rows())
    }

    /// Returns an iterator over the ranges row indices to display in the grid.
    ///
    /// Returns `None` if no filtering has been applied and all the rows should
    /// be displayed.
    pub fn displayed_output_row_ranges(
        &self,
    ) -> Option<impl DoubleEndedIterator<Item = RangeInclusive<usize>> + '_> {
        self.displayed_output.as_ref().map(|displayed_output| {
            displayed_output
                .displayed_rows()
                .iter()
                .map(|row| row.range.clone())
        })
    }

    /// Translates a point from its displayed location (the offset with the
    /// filter applied) to its original location.
    ///
    /// If the displayed location is not found in the row translation map or
    /// the grid does not have any displayed output (i.e. no filter is present),
    /// then it will return the given point.
    pub fn maybe_translate_point_from_displayed_to_original(
        &self,
        displayed_point: Point,
    ) -> Point {
        self.displayed_output
            .as_ref()
            .map(|displayed_output| {
                displayed_output.maybe_translate_point_from_displayed_to_original(displayed_point)
            })
            .unwrap_or(displayed_point)
    }

    /// Translates a point from its original location to the displayed point's
    /// location (the offset with the filter applied).
    ///
    /// If the original location is not found in the row translation map or
    /// the grid does not have any displayed output (i.e. no filter is present),
    /// then it will return the given point.
    pub fn maybe_translate_point_from_original_to_displayed(&self, original_point: Point) -> Point {
        self.displayed_output
            .as_ref()
            .map(|displayed_output| {
                displayed_output.maybe_translate_point_from_original_to_displayed(original_point)
            })
            .unwrap_or(original_point)
    }

    /// Translates a row from its original location to the displayed row's
    /// location (the offset with the filter applied).
    ///
    /// If the original location is not found in the row translation map or
    /// the grid does not have any displayed output (i.e. no filter is present),
    /// then it will return the given row.
    pub fn maybe_translate_row_from_displayed_to_original(&self, displayed_row: usize) -> usize {
        self.displayed_output
            .as_ref()
            .map(|displayed_output| {
                displayed_output.maybe_translate_row_from_displayed_to_original(displayed_row)
            })
            .unwrap_or(displayed_row)
    }

    /// Returns true if we are only displaying a subset of rows in the grid to
    /// the user, as a result of filtering or some other action.
    pub fn has_displayed_output(&self) -> bool {
        self.displayed_output.is_some()
    }

    /// Returns true if there is displayed output and the
    /// `row` is one of the displayed rows.
    pub fn is_displayed_row(&self, row: usize) -> bool {
        self.displayed_output
            .as_ref()
            .is_none_or(|displayed_output| displayed_output.is_displayed_row(row))
    }

    pub fn get_exact_or_next_displayed_row(&self, original_row: usize) -> Option<usize> {
        if let Some(displayed_output) = self.displayed_output.as_ref() {
            displayed_output.get_exact_or_next_displayed_row(original_row)
        } else {
            Some(original_row)
        }
    }

    /// Returns the matches corresponding to the current filter applied to the
    /// grid. Returns None if no filter is applied.
    pub fn filter_matches(&self) -> Option<&[RangeInclusive<Point>]> {
        self.filter_state
            .as_ref()
            .map(|state| state.matches.as_slice())
    }

    pub fn num_matched_lines_in_filter(&self) -> Option<usize> {
        self.filter_state
            .as_ref()
            .map(|state| state.num_matched_lines)
    }

    /// Return the matches and the logical lines they are contained in from this
    /// grid. Matches are found starting from the last row, last column. Logical
    /// lines are found starting from the last row.
    fn find_with_matched_lines(
        &self,
        dfas: &RegexDFAs,
    ) -> (Vec<RangeInclusive<Point>>, Vec<RangeInclusive<usize>>) {
        let matches: Vec<RangeInclusive<Point>> = self.find(dfas).collect();
        let line_ranges = self.get_line_ranges_from_matches(&matches);
        (matches, line_ranges)
    }

    fn find_with_matched_lines_in_range(
        &self,
        dfas: &RegexDFAs,
        start: Point,
        end: Point,
    ) -> (Vec<RangeInclusive<Point>>, Vec<RangeInclusive<usize>>) {
        let matches: Vec<RangeInclusive<Point>> = self.find_in_range(dfas, start, end).collect();
        let line_ranges = self.get_line_ranges_from_matches(&matches);
        (matches, line_ranges)
    }

    fn get_line_ranges_from_matches(
        &self,
        matches: &[RangeInclusive<Point>],
    ) -> Vec<RangeInclusive<usize>> {
        let mut prev_matched_line_range: Option<RangeInclusive<usize>> = None;
        let mut line_ranges: Vec<RangeInclusive<usize>> = Vec::new();
        for m in matches.iter() {
            // We iterate through the matches starting from the last row, last col
            // of the grid. So if the current match end falls within the previous
            // line, then we can skip matching this line again. This relies on our
            // find implementation not returning matches across line breaks.
            if prev_matched_line_range
                .as_ref()
                .is_some_and(|r| r.contains(&m.end().row))
            {
                continue;
            }

            let line_start_row = self.line_search_left(*m.start()).row;
            let line_end_row = self.line_search_right(*m.end()).row;
            line_ranges.push(line_start_row..=line_end_row);
            prev_matched_line_range = Some(line_start_row..=line_end_row);
        }
        line_ranges
    }

    /// Returns the logical lines in the grid that do not contain any matches,
    /// in decreasing order.
    fn find_non_matching_lines(&self, dfas: &RegexDFAs) -> Vec<RangeInclusive<usize>> {
        let matches: Vec<RangeInclusive<Point>> = self.find(dfas).collect();
        self.get_non_matching_line_ranges_from_matches(&matches, 0, self.max_content_row())
    }

    /// Returns the logical lines in the grid within a certain range that do not
    /// contain any matches, in decreasing order.
    fn find_non_matching_lines_in_range(
        &self,
        dfas: &RegexDFAs,
        start: Point,
        end: Point,
    ) -> Vec<RangeInclusive<usize>> {
        let matches: Vec<RangeInclusive<Point>> = self.find_in_range(dfas, start, end).collect();
        self.get_non_matching_line_ranges_from_matches(&matches, start.row, end.row)
    }

    /// Returns ranges representing the logical lines within `range_start_row`
    /// and `range_end_row` (both inclusive) that do not contain any matches,
    /// in decreasing order.
    ///
    /// Invariant: The given matches must be given in decreasing order and lie
    ///            within the given start/end rows.
    fn get_non_matching_line_ranges_from_matches(
        &self,
        matches: &[RangeInclusive<Point>],
        range_start_row: usize,
        range_end_row: usize,
    ) -> Vec<RangeInclusive<usize>> {
        let mut line_ranges: Vec<RangeInclusive<usize>> = Vec::new();

        let mut add_final_line_range = true;
        let mut next_end_row = range_end_row;
        // Loop through all of the matches, adding the rows between the matched
        // lines as non-matching line ranges.
        for m in matches.iter() {
            // Everything above `next_end_row` has previously been processed, so
            // if the current match exceeds `next_end_row`, we can skip it.
            if m.end().row > next_end_row {
                continue;
            }

            let line_start_row = self.line_search_left(*m.start()).row;
            let line_end_row = self.line_search_right(*m.end()).row;
            if line_end_row < next_end_row {
                line_ranges.push(line_end_row + 1..=next_end_row);
            }

            // If this condition is true, then we have processed the entire
            // range and can break out of the loop.
            if line_start_row == range_start_row {
                add_final_line_range = false;
                break;
            }
            next_end_row = line_start_row.saturating_sub(1);
        }
        // There may be additional lines between the start of the grid range
        // and the final match that we need to add.
        if add_final_line_range && range_start_row <= next_end_row {
            line_ranges.push(range_start_row..=next_end_row);
        }

        line_ranges
    }

    fn should_include_cursor_line_in_filter(&self) -> bool {
        !self.finished
    }

    /// Apply a filter to this grid. The logical lines containing the matches will
    /// be shown when the grid is rendered, while non-matching lines will be
    /// hidden.
    pub fn filter_lines(
        &mut self,
        dfas: Arc<RegexDFAs>,
        num_context_lines: usize,
        invert_filter: bool,
    ) {
        let (matches, line_ranges) = if invert_filter {
            // It doesn't make sense to have matches when the filter is
            // inverted, so we return an empty vector.
            (Vec::new(), self.find_non_matching_lines(&dfas))
        } else {
            self.find_with_matched_lines(&dfas)
        };
        let mut displayed_rows = if num_context_lines == 0 {
            line_ranges
                .iter()
                .rev()
                .map(|range| DisplayedRows {
                    range: range.clone(),
                    source: DisplaySource::FilterMatch,
                })
                .collect_vec()
        } else {
            self.add_context_lines(line_ranges.iter().rev(), num_context_lines)
        };

        if self.should_include_cursor_line_in_filter() {
            // The grid is not finished, we should include the cursor line in
            // the displayed output.
            let cursor_line_start = self.line_search_left(self.cursor_point());
            let cursor_line_end = self.line_search_right(self.cursor_point());

            let mut with_cursor_line = Vec::with_capacity(displayed_rows.len() + 1);
            add_displayed_rows_with_cursor_line(
                cursor_line_start.row..=cursor_line_end.row,
                displayed_rows.into_iter(),
                |rows| with_cursor_line.push(rows),
            );
            displayed_rows = with_cursor_line;
        }

        let displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_rows);
        self.displayed_output = Some(displayed_output);
        self.filter_state = Some(FilterState {
            dfas,
            matches,
            num_matched_lines: line_ranges.len(),
            num_context_lines,
            invert_filter,
        });
    }

    /// Re-applies the filter to the grid. The match points and matched lines
    /// might have changed after resizing or truncating rows after reaching the
    /// history limit, so we need to rescan the grid.
    pub fn refilter_lines(&mut self) {
        if let Some(filter_state) = self.filter_state.as_ref() {
            self.filter_lines(
                filter_state.dfas.clone(),
                filter_state.num_context_lines,
                filter_state.invert_filter,
            );
        }
    }

    /// Clear the applied filter on the grid, if any.
    pub fn clear_filter(&mut self) {
        self.displayed_output = None;
        self.filter_state = None;
    }

    /// Re-runs the applied filter (if any) on the dirty cells and update the
    /// filter state.
    pub(super) fn maybe_filter_dirty_lines(&mut self) {
        let Some(filter_state) = self.filter_state.as_ref() else {
            return;
        };
        let num_context_lines = filter_state.num_context_lines;

        let Some(dirty_cells_range) = self.dirty_cells_range() else {
            return;
        };

        let dirty_lines_start = self.line_search_left(*dirty_cells_range.start());
        let dirty_lines_end = self.line_search_right(*dirty_cells_range.end());

        let (matches, line_ranges) = if filter_state.invert_filter {
            (
                // It doesn't make sense to have matches when the filter is
                // inverted, so we return an empty vector.
                Vec::new(),
                self.find_non_matching_lines_in_range(
                    &filter_state.dfas,
                    dirty_lines_start,
                    dirty_lines_end,
                ),
            )
        } else {
            self.find_with_matched_lines_in_range(
                &filter_state.dfas,
                dirty_lines_start,
                dirty_lines_end,
            )
        };
        let mut new_displayed_rows = if num_context_lines == 0 {
            line_ranges
                .iter()
                .rev()
                .map(|range| DisplayedRows {
                    range: range.clone(),
                    source: DisplaySource::FilterMatch,
                })
                .collect_vec()
        } else {
            self.add_context_lines(line_ranges.iter().rev(), num_context_lines)
        };

        if let Some(filter_state_mut) = self.filter_state.as_mut() {
            filter_state_mut
                .update_dirty_matches(dirty_lines_start.row..=dirty_lines_end.row, matches);
        }

        if self.should_include_cursor_line_in_filter() {
            let cursor_point = self.cursor_point();
            let cursor_line_start = self.line_search_left(cursor_point);
            let cursor_line_end = self.line_search_right(cursor_point);

            // The cursor line should lie within the dirty lines.  If the
            // cursor is at the start of the line, the dirty content actually
            // ends on the previous line.
            debug_assert!(dirty_lines_start.row <= cursor_line_start.row);
            if cursor_point.row > 0 && cursor_point.col == 0 {
                debug_assert!(cursor_line_end.row <= dirty_lines_end.row + 1);
            } else {
                debug_assert!(cursor_line_end.row <= dirty_lines_end.row);
            }

            let mut with_cursor_line = Vec::with_capacity(new_displayed_rows.len() + 1);
            add_displayed_rows_with_cursor_line(
                cursor_line_start.row..=cursor_line_end.row,
                new_displayed_rows.into_iter(),
                |displayed_rows| with_cursor_line.push(displayed_rows),
            );
            new_displayed_rows = with_cursor_line
        }

        let removed = self.update_filtered_dirty_lines(
            dirty_lines_start.row..=dirty_lines_end.row,
            new_displayed_rows,
            num_context_lines,
        );
        if let Some(filter_state) = self.filter_state.as_mut() {
            let num_new_matched_lines = line_ranges.len();
            let num_removed_matched_lines = removed
                .iter()
                .filter(|rows| rows.source == DisplaySource::FilterMatch)
                .count();
            filter_state.num_matched_lines = filter_state
                .num_matched_lines
                .saturating_add(num_new_matched_lines)
                .saturating_sub(num_removed_matched_lines);
        }
    }

    /// Augments the line ranges with the context lines above and below each
    /// input line range. Returns a vector of [`DisplayedRows`] objects representing
    /// the line ranges and context lines in ascending order.
    fn add_context_lines<'a>(
        &self,
        line_ranges: impl Iterator<Item = &'a RangeInclusive<usize>>,
        num_lines: usize,
    ) -> Vec<DisplayedRows> {
        let mut displayed_rows = Vec::new();

        let mut prev_row_end = None;
        let mut line_ranges = line_ranges.peekable();
        while let Some(rows) = line_ranges.next() {
            let left_context_line_range =
                self.lines_range_left(*rows.start(), num_lines, prev_row_end);
            let next_line_range_start = line_ranges.peek().map(|range| *range.start());
            let right_context_line_range =
                self.lines_range_right(*rows.end(), num_lines, next_line_range_start);
            prev_row_end = Some(
                right_context_line_range
                    .as_ref()
                    .map_or_else(|| *rows.end(), |range| *range.end()),
            );

            if let Some(left_context_line_range) = left_context_line_range {
                displayed_rows.push(DisplayedRows {
                    range: left_context_line_range,
                    source: DisplaySource::FilterContext,
                });
            }
            displayed_rows.push(DisplayedRows {
                range: rows.clone(),
                source: DisplaySource::FilterMatch,
            });
            if let Some(right_context_line_range) = right_context_line_range {
                displayed_rows.push(DisplayedRows {
                    range: right_context_line_range,
                    source: DisplaySource::FilterContext,
                });
            }
        }

        displayed_rows
    }

    /// Finds the row range of the lines immediately before the given row. This
    /// is non-inclusive, so the range will not include the line of the given
    /// row. If a bound is given, the returned range will not extend past the
    /// bound.
    fn lines_range_left(
        &self,
        row: usize,
        num_lines: usize,
        left_bound: Option<usize>,
    ) -> Option<RangeInclusive<usize>> {
        if num_lines == 0 {
            return None;
        }
        let leftmost_valid_row = left_bound.map(|b| b + 1).unwrap_or(0);
        let mut current_line_start = self.line_search_left(Point { row, col: 0 });
        if current_line_start.row <= leftmost_valid_row {
            return None;
        }

        let end_row = current_line_start.row.saturating_sub(1);
        for _ in 0..num_lines {
            if current_line_start.row <= leftmost_valid_row {
                break;
            }
            current_line_start = self.line_search_left(Point {
                row: current_line_start.row.saturating_sub(1),
                col: 0,
            });
        }
        let start_row = max(current_line_start.row, leftmost_valid_row);
        Some(start_row..=end_row)
    }

    /// Finds the row range of the lines immediately after the given row. This
    /// is non-inclusive, so the range will not include the line of the given
    /// row. If a bound is given, the returned range will not extend past the
    /// bound.
    fn lines_range_right(
        &self,
        row: usize,
        num_lines: usize,
        right_bound: Option<usize>,
    ) -> Option<RangeInclusive<usize>> {
        if num_lines == 0 {
            return None;
        }
        let rightmost_valid_row = right_bound
            .map(|b| b.saturating_sub(1))
            .unwrap_or_else(|| self.max_content_row());
        let mut current_line_end = self.line_search_right(Point { row, col: 0 });
        if current_line_end.row >= rightmost_valid_row {
            return None;
        }

        let start_row = min(current_line_end.row + 1, rightmost_valid_row);
        for _ in 0..num_lines {
            if current_line_end.row >= rightmost_valid_row {
                break;
            }
            current_line_end = self.line_search_right(Point {
                row: current_line_end.row + 1,
                col: 0,
            });
        }
        let end_row = min(current_line_end.row, rightmost_valid_row);
        Some(start_row..=end_row)
    }

    pub fn filter_has_context_lines(&self) -> bool {
        self.filter_state
            .as_ref()
            .is_some_and(|filter_state| filter_state.num_context_lines > 0)
    }

    /// Updates the displayed output after a range of filtered lines have been
    /// dirtied and new line matches have been found. In general, this will
    /// remove existing line ranges that were in the dirty range and add the
    /// new line ranges in their place, while making some adjustments to make
    /// sure the context lines are still correct.
    ///
    /// Returns the [`DisplayedRows`] objects that were removed from the displayed
    /// output.
    ///
    /// Invariant: Caller must ensure that any new non-context lines lie within
    /// the dirty range. Context lines at the beginning/end of the new displayed
    /// rows are allowed to exceed the dirty range.
    fn update_filtered_dirty_lines(
        &mut self,
        dirty_range: RangeInclusive<usize>,
        mut new_displayed_rows: Vec<DisplayedRows>,
        num_context_lines: usize,
    ) -> Vec<DisplayedRows> {
        let Some(displayed_output) = self.displayed_output.as_ref() else {
            return Vec::new();
        };
        let displayed_rows = displayed_output.displayed_rows();

        // If there are no displayed rows or the dirty range is past the end of
        // the displayed rows, we can insert the new line ranges at the end.
        if displayed_rows.is_empty()
            || displayed_rows
                .last()
                .is_some_and(|rows| rows.range.end() < dirty_range.start())
        {
            // TODO(daniel): If the last row of the displayed output is a context
            // line, and the dirty range is adjacent to it, we should refresh it
            // to ensure it is up-to-date.
            //
            // AFAICT, we never actually hit this case, because dirty range
            // always includes the previous cursor line (which is displayed), so
            // we never have a case where the dirty range is completely past the
            // displayed rows. Leaving this in for the sake of completeness.
            let left_bound = displayed_rows.last().map(|rows| *rows.range.end());
            trim_context_lines(&mut new_displayed_rows, left_bound, None);
            self.extend_displayed_lines(new_displayed_rows);
            return Vec::new();
        }

        // If the dirty range is before the start of the displayed rows, we can
        // insert the new line ranges at the start.
        if displayed_rows
            .first()
            .is_some_and(|rows| dirty_range.end() < rows.range.start())
        {
            let right_bound = displayed_rows.first().map(|rows| *rows.range.start());
            trim_context_lines(&mut new_displayed_rows, None, right_bound);
            self.prepend_displayed_lines(new_displayed_rows);
            return Vec::new();
        }

        // Find the start/end indices and the corresponding values of row ranges
        // that are in the dirty range.
        let Some((replace_start_idx, replace_start)) =
            displayed_output.first_rows_greater_than_or_contained_in(*dirty_range.start())
        else {
            log::error!("Could not find replacement start when updating dirty filtered lines.");
            return Vec::new();
        };
        let Some((replace_end_idx, replace_end)) =
            displayed_output.last_rows_less_than_or_contained_in(*dirty_range.end())
        else {
            log::error!("Could not find replacement end when updating dirty filtered lines.");
            return Vec::new();
        };

        let replace_start_idx = self.adjust_replace_start_idx(
            dirty_range.clone(),
            displayed_rows,
            &mut new_displayed_rows,
            num_context_lines,
            replace_start_idx,
            replace_start,
            replace_end_idx,
        );
        let replace_end_idx = self.adjust_replace_end_idx(
            dirty_range.clone(),
            displayed_rows,
            &mut new_displayed_rows,
            num_context_lines,
            replace_start_idx,
            replace_end_idx,
            replace_end,
        );

        let replace_range = if replace_start_idx <= replace_end_idx {
            replace_start_idx..(replace_end_idx + 1)
        } else {
            // This should only happen if the entire dirty range lies between two
            // row ranges in the displayed output. i.e. if the displayed rows are
            // [0..=3, 7..=10] and the dirty range is 4..=5, then replace_start_idx
            // will be 1 and replace_end_idx will be 0. In this case we want to
            // insert new_line_ranges into position 1 without replacing any items.
            replace_start_idx..replace_start_idx
        };

        self.splice_displayed_lines(replace_range, new_displayed_rows)
    }

    fn extend_displayed_lines(&mut self, displayed_lines: Vec<DisplayedRows>) {
        if let Some(displayed_output) = self.displayed_output.as_mut() {
            displayed_output.extend_displayed_lines(displayed_lines);
        }
    }

    fn prepend_displayed_lines(&mut self, displayed_lines: Vec<DisplayedRows>) {
        if let Some(displayed_output) = self.displayed_output.as_mut() {
            displayed_output.prepend_displayed_lines(displayed_lines);
        }
    }

    fn splice_displayed_lines(
        &mut self,
        replace_range: Range<usize>,
        displayed_lines: Vec<DisplayedRows>,
    ) -> Vec<DisplayedRows> {
        if let Some(displayed_output) = self.displayed_output.as_mut() {
            displayed_output.splice_displayed_lines(replace_range, displayed_lines)
        } else {
            Vec::new()
        }
    }

    /// Makes manual adjustments to the index we are starting a replacement from
    /// depending on the dirty range position. Adjustments may be necessary if
    /// there are context lines or if the dirty range falls in the middle of a
    /// displayed range.
    ///
    /// May mutate `new_displayed_rows` with new rows.
    ///
    /// Note: I refer to left/right context lines, these are equivalent to context
    /// lines above/below a matched, respectively.
    /// If there are context lines, we find the closest filter match before the
    /// current replace index and re-search for its right context lines. Then,
    /// we update the replace index to include all `DisplayedRows` up until this
    /// filter match. This serves a few different cases:
    /// 1. The dirty range may have changed lines in the right context of a
    ///    filter match, or added new lines that should be included in the right
    ///    context. Re-searching for the right context of the previous filter
    ///    match ensures it's up-to-date.
    ///    e.g. ```
    ///         context
    ///         match
    ///         context <- start of dirty range, refresh this context
    ///         ```
    /// 2. If the start replace index ends on a filter match, we need to remove
    ///    the left context associated with the match. Replacing up until the
    ///    previous filter match ensures the left context is removed.
    ///    e.g. ```
    ///         context
    ///         match
    ///         context <- refresh this context, new replace_start_idx
    ///         context
    ///         match   <- start of dirty range, old replace_start_idx
    ///         context
    ///         ```
    /// 3. Due to the way contexts are found, the left context of a filter match
    ///    might include a part of the previous filter match's right context (to
    ///    avoid overlapping ranges). So if the start replace index ends on a
    ///    left context of a filter match, we should re-search for the right
    ///    context of the previous filter match, even though it is not in the
    ///    dirty range, to make sure it is up-to-date.
    ///    e.g. ```
    ///         context
    ///         match
    ///         context <- refresh this context, new replace_start_idx
    ///         context <- start of dirty range, old replace_start_idx
    ///         match
    ///         context
    ///         ```
    /// TODO(daniel): This approach only works when the only `DisplaySource`
    /// variants are `FilterMatch`, `FilterContext`, and `CursorLine`. If more
    /// variants are added, a more robust algorithm is needed.
    #[allow(clippy::too_many_arguments)]
    fn adjust_replace_start_idx(
        &self,
        dirty_range: RangeInclusive<usize>,
        displayed_rows: &[DisplayedRows],
        new_displayed_rows: &mut Vec<DisplayedRows>,
        num_context_lines: usize,
        mut replace_start_idx: usize,
        replace_start: &DisplayedRows,
        replace_end_idx: usize,
    ) -> usize {
        let mut new_start_rows = None;

        if replace_start_idx <= replace_end_idx {
            if num_context_lines > 0 {
                let rows_before = &displayed_rows[..replace_start_idx];
                // Find the position of the previous filter match.
                if let Some((prev_filter_match_idx, prev_filter_match)) = rows_before
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, rows)| rows.source == DisplaySource::FilterMatch)
                {
                    // Find the right context of the previous filter match.
                    if let Some(right_context_range) = self.lines_range_right(
                        *prev_filter_match.range.end(),
                        num_context_lines,
                        new_displayed_rows.first().map(|rows| *rows.range.start()),
                    ) {
                        // Add the refreshed right context into `new_displayed_rows`.
                        new_start_rows = Some(DisplayedRows {
                            range: right_context_range,
                            source: DisplaySource::FilterContext,
                        });
                    }
                    // Start the replacement after the previous filter match.
                    replace_start_idx = prev_filter_match_idx + 1;
                } else {
                    // No previous filter match was found, we should start the
                    // replacement at the first displayed rows to cover case 2.
                    replace_start_idx = 0;
                }
            } else if replace_start.range.contains(dirty_range.start())
                && replace_start.range.start() != dirty_range.start()
            {
                // Check if the start of the dirty range intersects with any
                // displayed row ranges. If so, we add back the part of the row
                // ranges that were in the displayed row range but not in the
                // dirty range. i.e. if the displayed rows are [0..=3, 7..=10]
                // and the dirty range is 2..=5, then we need to make sure
                // 0..=1 is included in the final result.
                let amended_range = *replace_start.range.start()..=(*dirty_range.start() - 1);
                new_start_rows = Some(DisplayedRows {
                    range: amended_range,
                    source: replace_start.source,
                });
            }
        }

        // The input should be bounded by the existing displayed rows just
        // before the dirty range.
        let existing_left_bound = if replace_start_idx == 0 {
            None
        } else {
            displayed_rows
                .get(replace_start_idx - 1)
                .map(|rows| *rows.range.end())
        };
        // If we are adding new start rows to `new_displayed_rows`, this is a
        // stricter left bound.
        let left_bound = new_start_rows
            .as_ref()
            .map(|rows| *rows.range.end())
            .or(existing_left_bound);
        trim_context_lines(new_displayed_rows, left_bound, None);

        if let Some(new_start_rows) = new_start_rows {
            new_displayed_rows.insert(0, new_start_rows);
        }

        replace_start_idx
    }

    /// Makes manual adjustments to the index we are ending a replacement on
    /// depending on the dirty range position. Adjustments may be necessary if
    /// there are context lines or if the dirty range falls in the middle of a
    /// displayed range.
    ///
    /// May mutate `new_displayed_rows` with new rows.
    ///
    /// For a detailed description of the algorithm we use to make adjustments
    /// when there are context lines, with examples, see the docstring for
    /// [`adjust_replace_start_idx`].
    #[allow(clippy::too_many_arguments)]
    fn adjust_replace_end_idx(
        &self,
        dirty_range: RangeInclusive<usize>,
        displayed_rows: &[DisplayedRows],
        new_displayed_rows: &mut Vec<DisplayedRows>,
        num_context_lines: usize,
        replace_start_idx: usize,
        mut replace_end_idx: usize,
        replace_end: &DisplayedRows,
    ) -> usize {
        let mut new_end_rows = None;

        if replace_start_idx <= replace_end_idx {
            if num_context_lines > 0 {
                let slice_start_idx = replace_end_idx + 1;
                let rows_after = &displayed_rows[slice_start_idx..];
                // Find the position of the next filter match.
                if let Some((next_filter_match_idx, next_filter_match)) = rows_after
                    .iter()
                    .enumerate()
                    .find(|(_, rows)| rows.source == DisplaySource::FilterMatch)
                {
                    // Find the left context of the next filter match.
                    if let Some(left_context_range) = self.lines_range_left(
                        *next_filter_match.range.start(),
                        num_context_lines,
                        new_displayed_rows.last().map(|rows| *rows.range.end()),
                    ) {
                        // Add the refreshed left context into `new_displayed_rows`.
                        new_end_rows = Some(DisplayedRows {
                            range: left_context_range,
                            source: DisplaySource::FilterContext,
                        });
                    }
                    // End the replacement before the next filter match.
                    replace_end_idx = (next_filter_match_idx + slice_start_idx).saturating_sub(1);
                } else {
                    // No next filter match was found, we should end the
                    // replacement at the last displayed rows to cover case 2.
                    replace_end_idx = displayed_rows.len().saturating_sub(1);
                }
            } else if replace_end.range.contains(dirty_range.end())
                && replace_end.range.end() != dirty_range.end()
            {
                // Check if the end of the dirty range intersects with any
                // displayed row ranges. If so, we add back the part of the row
                // ranges that were in the displayed row range but not in the
                // dirty range. i.e. if the displayed rows are [0..=3, 7..=10]
                // and the dirty range is 5..=9, then we need to make sure
                // 10..=10 is included in the final result.
                let amended_range = (dirty_range.end() + 1)..=*replace_end.range.end();
                new_end_rows = Some(DisplayedRows {
                    range: amended_range,
                    source: replace_end.source,
                });
            }
        }

        // The input should be bounded by the existing displayed rows just
        // after the dirty range.
        let existing_right_bound = displayed_rows
            .get(replace_end_idx + 1)
            .map(|rows| *rows.range.start());
        // If we are adding new end rows to `new_displayed_rows`, this is a
        // stricter right bound.
        let right_bound = new_end_rows
            .as_ref()
            .map(|rows| *rows.range.start())
            .or(existing_right_bound);
        trim_context_lines(new_displayed_rows, None, right_bound);

        if let Some(new_end_rows) = new_end_rows {
            new_displayed_rows.push(new_end_rows);
        }

        replace_end_idx
    }

    #[cfg(test)]
    pub(super) fn set_displayed_output(&mut self, displayed_output: DisplayedOutput) {
        self.displayed_output = Some(displayed_output);
    }
}

/// Calls `add_displayed_rows_fn` with the given cursor line and line ranges in
/// ascending order. If the cursor line range is being added,
/// `add_displayed_rows_fn` will be called with `DisplaySource::CursorLine`,
/// otherwise it will be called with the given `display_source`. If the cursor
/// line range is already included in the line range, will do nothing and return
/// false.
///
/// Invariants:
///   - All of the line ranges must correspond to logical lines in the grid. A
///     range bound cannot be in the middle of a logical line.
///   - The given line ranges must be in ascending order.
fn add_displayed_rows_with_cursor_line(
    cursor_line_range: RangeInclusive<usize>,
    line_ranges: impl Iterator<Item = DisplayedRows>,
    mut add_displayed_rows_fn: impl FnMut(DisplayedRows),
) -> bool {
    let mut cursor_greater_than_prev_rows = true;
    let mut cursor_line_inserted = false;
    for rows in line_ranges {
        if cursor_greater_than_prev_rows && cursor_line_range.end() < rows.range.start() {
            add_displayed_rows_fn(DisplayedRows {
                range: cursor_line_range.clone(),
                source: DisplaySource::CursorLine,
            });
            cursor_line_inserted = true;
        }

        cursor_greater_than_prev_rows = cursor_line_range.start() > rows.range.end();
        add_displayed_rows_fn(rows);
    }

    if !cursor_line_inserted && cursor_greater_than_prev_rows {
        add_displayed_rows_fn(DisplayedRows {
            range: cursor_line_range.clone(),
            source: DisplaySource::CursorLine,
        });
        cursor_line_inserted = true;
    }

    cursor_line_inserted
}

/// Adjusts the first/last context lines in the given displayed rows to respect
/// the given bounds. Will mutate the given displayed rows if an adjustment is
/// made.
///
/// Does nothing if no bounds are passed or if the given displayed rows does not
/// start/end with context lines.
fn trim_context_lines(
    displayed_rows: &mut Vec<DisplayedRows>,
    left_bound: Option<usize>,
    right_bound: Option<usize>,
) {
    if let Some(left_bound) = left_bound {
        if let Some(first_rows) = displayed_rows.first_mut() {
            if left_bound >= *first_rows.range.end() {
                displayed_rows.remove(0);
            } else if first_rows.range.contains(&left_bound) {
                first_rows.range = (left_bound + 1)..=*first_rows.range.end();
            }
        }
    }

    if let Some(right_bound) = right_bound {
        if let Some(last_rows) = displayed_rows.last_mut() {
            if right_bound <= *last_rows.range.start() {
                displayed_rows.pop();
            } else if last_rows.range.contains(&right_bound) {
                last_rows.range = *last_rows.range.start()..=(right_bound - 1);
            }
        }
    }
}

#[cfg(test)]
#[path = "filtering_tests.rs"]
mod tests;
