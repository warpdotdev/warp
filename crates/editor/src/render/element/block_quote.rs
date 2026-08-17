use warpui_core::geometry::rect::RectF;
use warpui_core::geometry::vector::vec2f;

use super::RenderableBlock;
use super::paint::RenderContext;
use crate::extract_block;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState};

const QUOTE_RAIL_WIDTH: f32 = 3.;
const QUOTE_RAIL_OFFSET: f32 = 12.;

pub struct RenderableBlockQuote {
    viewport_item: ViewportItem,
}

impl RenderableBlockQuote {
    pub fn new(viewport_item: ViewportItem) -> Self {
        Self { viewport_item }
    }
}

impl RenderableBlock for RenderableBlockQuote {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(
        &mut self,
        _model: &RenderState,
        _ctx: &mut warpui_core::LayoutContext,
        _app: &warpui_core::AppContext,
    ) {
    }

    fn paint(
        &mut self,
        model: &RenderState,
        ctx: &mut RenderContext,
        _app: &warpui_core::AppContext,
    ) {
        let content = model.content();
        let quote = extract_block!(self.viewport_item, content, (block, BlockItem::BlockQuote { paragraph }) => block.block_quote(paragraph));
        let visible_bounds = self.viewport_item.visible_bounds(ctx);
        let text_origin = ctx.content_to_screen(quote.content_origin());
        let rail = RectF::new(
            vec2f(
                text_origin.x() - QUOTE_RAIL_OFFSET,
                visible_bounds.origin_y(),
            ),
            vec2f(QUOTE_RAIL_WIDTH, visible_bounds.height()),
        );

        ctx.paint
            .scene
            .draw_rect_without_hit_recording(rail)
            .with_background(model.styles().horizontal_rule_style.color);
        ctx.draw_paragraph(&quote, &model.styles().base_text, model);
    }
}
