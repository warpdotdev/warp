use warpui_core::elements::{Border, CornerRadius, ListIndentLevel, Radius, Rect};
use warpui_core::geometry::vector::vec2f;
use warpui_core::{Element, SizeConstraint};

use super::RenderableBlock;
use super::paint::RenderContext;
use super::placeholder::{self, BlockPlaceholder};
use crate::content::text::BufferBlockStyle;
use crate::extract_block;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState, RichTextStyles, bounds};

// Minimum size constraint for the bullet point. If the size is smaller than the constraint,
// the svg won't render.
const MIN_BULLET_POINT_SIZE: f32 = 6.;

pub struct RenderableBulletList {
    viewport_item: ViewportItem,
    bullet_point: Box<dyn Element>,
    bullet_size: f32,
    placeholder: BlockPlaceholder,
}

impl RenderableBulletList {
    pub fn new(
        indent_level: ListIndentLevel,
        styles: &RichTextStyles,
        viewport_item: ViewportItem,
    ) -> Self {
        let bullet_point = match indent_level {
            // Solid bullet point.
            ListIndentLevel::One => Rect::new()
                .with_background_color(styles.base_text.text_color)
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .finish(),
            // Hollow bullet point.
            ListIndentLevel::Two => Rect::new()
                .with_border(Border::all(2.).with_border_color(styles.base_text.text_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .finish(),
            // Solid square.
            ListIndentLevel::Three => Rect::new()
                .with_background_color(styles.base_text.text_color)
                .finish(),
        };

        Self {
            viewport_item,
            bullet_point,
            bullet_size: (styles.base_text.font_size / 2.).max(MIN_BULLET_POINT_SIZE),
            placeholder: BlockPlaceholder::new(true),
        }
    }
}

impl RenderableBlock for RenderableBulletList {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(
        &mut self,
        model: &RenderState,
        ctx: &mut warpui_core::LayoutContext,
        app: &warpui_core::AppContext,
    ) {
        self.bullet_point.layout(
            SizeConstraint::strict(vec2f(self.bullet_size, self.bullet_size)),
            ctx,
            app,
        );
        self.placeholder
            .layout(&self.viewport_item, model, ctx, app, |block| {
                let indent_level = match block {
                    BlockItem::UnorderedList { indent_level, .. } => *indent_level,
                    _ => ListIndentLevel::One,
                };
                placeholder::Options {
                    text: "List",
                    block_style: BufferBlockStyle::UnorderedList { indent_level },
                }
            })
    }

    fn paint(
        &mut self,
        model: &RenderState,
        ctx: &mut RenderContext,
        app: &warpui_core::AppContext,
    ) {
        let content = model.content();
        let unordered_list = extract_block!(self.viewport_item, content, (block, BlockItem::UnorderedList{ paragraph: inner, ..}) => block.unordered_list(inner));

        let text_styling = &model.styles().base_text;

        // The real bound of the unordered list could be slightly lower than the bound in the viewport item
        // because we position it in the center of the minimum height bound (if the content height is smaller
        // than the minimum height).
        let line_origin = ctx.content_to_screen(bounds::visible_origin(
            unordered_list.start_y_offset,
            &self.viewport_item.spacing,
        ));

        let content_origin = unordered_list.content_origin();
        let space_width = ctx
            .paint
            .font_cache
            .em_width(text_styling.font_family, text_styling.font_size)
            / 2.;

        // Paint the bullet point in the buffer padding to the left of the text frame.
        let bullet_origin = vec2f(
            ctx.content_to_screen(content_origin).x() - space_width - self.bullet_size,
            // Position the bullet point to the middle of the first line in the text frame.
            line_origin.y() + text_styling.line_height().as_f32() / 2. - self.bullet_size / 2.,
        );
        self.bullet_point.paint(bullet_origin, ctx.paint, app);

        if !self.placeholder.paint(content_origin, model, ctx) {
            ctx.draw_paragraph(&unordered_list, text_styling, model);
        }
    }
}
