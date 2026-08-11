//! Utilities for traversing laid-out blocks along with their positioning
//! information.

use std::sync::Arc;

use string_offset::CharOffset;
use sum_tree::{Cursor, Dimension};
use warpui_core::geometry::vector::Vector2F;
use warpui_core::text_layout::Line;
use warpui_core::units::{IntoPixels, Pixels};

use super::{
    BlockItem, BlockSpacing, HorizontalRuleConfig, ImageBlockConfig, LaidOutEmbeddedItem,
    LaidOutTable, LayoutSummary, LineCount, Paragraph, ParagraphBlock, RenderContext, bounds,
};
use crate::render::layout::line_height;

/// Wrapper to track an item's position, both in the buffer and on the screen.
#[derive(Debug)]
pub struct Positioned<'a, T> {
    /// The starting character offset of this item, relative to the start of
    /// the buffer.
    pub start_char_offset: CharOffset,
    /// The starting line number of this item.
    pub start_line: LineCount,
    /// The starting y-offset of this item, relative to the origin of the laid-out
    /// content.
    // TODO: There are at least 4 valid origins for pixel coordinates (content start,
    //    viewport start, paint origin, and block start). If that starts getting
    //    confusing, we might want to introduce wrappers similar to `DisplayPoint`
    //    and `SoftWrapPoint` in the input editor or `WithinBlock` and `WithinModel`
    //    in the terminal.
    pub start_y_offset: Pixels,
    pub style: BlockSpacing,
    pub item: &'a T,
}

pub trait PositionedCursor<'a> {
    /// The block at the current cursor position, along with its position.
    fn positioned_item(&self) -> Option<Positioned<'a, BlockItem>>;
}

impl<'a, S: Dimension<'a, LayoutSummary>> PositionedCursor<'a>
    for Cursor<'a, BlockItem, S, LayoutSummary>
{
    fn positioned_item(&self) -> Option<Positioned<'a, BlockItem>> {
        let item = self.item()?;
        let summary = self.start();

        Some(Positioned {
            start_char_offset: summary.content_length,
            start_y_offset: (summary.height as f32).into_pixels(),
            start_line: summary.lines,
            style: item.spacing(),
            item,
        })
    }
}

impl<T> Positioned<'_, T> {
    /// The origin of this item's content, relative to the start of the buffer.
    pub fn content_origin(&self) -> Vector2F {
        bounds::content_origin(self.start_y_offset, &self.style)
    }

    /// The visible origin of this item, relative to the start of the buffer.
    pub fn visible_origin(&self) -> Vector2F {
        bounds::visible_origin(self.start_y_offset, &self.style)
    }

    /// The origin of this item, relative to the start of the buffer, with no padding
    /// or margin.
    pub fn reserved_origin(&self) -> Vector2F {
        bounds::reserved_origin(self.start_y_offset)
    }

    /// The origin of this item in rendering coordinates.
    pub fn render_origin(&self, ctx: &RenderContext) -> Vector2F {
        ctx.content_to_screen(self.reserved_origin())
    }
}

/// Helpers specific to positioned [`Line`]s.
impl Positioned<'_, Line> {
    pub fn end_y_offset(&self) -> Pixels {
        self.start_y_offset + line_height(self.item).into_pixels() + self.style.top_offset()
    }
}

/// Helpers specific to a positioned [`BlockItem`].
impl<'a> Positioned<'a, BlockItem> {
    /// The ending character offset of this item (exclusive).
    pub fn end_char_offset(&self) -> CharOffset {
        self.start_char_offset + self.item.content_length()
    }

    /// Check if this block item contains a content offset.
    pub fn contains_content(&self, offset: CharOffset) -> bool {
        self.start_char_offset <= offset && self.end_char_offset() > offset
    }

    /// The ending line number of this item (exclusive).
    pub fn end_line(&self) -> LineCount {
        self.start_line + self.item.lines()
    }

    /// Position this item's code block.
    pub fn code_block(&self, block: &'a ParagraphBlock) -> Positioned<'a, ParagraphBlock> {
        debug_assert!(
            matches!(self.item, BlockItem::RunnableCodeBlock { .. }),
            "Must be a runnable code block"
        );
        self.position(block)
    }

    pub fn temporary_block(&self, block: &'a ParagraphBlock) -> Positioned<'a, ParagraphBlock> {
        debug_assert!(
            matches!(self.item, BlockItem::TemporaryBlock { .. }),
            "Must be a temporary block"
        );
        self.position(block)
    }

    pub fn embedded(
        &self,
        embedded_item: &'a Arc<dyn LaidOutEmbeddedItem>,
    ) -> Positioned<'a, Arc<dyn LaidOutEmbeddedItem>> {
        debug_assert!(
            matches!(self.item, BlockItem::Embedded(_)),
            "Must be an embedded object"
        );
        self.position(embedded_item)
    }

    pub fn task_list(&self, paragraph: &'a Paragraph) -> Positioned<'a, Paragraph> {
        debug_assert!(
            matches!(self.item, BlockItem::TaskList { .. }),
            "Must be a task list block"
        );
        self.position(paragraph)
    }

    pub fn unordered_list(&self, paragraph: &'a Paragraph) -> Positioned<'a, Paragraph> {
        debug_assert!(
            matches!(self.item, BlockItem::UnorderedList { .. }),
            "Must be an unordered list block"
        );
        self.position(paragraph)
    }

    /// Position the content paragraph for an ordered list item.
    pub fn ordered_list(&self, paragraph: &'a Paragraph) -> Positioned<'a, Paragraph> {
        debug_assert!(
            matches!(self.item, BlockItem::OrderedList { .. }),
            "Must be an ordered list block"
        );
        self.position(paragraph)
    }

    pub fn header(&self, paragraph: &'a Paragraph) -> Positioned<'a, Paragraph> {
        debug_assert!(
            matches!(self.item, BlockItem::Header { .. }),
            "Must be a header block"
        );
        self.position_centered(paragraph, paragraph.height())
    }

    /// Position this item's paragraph.
    pub fn paragraph(&self, paragraph: &'a Paragraph) -> Positioned<'a, Paragraph> {
        debug_assert!(
            matches!(self.item, BlockItem::Paragraph(_)),
            "Must be a paragraph"
        );
        // Short paragraphs may have extra padding to meet the minimum paragraph height.
        self.position_centered(paragraph, paragraph.height())
    }

    pub fn text_block(
        &self,
        paragraph_block: &'a ParagraphBlock,
    ) -> Positioned<'a, ParagraphBlock> {
        debug_assert!(
            matches!(self.item, BlockItem::TextBlock { .. }),
            "Must be a text block"
        );
        self.position(paragraph_block)
    }

    /// Position the trailing newline cursor.
    pub fn trailing_newline(&self, cursor: &'a super::Cursor) -> Positioned<'a, super::Cursor> {
        // Match the spacing behavior of single-line paragraphs.
        self.position_centered(cursor, cursor.height)
    }

    pub fn horizontal_rule(
        &self,
        horizontal_rule: &'a HorizontalRuleConfig,
    ) -> Positioned<'a, HorizontalRuleConfig> {
        self.position_centered(horizontal_rule, horizontal_rule.line_height)
    }

    pub fn image(&self, image_config: &'a ImageBlockConfig) -> Positioned<'a, ImageBlockConfig> {
        self.position_centered(image_config, image_config.height)
    }

    pub fn table(&self, laid_out_table: &'a LaidOutTable) -> Positioned<'a, LaidOutTable> {
        debug_assert!(
            matches!(self.item, BlockItem::Table(_)),
            "Must be a table block"
        );
        self.position_centered(laid_out_table, laid_out_table.height())
    }

    /// Helper to position-wrap an item's contents when pattern-matching.
    ///
    /// ```ignore
    /// match positioned.item {
    ///   BlockItem::Paragraph(paragraph) => positioned.position(paragraph),
    /// }
    /// ```
    fn position<T>(&self, content: &'a T) -> Positioned<'a, T> {
        Positioned {
            start_char_offset: self.start_char_offset,
            start_y_offset: self.start_y_offset,
            start_line: self.start_line,
            style: self.style,
            item: content,
        }
    }

    /// Like `positioned`, but centers the content based on the block's expected content height.
    /// Use this for blocks with a minimum height.
    fn position_centered<T>(
        &self,
        content: &'a T,
        actual_content_height: Pixels,
    ) -> Positioned<'a, T> {
        let mut positioned = self.position(content);
        let gap = self.item.content_height() - actual_content_height;
        positioned.start_y_offset += gap / 2.0.into_pixels();
        positioned
    }
}
