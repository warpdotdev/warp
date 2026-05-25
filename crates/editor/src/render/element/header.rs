use super::placeholder::{BlockPlaceholder, Options};
use super::{RenderContext, RenderableBlock};
use crate::content::text::{BlockHeaderSize, BufferBlockStyle};
use crate::extract_block;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState};

pub struct RenderableHeader {
    viewport_item: ViewportItem,
    placeholder: BlockPlaceholder,
}

impl RenderableHeader {
    pub fn new(viewport_item: ViewportItem) -> Self {
        Self {
            viewport_item,
            placeholder: BlockPlaceholder::new(true),
        }
    }
}

impl RenderableBlock for RenderableHeader {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(
        &mut self,
        model: &RenderState,
        ctx: &mut warpui::LayoutContext,
        app: &warpui::AppContext,
    ) {
        self.placeholder
            .layout(&self.viewport_item, model, ctx, app, |block| {
                let header_size = match block {
                    BlockItem::Header { header_size, .. } => *header_size,
                    other => {
                        if cfg!(debug_assertions) {
                            panic!("Expected a header, got {other:?}");
                        }
                        BlockHeaderSize::Header6
                    }
                };

                Options {
                    text: header_size.label(),
                    block_style: BufferBlockStyle::Header { header_size },
                }
            });
    }

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, _app: &warpui::AppContext) {
        let content = model.content();
        let (paragraph, header_size) = extract_block!(
            self.viewport_item, content,
            (block, BlockItem::Header { header_size, paragraph }) => (block.header(paragraph), header_size)
        );

        if self
            .placeholder
            .paint(paragraph.content_origin(), model, ctx)
        {
            return;
        }

        let header_style = &model.styles().paragraph_styles(&BufferBlockStyle::Header {
            header_size: *header_size,
        });
        ctx.draw_paragraph(&paragraph, header_style, model);
    }
}
