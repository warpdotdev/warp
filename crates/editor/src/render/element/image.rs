use warpui_core::elements::{
    CacheOption, ConstrainedBox, Container, CrossAxisAlignment, Flex, Icon, Image, ParentElement,
    Text,
};
use warpui_core::geometry::vector::vec2f;
use warpui_core::{Element, SizeConstraint};

use super::{RenderContext, RenderableBlock};
use crate::extract_block;
use crate::render::element::paint::{CursorData, CursorDisplayType};
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState, RichTextStyles};

/// Text shown alongside the broken-image glyph when an image fails to load.
/// Prefers the image's alt text; falls back to a generic notice when the alt
/// text is empty or whitespace-only.
fn broken_image_label(alt_text: &str) -> String {
    let trimmed = alt_text.trim();
    if trimmed.is_empty() {
        "Image failed to load".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Builds the placeholder element painted in place of an image that failed to
/// load: a failed-image glyph followed by the image's alt text (or a generic
/// notice when no alt text is present).
fn broken_image_placeholder(alt_text: &str, styles: &RichTextStyles) -> Box<dyn Element> {
    let icon = ConstrainedBox::new(
        Icon::new(
            styles.broken_image_style.icon_path,
            styles.broken_image_style.icon_color,
        )
        .with_opacity(1.0)
        .finish(),
    )
    .with_height(styles.base_text.font_size + 2.)
    .with_width(styles.base_text.font_size + 2.)
    .finish();

    let text = Container::new(
        Text::new(
            broken_image_label(alt_text),
            styles.base_text.font_family,
            styles.base_text.font_size,
        )
        .with_color(styles.placeholder_color)
        .finish(),
    )
    .with_padding_left(8.)
    .finish();

    Flex::row()
        .with_child(icon)
        .with_child(text)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .finish()
}

pub struct RenderableImage {
    viewport_item: ViewportItem,
    // TODO: The AssetCache does not currently support automatic eviction of assets when they are
    // dropped. We should consider implementing a mechanism to unload images when they are no longer
    // visible or referenced.
    image_element: Option<Box<dyn Element>>,
}

impl RenderableImage {
    pub fn new(viewport_item: ViewportItem) -> Self {
        Self {
            viewport_item,
            image_element: None,
        }
    }
}

impl RenderableBlock for RenderableImage {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(
        &mut self,
        model: &RenderState,
        ctx: &mut warpui_core::LayoutContext,
        app: &warpui_core::AppContext,
    ) {
        let content = model.content();
        let (asset_source, config, alt_text) = extract_block!(
            self.viewport_item,
            content,
            (_block, BlockItem::Image { asset_source, config, alt_text, .. }) => (asset_source.clone(), *config, alt_text.clone())
        );

        let placeholder = broken_image_placeholder(&alt_text, model.styles());

        let size = vec2f(config.width.as_f32(), config.height.as_f32());
        let mut image = Image::new(asset_source, CacheOption::BySize)
            .contain()
            .first_frame_preview()
            .on_load_failure(placeholder);

        let constraint = SizeConstraint::new(vec2f(0., 0.), size);
        image.layout(constraint, ctx, app);

        self.image_element = Some(Box::new(image));
    }

    fn paint(
        &mut self,
        model: &RenderState,
        ctx: &mut RenderContext,
        app: &warpui_core::AppContext,
    ) {
        let content = model.content();
        let positioned_image = extract_block!(
            self.viewport_item,
            content,
            (block, BlockItem::Image { config, .. }) => block.image(config)
        );

        let selected = model.offset_in_active_selection(positioned_image.start_char_offset);
        let draw_cursor = model.is_selection_head(positioned_image.start_char_offset);

        let content_position = positioned_image.content_origin();
        let screen_position = ctx.content_to_screen(content_position);
        let size = vec2f(
            positioned_image.item.width.as_f32(),
            positioned_image.item.height.as_f32(),
        );

        if let Some(ref mut image_element) = self.image_element {
            image_element.paint(screen_position, ctx.paint, app);
        }

        if selected {
            let rect_bounds = warpui_core::geometry::rect::RectF::new(screen_position, size);
            ctx.paint
                .scene
                .draw_rect_with_hit_recording(rect_bounds)
                .with_background(model.styles().selection_fill);
        }

        if draw_cursor {
            let end_of_line_position = content_position + vec2f(size.x(), 0.);
            ctx.draw_and_save_cursor(
                CursorDisplayType::Bar,
                end_of_line_position,
                vec2f(model.styles().cursor_width, size.y()),
                CursorData::default(),
                model.styles(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::broken_image_label;

    #[test]
    fn label_uses_alt_text_when_present() {
        assert_eq!(broken_image_label("A red bicycle"), "A red bicycle");
    }

    #[test]
    fn label_trims_surrounding_whitespace() {
        assert_eq!(broken_image_label("  spaced alt  "), "spaced alt");
    }

    #[test]
    fn label_falls_back_when_alt_text_empty() {
        assert_eq!(broken_image_label(""), "Image failed to load");
    }

    #[test]
    fn label_falls_back_when_alt_text_only_whitespace() {
        assert_eq!(broken_image_label("   \t"), "Image failed to load");
    }
}
