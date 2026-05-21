use warpui::elements::{CornerRadius, Radius};
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{Vector2F, vec2f};

use super::{RenderContext, RenderableBlock};
use crate::extract_block;
use crate::render::element::paint::{CursorData, CursorDisplayType};
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState, RichTextStyles};

/// Renderable representation of a single horizontal rule separator.
pub struct HorizontalRule {
    viewport_item: ViewportItem,
}

impl HorizontalRule {
    pub fn new(viewport_item: ViewportItem) -> Self {
        Self { viewport_item }
    }

    pub fn draw_rect(
        content_position: Vector2F,
        selected: bool,
        draw_cursor: bool,
        size: Vector2F,
        styles: &RichTextStyles,
        ctx: &mut RenderContext,
    ) {
        let rect_origin = ctx.content_to_screen(content_position);
        let y_axis_offset = (size.y() - styles.horizontal_rule_style.rule_height).max(0.) / 2.;

        let rule_bounds = RectF::new(
            vec2f(rect_origin.x(), rect_origin.y() + y_axis_offset),
            vec2f(size.x(), styles.horizontal_rule_style.rule_height),
        );
        let line_bounds = RectF::new(rect_origin, size);

        ctx.paint
            .scene
            .draw_rect_with_hit_recording(rule_bounds)
            .with_background(styles.horizontal_rule_style.color)
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)));

        if selected {
            ctx.paint
                .scene
                .draw_rect_with_hit_recording(line_bounds)
                .with_background(styles.selection_fill);
        }

        if draw_cursor {
            let end_of_line_position = content_position + vec2f(size.x(), 0.);
            ctx.draw_and_save_cursor(
                CursorDisplayType::Bar,
                end_of_line_position,
                vec2f(styles.cursor_width, size.y()),
                CursorData::default(),
                styles,
            );
        }
    }
}

impl RenderableBlock for HorizontalRule {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(
        &mut self,
        _model: &RenderState,
        _ctx: &mut warpui::LayoutContext,
        _app: &warpui::AppContext,
    ) {
    }

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, _app: &warpui::AppContext) {
        let content = model.content();
        let horizontal_rule = extract_block!(self.viewport_item, content, (block, BlockItem::HorizontalRule(rule)) => block.horizontal_rule(rule));

        let selected = model.offset_in_active_selection(horizontal_rule.start_char_offset);
        let draw_cursor = model.is_selection_head(horizontal_rule.start_char_offset);

        Self::draw_rect(
            horizontal_rule.content_origin(),
            selected,
            draw_cursor,
            horizontal_rule.item.line_size()
                - vec2f(self.viewport_item.spacing.x_axis_offset().as_f32(), 0.),
            model.styles(),
            ctx,
        );
    }
}
