use super::paragraph::paragraph_placeholder_text;
use super::placeholder::{self, BlockPlaceholder};
use super::{RenderContext, RenderableBlock};
use crate::content::text::BufferBlockStyle;
use crate::extract_block;
use crate::render::element::paint::CursorData;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState};

/// Renderable representation of invisible rich-text items. This is used for the trailing newline
/// marker.
pub struct Empty {
    viewport_item: ViewportItem,
    placeholder: BlockPlaceholder,
}

impl Empty {
    pub fn new(viewport_item: ViewportItem) -> Self {
        Self {
            viewport_item,
            placeholder: BlockPlaceholder::new(false),
        }
    }
}

impl RenderableBlock for Empty {
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
            .layout(&self.viewport_item, model, ctx, app, |_| {
                placeholder::Options {
                    text: paragraph_placeholder_text(model.selections().len() == 1),
                    block_style: BufferBlockStyle::PlainText,
                }
            });
    }

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, _app: &warpui::AppContext) {
        let content = model.content();
        let cursor = extract_block!(self.viewport_item, content, (block, BlockItem::TrailingNewLine(cursor)) => block.trailing_newline(cursor));
        if self.placeholder.paint(cursor.content_origin(), model, ctx) {
            return;
        }

        let selections = model.selections();
        if selections
            .iter()
            .any(|selection| selection.is_cursor() && selection.head >= cursor.start_char_offset)
        {
            let base = &model.styles().base_text;
            let cursor_data = CursorData {
                block_width: None,
                font_size: Some(base.font_size),
            };
            ctx.draw_and_save_cursor(
                ctx.cursor_type,
                cursor.content_origin(),
                cursor.item.size(),
                cursor_data,
                model.styles(),
            );
        }
    }
}
