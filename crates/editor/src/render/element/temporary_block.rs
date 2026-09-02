use warp_core::ui::theme::Fill;

use super::RenderableBlock;
use super::paint::RenderContext;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, Decoration, RenderState};

pub struct RenderableTemporaryBlock {
    viewport_item: ViewportItem,
    decoration: Option<Fill>,
    text_decoration: Vec<Decoration>,
}

impl RenderableTemporaryBlock {
    pub fn new(
        viewport_item: ViewportItem,
        decoration: Option<Fill>,
        text_decoration: Vec<Decoration>,
    ) -> Self {
        Self {
            viewport_item,
            decoration,
            text_decoration,
        }
    }
}

impl RenderableBlock for RenderableTemporaryBlock {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn overlay_decoration(&self) -> Option<Fill> {
        self.decoration
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
        let paragraph_block = match (&block, block.item) {
            (
                block,
                BlockItem::TemporaryBlock {
                    paragraph_block, ..
                },
            ) => block.temporary_block(paragraph_block),
            other => {
                log::warn!(
                    "Unexpected block {other:?} at {}",
                    self.viewport_item.block_offset
                );
                return;
            }
        };

        let start = paragraph_block.start_char_offset;
        let paragraph_styles = &model.styles().base_text;
        let mut paragraphs = paragraph_block
            .paragraphs_in(self.viewport_item.paragraph_range())
            .peekable();
        let mut decoration_index = paragraphs.peek().map_or(0, |paragraph| {
            self.text_decoration
                .partition_point(|decoration| decoration.end + start <= paragraph.start_char_offset)
        });
        for paragraph in paragraphs {
            // We could draw text directly since temporary paragraph should have its own decoration and selection state.
            ctx.draw_text(
                paragraph.content_origin(),
                Default::default(),
                paragraph.item.frame(),
                paragraph_styles,
            );

            let paragraph_end = paragraph.end_char_offset();
            for (idx, decoration) in self.text_decoration[decoration_index..].iter().enumerate() {
                if decoration.start + start >= paragraph_end {
                    decoration_index += idx;
                    break;
                }
                if let Some(highlight) = decoration.background {
                    paragraph.draw_highlight(
                        decoration.start + start,
                        decoration.end + start,
                        highlight.into(),
                        ctx,
                        model.max_line(),
                    );
                }
            }
        }
    }

    fn is_temporary(&self) -> bool {
        true
    }
}
