//! Hit-testing implementation for the rendering model.

use num_traits::SaturatingSub;
use string_offset::CharOffset;
use sum_tree::SeekBias;
use warpui_core::units::{IntoPixels, Pixels};

use super::positioned::{Positioned, PositionedCursor};
use super::{
    BlockItem, Height, HitTestBlockType, LayoutSummary, ParagraphBlock, RenderState, bounds,
};

#[cfg(test)]
#[path = "location_tests.rs"]
mod tests;

/// A location within the editor, as resolved by hit-testing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// An entire block.
    Block {
        /// The starting character offset of the block (inclusive).
        start_offset: CharOffset,
        /// The ending character offset of the block (exclusive).
        end_offset: CharOffset,
        /// Type of the block that was hit.
        block_type: HitTestBlockType,
    },
    /// An exact location within the content space.
    Text {
        /// Offset of the hit character.
        char_offset: CharOffset,
        /// Whether or not we clamped to this location (for example, if the position
        /// was after the end of text on a line, or after the end of all text
        /// in the editor).
        clamped: bool,
        /// Cursor disposition for soft-wrapped text.
        wrap_direction: WrapDirection,
        /// The starting offset of the block that contains the hit character.
        block_start: CharOffset,
        link: Option<String>,
    },
}

impl Location {
    /// The starting [`CharOffset`] of the block that was hit. All hits are within a single block.
    pub fn block_start(&self) -> CharOffset {
        match self {
            Location::Block { start_offset, .. } => *start_offset,
            Location::Text { block_start, .. } => *block_start,
        }
    }
}

/// With soft-wrapping, the end of one line and the start of the next have the
/// same character offset. The `WrapDirection` indicates which one a location is
/// at.
///
/// Suppose we have the line `longword`, soft-wrapped to
/// ```text
/// long
/// word
/// ```
///
/// Visually, after `g` and before `w` are two distinct locations. However, they
/// have the same character offset, 4.
///
/// To represent the first location, we wrap up:
/// ```text
/// long|
/// word
/// ```
///
/// To represent the second, we wrap down:
/// ```text
/// long
/// |word
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum WrapDirection {
    /// Place the cursor at the end of the previous line.
    Up,
    /// Place the cursor at the start of the next line.
    #[default]
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HitTestOptions {
    /// If true, clamp block-level selections to text selections. Currently, this only matters for
    /// a hit in the padding area of a code block. Normally, that's considered a hit on the block
    /// rather than its text.
    pub force_text_selection: bool,
}

impl RenderState {
    /// Performs hit-testing on coordinates relative to the content origin
    /// (that is, non-viewported). The provided `options` configure how the hit-testing behaves.
    pub fn render_coordinates_to_location(
        &self,
        x: Pixels,
        y: Pixels,
        options: &HitTestOptions,
    ) -> Location {
        let content = self.content.borrow();
        let mut block_cursor = content.cursor::<Height, LayoutSummary>();
        block_cursor.seek(&y.into(), SeekBias::Left);

        let Some(block) = block_cursor.positioned_item() else {
            // If we're at the end of the editor, bias towards placing new text
            // on a new line.
            let char_offset = self.max_offset();
            log::debug!("Clamped to end: {char_offset}");
            return Location::Text {
                char_offset,
                clamped: true,
                wrap_direction: WrapDirection::Down,
                block_start: char_offset,
                link: None,
            };
        };

        block.coordinates_to_location(x, y, options)
    }

    /// Performs hit-testing on coordinates relative to the viewport origin.
    pub fn viewport_coordinates_to_location(
        &self,
        x: Pixels,
        y: Pixels,
        options: &HitTestOptions,
    ) -> Location {
        self.render_coordinates_to_location(
            (x + self.viewport.scroll_left()).max(Pixels::zero()),
            (y + self.viewport.scroll_top()).max(Pixels::zero()),
            options,
        )
    }
}

impl<'a> Positioned<'a, BlockItem> {
    /// Resolve coordinates to a location, assuming they're within this block.
    /// The coordinates are all relative to the content origin.
    fn coordinates_to_location(&self, x: Pixels, y: Pixels, options: &HitTestOptions) -> Location {
        match self.item {
            BlockItem::Paragraph(paragraph) => self
                .paragraph(paragraph)
                .coordinate_to_location(self.unpad_x(x), y),
            BlockItem::TextBlock { paragraph_block } => {
                self.location_in_paragraph_block(x, y, self.text_block(paragraph_block))
            }
            BlockItem::TaskList { paragraph, .. } => self
                .task_list(paragraph)
                .coordinate_to_location(self.unpad_x(x), y),
            BlockItem::UnorderedList { paragraph, .. } => self
                .unordered_list(paragraph)
                .coordinate_to_location(self.unpad_x(x), y),
            BlockItem::OrderedList { paragraph, .. } => self
                .ordered_list(paragraph)
                .coordinate_to_location(self.unpad_x(x), y),
            BlockItem::RunnableCodeBlock {
                paragraph_block, ..
            } => {
                // To make text selection more ergonomic, any point on a line with text is
                // considered part of the block's text area, including padding. Points within the
                // padding above or below a code block's text are considered part of the block
                // itself (unless `options.force_text_selection` is true), which allows
                // clicking a block to select it.
                let text_origin = bounds::content_origin(self.start_y_offset, &self.style);
                let text_height_range =
                    text_origin.y()..=text_origin.y() + paragraph_block.height().as_f32();

                if options.force_text_selection || text_height_range.contains(&y.as_f32()) {
                    // Note: we don't unpad `x` here because it's handled by `location_in_paragraph_block`.
                    self.location_in_paragraph_block(x, y, self.code_block(paragraph_block))
                } else {
                    Location::Block {
                        start_offset: self.start_char_offset,
                        end_offset: self.end_char_offset(),
                        block_type: HitTestBlockType::Code,
                    }
                }
            }
            BlockItem::MermaidDiagram { .. } => {
                let _ = options;
                Location::Block {
                    start_offset: self.start_char_offset,
                    end_offset: self.end_char_offset(),
                    block_type: HitTestBlockType::MermaidDiagram,
                }
            }
            BlockItem::Header { paragraph, .. } => self
                .header(paragraph)
                .coordinate_to_location(self.unpad_x(x), y),
            BlockItem::Embedded(_) => Location::Block {
                start_offset: self.start_char_offset,
                end_offset: self.end_char_offset(),
                block_type: HitTestBlockType::Embedding,
            },
            BlockItem::Table(laid_out_table) => {
                let relative_x = (x.as_f32() - self.content_origin().x()).max(0.0);
                let relative_y = (y.as_f32() - self.content_origin().y()).max(0.0);
                let cell_offset = laid_out_table.coordinate_to_offset(
                    relative_x + laid_out_table.scroll_left().as_f32(),
                    relative_y,
                );
                let char_offset = self.start_char_offset + cell_offset;
                let link = laid_out_table.link_at_offset(cell_offset);
                Location::Text {
                    char_offset,
                    clamped: false,
                    wrap_direction: WrapDirection::Down,
                    block_start: self.start_char_offset,
                    link,
                }
            }
            BlockItem::HorizontalRule { .. }
            | BlockItem::Image { .. }
            | BlockItem::TrailingNewLine(_)
            | BlockItem::TemporaryBlock { .. }
            | BlockItem::Hidden { .. } => Location::Text {
                char_offset: self.start_char_offset,
                clamped: true,
                wrap_direction: WrapDirection::Down,
                block_start: self.start_char_offset,
                link: None,
            },
        }
    }

    fn location_in_paragraph_block(
        &self,
        x: Pixels,
        y: Pixels,
        paragraph_block: Positioned<'a, ParagraphBlock>,
    ) -> Location {
        for paragraph in paragraph_block.paragraphs() {
            if paragraph.end_y_offset() > y {
                let mut location = paragraph.coordinate_to_location(self.unpad_x(x), y);
                // Adjust the paragraph-relative start offset to be the start of this block.
                if let Location::Text { block_start, .. } = &mut location {
                    *block_start = self.start_char_offset;
                }
                return location;
            }
        }

        Location::Text {
            char_offset: self.end_char_offset().saturating_sub(&CharOffset::from(1)),
            clamped: true,
            wrap_direction: WrapDirection::Up,
            block_start: self.start_char_offset,
            link: None,
        }
    }

    /// Remove horizontal padding from an x-coordinate in order to hit-test within a padded
    /// paragraph.
    fn unpad_x(&self, x: Pixels) -> Pixels {
        (x - self.content_origin().x().into_pixels()).max(Pixels::zero())
    }
}
