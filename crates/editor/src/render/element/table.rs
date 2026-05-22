use std::ops::Range;

use string_offset::CharOffset;
use warpui::elements::{
    Axis, CornerRadius, DEFAULT_SCROLL_WHEEL_PIXELS_PER_LINE, Radius, ScrollData,
    ScrollbarAppearance, ScrollbarGeometry, ScrollbarWidth, compute_scrollbar_geometry,
    project_scroll_delta_by_sensitivity, scroll_delta_for_pointer_movement,
};
use warpui::event::DispatchedEvent;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::units::{IntoPixels, Pixels};
use warpui::{AppContext, ClipBounds, Event, EventContext};

use super::paint::{CursorData, CursorDisplayType};
use super::{RenderContext, RenderableBlock};
use crate::extract_block;
use crate::render::model::table_offset_map::CellAtOffset;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, LaidOutTable, RenderState, RenderedSelection, TableStyle};

const TABLE_BORDER_WIDTH: f32 = 1.0;
const TABLE_SCROLL_SENSITIVITY: f32 = 1.0;

pub struct RenderableTable {
    viewport_item: ViewportItem,
    viewport_bounds: Option<RectF>,
    scrollbar: Option<ScrollbarGeometry>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct TableLayoutReport {
    column_widths: Vec<f32>,
    column_lefts: Vec<f32>,
    header_height: f32,
    row_heights: Vec<f32>,
    row_tops: Vec<f32>,
    total_height: f32,
}

#[cfg(test)]
fn model_table_layout_report(laid_out_table: &LaidOutTable) -> TableLayoutReport {
    let column_widths = laid_out_table
        .column_widths
        .iter()
        .map(|width| width.as_f32())
        .collect::<Vec<_>>();
    let mut running_left = 0.0;
    let column_lefts = column_widths
        .iter()
        .map(|width| {
            let left = running_left;
            running_left += *width;
            left
        })
        .collect::<Vec<_>>();
    let row_heights = laid_out_table
        .row_heights
        .iter()
        .map(|height| height.as_f32())
        .collect::<Vec<_>>();
    let header_height = row_heights.first().copied().unwrap_or_default();
    let body_row_heights = row_heights.iter().copied().skip(1).collect::<Vec<_>>();
    let mut running_top = 0.0;
    let row_tops = body_row_heights
        .iter()
        .map(|height| {
            let top = running_top;
            running_top += *height;
            top
        })
        .collect::<Vec<_>>();

    TableLayoutReport {
        column_widths,
        column_lefts,
        header_height,
        row_heights: body_row_heights,
        row_tops,
        total_height: laid_out_table.total_height.as_f32(),
    }
}

#[cfg(test)]
fn row_geometry_from_layout_report(report: &TableLayoutReport) -> (Vec<f32>, Vec<f32>) {
    let mut row_tops = Vec::with_capacity(1 + report.row_tops.len());
    let mut row_heights = Vec::with_capacity(1 + report.row_heights.len());
    row_tops.push(0.0);
    row_heights.push(report.header_height);

    for (row_top, row_height) in report.row_tops.iter().zip(report.row_heights.iter()) {
        row_tops.push(report.header_height + row_top);
        row_heights.push(*row_height);
    }

    (row_tops, row_heights)
}

impl RenderableTable {
    pub fn new(viewport_item: ViewportItem) -> Self {
        Self {
            viewport_item,
            viewport_bounds: None,
            scrollbar: None,
        }
    }
}

impl RenderableBlock for RenderableTable {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(&mut self, _: &RenderState, _: &mut warpui::LayoutContext, _: &AppContext) {}

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, _app: &AppContext) {
        let content = model.content();
        let positioned_table = extract_block!(
            self.viewport_item,
            content,
            (block, BlockItem::Table(laid_out_table)) => block.table(laid_out_table)
        );

        let content_position = positioned_table.content_origin();
        let table_start = positioned_table.start_char_offset;
        let laid_out_table = positioned_table.item;
        let style = &model.styles().table_style;
        let viewport_width = table_viewport_width(laid_out_table, &self.viewport_item);
        let viewport_bounds = RectF::new(
            ctx.content_to_screen(content_position),
            vec2f(viewport_width.as_f32(), laid_out_table.height().as_f32()),
        );
        let visible_content_position =
            content_position - vec2f(laid_out_table.scroll_left().as_f32(), 0.0);
        let screen_position = ctx.content_to_screen(visible_content_position);

        self.viewport_bounds = Some(viewport_bounds);
        self.scrollbar = table_scrollbar(laid_out_table, viewport_bounds, viewport_width);
        if self.scrollbar.is_none() {
            laid_out_table.clear_scrollbar_interaction_state();
        }

        ctx.paint
            .scene
            .start_layer(ClipBounds::BoundedByActiveLayerAnd(viewport_bounds));
        ctx.paint.scene.set_active_layer_click_through();
        paint_backgrounds(laid_out_table, style, screen_position, ctx);
        paint_cell_text(laid_out_table, style, screen_position, ctx);
        paint_borders(laid_out_table, style, screen_position, ctx);
        paint_selection(model, ctx, table_start, laid_out_table, screen_position);
        paint_cursor(
            model,
            ctx,
            table_start,
            laid_out_table,
            visible_content_position,
            screen_position,
        );
        paint_scrollbar(
            self.scrollbar,
            style,
            laid_out_table.scrollbar_hovered() || laid_out_table.scrollbar_drag_state().is_some(),
            ctx,
        );
        ctx.paint.scene.stop_layer();
    }

    fn dispatch_event(
        &mut self,
        model: &RenderState,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        let content = model.content();
        let Some(block) = content.block_at_offset(self.viewport_item.block_offset()) else {
            return false;
        };
        let BlockItem::Table(laid_out_table) = block.item else {
            return false;
        };
        let viewport_width = table_viewport_width(laid_out_table, &self.viewport_item);

        match event.raw_event() {
            Event::LeftMouseDown { position, .. } => {
                if let Some(scrollbar) = self.scrollbar {
                    let thumb_hit = scrollbar.thumb_bounds.contains_point(*position);
                    let track_hit = scrollbar.track_bounds.contains_point(*position);
                    if thumb_hit {
                        laid_out_table.start_scrollbar_drag(
                            position.x().into_pixels(),
                            table_scroll_data(laid_out_table, viewport_width),
                        );
                        ctx.notify();
                        return true;
                    }

                    if track_hit {
                        let changed = laid_out_table.scroll_horizontally(
                            scroll_delta_for_pointer_movement(
                                scrollbar.thumb_center_along(Axis::Horizontal),
                                position.x().into_pixels(),
                                table_scroll_data(laid_out_table, viewport_width),
                            ),
                            viewport_width,
                        );
                        if changed {
                            ctx.notify();
                        }
                        return true;
                    }
                }

                false
            }
            Event::LeftMouseDragged { position, .. } => {
                let Some(drag_state) = laid_out_table.scrollbar_drag_state() else {
                    return false;
                };
                let scroll_delta = scroll_delta_for_pointer_movement(
                    drag_state.start_position_x,
                    position.x().into_pixels(),
                    drag_state.scroll_data,
                );
                let changed = laid_out_table
                    .set_scroll_left(drag_state.start_scroll_left - scroll_delta, viewport_width);
                if changed {
                    ctx.notify();
                }
                true
            }
            Event::LeftMouseUp { .. } => {
                let had_drag = laid_out_table.end_scrollbar_drag();
                if had_drag {
                    ctx.notify();
                }
                had_drag
            }
            Event::MouseMoved { position, .. } => {
                let hovered = self
                    .scrollbar
                    .is_some_and(|scrollbar| scrollbar.thumb_bounds.contains_point(*position));
                if laid_out_table.set_scrollbar_hovered(hovered) {
                    ctx.notify();
                }
                // MouseMoved should never be consumed here so that downstream handlers
                // (hover-link detection, cursor changes, etc.) still receive the event even
                // when the pointer is over the scrollbar thumb.
                false
            }
            Event::ScrollWheel {
                position,
                delta,
                precise,
                modifiers,
            } if !modifiers.ctrl => {
                let Some(bounds) = self.viewport_bounds else {
                    return false;
                };
                if !bounds.contains_point(*position)
                    || laid_out_table.max_scroll_left(viewport_width) <= Pixels::zero()
                {
                    return false;
                }
                let Some(horizontal_delta) =
                    table_horizontal_scroll_delta(*delta, *precise, modifiers.shift)
                else {
                    return false;
                };
                let changed = laid_out_table.scroll_horizontally(horizontal_delta, viewport_width);
                if changed {
                    ctx.notify();
                }
                // If the table is already pinned at the scroll edge in the direction of the
                // delta, returning `false` lets the event fall through to the surrounding
                // vertical scroller instead of sticking at the horizontal edge.
                changed
            }
            _ => false,
        }
    }
}

fn table_viewport_width(laid_out_table: &LaidOutTable, viewport_item: &ViewportItem) -> Pixels {
    laid_out_table.viewport_width(Pixels::new(viewport_item.content_size.x()))
}

fn table_scroll_data(laid_out_table: &LaidOutTable, viewport_width: Pixels) -> ScrollData {
    ScrollData {
        scroll_start: laid_out_table.scroll_left(),
        visible_px: laid_out_table.viewport_width(viewport_width),
        total_size: laid_out_table.width(),
    }
}

fn table_horizontal_scroll_delta(delta: Vector2F, precise: bool, shift: bool) -> Option<Pixels> {
    let delta = if shift && delta.x().abs() <= f32::EPSILON {
        vec2f(delta.y(), 0.0)
    } else {
        delta
    };
    let projected_delta = project_scroll_delta_by_sensitivity(delta, TABLE_SCROLL_SENSITIVITY);
    let horizontal_delta = projected_delta.x();
    (horizontal_delta.abs() > f32::EPSILON).then_some(Pixels::new(if precise {
        horizontal_delta
    } else {
        horizontal_delta * DEFAULT_SCROLL_WHEEL_PIXELS_PER_LINE
    }))
}

fn table_scrollbar(
    laid_out_table: &LaidOutTable,
    viewport_bounds: RectF,
    viewport_width: Pixels,
) -> Option<ScrollbarGeometry> {
    let scrollbar = compute_scrollbar_geometry(
        Axis::Horizontal,
        viewport_bounds.origin(),
        viewport_bounds.size(),
        table_scroll_data(laid_out_table, viewport_width),
        ScrollbarAppearance::new(ScrollbarWidth::Auto, true),
    );
    scrollbar.has_thumb().then_some(scrollbar)
}

fn paint_scrollbar(
    scrollbar: Option<ScrollbarGeometry>,
    style: &TableStyle,
    active: bool,
    ctx: &mut RenderContext,
) {
    let Some(scrollbar) = scrollbar else {
        return;
    };
    ctx.paint
        .scene
        .draw_rect_without_hit_recording(scrollbar.thumb_bounds)
        .with_background(if active {
            style.scrollbar_active_thumb_color
        } else {
            style.scrollbar_nonactive_thumb_color
        })
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.0)));
}

fn paint_backgrounds(
    laid_out_table: &LaidOutTable,
    style: &TableStyle,
    screen_position: Vector2F,
    ctx: &mut RenderContext,
) {
    let total_rows = 1 + laid_out_table.table.rows.len();
    for row in 0..total_rows {
        let y = row_y_offset(laid_out_table, row);
        let h = row_height(laid_out_table, row);
        let bg = if row == 0 {
            style.header_background
        } else if let Some(alt) = style.alternate_row_background {
            if row % 2 == 0 {
                alt
            } else {
                style.cell_background
            }
        } else {
            style.cell_background
        };
        ctx.paint
            .scene
            .draw_rect_without_hit_recording(RectF::new(
                screen_position + vec2f(0.0, y),
                vec2f(laid_out_table.width().as_f32(), h),
            ))
            .with_background(bg);
    }
}

fn paint_cell_text(
    laid_out_table: &LaidOutTable,
    style: &TableStyle,
    screen_position: Vector2F,
    ctx: &mut RenderContext,
) {
    let total_rows = 1 + laid_out_table.table.rows.len();
    let num_cols = laid_out_table.column_widths.len();

    for row in 0..total_rows {
        for col in 0..num_cols {
            let is_header = row == 0;
            let text_color = if is_header {
                style.header_text_color
            } else {
                style.text_color
            };
            let Some(frame) = laid_out_table
                .cell_text_frames
                .get(row)
                .and_then(|row_frames| row_frames.get(col))
            else {
                continue;
            };
            let cell_content_width = laid_out_table
                .column_widths
                .get(col)
                .map(|w| w.as_f32())
                .unwrap_or(0.0)
                - style.cell_padding * 2.0;
            let cell_content_width = cell_content_width.max(0.0);
            let bounds = RectF::new(
                screen_position + laid_out_table.cell_content_origin(row, col),
                vec2f(cell_content_width, f32::MAX),
            );
            frame.paint(
                bounds,
                &Default::default(),
                text_color,
                ctx.paint.scene,
                ctx.paint.font_cache,
            );
        }
    }
}

fn paint_borders(
    laid_out_table: &LaidOutTable,
    style: &TableStyle,
    screen_position: Vector2F,
    ctx: &mut RenderContext,
) {
    let bw = TABLE_BORDER_WIDTH;
    let table_w = laid_out_table.width().as_f32();
    let table_h = laid_out_table.total_height.as_f32();
    let bc = style.border_color;

    let draw = |ctx: &mut RenderContext, rect: RectF| {
        ctx.paint
            .scene
            .draw_rect_without_hit_recording(rect)
            .with_background(bc);
    };

    if style.outer_border {
        draw(ctx, RectF::new(screen_position, vec2f(table_w, bw)));
        draw(
            ctx,
            RectF::new(
                screen_position + vec2f(0.0, table_h - bw),
                vec2f(table_w, bw),
            ),
        );
        draw(ctx, RectF::new(screen_position, vec2f(bw, table_h)));
        draw(
            ctx,
            RectF::new(
                screen_position + vec2f(table_w - bw, 0.0),
                vec2f(bw, table_h),
            ),
        );
    }

    let total_rows = 1 + laid_out_table.table.rows.len();
    if style.row_dividers {
        for row in 1..total_rows {
            let y = row_y_offset(laid_out_table, row);
            draw(
                ctx,
                RectF::new(screen_position + vec2f(0.0, y), vec2f(table_w, bw)),
            );
        }
    }

    if style.column_dividers {
        let mut col_x = 0.0f32;
        for col in 0..laid_out_table.column_widths.len().saturating_sub(1) {
            col_x += laid_out_table.column_widths[col].as_f32();
            draw(
                ctx,
                RectF::new(screen_position + vec2f(col_x, 0.0), vec2f(bw, table_h)),
            );
        }
    }
}

fn paint_selection(
    model: &RenderState,
    ctx: &mut RenderContext,
    table_start: CharOffset,
    laid_out_table: &LaidOutTable,
    screen_position: Vector2F,
) {
    let selections = model.selections();

    for selection in selections.iter() {
        let Some(selection_range) =
            table_selection_relative_range(selection, table_start, laid_out_table)
        else {
            continue;
        };
        let relative_start = selection_range.start;
        let relative_end = selection_range.end;

        let affected_cells = laid_out_table
            .offset_map
            .cells_in_range(relative_start, relative_end);

        for cell in affected_cells {
            let Some(cell_offset_map) = laid_out_table
                .cell_offset_maps
                .get(cell.row)
                .and_then(|row_maps| row_maps.get(cell.col))
            else {
                continue;
            };

            let cell_char_start_in_table = cell.start;
            let cell_char_end_in_table = cell.end;
            let sel_start_in_cell = cell_offset_map
                .source_to_rendered(if relative_start > cell_char_start_in_table {
                    relative_start - cell_char_start_in_table
                } else {
                    CharOffset::zero()
                })
                .as_usize();
            let sel_end_in_cell = cell_offset_map
                .source_to_rendered(if relative_end < cell_char_end_in_table {
                    relative_end - cell_char_start_in_table
                } else {
                    cell_char_end_in_table - cell_char_start_in_table
                })
                .as_usize();

            let cell_layout = laid_out_table
                .cell_layouts
                .get(cell.row)
                .and_then(|row_layouts| row_layouts.get(cell.col));
            let cell_content_origin =
                screen_position + laid_out_table.cell_content_origin(cell.row, cell.col);

            let Some(layout) = cell_layout else {
                continue;
            };

            let start_line = layout
                .line_at_char_offset(CharOffset::from(sel_start_in_cell))
                .unwrap_or(0);
            let end_line = layout
                .line_at_char_offset(CharOffset::from(sel_end_in_cell.saturating_sub(1)))
                .unwrap_or(start_line);

            for line_idx in start_line..=end_line {
                let line_y = layout.line_y_offsets.get(line_idx).copied().unwrap_or(0.0);
                let line_height = layout.line_heights.get(line_idx).copied().unwrap_or(20.0);

                let line_start_x = if line_idx == start_line {
                    layout.x_for_char_in_line(line_idx, sel_start_in_cell)
                } else {
                    0.0
                };

                let line_end_x = if line_idx == end_line {
                    layout.x_for_char_in_line(line_idx, sel_end_in_cell)
                } else {
                    let range = layout.line_char_ranges.get(line_idx);
                    layout
                        .x_for_char_in_line(line_idx, range.map(|r| r.end.as_usize()).unwrap_or(0))
                };

                let sel_rect = RectF::new(
                    vec2f(
                        cell_content_origin.x() + line_start_x,
                        cell_content_origin.y() + line_y,
                    ),
                    vec2f((line_end_x - line_start_x).max(1.0), line_height),
                );

                ctx.paint
                    .scene
                    .draw_rect_without_hit_recording(sel_rect)
                    .with_background(model.styles().selection_fill);
            }
        }
    }
}

fn paint_cursor(
    model: &RenderState,
    ctx: &mut RenderContext,
    table_start: CharOffset,
    laid_out_table: &LaidOutTable,
    content_position: Vector2F,
    screen_position: Vector2F,
) {
    let selections = model.selections();

    for selection in selections.iter() {
        let Some(relative_offset) =
            table_cursor_relative_offset(selection, table_start, laid_out_table)
        else {
            continue;
        };

        if let Some(CellAtOffset {
            row,
            col,
            offset_in_cell,
        }) = laid_out_table.offset_map.cell_at_offset(relative_offset)
        {
            let Some(cell_offset_map) = laid_out_table
                .cell_offset_maps
                .get(row)
                .and_then(|row_maps| row_maps.get(col))
            else {
                continue;
            };
            let cell_content_origin =
                screen_position + laid_out_table.cell_content_origin(row, col);

            let Some(cell_layout) = laid_out_table
                .cell_layouts
                .get(row)
                .and_then(|row_layouts| row_layouts.get(col))
            else {
                continue;
            };

            let rendered_offset_in_cell = cell_offset_map.source_to_rendered(offset_in_cell);
            let offset_in_cell_usize = rendered_offset_in_cell.as_usize();
            let line_idx = cell_layout
                .line_at_char_offset(rendered_offset_in_cell)
                .unwrap_or(0);
            let cursor_y_offset = cell_layout
                .line_y_offsets
                .get(line_idx)
                .copied()
                .unwrap_or(0.0);
            let cursor_height = cell_layout
                .line_heights
                .get(line_idx)
                .copied()
                .unwrap_or(20.0);
            let cursor_x_offset = cell_layout.x_for_char_in_line(line_idx, offset_in_cell_usize);

            let cursor_screen_x = cell_content_origin.x() + cursor_x_offset;
            let cursor_screen_y = cell_content_origin.y() + cursor_y_offset;
            let cursor_content_x = content_position.x() + (cursor_screen_x - screen_position.x());
            let cursor_content_y = content_position.y() + (cursor_screen_y - screen_position.y());

            ctx.draw_and_save_cursor(
                CursorDisplayType::Bar,
                vec2f(cursor_content_x, cursor_content_y),
                vec2f(model.styles().cursor_width, cursor_height),
                CursorData::default(),
                model.styles(),
            );
        }
    }
}

fn table_selection_relative_range(
    selection: &RenderedSelection,
    table_start: CharOffset,
    laid_out_table: &LaidOutTable,
) -> Option<Range<CharOffset>> {
    let table_end = table_start + laid_out_table.content_length();
    let start = selection.start().max(table_start);
    let end = selection.end().min(table_end);
    (start < end).then(|| (start - table_start)..(end - table_start))
}

fn table_cursor_relative_offset(
    selection: &RenderedSelection,
    table_start: CharOffset,
    laid_out_table: &LaidOutTable,
) -> Option<CharOffset> {
    let table_end = table_start + laid_out_table.content_length();
    let head = selection.head;
    (head >= table_start && head < table_end).then(|| head - table_start)
}

fn row_y_offset(laid_out_table: &LaidOutTable, row: usize) -> f32 {
    laid_out_table
        .row_y_offsets
        .get(row)
        .copied()
        .unwrap_or(0.0)
}

fn row_height(laid_out_table: &LaidOutTable, row: usize) -> f32 {
    laid_out_table
        .row_heights
        .get(row)
        .map(|h| h.as_f32())
        .unwrap_or(20.0)
}

#[cfg(test)]
#[path = "table_tests.rs"]
mod tests;
