use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warpui::SingletonEntity as _;
use warpui::fonts::Cache as FontCache;
use warpui::units::{IntoLines, Lines, Pixels};

use super::{CachedBackgroundColor, active_or_next_match};
use crate::terminal::grid_size_util::calculate_grid_baseline_position;
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::index::Point;
use crate::terminal::model::selection::SelectionPoint;
use crate::terminal::{SizeInfo, grid_renderer};

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

const HIDDEN_CURSOR_HINT: &str = "Press up to edit queued messages";
const HIDDEN_CURSOR_CELL_WIDTH: f32 = 8.;
const HIDDEN_CURSOR_CELL_HEIGHT: f32 = 16.;

struct HiddenCursorCellView {
    grid: crate::terminal::model::blockgrid::BlockGrid,
    hide_cursor_cell: bool,
    use_ligature_rendering: bool,
}

impl warpui::Entity for HiddenCursorCellView {
    type Event = ();
}

impl warpui::TypedActionView for HiddenCursorCellView {
    type Action = ();
}

impl warpui::View for HiddenCursorCellView {
    fn ui_name() -> &'static str {
        "HiddenCursorCellView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        Box::new(HiddenCursorCellElement {
            grid: self.grid.clone(),
            hide_cursor_cell: self.hide_cursor_cell,
            use_ligature_rendering: self.use_ligature_rendering,
            size: None,
            origin: None,
        })
    }
}

struct HiddenCursorCellElement {
    grid: crate::terminal::model::blockgrid::BlockGrid,
    hide_cursor_cell: bool,
    use_ligature_rendering: bool,
    size: Option<Vector2F>,
    origin: Option<warpui::elements::Point>,
}

impl warpui::Element for HiddenCursorCellElement {
    fn layout(
        &mut self,
        constraint: warpui::SizeConstraint,
        _ctx: &mut warpui::LayoutContext,
        _app: &warpui::AppContext,
    ) -> Vector2F {
        let size = vec2f(
            HIDDEN_CURSOR_CELL_WIDTH * self.grid.grid_handler().columns() as f32,
            HIDDEN_CURSOR_CELL_HEIGHT,
        )
        .min(constraint.max);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut warpui::AfterLayoutContext, _app: &warpui::AppContext) {}

    fn paint(
        &mut self,
        origin: Vector2F,
        ctx: &mut warpui::PaintContext,
        app: &warpui::AppContext,
    ) {
        self.origin = Some(warpui::elements::Point::from_vec2f(
            origin,
            ctx.scene.z_index(),
        ));
        let appearance = crate::appearance::Appearance::as_ref(app);
        let cell_size = vec2f(HIDDEN_CURSOR_CELL_WIDTH, HIDDEN_CURSOR_CELL_HEIGHT);
        let colors = crate::terminal::color::List::from(&crate::terminal::color::Colors::from(
            appearance.theme().clone(),
        ));
        let mut glyphs = crate::terminal::grid_renderer::CellGlyphCache::default();
        crate::terminal::grid_renderer::render_grid(
            self.grid.grid_handler(),
            0,
            1,
            &colors,
            &crate::terminal::color::OverrideList::empty(),
            appearance.theme(),
            warpui::fonts::Properties::default(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
            appearance.ui_builder().line_height_ratio(),
            cell_size,
            Pixels::zero(),
            origin,
            &mut glyphs,
            255,
            None,
            None,
            None::<std::iter::Empty<&std::ops::RangeInclusive<crate::terminal::model::index::Point>>>,
            None,
            crate::settings::EnforceMinimumContrast::Never,
            crate::terminal::model::ObfuscateSecrets::No,
            None,
            self.use_ligature_rendering,
            None,
            crate::terminal::model::grid::RespectDisplayedOutput::Yes,
            &std::collections::HashMap::new(),
            None,
            self.hide_cursor_cell,
            ctx,
            app,
        );
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<warpui::elements::Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        _event: &warpui::event::DispatchedEvent,
        _ctx: &mut warpui::EventContext,
        _app: &warpui::AppContext,
    ) -> bool {
        false
    }
}

fn hint_grid_with_cursor_on_leading_glyph() -> crate::terminal::model::blockgrid::BlockGrid {
    let mut grid = crate::test_util::mock_blockgrid(HIDDEN_CURSOR_HINT);
    grid.grid_handler_mut().update_cursor(|cursor| {
        cursor.point.row = crate::terminal::model::index::VisibleRow(0);
        cursor.point.col = 0;
    });
    grid
}

fn scene_glyph_count(hide_cursor_cell: bool, use_ligature_rendering: bool) -> usize {
    warpui::App::test((), |mut app| async move {
        app.add_singleton_model(|_| crate::appearance::Appearance::mock());
        let grid = hint_grid_with_cursor_on_leading_glyph();
        assert_eq!(
            grid.grid_handler().cursor_render_point(),
            crate::terminal::model::index::Point::new(0, 0)
        );
        assert_eq!(grid.grid_handler().row(0).unwrap()[0].c, 'P');

        let (window_id, _) = app.add_window(warpui::platform::WindowStyle::NotStealFocus, |_| {
            HiddenCursorCellView {
                grid,
                hide_cursor_cell,
                use_ligature_rendering,
            }
        });

        let mut presenter = warpui::Presenter::new(window_id);
        app.update(move |ctx| {
            let root = ctx.root_view_id(window_id).expect("root view");
            presenter.invalidate(
                warpui::WindowInvalidation {
                    updated: warpui::EntityIdSet::from_iter([root]),
                    ..Default::default()
                },
                ctx,
            );
            presenter
                .build_scene(vec2f(1024., 768.), 1., None, ctx)
                .layers()
                .map(|layer| layer.glyphs.len())
                .sum()
        })
    })
}

fn assert_hidden_cursor_cell_glyph_is_painted(use_ligature_rendering: bool) {
    let hidden = scene_glyph_count(true, use_ligature_rendering);
    let shown = scene_glyph_count(false, use_ligature_rendering);
    assert_eq!(
        hidden, shown,
        "hiding the Warp cursor overlay must still paint the PTY glyph under the cursor"
    );
    assert!(
        hidden > 0,
        "expected the PTY cursor cell glyph to be painted while hide_cursor_cell is set"
    );
}

#[test]
fn paints_glyph_at_hidden_pty_cursor_cell_without_ligatures() {
    assert_hidden_cursor_cell_glyph_is_painted(false);
}

#[test]
fn paints_glyph_at_hidden_pty_cursor_cell_with_ligatures() {
    assert_hidden_cursor_cell_glyph_is_painted(true);
}
