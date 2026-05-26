use std::sync::Arc;

use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::text_layout::Line;
use warpui::{AppContext, LayoutContext};

use super::{CursorData, RenderContext};
use crate::content::text::BufferBlockStyle;
use crate::render::element::paint::CursorDisplayType;
use crate::render::layout::{TextLayout, line_height};
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState};

/// Ghost/placeholder text that's shown in an empty block to provide context.
pub struct BlockPlaceholder {
    show_always: bool,
    state: State,
}

enum State {
    PendingLayout,
    LaidOut {
        /// Whether or not the block contains the text cursor.
        contains_cursor: bool,
        /// The placeholder text, laid out as a single clipped line.
        line: Arc<Line>,
        block_style: BufferBlockStyle,
    },
    NotShown,
}

impl BlockPlaceholder {
    pub fn new(show_always: bool) -> Self {
        Self {
            show_always,
            state: State::PendingLayout,
        }
    }

    /// Lay out the placeholder, if necessary.
    pub fn layout<'a, F>(
        &mut self,
        item: &ViewportItem,
        model: &RenderState,
        ctx: &mut LayoutContext,
        app: &AppContext,
        options: F,
    ) where
        F: FnOnce(&BlockItem) -> Options<'a>,
    {
        debug_assert!(
            matches!(self.state, State::PendingLayout),
            "Placeholder laid out multiple times"
        );
        self.state = State::NotShown;

        let content = model.content();
        let block_offset = item.block_offset();
        let block = match content.block_at_offset(block_offset) {
            Some(block) if block.item.is_empty() => block,
            _ => {
                // Placeholders are _never_ shown if the block has user content.
                return;
            }
        };

        let contains_cursor = model.selections().iter().any(|selection| {
            selection
                .single_cursor()
                .is_some_and(|cursor| block.contains_content(cursor))
        });

        if !model.styles().show_placeholder_text_on_empty_block
            || (!self.show_always && !contains_cursor)
        {
            // If the cursor isn't in this block, don't lay out the placeholder.
            return;
        }

        let layout = TextLayout::from_layout_context(ctx, app, model);
        let options = options(block.item);
        self.state = State::LaidOut {
            line: layout.layout_placeholder(options.text, &options.block_style, &item.spacing),
            block_style: options.block_style,
            contains_cursor,
        };
    }

    /// Paint this placeholder at the original block's origin. Returns `false` if there is no
    /// placeholder, and the regular content should be shown.
    pub fn paint(
        &self,
        content_origin: Vector2F,
        model: &RenderState,
        ctx: &mut RenderContext,
    ) -> bool {
        match &self.state {
            State::NotShown => false,
            State::PendingLayout => {
                log::warn!("Tried to paint placeholder before layout");
                false
            }
            State::LaidOut {
                contains_cursor,
                line,
                block_style,
            } => {
                if ctx.editable && (self.show_always || ctx.focused) {
                    let paragraph_styles = model.styles().paragraph_styles(block_style);
                    ctx.draw_line(content_origin, line, &paragraph_styles);
                }

                // If this placeholder contains the cursor, we must draw it, regardless of
                // focus (since the cursor positions other UI elements).
                if *contains_cursor {
                    let cursor_size = vec2f(model.styles().cursor_width, line_height(line));
                    ctx.draw_and_save_cursor(
                        CursorDisplayType::Bar,
                        content_origin,
                        cursor_size,
                        CursorData::default(),
                        model.styles(),
                    );
                }
                true
            }
        }
    }
}

/// Options for displaying a placeholder.
pub struct Options<'a> {
    /// The placeholder text.
    pub text: &'a str,
    /// Block-level styling.
    pub block_style: BufferBlockStyle,
}
