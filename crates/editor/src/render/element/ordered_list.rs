use std::sync::Arc;

use warpui_core::elements::ListIndentLevel;
use warpui_core::geometry::vector::vec2f;
use warpui_core::text_layout::TextFrame;

use super::RenderableBlock;
use super::paint::RenderContext;
use super::placeholder::{self, BlockPlaceholder};
use crate::content::text::BufferBlockStyle;
use crate::extract_block;
use crate::render::layout::TextLayout;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState};

pub struct RenderableOrderedListItem {
    viewport_item: ViewportItem,
    number: String,
    rendered_number: Option<Arc<TextFrame>>,
    placeholder: BlockPlaceholder,
}

impl RenderableOrderedListItem {
    pub fn new(indent_level: ListIndentLevel, viewport_item: ViewportItem, number: usize) -> Self {
        let number = indent_level.list_number_string(number);
        Self {
            viewport_item,
            number,
            rendered_number: None,
            placeholder: BlockPlaceholder::new(true),
        }
    }
}

impl RenderableBlock for RenderableOrderedListItem {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(
        &mut self,
        model: &RenderState,
        ctx: &mut warpui_core::LayoutContext,
        app: &warpui_core::AppContext,
    ) {
        let text_layout = TextLayout::from_layout_context(ctx, app, model);
        let block_style = BufferBlockStyle::OrderedList {
            indent_level: ListIndentLevel::One,
            number: None,
        };

        let paragraph_styles = &text_layout.paragraph_styles(&block_style);
        let number_text = format!("{}.", self.number);
        let style_runs = &[(
            0..number_text.chars().count(),
            text_layout.style_and_font(paragraph_styles, &Default::default()),
        )];
        self.rendered_number = Some(text_layout.layout_text(
            &number_text,
            paragraph_styles,
            &self.viewport_item.spacing,
            style_runs,
        ));

        self.placeholder
            .layout(&self.viewport_item, model, ctx, app, |_| {
                placeholder::Options {
                    block_style,
                    text: "List",
                }
            });
    }

    fn paint(
        &mut self,
        model: &RenderState,
        ctx: &mut RenderContext,
        _app: &warpui_core::AppContext,
    ) {
        let content = model.content();
        let paragraph = extract_block!(self.viewport_item, content, (block, BlockItem::OrderedList{ paragraph: inner, ..}) => block.ordered_list(inner));

        let text_styling = &model.styles().base_text;

        let number = self
            .rendered_number
            .as_ref()
            .expect("Number should be set during layout");
        // Position the numeric label in the margin to the left of the item content.
        let space_width = ctx
            .paint
            .font_cache
            .em_width(text_styling.font_family, text_styling.font_size)
            / 2.;
        let number_origin =
            paragraph.content_origin() - vec2f(number.max_width() + space_width, 0.);
        ctx.draw_text(number_origin, Default::default(), number, text_styling);

        if !self
            .placeholder
            .paint(paragraph.content_origin(), model, ctx)
        {
            ctx.draw_paragraph(&paragraph, text_styling, model);
        }
    }
}
