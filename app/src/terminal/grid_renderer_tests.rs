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

    use super::super::scan_indic_run;
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
}
