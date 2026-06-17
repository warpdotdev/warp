use warpui_core::elements::{CacheOption, Image};
use warpui_core::geometry::vector::vec2f;
use warpui_core::{Element, SizeConstraint};

use super::{RenderContext, RenderableBlock};
use crate::extract_block;
use crate::render::element::paint::{CursorData, CursorDisplayType};
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState};

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
        let (asset_source, config) = extract_block!(
            self.viewport_item,
            content,
            (_block, BlockItem::Image { asset_source, config, .. }) => (asset_source.clone(), *config)
        );

        let size = vec2f(config.width.as_f32(), config.height.as_f32());
        let mut image = Image::new(asset_source, CacheOption::BySize)
            .contain()
            .first_frame_preview();

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
