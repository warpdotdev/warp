use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warpui::fonts::Cache as FontCache;
use warpui::units::{IntoLines, Lines, Pixels};

use super::{active_or_next_match, CachedBackgroundColor};
use crate::terminal::grid_size_util::calculate_grid_baseline_position;
use crate::terminal::model::index::Point;
use crate::terminal::model::selection::SelectionPoint;
use crate::terminal::{grid_renderer, SizeInfo};

fn rect_from_points(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> RectF {
    RectF::from_points(vec2f(min_x, min_y), vec2f(max_x, max_y))
}

// TODO(CORE-2002): Make test non-Mac specific by switching to using bundled Roboto font.
#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Assumes existence of Arial font, which is only guaranteed on macOS"
)]
fn test_calculate_grid_baseline_position() {
    let font_db = warpui::platform::test::FontDB::new();
    let mut font_cache = FontCache::new(Box::new(font_db));
    // Note we've restricted this unit test to Mac, so we expect Arial to exist.
    let arial = font_cache
        .load_system_font("Arial")
        .expect("Arial must exist");
    let baseline_position = calculate_grid_baseline_position(
        &font_cache,
        arial,
        16., /* font_size */
        1.2, /* line_height_ratio */
        19., /* cell_size_y */
    );
    assert_eq!(baseline_position, vec2f(0., 15.));
}

#[test]
fn test_next_match_same_row_matches() {
    let match_1 = Point::new(0, 0)..=Point::new(0, 4);
    let match_2 = Point::new(1, 0)..=Point::new(1, 4);
    let matches = [match_1.clone(), match_2.clone()];
    let mut filter_match_iter = matches.iter();

    let mut current_match = None;

    // The first match should return for points (0,0) through (0,4).
    for i in 0..=4 {
        current_match =
            active_or_next_match(&mut filter_match_iter, current_match, &Point::new(0, i));
        assert_eq!(current_match, Some(&match_1));
    }

    // The second match should return for points (1,0) through (1,4).
    for i in 0..=4 {
        current_match =
            active_or_next_match(&mut filter_match_iter, current_match, &Point::new(1, i));
        assert_eq!(current_match, Some(&match_2));
    }

    // There should be no more matches left after we advance to point (2,0).
    current_match = active_or_next_match(&mut filter_match_iter, current_match, &Point::new(2, 0));
    assert_eq!(current_match, None);
}

#[test]
fn test_next_match_multi_row_matches() {
    let match_1 = Point::new(0, 0)..=Point::new(1, 2);
    let match_2 = Point::new(2, 0)..=Point::new(3, 2);
    let matches = [match_1.clone(), match_2.clone()];
    let mut match_iter = matches.iter();

    let mut current_match = None;

    // The first match should be returned for all points from (0,0) to (1,2).
    let points_1 = [
        Point::new(0, 0),
        Point::new(0, 1),
        Point::new(0, 2),
        Point::new(1, 0),
        Point::new(1, 1),
        Point::new(1, 2),
    ];
    for point in points_1.iter() {
        current_match = active_or_next_match(&mut match_iter, current_match, point);
        assert_eq!(current_match, Some(&match_1));
    }

    // The second match should be returned for all points from (2,0) to (3,2).
    let points_2 = [
        Point::new(2, 0),
        Point::new(2, 1),
        Point::new(2, 2),
        Point::new(3, 0),
        Point::new(3, 1),
        Point::new(3, 2),
    ];
    for point in points_2.iter() {
        current_match = active_or_next_match(&mut match_iter, current_match, point);
        assert_eq!(current_match, Some(&match_2));
    }

    // There should be no more matches left after we advance to point (4,0).
    current_match = active_or_next_match(&mut match_iter, current_match, &Point::new(4, 0));
    assert_eq!(current_match, None);
}

#[test]
fn test_active_or_next_match_point_before_next_match() {
    let match_1 = Point::new(1, 0)..=Point::new(1, 4);
    let match_2 = Point::new(3, 0)..=Point::new(3, 4);
    let matches = [match_1.clone(), match_2.clone()];
    let mut match_iter = matches.iter();

    // The match for (0,0) should be the first match.
    let mut current_match = active_or_next_match(&mut match_iter, None, &Point::new(0, 0));
    assert_eq!(current_match, Some(&match_1));

    // The match for (2,0) should be the second match.
    current_match = active_or_next_match(&mut match_iter, current_match, &Point::new(2, 0));
    assert_eq!(current_match, Some(&match_2));
}

#[test]
fn test_calculate_background_bounds() {
    let origin = vec2f(100., 100.);
    let cell_size = vec2f(2., 4.);
    let max_columns = 150;
    let create_cached = |start_row: usize, start_col: usize, end_row: usize, end_col: usize| {
        CachedBackgroundColor {
            start: SelectionPoint {
                row: start_row.into_lines(),
                col: start_col,
            },
            end: SelectionPoint {
                row: end_row.into_lines(),
                col: end_col,
            },
            background_color: Default::default(),
        }
    };

    // Background with 1 row
    let (start_row, start_col, end_row, end_col) = (10, 20, 10, 130);
    let cached = create_cached(start_row, start_col, end_row, end_col);
    assert_eq!(
        grid_renderer::calculate_background_bounds(origin, cached, cell_size, max_columns),
        vec![rect_from_points(
            origin.x() + (start_col as f32) * cell_size.x(),
            origin.y() + (start_row as f32) * cell_size.y(),
            origin.x() + (end_col as f32 + 1.) * cell_size.x(),
            origin.y() + (end_row as f32 + 1.) * cell_size.y()
        )]
    );

    // Background with 2 rows
    let (start_row, start_col, end_row, end_col) = (20, 30, 21, 100);
    let cached = create_cached(start_row, start_col, end_row, end_col);
    assert_eq!(
        grid_renderer::calculate_background_bounds(origin, cached, cell_size, max_columns),
        vec![
            rect_from_points(
                origin.x() + (start_col as f32) * cell_size.x(),
                origin.y() + (start_row as f32) * cell_size.y(),
                origin.x() + (max_columns as f32 + 1.) * cell_size.x(),
                origin.y() + (start_row as f32 + 1.) * cell_size.y()
            ),
            rect_from_points(
                origin.x(),
                origin.y() + (start_row as f32 + 1.) * cell_size.y(),
                origin.x() + (end_col as f32 + 1.) * cell_size.x(),
                origin.y() + (end_row as f32 + 1.) * cell_size.y()
            ),
        ]
    );

    // Background with 3+ rows
    let assert_multi_row_selection_bounds =
        |start_row: usize, start_col: usize, end_row: usize, end_col: usize| {
            let cached = create_cached(start_row, start_col, end_row, end_col);
            assert_eq!(
                grid_renderer::calculate_background_bounds(origin, cached, cell_size, max_columns),
                vec![
                    rect_from_points(
                        origin.x() + (start_col as f32) * cell_size.x(),
                        origin.y() + (start_row as f32) * cell_size.y(),
                        origin.x() + (max_columns as f32 + 1.) * cell_size.x(),
                        origin.y() + (start_row as f32 + 1.) * cell_size.y()
                    ),
                    rect_from_points(
                        origin.x(),
                        origin.y() + (start_row as f32 + 1.) * cell_size.y(),
                        origin.x() + (max_columns as f32 + 1.) * cell_size.x(),
                        origin.y() + (end_row as f32) * cell_size.y()
                    ),
                    rect_from_points(
                        origin.x(),
                        origin.y() + (end_row as f32) * cell_size.y(),
                        origin.x() + (end_col as f32 + 1.) * cell_size.x(),
                        origin.y() + (end_row as f32 + 1.) * cell_size.y()
                    ),
                ]
            );
        };
    assert_multi_row_selection_bounds(30, 80, 32, 40); // 3 lines
    assert_multi_row_selection_bounds(40, 60, 43, 10); // 4 lines
    assert_multi_row_selection_bounds(50, 140, 59, 20); // 10 lines
}

#[test]
fn test_calculate_selection_bounds() {
    let origin = vec2f(100., 100.);
    let size_info = SizeInfo::new(
        Vector2F::zero(),
        Pixels::new(2.),
        Pixels::new(4.),
        Pixels::new(8.),
        Pixels::new(16.),
    )
    .with_rows_and_columns(151, 151);

    let cell_width = size_info.cell_width_px.as_f32();
    let cell_height = size_info.cell_height_px.as_f32();
    let horizontal_padding = size_info.padding_x_px.as_f32();
    let max_columns = size_info.columns - 1;

    let make_selection_point = |row: usize, col: usize| SelectionPoint {
        row: row.into_lines(),
        col,
    };

    let start = make_selection_point(10, 10);
    let end = make_selection_point(20, 50);

    let assert_selection_bounds = |scroll_top: Lines| {
        assert_eq!(
            grid_renderer::calculate_selection_bounds(&start, &end, &size_info, scroll_top, origin),
            vec![
                rect_from_points(
                    origin.x() + horizontal_padding + (start.col as f32) * cell_width,
                    origin.y() + ((start.row - scroll_top).as_f64() as f32) * cell_height,
                    origin.x() + horizontal_padding + (max_columns as f32 + 1.) * cell_width,
                    origin.y() + ((start.row - scroll_top).as_f64() as f32 + 1.) * cell_height
                ),
                rect_from_points(
                    origin.x() + horizontal_padding,
                    origin.y() + ((start.row - scroll_top).as_f64() as f32 + 1.) * cell_height,
                    origin.x() + horizontal_padding + (max_columns as f32 + 1.) * cell_width,
                    origin.y() + ((end.row - scroll_top).as_f64() as f32) * cell_height
                ),
                rect_from_points(
                    origin.x() + horizontal_padding,
                    origin.y() + ((end.row - scroll_top).as_f64() as f32) * cell_height,
                    origin.x() + horizontal_padding + (end.col as f32 + 1.) * cell_width,
                    origin.y() + ((end.row - scroll_top).as_f64() as f32 + 1.) * cell_height
                ),
            ]
        );
    };
    assert_selection_bounds(5.into_lines()); // Without scroll clipping
    assert_selection_bounds(10.into_lines()); // Without scroll clipping (but on the cusp of clipping)
    assert_selection_bounds(80.into_lines()); // With scroll clipping
}

mod indic_run_tests {
    use std::sync::Arc;

    use super::super::{entry_word_ids, scan_indic_run};
    use crate::terminal::event_listener::ChannelEventListener;
    use crate::terminal::model::ansi::{self, Handler as _};
    use crate::terminal::model::cell::Flags;
    use crate::terminal::model::grid::grid_handler::{GridHandler, PerformResetGridChecks};
    use crate::terminal::model::grid::row::Row;
    use crate::terminal::model::grid::NoopMeasurer;
    use crate::terminal::model::secrets::{self, ObfuscateSecrets};
    use crate::terminal::SizeInfo;

    fn test_grid(obfuscate: ObfuscateSecrets) -> GridHandler {
        GridHandler::new(
            SizeInfo::new_without_font_metrics(5, 20),
            0,
            ChannelEventListener::new_for_test(),
            false,
            obfuscate,
            PerformResetGridChecks::No,
            Arc::new(NoopMeasurer),
        )
    }

    #[test]
    fn single_cluster_run_matches_cluster_span() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        row[0].c = 'ప';
        row[0].push_zerowidth('్', false);
        row[0].push_zerowidth('ర', false);
        row[0].set_span(3);
        row[1].flags_mut().insert(Flags::WIDE_CHAR_SPACER);
        row[2].flags_mut().insert(Flags::WIDE_CHAR_SPACER);
        row[3].c = 'a';

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "ప్ర");
        assert_eq!(shape.total_span, 3);
        assert_eq!(shape.char_ranges, vec![0..shape.full_text.chars().count()]);
    }

    #[test]
    fn multi_cluster_run_merges_whole_word() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        // "తె" (2 cols) + "లు" (2 cols) + "గు" (2 cols) -- one word, no spaces.
        row[0].c = 'త';
        row[0].push_zerowidth('ె', false);
        row[0].set_span(2);
        row[1].flags_mut().insert(Flags::WIDE_CHAR_SPACER);

        row[2].c = 'ల';
        row[2].push_zerowidth('ు', false);
        row[2].set_span(2);
        row[3].flags_mut().insert(Flags::WIDE_CHAR_SPACER);

        row[4].c = 'గ';
        row[4].push_zerowidth('ు', false);
        row[4].set_span(2);
        row[5].flags_mut().insert(Flags::WIDE_CHAR_SPACER);

        row[6].c = ' ';
        // ASCII after the space, not punctuation-then-space, so the space
        // is NOT absorbed (Phase 11 only absorbs a trailing space when
        // nothing follows, or when it's followed by more Indic content) --
        // this keeps the test's original intent (run stops at the word
        // boundary) valid alongside the new connector-absorption behaviour.
        row[7].c = 'X';

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "తెలుగు");
        assert_eq!(shape.total_span, 6);
        assert_eq!(shape.char_ranges.len(), 3);
        assert_eq!(shape.char_ranges[0].start, 0);
        assert_eq!(shape.char_ranges[2].end, shape.full_text.chars().count());
        for pair in shape.char_ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "ranges must partition full_text with no gaps");
        }
    }

    #[test]
    fn run_stops_at_non_indic_cell() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        row[0].c = 'త';
        row[0].push_zerowidth('ె', false);
        row[0].set_span(2);
        row[1].flags_mut().insert(Flags::WIDE_CHAR_SPACER);
        row[2].c = 'a'; // Immediately adjacent, no space.

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "తె");
        assert_eq!(shape.total_span, 2);
    }

    #[test]
    fn run_extends_to_end_of_row() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(4);
        row[0].c = 'త';
        row[0].push_zerowidth('ె', false);
        row[0].set_span(2);
        row[1].flags_mut().insert(Flags::WIDE_CHAR_SPACER);
        row[2].c = 'ల';
        row[2].push_zerowidth('ు', false);
        row[2].set_span(2);
        row[3].flags_mut().insert(Flags::WIDE_CHAR_SPACER);

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "తెలు");
        assert_eq!(shape.total_span, 4);
    }

    #[test]
    fn secret_redacted_cluster_is_excluded_from_run() {
        secrets::set_user_and_enterprise_secret_regexes(
            [&regex::Regex::new("SECRET").expect("valid regex")],
            std::iter::empty(),
        );
        let mut grid = test_grid(ObfuscateSecrets::Yes);
        for c in "SECRET".chars() {
            grid.input(c);
        }
        grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

        // Built as a SEPARATE row -- `scan_indic_run` only consults `grid`
        // for secret ranges by point, independent of the row argument's own
        // content, so this validates the exclusion mechanism directly.
        let mut row = Row::new(10);
        row[0].c = 'త';
        row[0].push_zerowidth('ె', false);
        row[0].set_span(2);
        row[1].flags_mut().insert(Flags::WIDE_CHAR_SPACER);

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::Yes);
        assert_eq!(
            shape.total_span, 0,
            "secret-redacted cluster must not be merged into a run"
        );
    }

    /// Builds a 2-column Telugu cluster ("తె") at `row[start_col]`, returning
    /// the column immediately after it.
    fn write_two_col_cluster(row: &mut Row, start_col: usize) -> usize {
        row[start_col].c = 'త';
        row[start_col].push_zerowidth('ె', false);
        row[start_col].set_span(2);
        row[start_col + 1].flags_mut().insert(Flags::WIDE_CHAR_SPACER);
        start_col + 2
    }

    #[test]
    fn space_joins_two_words() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ' ';
        write_two_col_cluster(&mut row, col + 1);

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "తె తె");
        assert_eq!(shape.total_span, 5, "both clusters plus the joining space");
        assert_eq!(shape.char_ranges.len(), 3, "cluster, space, cluster");
    }

    #[test]
    fn multiple_consecutive_spaces_join() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ' ';
        row[col + 1].c = ' ';
        write_two_col_cluster(&mut row, col + 2);

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "తె  తె");
        assert_eq!(shape.total_span, 6, "both clusters plus both joining spaces");
        assert_eq!(shape.char_ranges.len(), 4, "cluster, space, space, cluster");
    }

    #[test]
    fn trailing_punct_before_eol_absorbed() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(4);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ' ';
        row[col + 1].c = '.';

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "తె .");
        assert_eq!(shape.total_span, 4, "cluster plus trailing space and period at EOL");
    }

    #[test]
    fn space_at_eol_absorbed() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(3);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ' ';

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "తె ");
        assert_eq!(shape.total_span, 3, "cluster plus trailing space at EOL");
    }

    #[test]
    fn space_then_ascii_backs_off() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ' ';
        row[col + 1].c = 'a';

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "తె", "space before ASCII must not be absorbed");
        assert_eq!(shape.total_span, 2, "run ends at the word boundary, matching pre-Phase-11 behaviour");
    }

    #[test]
    fn punct_glued_to_ascii_not_absorbed() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ':';
        row[col + 1].c = 'a';

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(
            shape.full_text, "తె",
            "colon directly glued to ASCII must not be absorbed -- no gap to fabricate"
        );
        assert_eq!(shape.total_span, 2);
    }

    #[test]
    fn punct_then_space_then_ascii_commits_through_punct() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = '.';
        row[col + 1].c = ' ';
        row[col + 2].c = 'a';

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(
            shape.full_text, "తె.",
            "period followed by a space commits (gap lands in existing blank space before ASCII)"
        );
        assert_eq!(shape.total_span, 3);
    }

    #[test]
    fn secret_space_stops_absorption() {
        secrets::set_user_and_enterprise_secret_regexes(
            [&regex::Regex::new("SECRET").expect("valid regex")],
            std::iter::empty(),
        );
        let mut grid = test_grid(ObfuscateSecrets::Yes);
        // "SECRET" matches starting at column 2, so its range (cols 2-7)
        // covers column 2 -- the exact column our scanned row's joining
        // space will sit at (two-col cluster at 0-1, space at 2).
        for c in "XXSECRET".chars() {
            grid.input(c);
        }
        grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

        // Separate row (as in `secret_redacted_cluster_is_excluded_from_run`):
        // a Telugu cluster, then a space that the grid's own secret range
        // covers, then another cluster -- the space must not be absorbed,
        // since a merged run's draw bypasses per-cell secret redaction.
        let mut row = Row::new(10);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ' ';
        write_two_col_cluster(&mut row, col + 1);

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::Yes);
        assert_eq!(
            shape.total_span, 2,
            "run stops at the first cluster; the secret-covered space is never absorbed"
        );
    }

    #[test]
    fn empty_cells_not_treated_as_spaces() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        write_two_col_cluster(&mut row, 0);
        // Cols 2.. stay `Cell::default()` (never written) -- these must be
        // treated as a Blank terminator via `is_empty()`, not scanned as
        // displayable spaces (which would desync the render loop's
        // `next_cluster_idx` accounting against columns it actually skips).

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(shape.full_text, "తె");
        assert_eq!(shape.total_span, 2);
    }

    #[test]
    fn entry_word_ids_single_word_run_has_one_word() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        write_two_col_cluster(&mut row, 0);

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        assert_eq!(entry_word_ids(&shape), vec![0]);
    }

    #[test]
    fn entry_word_ids_space_joined_words_get_separate_ids() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ' ';
        write_two_col_cluster(&mut row, col + 1);

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        // cluster, space, cluster -- the space rides with word 0; the
        // second cluster starts word 1.
        assert_eq!(entry_word_ids(&shape), vec![0, 0, 1]);
    }

    #[test]
    fn entry_word_ids_multiple_spaces_stay_with_preceding_word() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ' ';
        row[col + 1].c = ' ';
        write_two_col_cluster(&mut row, col + 2);

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        // cluster, space, space, cluster -- both spaces ride with word 0.
        assert_eq!(entry_word_ids(&shape), vec![0, 0, 0, 1]);
    }

    #[test]
    fn entry_word_ids_trailing_punct_rides_with_preceding_word() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(4);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = ' ';
        row[col + 1].c = '.';

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        // cluster, space, period -- all one word (nothing follows the
        // period to start a new one).
        assert_eq!(entry_word_ids(&shape), vec![0, 0, 0]);
    }

    #[test]
    fn entry_word_ids_punct_then_space_then_more_indic_starts_new_word() {
        let grid = test_grid(ObfuscateSecrets::No);
        let mut row = Row::new(10);
        let col = write_two_col_cluster(&mut row, 0);
        row[col].c = '.';
        row[col + 1].c = ' ';
        write_two_col_cluster(&mut row, col + 2);

        let shape = scan_indic_run(&row, 0, row.len(), &grid, 0, ObfuscateSecrets::No);
        // cluster, period, space, cluster -- period rides with word 0
        // (immediately follows the cluster, no space between them), the
        // space rides with word 0 too, and the second cluster starts word 1.
        assert_eq!(entry_word_ids(&shape), vec![0, 0, 0, 1]);
    }
}

/// Phase 15 (Telugu variable-width-cells plan): diagnostic to attribute the
/// residual inter-word gap Mahārāja reported (visibly wider than a plain
/// ASCII space at the same font/size). Two candidate causes were identified
/// by reading the code: (1) connectors (space/punctuation) never go through
/// `quantize_indic_span` -- each always reserves exactly 1 full monospace
/// cell, stacking when several occur together (e.g. "word, word"); (2) an
/// unproven-but-plausible discrepancy between the ISOLATED per-cluster
/// natural-width measurement `flush_pending_indic_cluster` trusts at INPUT
/// time and the IN-CONTEXT continuous shaping `render_indic_run` actually
/// draws at RENDER time -- nothing today checks these agree.
///
/// This module measures both, using the real Menlo `CoreTextClusterMeasurer`
/// and the real `layout_line` shaping path (not a synthetic stand-in), so
/// the numbers are faithful to what actually renders.
#[cfg(target_os = "macos")]
mod indic_gap_diagnostics {
    use std::sync::Arc;

    use warpui::fonts::Properties;
    use warpui::platform::mac::FontDB as MacFontDB;
    use warpui::platform::{FontDB as _, LineStyle, TextLayoutSystem as _};
    use warpui::text_layout::{ClipConfig, StyleAndFont, DEFAULT_TOP_BOTTOM_RATIO};

    use super::super::{entry_word_ids, is_connector_punct, scan_indic_run};
    use crate::terminal::event_listener::ChannelEventListener;
    use crate::terminal::model::grid::cluster_measurer::{ClusterWidthMeasurer, CoreTextClusterMeasurer};
    use crate::terminal::model::grid::grid_handler::{GridHandler, PerformResetGridChecks};
    use crate::terminal::model::secrets::ObfuscateSecrets;
    use crate::terminal::SizeInfo;
    use warp_terminal::model::grid::Dimensions;

    const FONT_SIZE: f32 = 13.0;

    fn feed(grid: &mut GridHandler, text: &str) {
        let mut processor = crate::terminal::model::ansi::Processor::new();
        processor.parse_bytes(grid, text.as_bytes(), &mut std::io::sink());
    }

    /// Shapes `text` as ONE continuous line, exactly the way `render_indic_run`
    /// does -- same `layout_line` path `CoreTextClusterMeasurer` itself uses
    /// (`cluster_measurer.rs`), just over the whole run's text instead of one
    /// cluster, and via a directly-owned `MacFontDB` instead of the trait
    /// object, so this test has no dependency on a `PaintContext`/`FontCache`.
    fn shape_continuous(
        db: &MacFontDB,
        family: warpui::fonts::FamilyId,
        text: &str,
    ) -> warpui::text_layout::Line {
        let run_length_chars = text.chars().count();
        db.layout_line(
            text,
            LineStyle {
                font_size: FONT_SIZE,
                line_height_ratio: 1.0,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                fixed_width_tab_size: None,
            },
            &[(
                0..run_length_chars,
                StyleAndFont {
                    font_family: family,
                    properties: Properties::default(),
                    style: Default::default(),
                },
            )],
            f32::MAX,
            ClipConfig::default(),
        )
    }

    #[test]
    fn indic_gap_diag_isolated_vs_in_context_and_boundary_gaps() {
        let mut db = MacFontDB::default();
        let family = db.load_from_system("Menlo").expect("Menlo should be resolvable");
        // Real Menlo advance at 13pt, used as the cell width so the numbers
        // below are realistic (not the dummy 1px `new_without_font_metrics`
        // would give, which would make every cluster hit the 8-cell clamp
        // regardless of any real allocation behavior).
        let cell_width = shape_continuous(&db, family, "MMMMMMMMMM").width / 10.0;
        eprintln!(
            "cell_width = {cell_width:.3}px (this is the ASCII single-space baseline used for comparison)"
        );

        let measurer = CoreTextClusterMeasurer::new("Menlo", Properties::default(), FONT_SIZE)
            .expect("Menlo should be resolvable");

        for text in [
            "వార్త వెలుగు",
            "వార్త, వెలుగు",
            "ప్రభుత్వం చిత్రాలకు",
            "శంకరాభరణం, సాగరం",
        ] {
            // ---- INPUT-TIME path, verbatim: feed through a real GridHandler
            // so clustering + cumulative ceil() allocation are exactly what
            // production does.
            let size_info = SizeInfo::new(
                pathfinder_geometry::vector::vec2f(80.0 * cell_width, 3.0 * 18.0),
                warpui::units::Pixels::new(cell_width),
                warpui::units::Pixels::new(18.0),
                warpui::units::Pixels::zero(),
                warpui::units::Pixels::zero(),
            );
            let inner_measurer = CoreTextClusterMeasurer::new("Menlo", Properties::default(), FONT_SIZE)
                .expect("Menlo should be resolvable");
            let mut grid = GridHandler::new(
                size_info,
                0,
                ChannelEventListener::new_for_test(),
                false,
                ObfuscateSecrets::No,
                PerformResetGridChecks::No,
                Arc::new(inner_measurer),
            );
            feed(&mut grid, text);

            // ---- RENDER-TIME path, verbatim: scan the row into an
            // IndicRunShape (what the renderer does every frame), assign
            // word ids, shape the whole run's text as ONE continuous line.
            let row = grid.row(0).expect("row 0 should exist");
            let shape = scan_indic_run(&row, 0, grid.columns(), &grid, 0, ObfuscateSecrets::No);
            let word_ids = entry_word_ids(&shape);
            let num_words = word_ids.last().map_or(0, |w| w + 1);
            let line = shape_continuous(&db, family, &shape.full_text);

            let chars: Vec<char> = shape.full_text.chars().collect();
            let mut char_word = vec![usize::MAX; chars.len()];
            let mut char_is_conn = vec![false; chars.len()];
            for (i, range) in shape.char_ranges.iter().enumerate() {
                let is_conn = range.len() == 1
                    && matches!(chars.get(range.start), Some(&c) if c == ' ' || is_connector_punct(c));
                for ci in range.clone() {
                    char_word[ci] = word_ids[i];
                    char_is_conn[ci] = is_conn;
                }
            }

            // Per-word in-context extents from the ONE shaped line, over
            // CLUSTER glyphs only (connectors excluded so a trailing
            // absorbed comma/space doesn't drag a word's "ink end" right).
            let mut start = vec![f32::INFINITY; num_words];
            let mut adv_end = vec![f32::NEG_INFINITY; num_words];
            let mut conn_advance = vec![0.0f32; num_words];
            for run in &line.runs {
                for g in &run.glyphs {
                    let (Some(&w), Some(&is_conn)) = (char_word.get(g.index), char_is_conn.get(g.index))
                    else {
                        continue;
                    };
                    if w == usize::MAX {
                        continue;
                    }
                    if is_conn {
                        conn_advance[w] += g.width;
                        continue;
                    }
                    start[w] = start[w].min(g.position_along_baseline.x());
                    adv_end[w] = adv_end[w].max(g.position_along_baseline.x() + g.width);
                }
            }

            // Per-word grid columns and per-word input-time isolated-cluster
            // sum, read back from the actual grid cells (each base cell's
            // content is exactly one flushed cluster, so this reconstructs
            // precisely what `flush_pending_indic_cluster` measured).
            let mut word_start_col = vec![0usize; num_words];
            let mut seen = vec![false; num_words];
            let mut isolated_sum = vec![0.0f32; num_words];
            let mut alloc_cells = vec![0usize; num_words];
            for (i, range) in shape.char_ranges.iter().enumerate() {
                let w = word_ids[i];
                if !seen[w] {
                    seen[w] = true;
                    word_start_col[w] = shape.cols[i];
                }
                let is_conn = range.len() == 1 && char_is_conn[range.start];
                if !is_conn {
                    let entry_text: String = chars[range.clone()].iter().collect();
                    isolated_sum[w] += measurer.natural_width_px(&entry_text, cell_width);
                    alloc_cells[w] += row[shape.cols[i]].span().max(1) as usize;
                }
            }

            eprintln!("== {text:?}  full_text={:?}", shape.full_text);
            for w in 0..num_words {
                let in_ctx = adv_end[w] - start[w];
                eprintln!(
                    "  word {w}: isolated_sum={:.2}px  in_context={:.2}px  delta={:+.2}px ({:+.2} cells)  \
                     allocated={} cells ({:.2}px)  render_slack={:.2}px",
                    isolated_sum[w],
                    in_ctx,
                    isolated_sum[w] - in_ctx,
                    (isolated_sum[w] - in_ctx) / cell_width,
                    alloc_cells[w],
                    alloc_cells[w] as f32 * cell_width,
                    alloc_cells[w] as f32 * cell_width - in_ctx,
                );
                // The one thing that must NEVER be true: in-context shaping
                // wider than what the allocator trusted would mean the
                // allocation under-covers the real rendered width (risk of
                // visual overlap/merge, the exact failure mode the round()
                // experiment caused earlier in this rewrite).
                assert!(
                    in_ctx <= isolated_sum[w] + 0.5,
                    "in-context shaping ({in_ctx:.2}px) WIDER than isolated sum ({:.2}px) for word {w} \
                     of {text:?} -- allocation would under-cover this word's real rendered width",
                    isolated_sum[w]
                );
            }

            // Rendered gap at each word boundary, reproducing
            // `render_indic_run`'s real shift formula by hand from the same
            // data (`run_start_col` is 0 here, single run starting at col 0).
            for w in 1..num_words {
                let shift_prev = ((word_start_col[w - 1] as f32 * cell_width) - start[w - 1]).max(0.0);
                let prev_ink_end = adv_end[w - 1] + shift_prev;
                let gap = word_start_col[w] as f32 * cell_width - prev_ink_end;
                let n_connector_cells = word_start_col[w] - (word_start_col[w - 1] + alloc_cells[w - 1]);
                eprintln!(
                    "  boundary {}->{}: rendered_gap={:.2}px = {:.2} cells  (connector cells N={n_connector_cells}, \
                     connector in-context advance={:.2}px vs N*cell_width={:.2}px, excess over N*cw={:+.2}px)",
                    w - 1,
                    w,
                    gap,
                    gap / cell_width,
                    conn_advance[w - 1],
                    n_connector_cells as f32 * cell_width,
                    gap - n_connector_cells as f32 * cell_width,
                );
                assert!(
                    gap >= -0.5,
                    "negative gap ({gap:.2}px) at boundary {}->{} of {text:?} -- words would overlap",
                    w - 1,
                    w
                );
            }
        }
    }
}
