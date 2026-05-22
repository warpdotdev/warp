use warpui::elements::{
    Align, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, Icon,
    ParentElement, Radius, Shrinkable, Text,
};
use warpui::geometry::vector::vec2f;
use warpui::{AppContext, Element, SizeConstraint};

use super::RenderableBlock;
use crate::editor::EmbeddedItemModel;
use crate::extract_block;
use crate::render::element::paint::{CursorData, CursorDisplayType};
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RichTextStyles};

pub struct RenderableBrokenEmbedding {
    row: Box<dyn Element>,
    viewport_item: ViewportItem,
}

impl RenderableBrokenEmbedding {
    pub fn new(
        viewport_item: ViewportItem,
        styles: &RichTextStyles,
        model: Option<&dyn EmbeddedItemModel>,
        ctx: &AppContext,
    ) -> Self {
        let icon = ConstrainedBox::new(
            Icon::new(
                styles.broken_link_style.icon_path,
                styles.broken_link_style.icon_color,
            )
            .with_opacity(1.0)
            .finish(),
        )
        .with_height(styles.base_text.font_size + 2.)
        .with_width(styles.base_text.font_size + 2.)
        .finish();

        let text = Container::new(
            Text::new_inline(
                "Embed not found",
                styles.base_text.font_family,
                styles.base_text.font_size,
            )
            .with_color(styles.placeholder_color)
            .finish(),
        )
        .with_padding_left(8.)
        .finish();

        let mut row = Flex::row()
            .with_child(icon)
            .with_child(text)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(element) = model.and_then(|model| model.render_remove_embedding_button(ctx)) {
            row.add_child(Shrinkable::new(1., Empty::new().finish()).finish());
            row.add_child(Align::new(element).right().finish());
        }

        Self {
            viewport_item,
            row: row.finish(),
        }
    }
}

impl RenderableBlock for RenderableBrokenEmbedding {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(
        &mut self,
        model: &crate::render::model::RenderState,
        ctx: &mut warpui::LayoutContext,
        app: &warpui::AppContext,
    ) {
        self.row.layout(
            SizeConstraint::strict(vec2f(
                self.viewport_item.content_size.x(),
                // Depending on font size, the line height could be bigger
                // or smaller than font size + 2. (icon size). Choose the larger
                // of the two to avoid failing to layout the element.
                model
                    .styles()
                    .base_text
                    .line_height()
                    .as_f32()
                    .max(model.styles().base_text.font_size + 2.),
            )),
            ctx,
            app,
        );
    }

    fn paint(
        &mut self,
        model: &crate::render::model::RenderState,
        ctx: &mut super::RenderContext,
        app: &warpui::AppContext,
    ) {
        let content = model.content();
        let broken_link = extract_block!(self.viewport_item, content, (block, BlockItem::Embedded(item)) => block.embedded(item));

        // Render as selected if the broken link is within any selection.
        let selected = model.offset_in_active_selection(broken_link.start_char_offset);

        // Draw the cursor if the broken link is at any cursor.
        let draw_cursor = model.is_selection_head(broken_link.start_char_offset);

        let styles = model.styles();

        let background_rect = self.viewport_item.visible_bounds(ctx);

        ctx.paint
            .scene
            .draw_rect_without_hit_recording(background_rect)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_background(model.styles().embedding_background);

        let original_content_origin = broken_link.content_origin();
        let vertical_center_delta =
            styles.base_text.line_height().as_f32() * (styles.base_text.baseline_ratio - 0.5);

        let content_origin = original_content_origin - vec2f(0., vertical_center_delta);

        if selected {
            ctx.paint
                .scene
                .draw_rect_with_hit_recording(background_rect)
                .with_background(styles.selection_fill);
        }

        if draw_cursor {
            let line_height = styles.base_text.line_height().as_f32();
            // The lower right corner of the background rect is at reserved_origin + background_rect.size()
            // Add some horizontal padding and minus line height vertically so it's visible and aligned to
            // the bottom of the background rect.
            let end_of_line_position =
                broken_link.reserved_origin() + background_rect.size() + vec2f(5., -line_height);
            ctx.draw_and_save_cursor(
                CursorDisplayType::Bar,
                end_of_line_position,
                vec2f(styles.cursor_width, line_height),
                CursorData::default(),
                styles,
            );
        }

        ctx.paint.scene.start_layer(warpui::ClipBounds::ActiveLayer);
        self.row
            .paint(ctx.content_to_screen(content_origin), ctx.paint, app);
        ctx.paint.scene.stop_layer();
    }

    fn after_layout(&mut self, ctx: &mut warpui::AfterLayoutContext, app: &warpui::AppContext) {
        self.row.after_layout(ctx, app);
    }

    fn dispatch_event(
        &mut self,
        _model: &crate::render::model::RenderState,
        event: &warpui::event::DispatchedEvent,
        ctx: &mut warpui::EventContext,
        app: &AppContext,
    ) -> bool {
        self.row.dispatch_event(event, ctx, app)
    }
}
