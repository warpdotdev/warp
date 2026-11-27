use super::RenderableBlock;
use super::paint::RenderContext;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState};

pub struct RenderableTextBlock {
    viewport_item: ViewportItem,
}

impl RenderableTextBlock {
    pub fn new(viewport_item: ViewportItem) -> Self {
        Self { viewport_item }
    }
}

impl RenderableBlock for RenderableTextBlock {
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
        let Some(item) = self.viewport_item.resolved_block(&content) else {
            return;
        };
        let block = self.viewport_item.positioned_block(&item);
        let BlockItem::TextBlock { paragraph_block } = block.item else {
            return;
        };
        let text_block = block.text_block(paragraph_block);

        let paragraph_styles = &model.styles().base_text;
        for paragraph in text_block.paragraphs_in(self.viewport_item.paragraph_range()) {
            ctx.draw_paragraph(&paragraph, paragraph_styles, model);
        }
    }
}
