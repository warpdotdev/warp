use super::RenderableBlock;
use super::paint::RenderContext;
use super::placeholder::{self, BlockPlaceholder};
use crate::content::text::BufferBlockStyle;
use crate::extract_block;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState};

/// The placeholder text to show in empty plain-text blocks.
pub(super) const PARAGRAPH_PLACEHOLDER_TEXT: &str =
    "Type text or Markdown, or '/' to insert content";

pub(super) const PARAGRAPH_PLACEHOLDER_TEXT_WITHOUT_SLASH: &str = "Type text or Markdown";

pub fn paragraph_placeholder_text(slash_menu_enabled: bool) -> &'static str {
    if slash_menu_enabled {
        PARAGRAPH_PLACEHOLDER_TEXT
    } else {
        PARAGRAPH_PLACEHOLDER_TEXT_WITHOUT_SLASH
    }
}

/// [`RenderableBlock`] implementation for `Paragraph` blocks.
pub struct RenderableParagraph {
    viewport_item: ViewportItem,
    placeholder: BlockPlaceholder,
}

impl RenderableParagraph {
    pub fn new(viewport_item: ViewportItem) -> Self {
        Self {
            viewport_item,
            placeholder: BlockPlaceholder::new(false),
        }
    }
}

impl RenderableBlock for RenderableParagraph {
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
        let paragraph = extract_block!(self.viewport_item, content, (block, BlockItem::Paragraph(inner)) => block.paragraph(inner));

        if self
            .placeholder
            .paint(paragraph.content_origin(), model, ctx)
        {
            return;
        }

        let paragraph_styles = &model.styles().base_text;
        ctx.draw_paragraph(&paragraph, paragraph_styles, model);
    }
}
