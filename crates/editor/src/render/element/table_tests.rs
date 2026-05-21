use std::cell::Cell;
use std::sync::Arc;

use pathfinder_color::ColorU;
use string_offset::CharOffset;
use warpui::elements::{Axis, scroll_delta_for_pointer_movement};
use warpui::fonts::FamilyId;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::vec2f;
use warpui::text_layout::TextFrame;
use warpui::units::{IntoPixels, Pixels};

use crate::content::text::{FormattedTable, table_cell_offset_maps};
use crate::render::element::table::{
    model_table_layout_report, row_geometry_from_layout_report, table_cursor_relative_offset,
    table_horizontal_scroll_delta, table_scroll_data, table_scrollbar,
    table_selection_relative_range,
};
use crate::render::model::table_offset_map::TableOffsetMap;
use crate::render::model::{
    BlockSpacing, CellLayout, LaidOutTable, RenderedSelection, TableBlockConfig, TableStyle,
};

fn test_laid_out_table() -> LaidOutTable {
    let source = "h1\th2\nr1\tr2\nx\ty\n";
    let table = FormattedTable::from_internal_format(source);
    let cell_offset_maps = table_cell_offset_maps(&table, source);
    let row_heights = vec![24.0.into_pixels(), 30.0.into_pixels(), 42.0.into_pixels()];
    let column_widths = vec![80.0.into_pixels(), 120.0.into_pixels()];
    let total_height = row_heights
        .iter()
        .fold(Pixels::zero(), |acc, row_height| acc + *row_height);

    let mut row_y_offsets = Vec::with_capacity(row_heights.len() + 1);
    row_y_offsets.push(0.0);
    let mut running_y = 0.0;
    for row_height in &row_heights {
        running_y += row_height.as_f32();
        row_y_offsets.push(running_y);
    }

    let mut col_x_offsets = Vec::with_capacity(column_widths.len() + 1);
    col_x_offsets.push(0.0);
    let mut running_x = 0.0;
    for column_width in &column_widths {
        running_x += column_width.as_f32();
        col_x_offsets.push(running_x);
    }

    let offset_map = TableOffsetMap::new(
        cell_offset_maps
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.source_length().as_usize())
                    .collect()
            })
            .collect(),
    );
    let content_length = offset_map.total_length();
    let config = TableBlockConfig {
        width: 200.0.into_pixels(),
        spacing: BlockSpacing::default(),
        style: TableStyle {
            border_color: ColorU::new(10, 11, 12, 255),
            header_background: ColorU::new(20, 21, 22, 255),
            cell_background: ColorU::new(30, 31, 32, 255),
            alternate_row_background: None,
            text_color: ColorU::new(40, 41, 42, 255),
            header_text_color: ColorU::new(50, 51, 52, 255),
            scrollbar_nonactive_thumb_color: ColorU::new(60, 61, 62, 255),
            scrollbar_active_thumb_color: ColorU::new(70, 71, 72, 255),
            font_family: FamilyId(0),
            font_size: 14.0,
            cell_padding: 6.0,
            outer_border: true,
            column_dividers: true,
            row_dividers: true,
        },
    };
    let cell_layouts = vec![vec![CellLayout::default(); 2]; 3];
    let cell_text_frames = vec![vec![Arc::new(TextFrame::mock("")); 2]; 3];

    LaidOutTable {
        table,
        config,
        row_heights,
        column_widths,
        total_height,
        offset_map,
        content_length,
        cell_offset_maps,
        row_y_offsets,
        col_x_offsets,
        cell_text_frames,
        cell_layouts,
        cell_links: vec![
            vec![vec![], vec![]],
            vec![vec![], vec![]],
            vec![vec![], vec![]],
        ],
        scroll_left: Cell::new(Pixels::zero()),
        scrollbar_interaction_state: Default::default(),
        horizontal_scroll_allowed: true,
    }
}

#[test]
fn model_table_layout_report_matches_model_geometry() {
    let laid_out_table = test_laid_out_table();
    let report = model_table_layout_report(&laid_out_table);

    assert_eq!(report.column_widths, vec![80.0, 120.0]);
    assert_eq!(report.column_lefts, vec![0.0, 80.0]);
    assert_eq!(report.header_height, 24.0);
    assert_eq!(report.row_heights, vec![30.0, 42.0]);
    assert_eq!(report.row_tops, vec![0.0, 30.0]);
    assert_eq!(report.total_height, 96.0);
}

#[test]
fn row_geometry_from_layout_report_includes_header_row() {
    let report = model_table_layout_report(&test_laid_out_table());
    let (row_tops, row_heights) = row_geometry_from_layout_report(&report);

    assert_eq!(row_tops, vec![0.0, 24.0, 54.0]);
    assert_eq!(row_heights, vec![24.0, 30.0, 42.0]);
}

#[test]
fn coordinate_to_offset_returns_cell_start_for_first_cell() {
    let table = test_laid_out_table();
    let offset = table.coordinate_to_offset(10.0, 5.0);
    assert_eq!(offset, CharOffset::zero());
}

#[test]
fn coordinate_to_offset_targets_second_column() {
    let table = test_laid_out_table();
    let offset = table.coordinate_to_offset(90.0, 5.0);
    let cell_range = table.offset_map.cell_range(0, 1);
    assert!(cell_range.is_some());
    let range = cell_range.expect("cell (0,1) should exist");
    assert!(
        offset >= range.start && offset <= range.end,
        "offset {offset:?} should be within cell (0,1) range {:?}..{:?}",
        range.start,
        range.end,
    );
}

#[test]
fn coordinate_to_offset_targets_second_row() {
    let table = test_laid_out_table();
    let offset = table.coordinate_to_offset(10.0, 30.0);
    let cell_range = table.offset_map.cell_range(1, 0);
    assert!(cell_range.is_some());
    let range = cell_range.expect("cell (1,0) should exist");
    assert!(
        offset >= range.start && offset <= range.end,
        "offset {offset:?} should be within cell (1,0) range {:?}..{:?}",
        range.start,
        range.end,
    );
}

#[test]
fn cells_in_range_single_cell() {
    let table = test_laid_out_table();
    let cells = table
        .offset_map
        .cells_in_range(CharOffset::from(0usize), CharOffset::from(2usize));
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].row, 0);
    assert_eq!(cells[0].col, 0);
}

#[test]
fn cells_in_range_across_row() {
    let table = test_laid_out_table();
    let cells = table
        .offset_map
        .cells_in_range(CharOffset::from(0usize), CharOffset::from(5usize));
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0].row, 0);
    assert_eq!(cells[0].col, 0);
    assert_eq!(cells[1].row, 0);
    assert_eq!(cells[1].col, 1);
}

#[test]
fn cells_in_range_cross_row() {
    let table = test_laid_out_table();
    let cells = table
        .offset_map
        .cells_in_range(CharOffset::from(3usize), CharOffset::from(9usize));
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0].row, 0);
    assert_eq!(cells[0].col, 1);
    assert_eq!(cells[1].row, 1);
    assert_eq!(cells[1].col, 0);
}

#[test]
fn cells_in_range_entire_table() {
    let table = test_laid_out_table();
    let cells = table
        .offset_map
        .cells_in_range(CharOffset::from(0usize), table.content_length());
    assert_eq!(cells.len(), 6);
}

fn single_line_cell_layout(char_count: usize, line_height: f32, line_width: f32) -> CellLayout {
    use warpui::text_layout::CaretPosition;
    let mut carets = Vec::with_capacity(char_count);
    let char_width = if char_count > 0 {
        line_width / char_count as f32
    } else {
        0.0
    };
    for i in 0..char_count {
        carets.push(CaretPosition {
            start_offset: i,
            last_offset: i,
            position_in_line: i as f32 * char_width,
        });
    }
    CellLayout {
        line_heights: vec![line_height],
        line_y_offsets: vec![0.0],
        line_char_ranges: vec![CharOffset::from(0usize)..CharOffset::from(char_count)],
        line_widths: vec![line_width],
        line_caret_positions: vec![carets],
    }
}

#[test]
fn cell_layout_line_at_char_offset_single_line() {
    let layout = single_line_cell_layout(5, 21.0, 50.0);
    assert_eq!(
        layout.line_at_char_offset(CharOffset::from(0usize)),
        Some(0)
    );
    assert_eq!(
        layout.line_at_char_offset(CharOffset::from(4usize)),
        Some(0)
    );
}

#[test]
fn cell_layout_x_for_char_returns_proportional_position() {
    let layout = single_line_cell_layout(4, 21.0, 40.0);
    let x0 = layout.x_for_char_in_line(0, 0);
    let x2 = layout.x_for_char_in_line(0, 2);
    assert!(
        x0 < x2,
        "x at char 0 ({x0}) should be less than x at char 2 ({x2})"
    );
}

#[test]
fn cell_layout_x_past_end_returns_line_width() {
    let layout = single_line_cell_layout(3, 21.0, 30.0);
    let x = layout.x_for_char_in_line(0, 10);
    assert_eq!(x, 30.0);
}

#[test]
fn coordinate_to_offset_with_cell_layouts() {
    let mut table = test_laid_out_table();
    table.cell_layouts = vec![
        vec![
            single_line_cell_layout(2, 21.0, 40.0),
            single_line_cell_layout(2, 21.0, 40.0),
        ],
        vec![
            single_line_cell_layout(2, 21.0, 40.0),
            single_line_cell_layout(2, 21.0, 40.0),
        ],
        vec![
            single_line_cell_layout(1, 21.0, 20.0),
            single_line_cell_layout(1, 21.0, 20.0),
        ],
    ];

    let offset_start = table.coordinate_to_offset(7.0, 1.0);
    let cell_0_0 = table
        .offset_map
        .cell_range(0, 0)
        .expect("cell (0,0) should exist");
    assert!(
        offset_start >= cell_0_0.start && offset_start <= cell_0_0.end,
        "offset at (7,1) should be in cell (0,0)",
    );

    let offset_col1 = table.coordinate_to_offset(100.0, 30.0);
    let cell_1_1 = table
        .offset_map
        .cell_range(1, 1)
        .expect("cell (1,1) should exist");
    assert!(
        offset_col1 >= cell_1_1.start && offset_col1 <= cell_1_1.end,
        "offset at (100,30) should be in cell (1,1)",
    );
}

#[test]
fn table_selection_relative_range_starts_at_table_start() {
    let table = test_laid_out_table();
    let range = table_selection_relative_range(
        &RenderedSelection::new(CharOffset::from(5usize), CharOffset::from(6usize)),
        CharOffset::from(5usize),
        &table,
    )
    .expect("selection should overlap table");

    assert_eq!(range, CharOffset::from(0usize)..CharOffset::from(1usize));
}

#[test]
fn table_selection_relative_range_excludes_non_overlapping_selection() {
    let table = test_laid_out_table();
    let range = table_selection_relative_range(
        &RenderedSelection::new(CharOffset::from(1usize), CharOffset::from(4usize)),
        CharOffset::from(5usize),
        &table,
    );

    assert!(range.is_none());
}

#[test]
fn table_cursor_relative_offset_starts_at_zero_for_first_table_character() {
    let table = test_laid_out_table();
    let relative_offset = table_cursor_relative_offset(
        &RenderedSelection::new(CharOffset::from(5usize), CharOffset::from(5usize)),
        CharOffset::from(5usize),
        &table,
    );

    assert_eq!(relative_offset, Some(CharOffset::from(0usize)));
}

#[test]
fn table_cursor_relative_offset_excludes_cursor_before_table_start() {
    let table = test_laid_out_table();
    let relative_offset = table_cursor_relative_offset(
        &RenderedSelection::new(CharOffset::from(4usize), CharOffset::from(4usize)),
        CharOffset::from(5usize),
        &table,
    );

    assert_eq!(relative_offset, None);
}

#[test]
fn table_scrollbar_uses_shared_overlay_geometry() {
    let table = test_laid_out_table();
    let scrollbar = table_scrollbar(
        &table,
        RectF::new(vec2f(10.0, 20.0), vec2f(90.0, table.height().as_f32())),
        90.0.into_pixels(),
    )
    .expect("wide table should produce a horizontal scrollbar");

    assert_eq!(scrollbar.track_bounds.height(), 12.0);
    assert_eq!(scrollbar.track_bounds.width(), 90.0);
    assert_eq!(scrollbar.thumb_bounds.height(), 8.0);
    assert_eq!(scrollbar.thumb_bounds.origin_y(), 106.0);
    assert_eq!(scrollbar.thumb_bounds.width(), 40.5);
}

#[test]
fn table_horizontal_scroll_delta_projects_trackpad_motion_without_vertical_jitter() {
    assert_eq!(
        table_horizontal_scroll_delta(vec2f(12.0, 4.0), true, false),
        Some(12.0.into_pixels()),
    );
    assert_eq!(
        table_horizontal_scroll_delta(vec2f(4.0, 12.0), true, false),
        None,
    );
}

#[test]
fn table_scrollbar_pointer_movement_matches_drag_and_gutter_behavior() {
    let table = test_laid_out_table();
    let viewport_width = 90.0.into_pixels();
    let scroll_data = table_scroll_data(&table, viewport_width);
    let scrollbar = table_scrollbar(
        &table,
        RectF::new(vec2f(10.0, 20.0), vec2f(90.0, table.height().as_f32())),
        viewport_width,
    )
    .expect("wide table should produce a horizontal scrollbar");

    assert_eq!(
        scroll_delta_for_pointer_movement(20.0.into_pixels(), 38.0.into_pixels(), scroll_data),
        (-40.0).into_pixels(),
    );
    assert_eq!(
        scroll_delta_for_pointer_movement(
            scrollbar.thumb_center_along(Axis::Horizontal),
            75.25.into_pixels(),
            scroll_data,
        ),
        (-100.0).into_pixels(),
    );
}

#[test]
fn table_scrollbar_drag_state_survives_renderable_recreation() {
    let table = test_laid_out_table();
    let viewport_width = 90.0.into_pixels();
    table.start_scrollbar_drag(
        20.0.into_pixels(),
        table_scroll_data(&table, viewport_width),
    );

    let drag_state = table
        .scrollbar_drag_state()
        .expect("drag state should persist on the table");
    let scroll_delta = scroll_delta_for_pointer_movement(
        drag_state.start_position_x,
        38.0.into_pixels(),
        drag_state.scroll_data,
    );

    assert!(table.set_scroll_left(drag_state.start_scroll_left - scroll_delta, viewport_width));
    assert_eq!(table.scroll_left(), 40.0.into_pixels());
    assert!(table.end_scrollbar_drag());
}

#[test]
fn table_cursor_relative_offset_excludes_cursor_at_table_end() {
    let table = test_laid_out_table();
    let table_start = CharOffset::from(5usize);
    let relative_offset = table_cursor_relative_offset(
        &RenderedSelection::new(
            table_start + table.content_length(),
            table_start + table.content_length(),
        ),
        table_start,
        &table,
    );

    assert_eq!(relative_offset, None);
}
