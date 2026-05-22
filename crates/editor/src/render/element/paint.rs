//! Utilities for painting rich text.

use std::ops::Range;

use string_offset::CharOffset;
use vim::vim::VimMode;
use warp_core::ui::appearance::DEFAULT_UI_FONT_SIZE;
use warpui::PaintContext;
use warpui::elements::{CornerRadius, Point, Radius};
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::text_layout::{Line, PaintStyleOverride, TextFrame};

use crate::editor::TextDecoration;
use crate::render::layout::line_height;
use crate::render::model::saved_positions::SavedPositions;
use crate::render::model::{
    Decoration, Paragraph, ParagraphStyles, Positioned, RenderState, RichTextStyles,
};

const DEFAULT_BLOCK_CURSOR_WIDTH: f32 = 8.;

/// Cursor display types for vim mode support.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum CursorDisplayType {
    #[default]
    Bar,
    Block,
    Underline,
}

/// Cursor data struct for rendering block and underline cursors in vim mode
#[derive(Default)]
pub struct CursorData {
    pub block_width: Option<f32>,
    pub font_size: Option<f32>,
}

impl CursorData {
    /// Unzip cursor data, defaulting to constants if the values are `None`
    fn unzip(&self) -> (f32, f32) {
        let font_size = self.font_size.unwrap_or(DEFAULT_UI_FONT_SIZE);

        let fallback_block_cursor_width =
            DEFAULT_BLOCK_CURSOR_WIDTH * (font_size / DEFAULT_UI_FONT_SIZE);
        let block_width = self.block_width.unwrap_or(fallback_block_cursor_width);

        (font_size, block_width)
    }
}

/// Bundle of context needed to render a viewported rich text item.
pub struct RenderContext<'a, 'b> {
    /// The on-screen viewport bounds.
    pub bounds: RectF,
    /// The starting y-offset of content in the current viewport.
    pub content_offset: Vector2F,
    /// Whether or not the rich text is focused.
    pub focused: bool,
    /// Whether or not the rich text is editable.
    pub editable: bool,
    /// Cursor blink state - this is true if cursor blink is disabled _or_ blinking cursors are
    /// visible.
    blink_on: bool,
    /// The cursor type to display
    pub cursor_type: CursorDisplayType,
    text_decorations: TextDecoration<'a>,
    /// Underlying paint context for rendering.
    pub paint: &'a mut PaintContext<'b>,
    saved_positions: &'a SavedPositions,
    pub viewport_size: Vector2F,
    /// Current VimMode of the rich text element, if there is one.
    pub vim_mode: Option<VimMode>,
    /// Vim visual tails - stored cursor positions when entering vim visual mode
    pub vim_visual_tails: &'a [CharOffset],
}

impl<'a, 'b> RenderContext<'a, 'b> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bounds: RectF,
        focused: bool,
        editable: bool,
        blink_on: bool,
        cursor_type: CursorDisplayType,
        text_decorations: TextDecoration<'a>,
        scroll_top: f32,
        viewport_size: Vector2F,
        model: &'a RenderState,
        paint: &'a mut PaintContext<'b>,
        vim_mode: Option<VimMode>,
        vim_visual_tails: &'a [CharOffset],
    ) -> Self {
        Self {
            bounds,
            // Note that we use the scroll_top directly passed in from the element here because it could
            // be updated with the last layout (if the vertical display option is set to grow to max constraint).
            // This problem does not exist for vertical scrolls.
            content_offset: vec2f(model.viewport().scroll_left().as_f32(), scroll_top),
            focused,
            editable,
            blink_on,
            cursor_type,
            text_decorations,
            paint,
            saved_positions: model.saved_positions(),
            viewport_size,
            vim_mode,
            vim_visual_tails,
        }
    }

    pub fn paint_style_override(&self, range: Range<CharOffset>) -> PaintStyleOverride {
        self.text_decorations.to_paint_style_override(range)
    }

    /// Returns the visible bound of the viewport.
    pub fn visible_bound(&self) -> RectF {
        self.bounds
    }

    /// Convert a position within the buffer to the corresponding on-screen location.
    ///
    /// Note that the returned location may be out of viewport. The caller is responsible for
    /// bounds checking.
    pub fn content_to_screen(&self, position: Vector2F) -> Vector2F {
        let viewport_relative = position - self.content_offset;
        self.bounds.origin() + viewport_relative
    }

    /// Adjust a buffer-relative rectangle to its on-screen origin.
    ///
    /// Note that the returned rectangle may be out of viewport. The caller is responsible for
    /// bounds checking.
    pub fn content_rect_to_screen(&self, rect: RectF) -> RectF {
        RectF::new(self.content_to_screen(rect.origin()), rect.size())
    }

    /// Paint a paragraph of text, along with any decorations indicated by the rendering model.
    pub fn draw_paragraph(
        &mut self,
        paragraph: &Positioned<Paragraph>,
        style: &ParagraphStyles,
        state: &RenderState,
    ) {
        let paint_style_override =
            self.paint_style_override(paragraph.start_char_offset..paragraph.end_char_offset());
        self.draw_text(
            paragraph.content_origin(),
            paint_style_override,
            paragraph.item.frame(),
            style,
        );

        paragraph.draw_selection(state, self);
        self.draw_text_decorations(paragraph, state.decorations().text(), state);
    }

    /// Helper to draw text decorations over a paragraph. The decorations must be sorted by **end**
    /// offset.
    fn draw_text_decorations(
        &mut self,
        paragraph: &Positioned<Paragraph>,
        decorations: &[Decoration],
        state: &RenderState,
    ) {
        let paragraph_end = paragraph.end_char_offset();

        // Because decorations are sorted by their end offset, we binary search to find the last
        // decoration that overlaps with the paragraph.
        let last_decoration =
            decorations.partition_point(|decoration| decoration.end <= paragraph_end);

        for decoration in decorations[..last_decoration].iter().rev() {
            if decoration.end <= paragraph.start_char_offset {
                // Because we're looping backwards, this and all earlier decorations cannot apply
                // to the paragraph.
                break;
            }
            if let Some(highlight) = decoration.background {
                paragraph.draw_highlight(
                    decoration.start,
                    decoration.end,
                    highlight.into(),
                    self,
                    state.max_line(),
                );
            }
            if let Some(color) = decoration.dashed_underline {
                paragraph.draw_dashed_underline(decoration.start, decoration.end, color, self);
            }
        }
    }

    /// Paint the portion of a [`TextFrame`] that is within the viewport.
    ///
    /// The `content_position` is the position of the frame relative to the buffer's origin
    /// (that is, the same as [`Positioned::content_origin`]).
    pub fn draw_text(
        &mut self,
        content_position: Vector2F,
        paint_style_override: PaintStyleOverride,
        frame: &TextFrame,
        style: &ParagraphStyles,
    ) {
        // The origin of the item on the screen, which all lines are painted relative to.
        let mut render_origin = self.content_to_screen(content_position);
        for (index, line) in frame.lines().iter().enumerate() {
            // Order matters for these tests. First, we check if we've
            // finished rendering all in-viewport lines, which is true
            // once the current line's starting offset is past the max
            // render height. Then, we figure out where the current line
            // starts and ends. If the end of the line is outside the viewport,
            // skip past it. It's important that we still update the origin, otherwise
            // we skip the whole paragraph.

            if render_origin.y() > self.bounds.max_y() {
                // This line is completely below the viewport, and so
                // all subsequent lines will be as well.
                log::trace!("Lines {index}+ are below viewport, skipping");
                break;
            }

            let line_origin = render_origin;

            // Add the line height to the render origin so that we know where the next line begins.
            // Conveniently for viewporting, this also tells us where the current line ends.
            render_origin.set_y(render_origin.y() + line_height(line));
            if render_origin.y() < self.bounds.min_y() {
                // This line is completely above the viewport. Skip past
                // it until we get to an in-viewport line.
                log::trace!("Line {index} is above the viewport, skipping");
                continue;
            }

            log::trace!("Painting line {index}");
            line.paint(
                RectF::from_points(line_origin, self.bounds.lower_right()),
                &paint_style_override,
                style.text_color,
                self.paint.font_cache,
                self.paint.scene,
            );
        }
    }

    /// Paint a single [`Line`] of text, if it's within the viewport.
    ///
    /// The `content_position` is the position of the line relative to the buffer's origin (that
    /// is, the same as [`Positioned::content_origin`]).
    pub fn draw_line(&mut self, content_position: Vector2F, line: &Line, style: &ParagraphStyles) {
        // This is a simplified version of the inner loop of [`Self::draw_text`], since there's
        // only a single line to consider.
        let render_origin = self.content_to_screen(content_position);
        if render_origin.y() > self.bounds.max_y() {
            log::trace!("Line is below viewport, skipping");
            return;
        }
        if render_origin.y() + line_height(line) < self.bounds.min_y() {
            log::trace!("Line is above viewport, skipping");
            return;
        }

        log::trace!("Painting single line");
        line.paint(
            RectF::from_points(render_origin, self.bounds.lower_right()),
            &Default::default(),
            style.text_color,
            self.paint.font_cache,
            self.paint.scene,
        );
    }

    /// Draws a cursor at the content position and save it into the position cache.
    ///
    /// If cursors are not visible, they will be saved but not drawn.
    pub fn draw_and_save_cursor(
        &mut self,
        cursor_display_type: CursorDisplayType,
        content_position: Vector2F,
        size: Vector2F,
        cursor_data: CursorData,
        styles: &RichTextStyles,
    ) {
        let (font_size, block_width) = cursor_data.unzip();
        let height = size.y();

        let cursor_size = match cursor_display_type {
            CursorDisplayType::Bar => size,
            CursorDisplayType::Block => vec2f(block_width, height),
            CursorDisplayType::Underline => vec2f(block_width, height - font_size),
        };

        let cursor_origin = match cursor_display_type {
            CursorDisplayType::Bar => {
                // Center the cursor on its origin. This reduces the amount of overlap with glyphs,
                // especially for wider cursors.
                self.content_to_screen(content_position) - vec2f(size.x() / 2., 0.)
            }
            CursorDisplayType::Block => self.content_to_screen(content_position),
            CursorDisplayType::Underline => {
                self.content_to_screen(content_position) + vec2f(0., font_size)
            }
        };

        let bounds = RectF::new(cursor_origin, cursor_size);

        let cursor_corner_radius = match cursor_display_type {
            CursorDisplayType::Block | CursorDisplayType::Underline => Radius::Pixels(0.),
            _ => Radius::Percentage(50.),
        };

        if self.cursors_visible() {
            self.paint
                .scene
                .draw_rect_with_hit_recording(bounds)
                .with_background(styles.cursor_fill)
                .with_corner_radius(CornerRadius::with_all(cursor_corner_radius));
        }

        // The cursor should only exist at one location, so we can save it here.
        self.paint
            .position_cache
            .cache_position_indefinitely(self.saved_positions.cursor_id(), bounds);
    }

    /// Returns `true` if cursors should be visible.
    fn cursors_visible(&self) -> bool {
        self.editable && self.focused && self.blink_on
    }

    /// Tests whether or not any portion of the given rectangle is visible at the current z-index.
    pub fn is_visible(&self, rect: RectF) -> bool {
        let origin = Point::from_vec2f(rect.origin(), self.paint.scene.z_index());
        self.paint.scene.visible_rect(origin, rect.size()).is_some()
    }
}
