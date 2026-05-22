use warp_terminal::model::grid::cell::Flags;

use super::*;
use crate::terminal::model::ansi::Handler as _;
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::index::{VisiblePoint, VisibleRow};
use crate::test_util::mock_blockgrid;

/// Converts a vector of ranges into DisplayedRows objects with source FilterMatch.
fn make_displayed_rows_from_ranges(ranges: Vec<RangeInclusive<usize>>) -> Vec<DisplayedRows> {
    ranges
        .into_iter()
        .map(|range| DisplayedRows {
            range,
            source: DisplaySource::FilterMatch,
        })
        .collect_vec()
}

/// Helper function for creating the string used to mock a blockgrid. Adds one
/// or more context lines where every row contains "context".
fn add_context_lines_to_grid_str(
    grid_str: &mut String,
    range: RangeInclusive<usize>,
    num_lines: usize,
) {
    let range_len = (*range.end() - *range.start()) + 1;
    // Ceiling division to find the interval at which we end the logical lines.
    let gap = range_len.div_ceil(num_lines);

    for row in range.clone() {
        grid_str.push_str("context");

        let i = row - *range.start();
        let should_end_line = row == *range.end() || i % gap == gap - 1;
        if should_end_line {
            grid_str.push_str("\r\n");
        } else {
            grid_str.push('\n');
        }
    }
}

/// Helper function for creating the string used to mock a blockgrid. Adds a
/// single match line where every row contains "match", and all rows contain
/// a wrapline except the last one.
fn add_match_line_to_grid_str(grid_str: &mut String, range: RangeInclusive<usize>) {
    for row in range.clone() {
        grid_str.push_str("match");
        if row == *range.end() {
            grid_str.push_str("\r\n");
        } else {
            grid_str.push('\n');
        }
    }
}

/// Helper function for creating a mock blockgrid from a set of displayed rows.
/// The input vector should have format: (range, source, num_lines)
///     {range}: the range of row indexes these displayed rows should occupy.
///     {source}: the type of these displayed rows.
///     {num_lines}: the number of logical lines to divide the row range into.
///
/// Example:
/// Calling this function with input
/// [
///     (0..=1, DisplaySource::FilterContext, 2),
///     (2..=3, DisplaySource::FilterMatch, 1),
///     (3..=5, DisplaySource::FilterContext, 2),
/// ]
/// will result in constructing a blockgrid with format:
///     "context\r\n\
///     context\r\n\
///     match\n\
///     match\r\n
///     context\n\
///     context\r\n\
///     context\r\n\"
/// where "\r\n" represents a logical line end, and "\n" represents a line wrap.
/// See `mock_blockgrid` for more details on how we mock the blockgrid from this
/// string.
fn make_blockgrid_from_displayed_rows(
    input: Vec<(RangeInclusive<usize>, DisplaySource, usize)>,
    num_context_lines: usize,
) -> BlockGrid {
    let mut grid_str = String::new();
    let mut prev_end_row = None;
    let mut displayed_lines = Vec::new();
    for (range, source, num_lines) in input {
        displayed_lines.push(DisplayedRows::new(range.clone(), source));

        // Fill in any gaps between displayed rows.
        if let Some(prev_end_row) = prev_end_row {
            for _ in (prev_end_row + 1)..*range.start() {
                grid_str.push_str("other\r\n");
            }
        }

        match source {
            DisplaySource::FilterMatch => {
                add_match_line_to_grid_str(&mut grid_str, range.clone());
            }
            DisplaySource::FilterContext => {
                add_context_lines_to_grid_str(&mut grid_str, range.clone(), num_lines);
            }
            _ => {
                for _ in range.clone() {
                    grid_str.push_str("other\r\n");
                }
            }
        }
        prev_end_row = Some(*range.end());
    }

    let displayed_output = DisplayedOutput::new_from_displayed_lines(displayed_lines);
    let mut blockgrid = mock_blockgrid(&grid_str);
    let dfas = RegexDFAs::new("match").unwrap();
    blockgrid.filter_lines(Arc::new(dfas), num_context_lines, false);
    // Override the DisplayedOutput created by filter_lines with our own.
    blockgrid
        .grid_handler_mut()
        .set_displayed_output(displayed_output);

    blockgrid
}

fn update_line_in_blockgrid(
    grid: &mut BlockGrid,
    row: usize,
    source: DisplaySource,
    wrap_line: bool,
) {
    let line_text = match source {
        DisplaySource::FilterMatch => "match",
        DisplaySource::FilterContext => "context",
        _ => "other",
    };

    let grid = grid.grid_handler_mut().grid_storage_mut();

    for (col, c) in line_text.chars().enumerate() {
        grid[row][col].c = c;
    }

    let num_cols = grid.columns();
    let flags = grid[row][num_cols - 1].flags_mut();
    if wrap_line {
        flags.insert(Flags::WRAPLINE);
    } else {
        flags.remove(Flags::WRAPLINE);
    }
}

#[test]
pub fn test_filter_lines_with_no_cursor_line_sets_num_matched_lines() {
    let mut blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        saskatchewan",
    );

    let dfas = Arc::new(RegexDFAs::new("alberta").unwrap());
    blockgrid.grid_handler_mut().filter_lines(dfas, 0, false);

    let num_matched_lines = blockgrid
        .grid_handler()
        .filter_state
        .as_ref()
        .unwrap()
        .num_matched_lines;
    assert_eq!(num_matched_lines, 2);
}

#[test]
pub fn test_filter_lines_with_cursor_line_sets_num_matched_lines() {
    let mut blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        saskatchewan",
    );

    let dfas = Arc::new(RegexDFAs::new("alberta").unwrap());
    // Set the cursor to the row after "saskatchewan".
    blockgrid.grid_handler_mut().grid_storage_mut().cursor.point = VisiblePoint {
        row: VisibleRow(5),
        col: 0,
    };
    blockgrid.grid_handler_mut().filter_lines(dfas, 0, false);
    // The displayed output should contain 3 logical lines: 2 lines from the
    // filter and 1 for the cursor line.
    assert_eq!(
        blockgrid
            .grid_handler()
            .displayed_output
            .as_ref()
            .unwrap()
            .displayed_rows()
            .len(),
        3
    );

    let num_matched_lines = blockgrid
        .grid_handler()
        .filter_state
        .as_ref()
        .unwrap()
        .num_matched_lines;
    assert_eq!(num_matched_lines, 2);
}

#[test]
pub fn test_filter_lines_with_context_lines_sets_num_matched_lines() {
    let mut blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\r\n\
        alberta\r\n\
        saskatchewan\r\n\
        ",
    );
    blockgrid.finish();

    let dfas = Arc::new(RegexDFAs::new("quebec").unwrap());
    blockgrid.grid_handler_mut().filter_lines(dfas, 2, false);

    let num_displayed_lines = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .height();
    // There is 1 matched line and 4 context lines.
    assert_eq!(num_displayed_lines, 5);

    let num_matched_lines = blockgrid
        .grid_handler()
        .filter_state
        .as_ref()
        .unwrap()
        .num_matched_lines;
    // There is only 1 matched line despite there being 5 displayed lines.
    assert_eq!(num_matched_lines, 1);
}

#[test]
pub fn test_maybe_filter_dirty_lines_with_no_cursor_line_sets_num_matched_lines() {
    let mut blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        alberta\r\n\
        ",
    );

    let dfas = Arc::new(RegexDFAs::new("alberta").unwrap());
    blockgrid.grid_handler_mut().filter_lines(dfas, 0, false);
    // After this filter we should have 3 matched lines.
    let num_matched_lines = blockgrid
        .grid_handler()
        .filter_state
        .as_ref()
        .unwrap()
        .num_matched_lines;
    assert_eq!(num_matched_lines, 3);

    // Update row 1 to be "alberta".
    let grid = blockgrid.grid_handler_mut();
    grid.set_cursor_point(1, 0);
    grid.input_at_cursor("alberta");

    // Update rows 3/4 to no longer match "alberta".
    grid.set_cursor_point(3, 0);
    grid.input_at_cursor("z");
    grid.set_cursor_point(4, 0);
    grid.input_at_cursor("y");

    blockgrid.grid_handler_mut().maybe_filter_dirty_lines();
    let num_matched_lines = blockgrid
        .grid_handler()
        .filter_state
        .as_ref()
        .unwrap()
        .num_matched_lines;

    // There was 1 matching line added, and 2 matching lines removed, so there
    // are only 3 + 1 - 2 = 2 matching lines remaining.
    assert_eq!(num_matched_lines, 2);
}

#[test]
pub fn test_maybe_filter_dirty_lines_with_cursor_line_sets_num_matched_lines() {
    let mut blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        saskatchewan\r\n\
        ",
    );

    let dfas = Arc::new(RegexDFAs::new("alberta").unwrap());
    // After this filter we should have 2 matched lines.
    blockgrid.grid_handler_mut().filter_lines(dfas, 0, false);

    let grid = blockgrid.grid_handler_mut();
    grid.set_cursor_point(1, 0);
    grid.input_at_cursor("alberta");

    // Set the cursor to the row after "saskatchewan".
    grid.set_cursor_point(5, 0);

    // Make sure the dirty cell range is updated.
    grid.update_dirty_cells_range();

    blockgrid.grid_handler_mut().maybe_filter_dirty_lines();
    // The displayed output should contain 4 logical lines: 3 lines from the
    // filter and 1 for the cursor line.
    assert_eq!(
        blockgrid
            .grid_handler()
            .displayed_output
            .as_ref()
            .unwrap()
            .displayed_rows()
            .len(),
        4
    );
    let num_matched_lines = blockgrid
        .grid_handler()
        .filter_state
        .as_ref()
        .unwrap()
        .num_matched_lines;
    assert_eq!(num_matched_lines, 3);
}

#[test]
pub fn test_add_displayed_rows_with_cursor_line() {
    let line_ranges = make_displayed_rows_from_ranges(vec![6..=11, 15..=17, 20..=25]);
    let mut displayed_rows = Vec::new();

    let cursor_line_inserted =
        add_displayed_rows_with_cursor_line(18..=19, line_ranges.into_iter(), |rows| {
            displayed_rows.push(rows)
        });

    assert!(cursor_line_inserted);
    assert_eq!(
        displayed_rows,
        vec![
            DisplayedRows {
                range: 6..=11,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 15..=17,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 18..=19,
                source: DisplaySource::CursorLine
            },
            DisplayedRows {
                range: 20..=25,
                source: DisplaySource::FilterMatch
            },
        ]
    );
}

#[test]
pub fn test_add_displayed_rows_with_cursor_line_at_front() {
    let line_ranges = make_displayed_rows_from_ranges(vec![6..=11, 15..=17, 20..=25]);
    let mut displayed_rows = Vec::new();

    let cursor_line_inserted =
        add_displayed_rows_with_cursor_line(2..=5, line_ranges.into_iter(), |rows| {
            displayed_rows.push(rows)
        });

    assert!(cursor_line_inserted);
    assert_eq!(
        displayed_rows,
        vec![
            DisplayedRows {
                range: 2..=5,
                source: DisplaySource::CursorLine
            },
            DisplayedRows {
                range: 6..=11,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 15..=17,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 20..=25,
                source: DisplaySource::FilterMatch
            },
        ]
    );
}

#[test]
pub fn test_add_displayed_rows_with_cursor_line_at_end() {
    let line_ranges = make_displayed_rows_from_ranges(vec![6..=11, 15..=17, 20..=25]);
    let mut displayed_rows = Vec::new();

    let cursor_line_inserted =
        add_displayed_rows_with_cursor_line(26..=28, line_ranges.into_iter(), |rows| {
            displayed_rows.push(rows)
        });

    assert!(cursor_line_inserted);
    assert_eq!(
        displayed_rows,
        vec![
            DisplayedRows {
                range: 6..=11,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 15..=17,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 20..=25,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 26..=28,
                source: DisplaySource::CursorLine
            },
        ]
    );
}

#[test]
pub fn test_add_displayed_rows_with_cursor_line_already_included() {
    let line_ranges = make_displayed_rows_from_ranges(vec![6..=11, 15..=17, 20..=25]);
    let mut displayed_rows = Vec::new();

    let cursor_line_inserted =
        add_displayed_rows_with_cursor_line(15..=17, line_ranges.into_iter(), |rows| {
            displayed_rows.push(rows)
        });

    assert!(!cursor_line_inserted);
    assert_eq!(
        displayed_rows,
        vec![
            DisplayedRows {
                range: 6..=11,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 15..=17,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 20..=25,
                source: DisplaySource::FilterMatch
            },
        ]
    );
}

#[test]
pub fn test_update_dirty_matches() {
    let mut filter_state = FilterState::new_with_matches_for_test(vec![
        Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
        Point { row: 15, col: 8 }..=Point { row: 16, col: 1 },
        Point { row: 12, col: 0 }..=Point { row: 12, col: 2 },
        Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
        Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
    ]);
    let new_matches = vec![
        Point { row: 12, col: 3 }..=Point { row: 15, col: 2 },
        Point { row: 10, col: 1 }..=Point { row: 10, col: 2 },
    ];

    filter_state.update_dirty_matches(7..=17, new_matches);

    assert_eq!(
        filter_state.matches,
        vec![
            Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
            Point { row: 12, col: 3 }..=Point { row: 15, col: 2 },
            Point { row: 10, col: 1 }..=Point { row: 10, col: 2 },
            Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
        ]
    );
}

#[test]
pub fn test_update_dirty_matches_exclude_partial_matches() {
    let mut filter_state = FilterState::new_with_matches_for_test(vec![
        Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
        Point { row: 15, col: 8 }..=Point { row: 16, col: 1 },
        Point { row: 12, col: 0 }..=Point { row: 12, col: 2 },
        Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
        Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
    ]);
    let new_matches = vec![
        Point { row: 12, col: 3 }..=Point { row: 15, col: 2 },
        Point { row: 9, col: 1 }..=Point { row: 10, col: 2 },
    ];

    filter_state.update_dirty_matches(9..=19, new_matches);

    assert_eq!(
        filter_state.matches,
        vec![
            Point { row: 12, col: 3 }..=Point { row: 15, col: 2 },
            Point { row: 9, col: 1 }..=Point { row: 10, col: 2 },
            Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
        ]
    );
}

#[test]
pub fn test_update_dirty_matches_insert_at_front() {
    let mut filter_state = FilterState::new_with_matches_for_test(vec![
        Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
        Point { row: 15, col: 8 }..=Point { row: 16, col: 1 },
        Point { row: 12, col: 0 }..=Point { row: 12, col: 2 },
    ]);
    let new_matches = vec![
        Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
        Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
    ];

    filter_state.update_dirty_matches(0..=11, new_matches);

    assert_eq!(
        filter_state.matches,
        vec![
            Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
            Point { row: 15, col: 8 }..=Point { row: 16, col: 1 },
            Point { row: 12, col: 0 }..=Point { row: 12, col: 2 },
            Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
            Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
        ]
    );
}

#[test]
pub fn test_update_dirty_matches_insert_at_back() {
    let mut filter_state = FilterState::new_with_matches_for_test(vec![
        Point { row: 12, col: 0 }..=Point { row: 12, col: 2 },
        Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
        Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
    ]);
    let new_matches = vec![
        Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
        Point { row: 15, col: 8 }..=Point { row: 16, col: 1 },
    ];

    filter_state.update_dirty_matches(14..=21, new_matches);

    assert_eq!(
        filter_state.matches,
        vec![
            Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
            Point { row: 15, col: 8 }..=Point { row: 16, col: 1 },
            Point { row: 12, col: 0 }..=Point { row: 12, col: 2 },
            Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
            Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
        ]
    );
}

#[test]
pub fn test_update_dirty_matches_replace_single_match() {
    let mut filter_state = FilterState::new_with_matches_for_test(vec![
        Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
        Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
        Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
    ]);
    let new_matches = vec![
        Point { row: 10, col: 1 }..=Point { row: 10, col: 2 },
        Point { row: 8, col: 3 }..=Point { row: 9, col: 2 },
    ];

    filter_state.update_dirty_matches(8..=10, new_matches);

    assert_eq!(
        filter_state.matches,
        vec![
            Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
            Point { row: 10, col: 1 }..=Point { row: 10, col: 2 },
            Point { row: 8, col: 3 }..=Point { row: 9, col: 2 },
            Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
        ]
    );
}

#[test]
pub fn test_update_dirty_matches_no_replacement() {
    let mut filter_state = FilterState::new_with_matches_for_test(vec![
        Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
        Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
        Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
    ]);
    let new_matches = vec![
        Point { row: 15, col: 1 }..=Point { row: 15, col: 2 },
        Point { row: 12, col: 3 }..=Point { row: 14, col: 2 },
    ];

    filter_state.update_dirty_matches(12..=15, new_matches);

    assert_eq!(
        filter_state.matches,
        vec![
            Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
            Point { row: 15, col: 1 }..=Point { row: 15, col: 2 },
            Point { row: 12, col: 3 }..=Point { row: 14, col: 2 },
            Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
            Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
        ]
    );
}

#[test]
pub fn test_update_dirty_matches_empty_new_matches() {
    let mut filter_state = FilterState::new_with_matches_for_test(vec![
        Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
        Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
        Point { row: 2, col: 0 }..=Point { row: 4, col: 2 },
    ]);
    let new_matches = vec![];

    filter_state.update_dirty_matches(2..=4, new_matches);

    assert_eq!(
        filter_state.matches,
        vec![
            Point { row: 18, col: 0 }..=Point { row: 20, col: 2 },
            Point { row: 8, col: 4 }..=Point { row: 10, col: 6 },
        ]
    );
}

#[test]
pub fn test_block_filter_works_with_truncated_history() {
    let mut blockgrid = mock_blockgrid("palak\r\npaneer\r\nis yummy");
    // We set the max_scroll_limit to 0 to make sure that rows are truncated on scroll_up.
    blockgrid.grid_handler_mut().set_max_scroll_limit(0);

    let dfas = RegexDFAs::new("pa").unwrap();
    blockgrid.filter_lines(Arc::new(dfas), 0, false);
    let num_matched_lines = blockgrid
        .grid_handler()
        .filter_state
        .as_ref()
        .unwrap()
        .num_matched_lines;
    assert_eq!(num_matched_lines, 2);

    blockgrid.grid_handler_mut().scroll_up(1);

    blockgrid.maybe_refilter_lines();
    let num_matched_lines = blockgrid
        .grid_handler()
        .filter_state
        .as_ref()
        .unwrap()
        .num_matched_lines;
    assert_eq!(num_matched_lines, 1);
}

#[test]
pub fn test_add_context_lines() {
    let blockgrid = mock_blockgrid(
        "\
        line\r\n\
        line\r\n\
        match\r\n\
        line\r\n\
        line\r\n\
        line\r\n\
        line\r\n\
        line\r\n\
        match\r\n\
        line\r\n\
        line\r\n\
        line\r\n\
        line\r\n\
        match\r\n\
        line\r\n\
        line\r\n",
    );
    let line_ranges = [2..=2, 8..=8, 13..=13];

    let displayed_rows = blockgrid
        .grid_handler()
        .add_context_lines(line_ranges.iter(), 2);
    assert_eq!(
        displayed_rows,
        vec![
            DisplayedRows {
                range: 0..=1,
                source: DisplaySource::FilterContext
            },
            DisplayedRows {
                range: 2..=2,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 3..=4,
                source: DisplaySource::FilterContext
            },
            DisplayedRows {
                range: 6..=7,
                source: DisplaySource::FilterContext
            },
            DisplayedRows {
                range: 8..=8,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 9..=10,
                source: DisplaySource::FilterContext
            },
            DisplayedRows {
                range: 11..=12,
                source: DisplaySource::FilterContext
            },
            DisplayedRows {
                range: 13..=13,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 14..=15,
                source: DisplaySource::FilterContext
            },
        ]
    );
}

#[test]
pub fn test_add_context_lines_overlapping() {
    let blockgrid = mock_blockgrid(
        "\
        line\r\n\
        line\r\n\
        line\r\n\
        match\r\n\
        line\r\n\
        line\r\n\
        line\r\n\
        line\r\n\
        match\r\n\
        match\r\n\
        line\r\n\
        line\r\n\
        line\r\n",
    );
    let line_ranges = [3..=3, 8..=8, 9..=9];

    let displayed_rows = blockgrid
        .grid_handler()
        .add_context_lines(line_ranges.iter(), 3);
    assert_eq!(
        displayed_rows,
        vec![
            DisplayedRows {
                range: 0..=2,
                source: DisplaySource::FilterContext
            },
            DisplayedRows {
                range: 3..=3,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 4..=6,
                source: DisplaySource::FilterContext
            },
            DisplayedRows {
                range: 7..=7,
                source: DisplaySource::FilterContext
            },
            DisplayedRows {
                range: 8..=8,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 9..=9,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 10..=12,
                source: DisplaySource::FilterContext
            },
        ]
    );
}

#[test]
pub fn test_add_context_lines_at_grid_bounds() {
    let blockgrid = mock_blockgrid(
        "\
        line\r\n\
        line\r\n\
        match\r\n\
        line\r\n\
        line\r\n",
    );
    let line_ranges = [2..=2];

    let displayed_rows = blockgrid
        .grid_handler()
        .add_context_lines(line_ranges.iter(), 5);
    assert_eq!(
        displayed_rows,
        vec![
            DisplayedRows {
                range: 0..=1,
                source: DisplaySource::FilterContext
            },
            DisplayedRows {
                range: 2..=2,
                source: DisplaySource::FilterMatch
            },
            DisplayedRows {
                range: 3..=4,
                source: DisplaySource::FilterContext
            },
        ]
    );
}

#[test]
pub fn test_add_context_lines_no_context() {
    let blockgrid = mock_blockgrid("match");
    let line_ranges = [0..=0];

    let displayed_rows = blockgrid
        .grid_handler()
        .add_context_lines(line_ranges.iter(), 5);
    assert_eq!(
        displayed_rows,
        vec![DisplayedRows {
            range: 0..=0,
            source: DisplaySource::FilterMatch
        },]
    );
}

#[test]
pub fn test_trim_context_lines_no_trimming() {
    let mut displayed_rows = vec![
        DisplayedRows::new(1..=4, DisplaySource::FilterContext),
        DisplayedRows::new(5..=6, DisplaySource::FilterMatch),
        DisplayedRows::new(7..=8, DisplaySource::FilterMatch),
        DisplayedRows::new(9..=12, DisplaySource::FilterContext),
    ];

    let expected_displayed_rows = displayed_rows.clone();
    trim_context_lines(&mut displayed_rows, Some(0), Some(13));

    assert_eq!(displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_trim_context_lines_trim_partially() {
    let mut displayed_rows = vec![
        DisplayedRows::new(1..=4, DisplaySource::FilterContext),
        DisplayedRows::new(5..=6, DisplaySource::FilterMatch),
        DisplayedRows::new(7..=8, DisplaySource::FilterMatch),
        DisplayedRows::new(9..=12, DisplaySource::FilterContext),
    ];

    trim_context_lines(&mut displayed_rows, Some(1), Some(12));

    let expected_displayed_rows = vec![
        DisplayedRows::new(2..=4, DisplaySource::FilterContext),
        DisplayedRows::new(5..=6, DisplaySource::FilterMatch),
        DisplayedRows::new(7..=8, DisplaySource::FilterMatch),
        DisplayedRows::new(9..=11, DisplaySource::FilterContext),
    ];
    assert_eq!(displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_trim_context_lines_trim_entirely() {
    let mut displayed_rows = vec![
        DisplayedRows::new(1..=4, DisplaySource::FilterContext),
        DisplayedRows::new(5..=6, DisplaySource::FilterMatch),
        DisplayedRows::new(7..=8, DisplaySource::FilterMatch),
        DisplayedRows::new(9..=12, DisplaySource::FilterContext),
    ];

    trim_context_lines(&mut displayed_rows, Some(4), Some(9));

    let expected_displayed_rows = vec![
        DisplayedRows::new(5..=6, DisplaySource::FilterMatch),
        DisplayedRows::new(7..=8, DisplaySource::FilterMatch),
    ];
    assert_eq!(displayed_rows, expected_displayed_rows);

    let mut displayed_rows = vec![
        DisplayedRows::new(1..=4, DisplaySource::FilterContext),
        DisplayedRows::new(5..=6, DisplaySource::FilterMatch),
        DisplayedRows::new(7..=8, DisplaySource::FilterMatch),
        DisplayedRows::new(9..=12, DisplaySource::FilterContext),
    ];

    trim_context_lines(&mut displayed_rows, Some(5), Some(8));

    let expected_displayed_rows = vec![
        DisplayedRows::new(5..=6, DisplaySource::FilterMatch),
        DisplayedRows::new(7..=8, DisplaySource::FilterMatch),
    ];
    assert_eq!(displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=11, 14..=16]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![5..=5, 7..=9]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(5..=12, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 5..=5, 7..=9, 14..=16])
    );
    assert_eq!(updated_displayed_output.height(), 10);

    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=11, 14..=16, 20..=21]);
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![5..=5]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(5..=17, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 5..=5, 20..=21])
    );
    assert_eq!(updated_displayed_output.height(), 6);
}

#[test]
pub fn test_update_filtered_dirty_lines_at_range_borders() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=10, 13..=15, 17..=20]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![11..=12]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(6..=15, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 11..=12, 17..=20])
    );
    assert_eq!(updated_displayed_output.height(), 9);

    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=10, 13..=15, 17..=20]);
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![11..=12]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(10..=13, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 6..=9, 11..=12, 14..=15, 17..=20])
    );
    assert_eq!(updated_displayed_output.height(), 15);
}

#[test]
pub fn test_update_filtered_dirty_lines_middle_of_range() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=11, 12..=14, 17..=20]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![8..=10, 12..=12]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(8..=13, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 6..=7, 8..=10, 12..=12, 14..=14, 17..=20])
    );
    assert_eq!(updated_displayed_output.height(), 14);
}

#[test]
pub fn test_update_filtered_dirty_lines_no_replacement() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=11, 12..=14]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = vec![];
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(5..=11, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 12..=14])
    );
    assert_eq!(updated_displayed_output.height(), 6);
}

#[test]
pub fn test_update_filtered_dirty_lines_replace_one_line() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=10, 12..=14]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![7..=7]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(6..=10, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 7..=7, 12..=14])
    );
    assert_eq!(updated_displayed_output.height(), 7);
}

#[test]
pub fn test_update_filtered_dirty_lines_replace_single_row_line() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 7..=7, 12..=14]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![6..=8]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(7..=7, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 6..=8, 12..=14])
    );
    assert_eq!(updated_displayed_output.height(), 9);
}

#[test]
pub fn test_update_filtered_dirty_lines_replace_at_start() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=10, 12..=14]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![3..=5]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(1..=11, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![3..=5, 12..=14])
    );
    assert_eq!(updated_displayed_output.height(), 6);
}

#[test]
pub fn test_update_filtered_dirty_lines_replace_at_end() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=10, 12..=14]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![16..=18]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(12..=16, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 6..=10, 16..=18])
    );
    assert_eq!(updated_displayed_output.height(), 11);
}

#[test]
pub fn test_update_filtered_dirty_lines_insert_at_end() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=11, 12..=14]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![16..=18, 25..=30]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(16..=32, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 6..=11, 12..=14, 16..=18, 25..=30])
    );
    assert_eq!(updated_displayed_output.height(), 21);
}

#[test]
pub fn test_update_filtered_dirty_lines_insert_at_start() {
    let displayed_output = DisplayedOutput::new_for_test(vec![6..=11, 12..=14]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![0..=0, 2..=3]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(0..=4, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![0..=0, 2..=3, 6..=11, 12..=14])
    );
    assert_eq!(updated_displayed_output.height(), 12);
}

#[test]
pub fn test_update_filtered_dirty_lines_no_lines_removed() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 10..=11]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![6..=7, 8..=9]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(5..=9, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 6..=7, 8..=9, 10..=11])
    );
    assert_eq!(updated_displayed_output.height(), 9);
}

#[test]
pub fn test_update_filtered_dirty_lines_within_single_range() {
    let displayed_output = DisplayedOutput::new_for_test(vec![2..=4, 6..=15, 17..=20]);
    let mut grid = mock_blockgrid("");
    grid.grid_handler_mut()
        .set_displayed_output(displayed_output);

    let new_displayed_rows = make_displayed_rows_from_ranges(vec![8..=10, 12..=12]);
    grid.grid_handler_mut()
        .update_filtered_dirty_lines(8..=13, new_displayed_rows, 0);

    let updated_displayed_output = grid.grid_handler().displayed_output.as_ref().unwrap();
    assert_eq!(
        updated_displayed_output.displayed_rows(),
        make_displayed_rows_from_ranges(vec![2..=4, 6..=7, 8..=10, 12..=12, 14..=15, 17..=20])
    );
    assert_eq!(updated_displayed_output.height(), 15);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_all_greater() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=7, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(7..=8, DisplaySource::FilterContext),
        DisplayedRows::new(9..=10, DisplaySource::FilterMatch),
        DisplayedRows::new(11..=12, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        8..=15,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=7, DisplaySource::FilterContext),
        DisplayedRows::new(8..=8, DisplaySource::FilterContext),
        DisplayedRows::new(9..=10, DisplaySource::FilterMatch),
        DisplayedRows::new(11..=12, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_all_lesser() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (10..=12, DisplaySource::FilterContext, 2),
        (13..=15, DisplaySource::FilterMatch, 1),
        (16..=17, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(7..=8, DisplaySource::FilterContext),
        DisplayedRows::new(9..=9, DisplaySource::FilterMatch),
        DisplayedRows::new(10..=12, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        5..=9,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(7..=8, DisplaySource::FilterContext),
        DisplayedRows::new(9..=9, DisplaySource::FilterMatch),
        DisplayedRows::new(10..=12, DisplaySource::FilterContext),
        DisplayedRows::new(13..=15, DisplaySource::FilterMatch),
        DisplayedRows::new(16..=17, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_bound_on_filter_match() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=11, DisplaySource::FilterContext, 3),
        (12..=12, DisplaySource::FilterContext, 1),
        (13..=15, DisplaySource::FilterMatch, 1),
        (16..=18, DisplaySource::FilterContext, 2),
        (19..=21, DisplaySource::FilterContext, 2),
        (22..=24, DisplaySource::FilterMatch, 1),
        (25..=27, DisplaySource::FilterContext, 3),
        (28..=29, DisplaySource::FilterContext, 1),
        (30..=32, DisplaySource::FilterMatch, 1),
        (33..=36, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        13..=24,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=9, DisplaySource::FilterContext),
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
        DisplayedRows::new(27..=29, DisplaySource::FilterContext),
        DisplayedRows::new(30..=32, DisplaySource::FilterMatch),
        DisplayedRows::new(33..=36, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_bound_on_inner_context() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=11, DisplaySource::FilterContext, 3),
        (12..=12, DisplaySource::FilterContext, 1),
        (13..=15, DisplaySource::FilterMatch, 1),
        (16..=18, DisplaySource::FilterContext, 2),
        (19..=21, DisplaySource::FilterContext, 2),
        (22..=24, DisplaySource::FilterMatch, 1),
        (25..=27, DisplaySource::FilterContext, 3),
        (28..=29, DisplaySource::FilterContext, 1),
        (30..=32, DisplaySource::FilterMatch, 1),
        (33..=36, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        12..=26,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=9, DisplaySource::FilterContext),
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
        DisplayedRows::new(27..=29, DisplaySource::FilterContext),
        DisplayedRows::new(30..=32, DisplaySource::FilterMatch),
        DisplayedRows::new(33..=36, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_bound_on_outer_context() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=6, DisplaySource::FilterContext, 1),
        (7..=12, DisplaySource::FilterContext, 3),
        (13..=15, DisplaySource::FilterMatch, 1),
        (16..=18, DisplaySource::FilterContext, 2),
        (19..=21, DisplaySource::FilterContext, 2),
        (22..=24, DisplaySource::FilterMatch, 1),
        (25..=28, DisplaySource::FilterContext, 3),
        (29..=29, DisplaySource::FilterContext, 1),
        (30..=32, DisplaySource::FilterMatch, 1),
        (33..=36, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        6..=29,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=8, DisplaySource::FilterContext),
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
        DisplayedRows::new(27..=29, DisplaySource::FilterContext),
        DisplayedRows::new(30..=32, DisplaySource::FilterMatch),
        DisplayedRows::new(33..=36, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_bound_on_filter_match_no_surroundings() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (12..=12, DisplaySource::FilterContext, 1),
        (13..=15, DisplaySource::FilterMatch, 1),
        (16..=18, DisplaySource::FilterContext, 2),
        (19..=21, DisplaySource::FilterContext, 2),
        (22..=24, DisplaySource::FilterMatch, 1),
        (25..=27, DisplaySource::FilterContext, 3),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        13..=24,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_bound_on_inner_context_no_surroundings()
{
    let num_context_lines = 2;
    let displayed_rows = vec![
        (12..=12, DisplaySource::FilterContext, 1),
        (13..=15, DisplaySource::FilterMatch, 1),
        (16..=18, DisplaySource::FilterContext, 2),
        (19..=21, DisplaySource::FilterContext, 2),
        (22..=24, DisplaySource::FilterMatch, 1),
        (25..=27, DisplaySource::FilterContext, 3),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        12..=26,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_bound_on_merged_context() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=9, DisplaySource::FilterContext, 2),
        (10..=15, DisplaySource::FilterMatch, 1),
        (16..=18, DisplaySource::FilterContext, 2),
        (19..=21, DisplaySource::FilterContext, 2),
        (22..=24, DisplaySource::FilterMatch, 1),
        (25..=28, DisplaySource::FilterContext, 2),
        (29..=32, DisplaySource::FilterMatch, 1),
        (33..=36, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        8..=28,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=9, DisplaySource::FilterContext),
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=16, DisplaySource::FilterMatch),
        DisplayedRows::new(17..=19, DisplaySource::FilterContext),
        DisplayedRows::new(25..=28, DisplaySource::FilterContext),
        DisplayedRows::new(29..=32, DisplaySource::FilterMatch),
        DisplayedRows::new(33..=36, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_bound_on_adjacent_matched_lines() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=6, DisplaySource::FilterMatch, 1),
        (7..=8, DisplaySource::FilterContext, 2),
        (10..=11, DisplaySource::FilterContext, 2),
        (12..=12, DisplaySource::FilterMatch, 1),
        (13..=13, DisplaySource::FilterMatch, 1),
        (14..=16, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(7..=7, DisplaySource::FilterContext),
        DisplayedRows::new(8..=8, DisplaySource::FilterMatch),
        DisplayedRows::new(9..=9, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        6..=12,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=6, DisplaySource::FilterContext),
        DisplayedRows::new(7..=7, DisplaySource::FilterContext),
        DisplayedRows::new(8..=8, DisplaySource::FilterMatch),
        DisplayedRows::new(9..=9, DisplaySource::FilterContext),
        DisplayedRows::new(11..=12, DisplaySource::FilterContext),
        DisplayedRows::new(13..=13, DisplaySource::FilterMatch),
        DisplayedRows::new(14..=16, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_bound_on_same_context() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=6, DisplaySource::FilterMatch, 1),
        (7..=14, DisplaySource::FilterContext, 2),
        (15..=17, DisplaySource::FilterContext, 2),
        (18..=18, DisplaySource::FilterMatch, 1),
        (19..=19, DisplaySource::FilterMatch, 1),
        (20..=22, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);
    // Modify some lines in the dirty range.
    update_line_in_blockgrid(&mut blockgrid, 7, DisplaySource::FilterContext, false);
    update_line_in_blockgrid(&mut blockgrid, 8, DisplaySource::FilterContext, false);
    update_line_in_blockgrid(&mut blockgrid, 15, DisplaySource::FilterContext, false);
    update_line_in_blockgrid(&mut blockgrid, 16, DisplaySource::FilterContext, false);

    let new_displayed_rows = vec![
        DisplayedRows::new(10..=10, DisplaySource::FilterContext),
        DisplayedRows::new(11..=11, DisplaySource::FilterMatch),
        DisplayedRows::new(12..=12, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        7..=14,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=6, DisplaySource::FilterMatch),
        DisplayedRows::new(7..=8, DisplaySource::FilterContext),
        DisplayedRows::new(10..=10, DisplaySource::FilterContext),
        DisplayedRows::new(11..=11, DisplaySource::FilterMatch),
        DisplayedRows::new(12..=12, DisplaySource::FilterContext),
        DisplayedRows::new(16..=17, DisplaySource::FilterContext),
        DisplayedRows::new(18..=18, DisplaySource::FilterMatch),
        DisplayedRows::new(19..=19, DisplaySource::FilterMatch),
        DisplayedRows::new(20..=22, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_outside_of_dirty_range() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=7, DisplaySource::FilterContext, 2),
        (8..=9, DisplaySource::CursorLine, 2), // Represents non-filter lines.
        (10..=11, DisplaySource::FilterContext, 2),
        (12..=13, DisplaySource::FilterMatch, 1),
        (14..=15, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(6..=7, DisplaySource::FilterContext),
        DisplayedRows::new(8..=8, DisplaySource::FilterMatch),
        DisplayedRows::new(9..=10, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        8..=9,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=7, DisplaySource::FilterContext),
        DisplayedRows::new(8..=8, DisplaySource::FilterMatch),
        DisplayedRows::new(9..=10, DisplaySource::FilterContext),
        DisplayedRows::new(11..=11, DisplaySource::FilterContext),
        DisplayedRows::new(12..=13, DisplaySource::FilterMatch),
        DisplayedRows::new(14..=15, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_trimmed_new_displayed_rows() {
    // Tests the case where the context lines in the new displayed rows overlap
    // with the existing displayed rows and need to be trimmed.
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=7, DisplaySource::FilterContext, 2),
        (8..=12, DisplaySource::CursorLine, 5), // Represents non-filter lines.
        (13..=14, DisplaySource::FilterContext, 2),
        (15..=15, DisplaySource::FilterMatch, 1),
        (16..=17, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);
    // 7..=9 is now a logical line.
    update_line_in_blockgrid(&mut blockgrid, 7, DisplaySource::FilterContext, true);
    update_line_in_blockgrid(&mut blockgrid, 8, DisplaySource::FilterContext, true);
    update_line_in_blockgrid(&mut blockgrid, 9, DisplaySource::FilterContext, false);
    // 12..=13 is now a logical line.
    update_line_in_blockgrid(&mut blockgrid, 12, DisplaySource::FilterContext, true);

    let new_displayed_rows = vec![
        DisplayedRows::new(6..=9, DisplaySource::FilterContext),
        DisplayedRows::new(10..=10, DisplaySource::FilterMatch),
        DisplayedRows::new(11..=13, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        7..=13,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=9, DisplaySource::FilterContext),
        DisplayedRows::new(10..=10, DisplaySource::FilterMatch),
        DisplayedRows::new(11..=13, DisplaySource::FilterContext),
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=15, DisplaySource::FilterMatch),
        DisplayedRows::new(16..=17, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_dirty_range_between_two_ranges() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=7, DisplaySource::FilterContext, 2),
        (13..=14, DisplaySource::FilterContext, 2),
        (15..=15, DisplaySource::FilterMatch, 1),
        (16..=17, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(7..=8, DisplaySource::FilterContext),
        DisplayedRows::new(9..=9, DisplaySource::FilterMatch),
        DisplayedRows::new(10..=11, DisplaySource::FilterContext),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        8..=12,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=7, DisplaySource::FilterContext),
        DisplayedRows::new(8..=8, DisplaySource::FilterContext),
        DisplayedRows::new(9..=9, DisplaySource::FilterMatch),
        DisplayedRows::new(10..=11, DisplaySource::FilterContext),
        DisplayedRows::new(13..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=15, DisplaySource::FilterMatch),
        DisplayedRows::new(16..=17, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
pub fn test_update_filtered_dirty_lines_with_context_lines_existing_context_bounded_by_new_lines() {
    let num_context_lines = 2;
    let displayed_rows = vec![
        (0..=2, DisplaySource::FilterContext, 2),
        (3..=5, DisplaySource::FilterMatch, 1),
        (6..=7, DisplaySource::FilterContext, 2),
        (13..=14, DisplaySource::FilterContext, 2),
        (15..=15, DisplaySource::FilterMatch, 1),
        (16..=17, DisplaySource::FilterContext, 2),
    ];
    let mut blockgrid = make_blockgrid_from_displayed_rows(displayed_rows, num_context_lines);

    let new_displayed_rows = vec![
        DisplayedRows::new(7..=8, DisplaySource::CursorLine),
        DisplayedRows::new(10..=13, DisplaySource::CursorLine),
    ];
    blockgrid.grid_handler_mut().update_filtered_dirty_lines(
        7..=13,
        new_displayed_rows,
        num_context_lines,
    );

    let updated_displayed_rows = blockgrid
        .grid_handler()
        .displayed_output
        .as_ref()
        .unwrap()
        .displayed_rows();
    let expected_displayed_rows = vec![
        DisplayedRows::new(0..=2, DisplaySource::FilterContext),
        DisplayedRows::new(3..=5, DisplaySource::FilterMatch),
        DisplayedRows::new(6..=6, DisplaySource::FilterContext),
        DisplayedRows::new(7..=8, DisplaySource::CursorLine),
        DisplayedRows::new(10..=13, DisplaySource::CursorLine),
        DisplayedRows::new(14..=14, DisplaySource::FilterContext),
        DisplayedRows::new(15..=15, DisplaySource::FilterMatch),
        DisplayedRows::new(16..=17, DisplaySource::FilterContext),
    ];
    assert_eq!(updated_displayed_rows, expected_displayed_rows);
}

#[test]
fn test_truncate_cursor_updates_displayed_output() {
    let mut blockgrid = mock_blockgrid("abcde\n\n\n\n");
    let grid_handler = blockgrid.grid_handler_mut();

    // Create a filled grid with no scrollback buffer
    let grid = grid_handler.grid_storage_mut();
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid[3][0].c = 'd';
    grid[4][0].c = 'e';

    // Initialize displayed output with all rows in the grid.
    grid_handler.displayed_output = Some(DisplayedOutput::new_for_test(vec![0..=4]));

    // Set the cursor to the middle of the grid
    grid_handler.set_cursor_point(2, 0);

    // Truncate everything after the cursor
    grid_handler.truncate_to_cursor_rows();

    // Verify that the grid is truncated
    let grid = grid_handler.grid_storage();
    assert_eq!(grid.rows, 3);
    assert_eq!(grid[VisibleRow(0)][0].c, 'a');
    assert_eq!(grid[VisibleRow(1)][0].c, 'b');
    assert_eq!(grid[VisibleRow(2)][0].c, 'c');

    // Verify that the displayed output is truncated.
    assert_eq!(
        grid_handler
            .displayed_output
            .as_ref()
            .unwrap()
            .displayed_rows(),
        &vec![DisplayedRows {
            range: 0..=2,
            source: DisplaySource::FilterMatch
        }]
    );
    assert_eq!(grid_handler.displayed_output.as_ref().unwrap().height(), 3);
}

#[test]
pub fn test_find_with_matched_lines() {
    let blockgrid = mock_blockgrid(
        "\
        non-match\n\
        ing line\r\n\
        matching\n\
        foo\n\
        line\r\n\
        non-match\n\
        ing line\r\n\
        foo foo\n\
        foo bar\r\n\
        match   f\n\
        oo line\r\n\
        foo",
    );

    let expected_matches = vec![
        Point { row: 11, col: 0 }..=Point { row: 11, col: 2 },
        Point { row: 9, col: 8 }..=Point { row: 10, col: 1 },
        Point { row: 8, col: 0 }..=Point { row: 8, col: 2 },
        Point { row: 7, col: 4 }..=Point { row: 7, col: 6 },
        Point { row: 7, col: 0 }..=Point { row: 7, col: 2 },
        Point { row: 3, col: 0 }..=Point { row: 3, col: 2 },
    ];
    let expected_matched_lines = vec![11..=11, 9..=10, 7..=8, 2..=4];

    let dfas = RegexDFAs::new("foo").unwrap();
    let (matches, matched_lines) = blockgrid.grid_handler().find_with_matched_lines(&dfas);

    assert_eq!(matches, expected_matches);
    assert_eq!(matched_lines, expected_matched_lines);
}

#[test]
pub fn test_find_non_matching_lines() {
    // A blockgrid where the start/end of the grid contains non-matching lines.
    let blockgrid = mock_blockgrid(
        "\
        foo\n\
        foo\r\n\
        foo\r\n\
        match\r\n\
        foo\r\n\
        foo\n\
        foo\r\n\
        match match\n\
        match\r\n\
        foo\r\n\
        foo\r\n\
        match\r\n\
        match\n\
        match\r\n\
        match\r\n\
        foo\r\n\
        match match\r\n\
        foo\r\n\
        foo\r\n\
        ",
    );

    let expected_non_matching_lines = vec![17..=18, 15..=15, 9..=10, 4..=6, 0..=2];

    let dfas = RegexDFAs::new("match").unwrap();
    let non_matching_lines = blockgrid.grid_handler().find_non_matching_lines(&dfas);

    assert_eq!(non_matching_lines, expected_non_matching_lines);
}

#[test]
pub fn test_find_non_matching_lines_none_at_bounds() {
    // A blockgrid where the start/end of the grid does not contain non-matching lines.
    let blockgrid = mock_blockgrid(
        "\
        match\n\
        match match\r\n\
        foo\r\n\
        foo\n\
        foo\r\n\
        match match\n\
        match\r\n\
        foo\r\n\
        foo\r\n\
        match\r\n\
        match\n\
        match\r\n\
        match\r\n\
        foo\r\n\
        match\r\n\
        match match\r\n\
        ",
    );

    let expected_non_matching_lines = vec![13..=13, 7..=8, 2..=4];

    let dfas = RegexDFAs::new("match").unwrap();
    let non_matching_lines = blockgrid.grid_handler().find_non_matching_lines(&dfas);

    assert_eq!(non_matching_lines, expected_non_matching_lines);
}

#[test]
pub fn test_find_non_matching_lines_in_range() {
    // A blockgrid where the start/end of the range contains non-matching lines.
    let blockgrid = mock_blockgrid(
        "\
        match\r\n\
        ------\r\n\
        foo\n\
        foo\r\n\
        foo\r\n\
        match\r\n\
        foo\r\n\
        foo\n\
        foo\r\n\
        match match\n\
        match\r\n\
        foo\r\n\
        foo\r\n\
        match\r\n\
        match\n\
        match\r\n\
        match\r\n\
        foo\r\n\
        match match\r\n\
        foo\r\n\
        foo\r\n\
        ------\r\n\
        match\r\n\
        ",
    );

    let expected_non_matching_lines = vec![19..=20, 17..=17, 11..=12, 6..=8, 2..=4];

    let dfas = RegexDFAs::new("match").unwrap();
    let non_matching_lines = blockgrid.grid_handler().find_non_matching_lines_in_range(
        &dfas,
        Point { row: 2, col: 0 },
        Point { row: 20, col: 2 },
    );

    assert_eq!(non_matching_lines, expected_non_matching_lines);
}

#[test]
pub fn test_find_non_matching_lines_in_range_none_at_bounds() {
    // A blockgrid where the start/end of the range does not contain non-matching lines.
    let blockgrid = mock_blockgrid(
        "\
        match\r\n\
        ------\r\n\
        match\n\
        match match\r\n\
        foo\r\n\
        foo\n\
        foo\r\n\
        match match\n\
        match\r\n\
        foo\r\n\
        foo\r\n\
        match\r\n\
        match\n\
        match\r\n\
        match\r\n\
        foo\r\n\
        match\r\n\
        match match\r\n\
        ------\r\n\
        match\r\n\
        ",
    );

    let expected_non_matching_lines = vec![15..=15, 9..=10, 4..=6];

    let dfas = RegexDFAs::new("match").unwrap();
    let non_matching_lines = blockgrid.grid_handler().find_non_matching_lines_in_range(
        &dfas,
        Point { row: 2, col: 0 },
        Point { row: 17, col: 10 },
    );

    assert_eq!(non_matching_lines, expected_non_matching_lines);
}

#[test]
pub fn test_find_non_matching_lines_all_returned() {
    // A blockgrid where all lines are non-matching.
    let blockgrid = mock_blockgrid(
        "\
        foo\n\
        foo\r\n\
        foo foo\r\n\
        foo\r\n\
        foo\n\
        foo\r\n\
        foo foo\n\
        foo\r\n\
        foo foo foo\r\n\
        foo\r\n\
        foo\r\n\
        ",
    );

    let expected_non_matching_lines = vec![0..=10];

    let dfas = RegexDFAs::new("match").unwrap();
    let non_matching_lines = blockgrid.grid_handler().find_non_matching_lines(&dfas);

    assert_eq!(non_matching_lines, expected_non_matching_lines);
}

#[test]
pub fn test_find_non_matching_lines_in_range_all_returned() {
    // A blockgrid where the entire range contains non-matching lines.
    let blockgrid = mock_blockgrid(
        "\
        match\r\n\
        ------\r\n\
        foo\n\
        foo\r\n\
        foo foo\r\n\
        foo\r\n\
        foo\n\
        foo\r\n\
        foo foo\n\
        foo\r\n\
        foo foo foo\r\n\
        foo\r\n\
        foo\r\n\
        ------\r\n\
        match\r\n\
        ",
    );

    let expected_non_matching_lines = vec![2..=12];

    let dfas = RegexDFAs::new("match").unwrap();
    let non_matching_lines = blockgrid.grid_handler().find_non_matching_lines_in_range(
        &dfas,
        Point { row: 2, col: 0 },
        Point { row: 12, col: 10 },
    );

    assert_eq!(non_matching_lines, expected_non_matching_lines);
}

#[test]
pub fn test_find_non_matching_lines_none_returned() {
    // A blockgrid where there are no non-matching lines.
    let blockgrid = mock_blockgrid(
        "\
        match\n\
        match\r\n\
        match match\r\n\
        match\r\n\
        match\n\
        match\r\n\
        match match\n\
        match\r\n\
        match match match\r\n\
        match\r\n\
        match\r\n\
        ",
    );

    let expected_non_matching_lines = vec![];

    let dfas = RegexDFAs::new("match").unwrap();
    let non_matching_lines = blockgrid.grid_handler().find_non_matching_lines(&dfas);

    assert_eq!(non_matching_lines, expected_non_matching_lines);
}

#[test]
pub fn test_find_non_matching_lines_in_range_none_returned() {
    // A blockgrid where the entire range contains no non-matching lines.
    let blockgrid = mock_blockgrid(
        "\
        foo\r\n\
        ------\r\n\
        match\n\
        match\r\n\
        match match\r\n\
        match\r\n\
        match\n\
        match\r\n\
        match match\n\
        match\r\n\
        match match match\r\n\
        match\r\n\
        match\r\n\
        ------\r\n\
        foo\r\n\
        ",
    );

    let expected_non_matching_lines = vec![];

    let dfas = RegexDFAs::new("match").unwrap();
    let non_matching_lines = blockgrid.grid_handler().find_non_matching_lines_in_range(
        &dfas,
        Point { row: 2, col: 0 },
        Point { row: 12, col: 10 },
    );

    assert_eq!(non_matching_lines, expected_non_matching_lines);
}

#[test]
pub fn test_lines_range_left() {
    let blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        manitoba\r\n\
        newfoundland\r\n\
        ",
    );

    let left_range = blockgrid.grid_handler().lines_range_left(5, 3, None);
    assert_eq!(left_range, Some(1..=4));
}

#[test]
pub fn test_lines_range_left_with_bound() {
    let blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        manitoba\r\n\
        newfoundland\r\n\
        ",
    );

    let left_range = blockgrid.grid_handler().lines_range_left(5, 3, Some(2));
    assert_eq!(left_range, Some(3..=4));
}

#[test]
pub fn test_lines_range_left_bounded_by_grid_start() {
    let blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        manitoba\r\n\
        newfoundland\r\n\
        ",
    );

    let left_range = blockgrid.grid_handler().lines_range_left(2, 3, None);
    assert_eq!(left_range, Some(0..=1));
}

#[test]
pub fn test_lines_range_left_no_context_available() {
    let blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        manitoba\r\n\
        newfoundland\r\n\
        ",
    );

    // No rows to the left of row 3 with the bound of 2.
    let left_range = blockgrid.grid_handler().lines_range_left(3, 3, Some(2));
    assert_eq!(left_range, None);

    // No rows to the left of row 0.
    let left_range = blockgrid.grid_handler().lines_range_left(0, 3, None);
    assert_eq!(left_range, None);
}

#[test]
pub fn test_lines_range_right() {
    let blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        manitoba\r\n\
        newfoundland\r\n\
        ",
    );

    let right_range = blockgrid.grid_handler().lines_range_right(0, 3, None);
    assert_eq!(right_range, Some(1..=4));
}

#[test]
pub fn test_lines_range_right_with_bound() {
    let blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        manitoba\r\n\
        newfoundland\r\n\
        ",
    );

    let right_range = blockgrid.grid_handler().lines_range_right(0, 3, Some(2));
    assert_eq!(right_range, Some(1..=1));
}

#[test]
pub fn test_lines_range_right_bounded_by_grid_end() {
    let blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        manitoba\r\n\
        newfoundland\r\n\
        ",
    );

    let right_range = blockgrid.grid_handler().lines_range_right(3, 3, None);
    assert_eq!(right_range, Some(4..=5));
}

#[test]
pub fn test_lines_range_right_no_context_available() {
    let blockgrid = mock_blockgrid(
        "\
        alberta\r\n\
        ontario\r\n\
        quebec\n\
        alberta\r\n\
        manitoba\r\n\
        newfoundland\r\n\
        ",
    );

    // No rows to the right of row 3 with the bound of 4.
    let right_range = blockgrid.grid_handler().lines_range_right(3, 3, Some(4));
    assert_eq!(right_range, None);

    // No rows to the right of row 5.
    let right_range = blockgrid.grid_handler().lines_range_right(5, 3, None);
    assert_eq!(right_range, None);
}
