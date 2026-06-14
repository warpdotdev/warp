use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::default::Default;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Once};

use itertools::Itertools;
use markdown_parser::{Action, FormattedText, FormattedTextFragment, FormattedTextLine, Hyperlink};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use string_offset::{ByteOffset, CharOffset};
use vec1::vec1;

use super::{Highlight, ListNumbering, Selection};
use crate::elements::{
    Axis, ClickableCharRange, CornerRadius, Fill, HighlightedRange, HoverableCharRange,
    MouseStateHandle, PartialClickableElement, Point, Radius, SecretRange, SelectableElement,
    SelectionFragment, SmartSelectFn, ZIndex, SELECTED_HIGHLIGHT_COLOR,
};
use crate::event::{DispatchedEvent, ModifiersState};
use crate::fonts::{FamilyId, Properties, Style, Weight};
use crate::geometry::rect::RectF;
use crate::platform::{Cursor, LineStyle};
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{
    char_slice, count_chars_up_to_byte, BlockHeaderSize, IsRect, SelectionDirection, SelectionType,
    TextBuffer,
};
use crate::text_layout::{
    ClipConfig, StyleAndFont, TextAlignment, TextFrame, TextStyle, DEFAULT_TOP_BOTTOM_RATIO,
};
use crate::{
    AfterLayoutContext, AppContext, Element, Event, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};
#[derive(Debug, Clone, PartialEq)]
pub struct HeadingFontSizeMultipliers {
    pub h1: f32,
    pub h2: f32,
    pub h3: f32,
    pub h4: f32,
    pub h5: f32,
    pub h6: f32,
}

impl Default for HeadingFontSizeMultipliers {
    fn default() -> Self {
        Self {
            h1: BlockHeaderSize::Header1.font_size_multiplication_ratio(),
            h2: BlockHeaderSize::Header2.font_size_multiplication_ratio(),
            h3: BlockHeaderSize::Header3.font_size_multiplication_ratio(),
            h4: BlockHeaderSize::Header4.font_size_multiplication_ratio(),
            h5: BlockHeaderSize::Header5.font_size_multiplication_ratio(),
            h6: BlockHeaderSize::Header6.font_size_multiplication_ratio(),
        }
    }
}

impl HeadingFontSizeMultipliers {
    pub fn get_multiplier(&self, heading_level: usize) -> f32 {
        match heading_level {
            1 => self.h1,
            2 => self.h2,
            3 => self.h3,
            4 => self.h4,
            5 => self.h5,
            6 => self.h6,
            _ => 1.0, // Default to normal font size for invalid heading levels
        }
    }
}

pub type HighlightedHyperlink = Arc<Mutex<Option<HyperlinkPosition>>>;

const CODE_BLOCK_OFFSET: usize = 1;

// TODO: We should think about whether line height applies to notebooks as well.
// Consider whether this element really needs a different default than DEFAULT_UI_LINE_HEIGHT_RATIO
// used by the Text element.
pub const DEFAULT_LINE_HEIGHT_RATIO: f32 = 1.4;
const FRAME_SPACER_HEIGHT: f32 = 4.;
const LINE_BREAK_HEIGHT: f32 = 13.;

const FULL_BULLET: &str = "•";
const EMPTY_BULLET: &str = "◦";
const SQUARE_BULLET: &str = "▪";

// Background color for the code block.
const CODE_BLOCK_BACKGROUND: u32 = 0x00000055;
const DEFAULT_HYPERLINK_COLOR: u32 = 0x7aa6daff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperlinkUrl {
    pub url: String,
}

impl<'a> From<&'a Hyperlink> for HyperlinkLens<'a> {
    fn from(url_or_action: &'a Hyperlink) -> Self {
        match url_or_action {
            Hyperlink::Url(url) => HyperlinkLens::Url(url.as_str()),
            Hyperlink::Action(action) => HyperlinkLens::Action(action.as_ref()),
        }
    }
}

/// A lens into a [`markdown_parser::Hyperlink`].
pub enum HyperlinkLens<'a> {
    Url(&'a str),
    Action(&'a dyn Action),
}

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct HyperlinkPosition {
    frame_index: usize,
    link_range: Range<usize>,
}

struct HyperlinkSupport {
    /// The highlighted hyperlink index.
    highlighted_hyperlink: HighlightedHyperlink,
    /// The highlighted hyperlink index.
    hyperlink_font_color: ColorU,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormattedTextSelectionLocation {
    pub frame_index: usize,
    pub row_index: usize,
    pub glyph_index: usize,
}

enum SavedGlyphPosition {
    FormattedTextLinePosition(FormattedTextSelectionLocation),
    LaidOutTextFramePosition(FormattedTextSelectionLocation),
}

struct SavedGlyphPositionIds {
    position: SavedGlyphPosition,
    position_id: String,
}

pub struct FormattedTextElement {
    formatted_text: Arc<FormattedText>,
    family_id: FamilyId,
    code_block_family_id: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
    heading_to_font_size_multipliers: HeadingFontSizeMultipliers,
    size: Option<Vector2F>,
    origin: Option<Point>,
    text_color: ColorU,
    text_selection_color: ColorU,
    laid_out_text: Vec<LaidOutTextFrame>,
    text_frame_mouse_handlers: Vec<Rc<RefCell<FrameMouseHandlers>>>,
    alignment: TextAlignment,
    inline_code_font_color: Option<ColorU>,
    inline_code_bg_color: Option<ColorU>,
    hyperlink_support: HyperlinkSupport,
    saved_glyph_positions: Vec<SavedGlyphPositionIds>,
    is_selectable: bool,
    is_mouse_interaction_disabled: bool,
    disable_text_wrapping: bool,
    clip_config: Option<ClipConfig>,
    #[cfg(debug_assertions)]
    /// Captures the location of the constructor call site. This is used for debugging purposes.
    constructor_location: Option<&'static std::panic::Location<'static>>,
}

impl FormattedTextElement {
    #[cfg_attr(debug_assertions, track_caller)]
    fn internal_constructor(
        formatted_text: Arc<FormattedText>,
        font_size: f32,
        family_id: FamilyId,
        code_block_family_id: FamilyId,
        text_color: ColorU,
        text_selection_color: ColorU,
        hyperlink_support: HyperlinkSupport,
    ) -> Self {
        Self {
            formatted_text,
            family_id,
            code_block_family_id,
            font_size,
            line_height_ratio: DEFAULT_LINE_HEIGHT_RATIO,
            heading_to_font_size_multipliers: HeadingFontSizeMultipliers::default(),
            text_color,
            text_selection_color,
            size: None,
            origin: None,
            laid_out_text: vec![],
            text_frame_mouse_handlers: vec![],
            inline_code_font_color: None,
            inline_code_bg_color: None,
            alignment: Default::default(),
            hyperlink_support,
            saved_glyph_positions: vec![],
            is_selectable: false,
            is_mouse_interaction_disabled: false,
            disable_text_wrapping: false,
            clip_config: None,
            #[cfg(debug_assertions)]
            constructor_location: Some(std::panic::Location::caller()),
        }
    }

    /// Creates a new FormattedTextElement. Allows multiple [FormattedTextLine]s to be passed in.
    /// This enables features like in-line hyperlinks and code blocks. If this is not needed,
    /// consider the simpler [FormattedTextElement::from_str] constructor.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn new(
        formatted_text: FormattedText,
        font_size: f32,
        family_id: FamilyId,
        code_block_family_id: FamilyId,
        text_color: ColorU,
        highlight_index: HighlightedHyperlink,
    ) -> Self {
        Self::new_arc(
            Arc::new(formatted_text),
            font_size,
            family_id,
            code_block_family_id,
            text_color,
            highlight_index,
        )
    }

    /// Like [`FormattedTextElement::new`], but accepts an already-allocated [`Arc<FormattedText>`]
    /// so callers that have a cached Arc can avoid an extra deep clone.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn new_arc(
        formatted_text: Arc<FormattedText>,
        font_size: f32,
        family_id: FamilyId,
        code_block_family_id: FamilyId,
        text_color: ColorU,
        highlight_index: HighlightedHyperlink,
    ) -> Self {
        Self::internal_constructor(
            formatted_text,
            font_size,
            family_id,
            code_block_family_id,
            text_color,
            *SELECTED_HIGHLIGHT_COLOR,
            HyperlinkSupport {
                highlighted_hyperlink: highlight_index,
                hyperlink_font_color: ColorU::from_u32(DEFAULT_HYPERLINK_COLOR),
            },
        )
    }

    /// Creates a new FormattedTextElement from a single `str`. Use this method similar to how you'd use
    /// [Text::new], though currently FormattedTextElement is missing many features from `Text`. If you
    /// find yourself needing multiple styles throughout the text body, or need to make use of hyperlinks,
    /// consider using [FormattedTextElement::new] instead. The FormattedTextElement created will have mouse
    /// interactions disabled by default
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn from_str(
        text: impl Into<Cow<'static, str>>,
        family_id: FamilyId,
        font_size: f32,
    ) -> Self {
        Self::internal_constructor(
            Arc::new(FormattedText::new([FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text(text.into()),
            ])])),
            font_size,
            family_id,
            family_id,
            ColorU::white(),
            *SELECTED_HIGHLIGHT_COLOR,
            HyperlinkSupport {
                highlighted_hyperlink: Default::default(),
                hyperlink_font_color: ColorU::from_u32(DEFAULT_HYPERLINK_COLOR),
            },
        )
        .disable_mouse_interaction()
    }

    /// TODO (roland): There are cases where we need to set the line height to the UI default
    /// of 1.2 used by the Text element so that a FormattedTextElement and Text element in the same
    /// row are laid out with the same behavior and occupy the same height. If the row has height restrictions,
    /// the larger line height of FormattedTextElement can cause the text to not render due to CoreText
    /// thinking there isn't enough vertical space.
    /// Consider whether this element really needs a different default of 1.4.
    pub fn with_line_height_ratio(mut self, line_height_ratio: f32) -> Self {
        self.line_height_ratio = line_height_ratio;
        self
    }

    pub fn with_heading_to_font_size_multipliers(
        mut self,
        heading_to_font_size_multipliers: HeadingFontSizeMultipliers,
    ) -> Self {
        self.heading_to_font_size_multipliers = heading_to_font_size_multipliers;
        self
    }

    pub fn with_color(mut self, color: ColorU) -> Self {
        self.text_color = color;
        self
    }

    pub fn with_selection_color(mut self, color: ColorU) -> Self {
        self.text_selection_color = color;
        self
    }

    pub fn with_weight(mut self, weight: Weight) -> Self {
        let ft = Arc::make_mut(&mut self.formatted_text);
        let lines = std::mem::take(&mut ft.lines);
        ft.lines = lines
            .into_iter()
            .map(|mut line| {
                line.set_weight(weight.to_custom_weight());
                line
            })
            .collect();
        self
    }

    #[allow(dead_code)]
    pub fn with_alignment(mut self, alignment: TextAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Register a default handler for all _URL_ hyperlinks detected by the parser.
    /// Note that this will clear all existing registered handlers!
    /// For a version with support for actions, see [FormattedTextElement::register_default_click_handlers_with_actions].
    pub fn register_default_click_handlers<S>(mut self, default_click_handler: S) -> Self
    where
        S: 'static + Fn(HyperlinkUrl, &mut EventContext, &AppContext),
    {
        self.text_frame_mouse_handlers.clear();

        let callback = Rc::new(default_click_handler);
        self.register_handlers(|frame_mouse_handlers, (_index, line)| {
            line.hyperlinks(false).into_iter().fold(
                frame_mouse_handlers,
                |mut frame_mouse_handlers, (range, link)| {
                    let callback = callback.clone();
                    frame_mouse_handlers = frame_mouse_handlers.with_hoverable_char_range(
                        range.clone(),
                        MouseStateHandle::default(),
                        Some(Cursor::PointingHand),
                        // Default hover handler does nothing. TODO: highlighting
                        move |_is_hovering, _ctx, _app| {},
                    );
                    frame_mouse_handlers = frame_mouse_handlers.with_clickable_char_range(
                        range,
                        move |_modifiers, ctx, app| {
                            if let Hyperlink::Url(url) = &link {
                                callback(HyperlinkUrl { url: url.clone() }, ctx, app);
                            }
                        },
                    );
                    frame_mouse_handlers
                },
            )
        });

        let highlighted_link_pos = self
            .hyperlink_support
            .highlighted_hyperlink
            .lock()
            .expect("Failed to acquire lock on highlighted_hyperlink")
            .clone();

        for (line_index, line) in self.formatted_text.lines.iter().enumerate() {
            let style_ranges =
                line.hyperlinks(false)
                    .into_iter()
                    .filter_map(|(range, _)| {
                        let mut text_style = TextStyle::new()
                            .with_foreground_color(self.hyperlink_support.hyperlink_font_color);
                        if highlighted_link_pos.as_ref().is_some_and(|pos| {
                            pos.frame_index == line_index && pos.link_range == range
                        }) {
                            text_style = text_style
                                .with_underline_color(self.hyperlink_support.hyperlink_font_color);
                        }

                        let highlight_indices = range.clone().collect_vec();
                        if highlight_indices.is_empty() {
                            None
                        } else {
                            Some(HighlightedRange {
                                highlight_indices,
                                highlight: Highlight::new().with_text_style(text_style),
                            })
                        }
                    })
                    .collect_vec();

            let merged_range = HighlightedRange::merge_overlapping_ranges(style_ranges);
            let sorted_range = merged_range
                .into_iter()
                .sorted_by_key(|range| range.highlight_indices[0]);

            if let Some(handler) = self.text_frame_mouse_handlers.get(line_index) {
                let mut handler = handler.borrow_mut();
                sorted_range.for_each(|style| {
                    handler.add_style(style);
                });
            }
        }

        self
    }

    /// Register a default handler for all hyperlinks detected by the parser.
    /// Note that this will clear all existing registered handlers!
    pub fn register_default_click_handlers_with_action_support<S>(
        mut self,
        default_click_handler: S,
    ) -> Self
    where
        S: 'static + Fn(HyperlinkLens, &mut EventContext, &AppContext),
    {
        self.text_frame_mouse_handlers.clear();

        let callback = Rc::new(default_click_handler);
        self.register_handlers(|frame_mouse_handlers, (_index, line)| {
            line.hyperlinks(false).into_iter().fold(
                frame_mouse_handlers,
                |mut frame_mouse_handlers, (range, link)| {
                    let callback = callback.clone();
                    frame_mouse_handlers = frame_mouse_handlers.with_hoverable_char_range(
                        range.clone(),
                        MouseStateHandle::default(),
                        Some(Cursor::PointingHand),
                        // Default hover handler does nothing. TODO: highlighting
                        move |_is_hovering, _ctx, _app| {},
                    );
                    frame_mouse_handlers = frame_mouse_handlers.with_clickable_char_range(
                        range,
                        move |_modifiers, ctx, app| {
                            callback(HyperlinkLens::from(&link), ctx, app);
                        },
                    );
                    frame_mouse_handlers
                },
            )
        });

        let highlighted_link_pos = self
            .hyperlink_support
            .highlighted_hyperlink
            .lock()
            .expect("Failed to acquire lock on highlighted_hyperlink")
            .clone();

        for (line_index, line) in self.formatted_text.lines.iter().enumerate() {
            let style_ranges =
                line.hyperlinks(false)
                    .into_iter()
                    .filter_map(|(range, _)| {
                        let mut text_style = TextStyle::new()
                            .with_foreground_color(self.hyperlink_support.hyperlink_font_color);
                        if highlighted_link_pos.as_ref().is_some_and(|pos| {
                            pos.frame_index == line_index && pos.link_range == range
                        }) {
                            text_style = text_style
                                .with_underline_color(self.hyperlink_support.hyperlink_font_color);
                        }

                        let highlight_indices = range.clone().collect_vec();
                        if highlight_indices.is_empty() {
                            None
                        } else {
                            Some(HighlightedRange {
                                highlight_indices,
                                highlight: Highlight::new().with_text_style(text_style),
                            })
                        }
                    })
                    .collect_vec();

            let merged_range = HighlightedRange::merge_overlapping_ranges(style_ranges);
            let sorted_range = merged_range
                .into_iter()
                .sorted_by_key(|range| range.highlight_indices[0]);

            if let Some(handler) = self.text_frame_mouse_handlers.get(line_index) {
                let mut handler = handler.borrow_mut();
                sorted_range.for_each(|style| {
                    handler.add_style(style);
                });
            }
        }

        self
    }

    pub fn with_inline_code_properties(
        mut self,
        inline_code_font_color: Option<ColorU>,
        inline_code_bg_color: Option<ColorU>,
    ) -> Self {
        self.inline_code_font_color = inline_code_font_color;
        self.inline_code_bg_color = inline_code_bg_color;
        self
    }

    pub fn with_hyperlink_font_color(mut self, hyperlink_font_color: ColorU) -> Self {
        self.hyperlink_support.hyperlink_font_color = hyperlink_font_color;
        self
    }

    pub fn set_selectable(mut self, selectable: bool) -> Self {
        self.is_selectable = selectable;
        self
    }

    pub fn disable_mouse_interaction(mut self) -> Self {
        self.is_mouse_interaction_disabled = true;
        self
    }

    /// Disables text wrapping within the element. When text doesn't fit within the available
    /// width, the element will extend beyond its constraints rather than wrapping the text.
    /// This forces the parent container to handle overflow by moving the element to a new line.
    pub fn with_no_text_wrapping(mut self) -> Self {
        self.disable_text_wrapping = true;
        self
    }

    /// Sets the clip configuration for text that doesn't fit within the available width.
    /// This automatically disables text wrapping, as clipping only applies to single-line text.
    pub fn with_clip(mut self, clip_config: ClipConfig) -> Self {
        self.disable_text_wrapping = true;
        self.clip_config = Some(clip_config);
        self
    }

    /// Given an absolute point, returns the frame, row, and glyph indexes that makes the
    /// most sense for a caret position.
    /// For example, with example text "here is example text", if the mouse point is over
    /// |i|, the index could be either 5 or 6 depending which side of the |i| the mouse is
    /// closest to.
    ///
    /// # Parameters
    /// - `snapping_policy`: Defines how the function should behave when the point is not in the bounds
    ///   of any frame. The default is to snap to the beginning or the end of the frame if the point is to the
    ///   left or right of the text, but not snap to a frame if it's in between 2 frames.
    fn position_for_point(
        &self,
        absolute_point: Vector2F,
        snapping_policy: SnappingPolicy,
    ) -> Option<FormattedTextSelectionLocation> {
        let (Some(origin), Some(size)) = (self.origin(), self.size()) else {
            return None;
        };

        let relative_point = absolute_point - origin.xy;

        // Snap to first/last character if above/below text.
        if relative_point.y() < 0. {
            return (!self.laid_out_text.is_empty()).then_some(FormattedTextSelectionLocation {
                frame_index: 0,
                row_index: 0,
                glyph_index: 0,
            });
        } else if relative_point.y() > size.y() {
            // Get the last valid frame and its last character index.
            let frame = self.laid_out_text.last()?;
            let (row_index, glyph_index) = frame
                .get_last_row_and_glyph_index(!snapping_policy.should_adjust_to_char_indices)?;
            return Some(FormattedTextSelectionLocation {
                frame_index: self.laid_out_text.len() - 1,
                row_index,
                glyph_index,
            });
        }

        // Find which frame contains the point and which row and glyph index it is in.
        // Keep track of whether the last frame was before or after the point to check if the point
        // landed in a gap between frames.
        let mut point_after_last_frame = false;
        for (frame_index, frame) in self.laid_out_text.iter().enumerate() {
            let frame_bounds = frame.get_frame_bounds();

            let point_before_current_frame = absolute_point.y() < frame_bounds.min_y();
            let point_after_current_frame = absolute_point.y() > frame_bounds.max_y();
            if point_before_current_frame || point_after_current_frame {
                if snapping_policy.should_snap_on_gap
                    && point_after_last_frame
                    && point_before_current_frame
                {
                    // Snap to the beginning of the current frame
                    return Some(FormattedTextSelectionLocation {
                        frame_index,
                        row_index: 0,
                        glyph_index: 0,
                    });
                }

                point_after_last_frame = point_after_current_frame;
                continue;
            }

            return match frame {
                LaidOutTextFrame::Text { text_frame, .. }
                | LaidOutTextFrame::CodeBlock { text_frame, .. }
                | LaidOutTextFrame::Indented { text_frame, .. } => {
                    let relative_y_within_frame = absolute_point.y() - frame_bounds.min_y();

                    // Find the line that the point is in.
                    let mut row_index = 0;
                    let mut line_height = 0.;
                    for line in text_frame.lines() {
                        line_height += line.height();
                        if relative_y_within_frame > line_height {
                            row_index += 1;
                        } else {
                            break;
                        }
                    }
                    if row_index >= text_frame.lines().len() {
                        row_index = text_frame.lines().len().saturating_sub(1);
                    }
                    let line = text_frame.lines().get(row_index)?;

                    // Compute the relative x position within the frame.
                    let relative_x_within_frame =
                        absolute_point.x() - frame_bounds.min_x() - text_frame.line_x_offset(line);

                    let mut glyph_index = if snapping_policy.should_snap_to_ends {
                        line.caret_index_for_x_unbounded(relative_x_within_frame)
                    } else {
                        line.caret_index_for_x(relative_x_within_frame)?
                    };

                    if glyph_index == line.end_index()
                        && snapping_policy.should_adjust_to_char_indices
                    {
                        // If the point is at the end of the line, we should adjust to the last character
                        // index if the snapping policy forces it.
                        glyph_index = line.end_index() - 1;
                    }

                    Some(FormattedTextSelectionLocation {
                        frame_index,
                        row_index,
                        glyph_index,
                    })
                }
                LaidOutTextFrame::LineBreak { .. } => {
                    // Treat line breaks as a single newline character
                    Some(FormattedTextSelectionLocation {
                        frame_index,
                        row_index: 0,
                        glyph_index: 0,
                    })
                }
            };
        }
        None
    }

    /// Determines rendering boundaries for drawing the given selection.
    /// Assumes that [`selection_start`] comes before [`selection_end`].
    fn calculate_selection_bounds(
        &self,
        selection_start: FormattedTextSelectionLocation,
        selection_end: FormattedTextSelectionLocation,
    ) -> Vec<RectF> {
        let start_frame_idx = selection_start.frame_index;
        let start_row_idx = selection_start.row_index;
        let end_frame_idx = selection_end.frame_index;
        let end_row_idx = selection_end.row_index;

        let mut selection_bounds = Vec::new();

        for (frame_index, frame) in self
            .laid_out_text
            .iter()
            .enumerate()
            .skip(start_frame_idx)
            .take(end_frame_idx - start_frame_idx + 1)
        {
            match frame {
                LaidOutTextFrame::Text { text_frame, .. }
                | LaidOutTextFrame::CodeBlock { text_frame, .. }
                | LaidOutTextFrame::Indented { text_frame, .. } => {
                    let frame_bounds = frame.get_frame_bounds();
                    let mut frame_y = frame_bounds.min_y();

                    let is_starting_frame = frame_index == start_frame_idx;
                    let is_ending_frame = frame_index == end_frame_idx;
                    let line_count = text_frame.lines().len();

                    // Start drawing on the starting row if the current frame is the starting frame;
                    // otherwise, start from the first row.
                    let row_start = if is_starting_frame { start_row_idx } else { 0 };

                    // Start drawing on the ending row if the current frame is the ending frame;
                    // otherwise, draw all rows.
                    let row_end = if is_ending_frame {
                        end_row_idx
                    } else {
                        line_count.saturating_sub(1)
                    };

                    for row in text_frame.lines().iter().take(row_start) {
                        frame_y += row.height();
                    }

                    for row_index in row_start..=row_end {
                        let line = match text_frame.lines().get(row_index) {
                            Some(line) => line,
                            None => continue,
                        };
                        let line_height = line.height();
                        let line_origin_x = frame_bounds.min_x() + text_frame.line_x_offset(line);

                        // Draw highlight on the entire line if it's not the starting or the ending row;
                        // otherwise, draw highlight that starts/ends on the selection bound.
                        let start_x = if is_starting_frame && row_index == start_row_idx {
                            line.x_for_index(selection_start.glyph_index)
                        } else {
                            0.
                        };
                        let end_x = if is_ending_frame && row_index == end_row_idx {
                            line.x_for_index(selection_end.glyph_index)
                        } else {
                            line.width
                        };
                        let rect_origin = vec2f(line_origin_x + start_x, frame_y);
                        let rect_size = vec2f(end_x - start_x, line_height);
                        selection_bounds.push(RectF::new(rect_origin, rect_size));
                        frame_y += line_height;
                    }
                }
                LaidOutTextFrame::LineBreak { .. } => (),
            }
        }
        selection_bounds
    }

    /// Assumes that [`selection_start`] comes before [`selection_end`].
    fn draw_selection(
        &self,
        selection_start: FormattedTextSelectionLocation,
        selection_end: FormattedTextSelectionLocation,
        ctx: &mut PaintContext,
    ) {
        if !self.is_selectable {
            return;
        }
        for rect in self.calculate_selection_bounds(selection_start, selection_end) {
            ctx.scene
                .draw_rect_without_hit_recording(rect)
                .with_background(Fill::Solid(self.text_selection_color));
        }
    }

    fn translate_selection_bound_to_line_bound(
        &self,
        bound: FormattedTextSelectionLocation,
        line_start: bool,
    ) -> Option<Vector2F> {
        let origin = self.origin()?;

        let base_y_offset = self.laid_out_text[..bound.frame_index]
            .iter()
            .map(|f| f.calculate_frame_height())
            .sum::<f32>();
        let frame = self.laid_out_text.get(bound.frame_index)?;

        let (relative_x, y_offset) = match frame {
            LaidOutTextFrame::Text { text_frame, .. }
            | LaidOutTextFrame::CodeBlock { text_frame, .. }
            | LaidOutTextFrame::Indented { text_frame, .. } => {
                let frame_bounds = frame.get_frame_bounds();
                let mut y_offset = frame_bounds.min_y() - origin.y();
                // add the height of all lines before the selected line
                y_offset += text_frame
                    .lines()
                    .iter()
                    .take(bound.row_index)
                    .map(|line| line.height())
                    .sum::<f32>();
                // use the y-pos of the middle of the line to be selected
                y_offset += text_frame.lines().get(bound.row_index)?.height() / 2.;

                let line = text_frame.lines().get(bound.row_index)?;
                let relative_x = frame_bounds.min_x() - origin.x()
                    + text_frame.line_x_offset(line)
                    + if line_start { 0. } else { line.width };

                (relative_x, y_offset)
            }
            LaidOutTextFrame::LineBreak { frame_bounds } => {
                // use the y-pos of the middle of the frame to select
                (0., base_y_offset + frame_bounds.height() / 2.)
            }
        };

        Some(origin.xy + vec2f(relative_x, y_offset))
    }

    pub fn register_handlers<F>(&mut self, register: F)
    where
        F: Fn(FrameMouseHandlers, (usize, &FormattedTextLine)) -> FrameMouseHandlers,
    {
        self.text_frame_mouse_handlers = self
            .formatted_text
            .lines
            .iter()
            .enumerate()
            .map(|line| Rc::new(RefCell::new(register(FrameMouseHandlers::default(), line))))
            .collect();
    }

    /// Save a position_id in the position cache for a given glyph and frame in the text.
    /// This can be used to position other elements relative to a char in the text element.
    pub fn with_saved_glyph_position(
        mut self,
        glyph_index: usize,
        frame_index: usize,
        position_id: String,
    ) -> Self {
        self.saved_glyph_positions.push(SavedGlyphPositionIds {
            position: SavedGlyphPosition::FormattedTextLinePosition(
                FormattedTextSelectionLocation {
                    frame_index,
                    row_index: 0,
                    glyph_index,
                },
            ),
            position_id,
        });
        self
    }

    pub fn add_styles(
        &mut self,
        frame_index: usize,
        sorted_styles: impl IntoIterator<Item = HighlightedRange>,
    ) {
        if let Some(handler) = self.text_frame_mouse_handlers.get(frame_index) {
            let mut handler = handler.borrow_mut();
            sorted_styles.into_iter().for_each(|style| {
                handler.add_style(style);
            });
        }
    }

    fn handle_mouse_moved(
        &mut self,
        position: Vector2F,
        z_index: ZIndex,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.is_mouse_interaction_disabled {
            return false;
        }

        let is_covered = ctx.is_covered(Point::from_vec2f(position, z_index));
        let mut handled = false;

        let mut highlighted_link = self
            .hyperlink_support
            .highlighted_hyperlink
            .lock()
            .expect("Failed to acquire lock on highlighted_hyperlink");
        // Set to None. If all checks passed, we will set it to the hovered link.
        *highlighted_link = None;

        if let Some(bounds) = self.bounds() {
            // If the mouse is moving outside the bounds, ignore the event; otherwise,
            // reset the cursor and clear all hover states.
            if !bounds.contains_point(position) {
                // If there was a link hovered, consider mouse to be moving from within the element to outside.
                let was_hovered = self
                    .text_frame_mouse_handlers
                    .iter()
                    .any(|handlers| handlers.borrow_mut().unhover_all(ctx, app));
                if was_hovered {
                    ctx.reset_cursor();
                }
                return false;
            }
        } else {
            return false;
        };

        let Some(text_pos) =
            self.position_for_point(position, SnappingPolicy::precise_char_range())
        else {
            // If the mouse is within the bound of the element but not on any text frame,
            // reset the cursor and clear all hover states.
            for handlers in self.text_frame_mouse_handlers.iter() {
                handlers.borrow_mut().unhover_all(ctx, app);
            }
            ctx.reset_cursor();
            return true;
        };

        let handlers = match self.laid_out_text.get(text_pos.frame_index) {
            Some(
                LaidOutTextFrame::Text { mouse_handlers, .. }
                | LaidOutTextFrame::Indented { mouse_handlers, .. },
            ) => mouse_handlers,
            _ => return false,
        };

        // Note that these are all char indices!
        let mut handlers = handlers.borrow_mut();
        let glyph_offset = handlers.glyph_offset;

        // Unhover all other frames.
        let was_hovered = self
            .laid_out_text
            .iter()
            .enumerate()
            .any(|(i, laid_out_frame)| {
                if i == text_pos.frame_index {
                    return false;
                }
                let handlers = match laid_out_frame {
                    LaidOutTextFrame::Text { mouse_handlers, .. }
                    | LaidOutTextFrame::Indented { mouse_handlers, .. } => mouse_handlers,
                    _ => return false,
                };
                handlers.borrow_mut().unhover_all(ctx, app)
            });
        if was_hovered {
            ctx.reset_cursor();
        }

        // If the mouse is on a frame without any hover handlers, reset the cursor
        if handlers.hover_handlers.is_empty() {
            ctx.reset_cursor();
            return true;
        }

        handlers
            .hover_handlers
            .iter_mut()
            .for_each(|hoverable_range| {
                let was_hovered = hoverable_range.mouse_state().is_hovered();
                let adjusted_range = hoverable_range.char_range.start + glyph_offset.as_usize()
                    ..hoverable_range.char_range.end + glyph_offset.as_usize();
                let is_hovered = !is_covered && adjusted_range.contains(&text_pos.glyph_index);

                if is_hovered != was_hovered {
                    hoverable_range.mouse_state().is_hovered = is_hovered;
                    let handler = hoverable_range.hover_handler.as_mut();
                    handler(is_hovered, ctx, app);
                    handled = true;
                    if let Some(cursor_on_hover) = hoverable_range.cursor_on_hover {
                        if is_hovered {
                            ctx.set_cursor(cursor_on_hover, z_index)
                        } else {
                            ctx.reset_cursor()
                        }
                    }
                }

                if is_hovered {
                    *highlighted_link = Some(HyperlinkPosition {
                        // formatted text line index, not laid-out frame index
                        frame_index: self.frame_index_to_line_index(text_pos.frame_index),
                        link_range: hoverable_range.char_range.clone(),
                    });
                }
            });
        handled
    }

    fn handle_mouse_down(
        &mut self,
        position: Vector2F,
        z_index: ZIndex,
        modifiers: &ModifiersState,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.is_mouse_interaction_disabled
            || ctx.is_covered(Point::from_vec2f(position, z_index))
        {
            return false;
        }

        if !self
            .bounds()
            .is_some_and(|bounds| bounds.contains_point(position))
        {
            return false;
        }

        let Some(click_pos) =
            self.position_for_point(position, SnappingPolicy::precise_char_range())
        else {
            return false;
        };
        let handlers = match self.laid_out_text.get(click_pos.frame_index) {
            Some(
                LaidOutTextFrame::Text { mouse_handlers, .. }
                | LaidOutTextFrame::Indented { mouse_handlers, .. },
            ) => mouse_handlers,
            _ => return false,
        };

        let mut handlers = handlers.borrow_mut();
        let glyph_offset = handlers.glyph_offset;

        let mut handled = false;
        handlers
            .click_handlers
            .iter_mut()
            .for_each(|clickable_range| {
                let handler_char_range = (clickable_range.char_range.start
                    + glyph_offset.as_usize())
                    ..(clickable_range.char_range.end + glyph_offset.as_usize());
                if handler_char_range.contains(&click_pos.glyph_index) {
                    let handler = clickable_range.click_handler.as_mut();
                    handler(modifiers, ctx, app);
                    handled = true;
                }
            });
        handled
    }

    fn frame_index_to_line_index(&self, frame_index: usize) -> usize {
        // Render text line-by-line.
        let mut lines = self.formatted_text.lines.iter().enumerate().peekable();
        // We don't use the frame_index from lines.next() because this creates inconsistencies in the frame index when we have lists
        let mut curr_frame_index = 0;
        let mut last_line_was_list_item = false;
        while let Some((line_index, line)) = lines.next() {
            if curr_frame_index == frame_index {
                return line_index;
            }

            let line_type = LineType::from(line);
            // Since list items are flattened into multiple lines rather than a single list segment,
            // this ends up causing line breaks between each individual list item. To avoid this,
            // we only show the line break if we already started a list and there isn't a following list item.
            let curr_line_is_line_break = matches!(line_type, LineType::LineBreak);
            let next_line_is_list_item = matches!(
                lines.peek(),
                Some((_, FormattedTextLine::UnorderedList(_)))
                    | Some((_, FormattedTextLine::OrderedList(_)))
            );
            if last_line_was_list_item && curr_line_is_line_break && next_line_is_list_item {
                continue;
            }

            last_line_was_list_item =
                matches!(line_type, LineType::OrderedList | LineType::UnorderedList);

            curr_frame_index += 1;
        }

        curr_frame_index
    }

    /// Guarantees that any returned ranges are non-inverted (i.e. start before end)
    fn calculate_point_ranges(
        &self,
        current_selection: Option<Selection>,
    ) -> Option<
        Vec<(
            FormattedTextSelectionLocation,
            FormattedTextSelectionLocation,
        )>,
    > {
        let current_selection = current_selection?;
        let ((start_bound, start_pos), (end_bound, end_pos)) = {
            let start_point = self.position_for_point(
                current_selection.start,
                SnappingPolicy::default().snap_on_gap(),
            )?;
            let end_point = self.position_for_point(
                current_selection.end,
                SnappingPolicy::default().snap_on_gap(),
            )?;
            match start_point.frame_index.cmp(&end_point.frame_index) {
                std::cmp::Ordering::Equal => {
                    if end_point.glyph_index >= start_point.glyph_index {
                        (
                            (start_point, current_selection.start),
                            (end_point, current_selection.end),
                        )
                    } else {
                        (
                            (end_point, current_selection.end),
                            (start_point, current_selection.start),
                        )
                    }
                }
                std::cmp::Ordering::Less => (
                    (start_point, current_selection.start),
                    (end_point, current_selection.end),
                ),
                std::cmp::Ordering::Greater => (
                    (end_point, current_selection.end),
                    (start_point, current_selection.start),
                ),
            }
        };

        match current_selection.is_rect {
            IsRect::True => self.compute_rect_selection_bounds(
                start_bound,
                end_bound,
                start_pos.x(),
                end_pos.x(),
            ),
            IsRect::False => Some(vec![(start_bound, end_bound)]),
        }
    }

    fn compute_rect_selection_bounds(
        &self,
        start_bound: FormattedTextSelectionLocation,
        end_bound: FormattedTextSelectionLocation,
        selection_start_x: f32,
        selection_end_x: f32,
    ) -> Option<
        Vec<(
            FormattedTextSelectionLocation,
            FormattedTextSelectionLocation,
        )>,
    > {
        if start_bound == end_bound {
            return None;
        }
        let mut rows_bounds = Vec::new();

        for (frame_index, frame) in self
            .laid_out_text
            .iter()
            .enumerate()
            .skip(start_bound.frame_index)
            .take(end_bound.frame_index - start_bound.frame_index + 1)
        {
            let text_frame = match frame {
                LaidOutTextFrame::Text { text_frame, .. }
                | LaidOutTextFrame::CodeBlock { text_frame, .. }
                | LaidOutTextFrame::Indented { text_frame, .. } => text_frame,
                LaidOutTextFrame::LineBreak { .. } => {
                    continue;
                }
            };
            let lines = text_frame.lines();

            let start_row_index = if frame_index == start_bound.frame_index {
                start_bound.row_index
            } else {
                0
            };
            let end_row_index = if frame_index == end_bound.frame_index {
                end_bound.row_index
            } else {
                lines.len().saturating_sub(1)
            };

            for (row_index, line) in lines
                .iter()
                .enumerate()
                .skip(start_row_index)
                .take(end_row_index - start_row_index + 1)
            {
                let line_origin_x =
                    frame.get_frame_bounds().min_x() + text_frame.line_x_offset(line);
                let start_caret_index =
                    line.caret_index_for_x_unbounded(selection_start_x - line_origin_x);
                let end_caret_index =
                    line.caret_index_for_x_unbounded(selection_end_x - line_origin_x);
                rows_bounds.push((
                    FormattedTextSelectionLocation {
                        frame_index,
                        row_index,
                        glyph_index: start_caret_index,
                    },
                    FormattedTextSelectionLocation {
                        frame_index,
                        row_index,
                        glyph_index: end_caret_index,
                    },
                ));
            }
        }

        Some(rows_bounds)
    }

    fn build_regular_selection_text(
        &self,
        start_bound: FormattedTextSelectionLocation,
        end_bound: FormattedTextSelectionLocation,
    ) -> Option<String> {
        // Handle selection within a single frame
        if start_bound.frame_index == end_bound.frame_index {
            let frame = self.laid_out_text.get(start_bound.frame_index)?;
            let text = frame.get_raw_text();

            let start_glyph_index = start_bound.glyph_index;
            let end_glyph_index = end_bound.glyph_index.min(text.chars().count());

            Some(char_slice(text, start_glyph_index, end_glyph_index)?.to_owned())
        } else {
            // Handle selection across multiple frames
            let mut result = String::new();

            // Handle start frame
            let start_frame = self.laid_out_text.get(start_bound.frame_index)?;
            let start_text = start_frame.get_raw_text();

            // The `if let` is necessary because `glyph_index` might point to the end of the line.
            // In such cases, `get_selection()` should ignore the starting line and resume from the next line.
            if let Some((start_byte_index, _)) =
                start_text.char_indices().nth(start_bound.glyph_index)
            {
                result.push_str(&start_text[start_byte_index..]);
            }

            // Handle middle frames
            for frame in self
                .laid_out_text
                .iter()
                .skip(start_bound.frame_index + 1)
                .take(end_bound.frame_index - start_bound.frame_index - 1)
            {
                let text = frame.get_raw_text();
                result.push('\n');
                result.push_str(text);
            }

            // Handle end frame
            let end_frame = self.laid_out_text.get(end_bound.frame_index)?;
            let end_text = end_frame.get_raw_text();
            let end_glyph_index = end_bound.glyph_index.min(end_text.chars().count());

            // This is to prevent slicing an empty string (i.e. a newline frame) and returning a None.
            if let Some(text) = char_slice(end_text, 0, end_glyph_index) {
                if !text.is_empty() {
                    result.push('\n');
                    result.push_str(text);
                }
            }

            Some(result)
        }
    }

    /// Returns a reference to the formatted text content of this element.
    pub fn formatted_text(&self) -> &FormattedText {
        &self.formatted_text
    }
}

enum LaidOutTextFrame {
    Text {
        text_frame: Arc<TextFrame>,
        frame_bounds: RectF,
        bottom_padding: f32,
        raw_text: String,
        mouse_handlers: Rc<RefCell<FrameMouseHandlers>>,
    },
    CodeBlock {
        text_frame: Arc<TextFrame>,
        frame_bounds: RectF,
        bottom_padding: f32,
        raw_text: String,
    },
    Indented {
        text_frame: Arc<TextFrame>,
        indent: usize,
        frame_bounds: RectF,
        top_padding: f32,
        bottom_padding: f32,
        left_padding: f32,
        raw_text: String,
        mouse_handlers: Rc<RefCell<FrameMouseHandlers>>,
    },
    LineBreak {
        frame_bounds: RectF,
    },
}

impl LaidOutTextFrame {
    /// Returns if a frame has a certain position inside of it.
    #[allow(dead_code)]
    fn contains(&self, position: Vector2F) -> bool {
        self.get_frame_bounds().contains_point(position)
    }

    fn get_frame_bounds(&self) -> &RectF {
        match self {
            LaidOutTextFrame::Text { frame_bounds, .. }
            | LaidOutTextFrame::CodeBlock { frame_bounds, .. }
            | LaidOutTextFrame::Indented { frame_bounds, .. }
            | LaidOutTextFrame::LineBreak { frame_bounds, .. } => frame_bounds,
        }
    }

    /// Helper function to calculate the x ofset of a text frame.
    pub fn calculate_x_offset(
        &self,
        font_size: f32,
        x_origin: f32,
        alignment: TextAlignment,
        frame_width: f32,
    ) -> f32 {
        let indent = match self {
            LaidOutTextFrame::Text { .. } | LaidOutTextFrame::LineBreak { .. } => 0,
            LaidOutTextFrame::CodeBlock { .. } => CODE_BLOCK_OFFSET,
            LaidOutTextFrame::Indented { indent, .. } => *indent,
        };

        match alignment {
            TextAlignment::Left => x_origin + font_size * indent as f32,
            TextAlignment::Right => x_origin + frame_width - self.width(),
            TextAlignment::Center => x_origin + (frame_width - self.width()) / 2.,
        }
    }

    pub fn width(&self) -> f32 {
        match self {
            LaidOutTextFrame::CodeBlock { text_frame, .. }
            | LaidOutTextFrame::Indented { text_frame, .. }
            | LaidOutTextFrame::Text { text_frame, .. } => text_frame.max_width(),
            LaidOutTextFrame::LineBreak { .. } => 0.,
        }
    }

    /// Helper function to get the frame height.
    pub fn calculate_frame_height(&self) -> f32 {
        match self {
            LaidOutTextFrame::Text {
                text_frame,
                bottom_padding,
                ..
            }
            | LaidOutTextFrame::CodeBlock {
                text_frame,
                bottom_padding,
                ..
            } => text_frame.height() + bottom_padding,
            LaidOutTextFrame::Indented {
                text_frame,
                top_padding,
                bottom_padding,
                ..
            } => text_frame.height() + bottom_padding + top_padding,
            LaidOutTextFrame::LineBreak { .. } => LINE_BREAK_HEIGHT,
        }
    }

    /// Returns the last row index and glyph index as `(row_index, glyph_index)` if there is at least one Line;
    /// otherwise returns `None`.
    /// # Parameters
    /// - `use_end_index`: If true, returns the end index of the last line; otherwise, returns the index of the last character.
    pub fn get_last_row_and_glyph_index(&self, use_end_index: bool) -> Option<(usize, usize)> {
        match self {
            LaidOutTextFrame::Text { text_frame, .. }
            | LaidOutTextFrame::CodeBlock { text_frame, .. }
            | LaidOutTextFrame::Indented { text_frame, .. } => {
                text_frame.lines().last().map(|line| {
                    (
                        text_frame.lines().len() - 1,
                        if use_end_index {
                            line.end_index()
                        } else {
                            line.last_index()
                        },
                    )
                })
            }
            LaidOutTextFrame::LineBreak { .. } => {
                // Treat line breaks as a single newline character
                Some((0, 0))
            }
        }
    }

    pub fn get_raw_text(&self) -> &str {
        match self {
            LaidOutTextFrame::Text { raw_text, .. }
            | LaidOutTextFrame::CodeBlock { raw_text, .. }
            | LaidOutTextFrame::Indented { raw_text, .. } => raw_text,
            LaidOutTextFrame::LineBreak { .. } => "",
        }
    }
}

#[derive(Default)]
pub struct FrameMouseHandlers {
    // contains the clickable char ranges and the corresponding click
    // handler for each char range
    click_handlers: Vec<ClickableCharRange>,
    // contains the hoverable char ranges and the corresponding hover
    // handler for each char range
    hover_handlers: Vec<HoverableCharRange>,
    glyph_offset: CharOffset,
    byte_offset: ByteOffset,
    secret_replacement: Vec<(SecretRange, Cow<'static, str>)>,
    styles: Vec<HighlightedRange>,
}

impl FrameMouseHandlers {
    fn add_offset(&mut self, offset: CharOffset, byte_offset: ByteOffset) {
        self.glyph_offset = offset;
        self.byte_offset = byte_offset;
    }

    /// Returns `true` if any of the hoverable ranges was hovered.
    fn unhover_all(&mut self, ctx: &mut EventContext, app: &AppContext) -> bool {
        let mut any_hovered = false;
        self.hover_handlers.iter_mut().for_each(|hoverable_range| {
            let hovered = hoverable_range.mouse_state().is_hovered();
            any_hovered |= hovered;
            if hovered {
                let handler = hoverable_range.hover_handler.as_mut();
                handler(false, ctx, app);
                hoverable_range.mouse_state().is_hovered = false;
            }
        });
        any_hovered
    }

    fn add_style(&mut self, style: HighlightedRange) {
        self.styles.push(style);
    }
}

static SECRET_REPLACEMENT_OOB_ONCE: Once = Once::new();

impl PartialClickableElement for FrameMouseHandlers {
    fn with_clickable_char_range<F>(
        mut self,
        clickable_char_range: Range<usize>,
        callback: F,
    ) -> Self
    where
        F: 'static + FnMut(&ModifiersState, &mut EventContext, &AppContext),
    {
        self.click_handlers.push(ClickableCharRange {
            char_range: clickable_char_range,
            click_handler: Box::new(callback),
        });
        self
    }

    fn with_hoverable_char_range<F>(
        mut self,
        hoverable_char_range: Range<usize>,
        mouse_state: MouseStateHandle,
        cursor_on_hover: Option<Cursor>,
        callback: F,
    ) -> Self
    where
        F: 'static + FnMut(bool, &mut EventContext, &AppContext),
    {
        self.hover_handlers.push(HoverableCharRange {
            char_range: hoverable_char_range,
            hover_handler: Box::new(callback),
            cursor_on_hover,
            mouse_state,
        });
        self
    }

    fn replace_text_range(&mut self, range: SecretRange, replacement: Cow<'static, str>) {
        self.secret_replacement.push((range, replacement));
    }
}

/// Applies secret replacements to the given text using char indices adjusted by glyph_offset.
/// Replacements are applied in descending order of start position to avoid shifting ranges.
fn apply_secret_replacements(
    text: &mut String,
    glyph_offset: usize,
    secret_replacements: &[(SecretRange, Cow<'static, str>)],
) {
    let mut replacements = secret_replacements.to_vec();
    replacements.sort_by_key(|(range, _)| Reverse(range.char_range.start));

    let total_chars = text.chars().count();
    let mut out_of_bound_message: Option<String> = None;

    for (range, replacement) in replacements.iter() {
        let start_char = range.char_range.start + glyph_offset;
        let end_char = range.char_range.end + glyph_offset;

        if start_char >= end_char {
            continue;
        }
        if start_char > total_chars {
            out_of_bound_message = Some(format!(
                "Secret redaction OOB: char start beyond length. range={:?}, start_char={}, end_char={}, total_chars={}, byte_len={}",
                start_char..end_char,
                start_char,
                end_char,
                total_chars,
                text.len()
            ));
            continue;
        }

        let start_byte = if start_char == total_chars {
            text.len()
        } else if let Some((byte_idx, _)) = text.char_indices().nth(start_char) {
            byte_idx
        } else {
            out_of_bound_message = Some(format!(
                "Secret redaction OOB: failed to map start_char to byte index. range={:?}, start_char={}, end_char={}, total_chars={}, byte_len={}",
                start_char..end_char,
                start_char,
                end_char,
                total_chars,
                text.len()
            ));
            continue;
        };

        let end_byte = if end_char >= total_chars {
            text.len()
        } else if let Some((byte_idx, _)) = text.char_indices().nth(end_char) {
            byte_idx
        } else {
            out_of_bound_message = Some(format!(
                "Secret redaction OOB: failed to map end_char to byte index. range={:?}, start_char={}, end_char={}, total_chars={}, byte_len={}",
                start_char..end_char,
                start_char,
                end_char,
                total_chars,
                text.len()
            ));
            continue;
        };

        if start_byte > end_byte || end_byte > text.len() {
            out_of_bound_message = Some(format!(
                "Secret redaction OOB: byte range invalid. start_byte={}, end_byte={}, byte_len={}, start_char={}, end_char={}, total_chars={}",
                start_byte,
                end_byte,
                text.len(),
                start_char,
                end_char,
                total_chars
            ));
            continue;
        }

        text.replace_range(start_byte..end_byte, replacement);
    }

    if let Some(msg) = out_of_bound_message {
        SECRET_REPLACEMENT_OOB_ONCE.call_once(|| log::error!("{msg}"));
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum LineType {
    OrderedList,
    UnorderedList,
    FormattedLine,
    CodeBlock,
    LineBreak,
}

impl From<&FormattedTextLine> for LineType {
    fn from(line: &FormattedTextLine) -> Self {
        match line {
            FormattedTextLine::OrderedList(_) => LineType::OrderedList,
            FormattedTextLine::UnorderedList(_) => LineType::UnorderedList,
            FormattedTextLine::Heading(_)
            | FormattedTextLine::Line(_)
            | FormattedTextLine::TaskList(_)
            | FormattedTextLine::Table(_) => LineType::FormattedLine,
            FormattedTextLine::CodeBlock(_) => LineType::CodeBlock,
            FormattedTextLine::LineBreak
            | FormattedTextLine::HorizontalRule
            | FormattedTextLine::Embedded(_)
            | FormattedTextLine::Image(_) => LineType::LineBreak,
        }
    }
}

impl Element for FormattedTextElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.laid_out_text = vec![];
        let max_width = constraint.max_along(Axis::Horizontal);
        let max_height = constraint.max_along(Axis::Vertical);

        // Frame width should at least be the minimum constraint width.
        let mut frame_width = constraint.min.x();
        let mut frame_height = 0.;
        let mut should_expand_to_max_width = false;

        // Render text line-by-line.
        let mut lines = self.formatted_text.lines.iter().enumerate().peekable();
        let mut last_line_was_list_item = false;
        let mut list_numbering = ListNumbering::new();

        // We don't use the frame_index from lines.next() because this creates inconsistencies in the frame index when we have lists
        let mut frame_index = 0;
        while let Some((line_index, line)) = lines.next() {
            let mut res = Vec::new();
            let (font_size, texts, indent, line_type) = match line {
                FormattedTextLine::Heading(header) => (
                    self.heading_to_font_size_multipliers
                        .get_multiplier(header.heading_size)
                        * self.font_size,
                    &header.text,
                    0,
                    LineType::FormattedLine,
                ),
                FormattedTextLine::Line(texts) => {
                    (self.font_size, texts, 0, LineType::FormattedLine)
                }
                // TODO: Update when we support task lists.
                FormattedTextLine::TaskList(list) => {
                    (self.font_size, &list.text, 0, LineType::FormattedLine)
                }
                // Increment indent_level by 1 since even if no indent is given, still need to format list to the right.
                FormattedTextLine::OrderedList(texts) => (
                    self.font_size,
                    &texts.indented_text.text,
                    texts.indented_text.indent_level + 1,
                    LineType::OrderedList,
                ),
                FormattedTextLine::UnorderedList(texts) => (
                    self.font_size,
                    &texts.text,
                    texts.indent_level + 1,
                    LineType::UnorderedList,
                ),
                FormattedTextLine::CodeBlock(texts) => {
                    // If there are any code block lines, this element should expand to max width.
                    // This is because we want the code block background to take up the full width even
                    // if the text itself is very short.
                    should_expand_to_max_width = true;
                    // Add a line of padding before and after the code body.
                    let new_line = "\n".to_owned();
                    let formatted = new_line.clone() + &texts.code + &new_line;

                    // TODO: In the future, can use the first parameter (the lang field) to modify the actual style of the text fragment.
                    res.push(FormattedTextFragment::plain_text(formatted));

                    (self.font_size, &res, 1, LineType::CodeBlock)
                }
                FormattedTextLine::Table(table) => {
                    res.push(FormattedTextFragment::plain_text(table.to_plain_text()));
                    (self.font_size, &res, 0, LineType::FormattedLine)
                }
                FormattedTextLine::LineBreak
                | FormattedTextLine::HorizontalRule
                | FormattedTextLine::Embedded(_)
                | FormattedTextLine::Image(_) => (self.font_size, &res, 0, LineType::LineBreak),
            };

            // Appends either the number or bullet type in the case of list based on the indent.
            let mut text = match line {
                FormattedTextLine::OrderedList(texts) => {
                    format!(
                        "{}. ",
                        list_numbering
                            // Subtracting by 1 as `indent` of lists starts at 1,
                            // but list_numbering expects it to start at 0.
                            .advance(indent.saturating_sub(1), texts.number)
                            .display_label
                    )
                }

                FormattedTextLine::UnorderedList(_) => {
                    let bullet = match indent {
                        1 => FULL_BULLET,
                        2 => EMPTY_BULLET,
                        _ => SQUARE_BULLET,
                    };
                    // Align with the ordered list - assuming the ordered list indices are only single digits.
                    format!("{bullet:<3}")
                }
                _ => String::new(),
            };
            // Offset to account for the bullet or number.
            let glyph_offset = text.chars().count();
            let byte_offset = text.len();

            // Reset ordered list numbering if this item isn't an ordered list.
            if !matches!(line, FormattedTextLine::OrderedList(_)) {
                list_numbering.reset();
            }

            // Keep a vec of running styles and the range of indices they are decorating.
            let mut styles = vec![];

            // If there is a prefix (e.g. bullet points, numbers, etc), accounts for the style of it which will be default.
            if !text.is_empty() {
                let style = match line {
                    // Round bullets are a bit small comparing to the square bullet, so we bold them to increase their size.
                    FormattedTextLine::UnorderedList(_) if indent == 1 || indent == 2 => {
                        Properties::default().weight(Weight::Bold)
                    }
                    _ => Properties::default(),
                };
                styles.push((
                    0..glyph_offset,
                    StyleAndFont::new(self.family_id, style, TextStyle::new()),
                ));
            }

            // Scope to contain a borrow of the mouse handlers.
            {
                // Preserves formatting for the innner text in case of list.
                let mut prev_index = glyph_offset;

                let borrowed_handler = self
                    .text_frame_mouse_handlers
                    .get(line_index)
                    .map(|handler| handler.borrow());
                let mut link_styles_iter = if let Some(borrowed_handler) = &borrowed_handler {
                    borrowed_handler.styles.iter()
                } else {
                    [].iter()
                }
                .peekable();

                let mut current_link_style: Option<HighlightedRange> = None;

                for inline in texts {
                    let fragment_char_count = inline.text.chars().count();
                    let mut character_count = 0;

                    while character_count < fragment_char_count {
                        let mut style = Properties::default();
                        let mut text_style = TextStyle::default();

                        if let Some(style) = link_styles_iter.peek() {
                            if style.highlight_indices[0] + glyph_offset
                                == prev_index + character_count
                            {
                                current_link_style = Some((*style).clone());
                                link_styles_iter.next();
                            }
                        }

                        let start_char_index = prev_index + character_count;
                        let style_char_len;

                        if let Some(link_style) = &current_link_style {
                            text_style = link_style.highlight.text_style;
                            let end_char_index = *link_style.highlight_indices.last().unwrap_or(&0)
                                + 1
                                + glyph_offset;
                            if end_char_index - start_char_index
                                <= fragment_char_count - character_count
                            {
                                // If the length of the currently registered link style is less than the remaining length of the current fragment,
                                // then the style should be applied to the link only.
                                style_char_len = end_char_index - start_char_index;
                                // Reset the current link style since it has been fully applied.
                                current_link_style = None;
                            } else {
                                // If the length of the currently registered link style is greater than the remaining length of the current fragment,
                                // then the style should be applied to the entire fragment.
                                style_char_len = fragment_char_count - character_count;
                                // We keep the current_link_style, as it might overflow to the next fragment.
                            }
                        } else if let Some(style) = link_styles_iter.peek() {
                            // If we are not currently working with a link style, then the length of the current style should be the minimum
                            // of the remaining length of the fragment or the remaining length until the next link style within the fragment.
                            style_char_len = (style.highlight_indices[0] + glyph_offset
                                - start_char_index)
                                .min(fragment_char_count - character_count);
                        } else {
                            // If there's no more link styles, then the length of the current style should be the remaining length of the fragment.
                            style_char_len = fragment_char_count - character_count;
                        }

                        if inline.styles.weight.is_some() {
                            style.weight = Weight::from_custom_weight(inline.styles.weight);
                        }
                        if inline.styles.italic {
                            style.style = Style::Italic;
                        }
                        if inline.styles.strikethrough {
                            text_style = text_style.with_show_strikethrough(true);
                        }
                        if inline.styles.underline {
                            let underline_color =
                                text_style.foreground_color.unwrap_or(self.text_color);
                            text_style = text_style.with_underline_color(underline_color)
                        }
                        if inline.styles.inline_code {
                            // If we have existing background and foreground highlighting from, for example,
                            // a link or a search, we don't want to override it.
                            if let Some(font_color) = self.inline_code_font_color {
                                if text_style.foreground_color.is_none() {
                                    text_style.foreground_color = Some(font_color);
                                }
                            }
                            if let Some(bg_color) = self.inline_code_bg_color {
                                if text_style.background_color.is_none() {
                                    text_style.background_color = Some(bg_color);
                                }
                            }
                        }

                        let font_family_id = if matches!(line, FormattedTextLine::CodeBlock(_))
                            || inline.styles.inline_code
                        {
                            self.code_block_family_id
                        } else {
                            self.family_id
                        };

                        styles.push((
                            start_char_index..start_char_index + style_char_len,
                            StyleAndFont::new(font_family_id, style, text_style),
                        ));

                        character_count += style_char_len;
                    }
                    prev_index += character_count;
                    text.push_str(&inline.text);
                    // TODO: ensure the constructed text is the same as `line.raw_text()` (test?)
                }
            }

            // Since list items are flattened into multiple lines rather than a single list segment,
            // this ends up causing line breaks between each individual list item. To avoid this,
            // we only show the line break if we already started a list and there isn't a following list item.
            let curr_line_is_line_break = matches!(line_type, LineType::LineBreak);
            let next_line_is_list_item = matches!(
                lines.peek(),
                Some((_, FormattedTextLine::UnorderedList(_)))
                    | Some((_, FormattedTextLine::OrderedList(_)))
            );
            if last_line_was_list_item && curr_line_is_line_break && next_line_is_list_item {
                continue;
            }

            if let Some(handler) = self.text_frame_mouse_handlers.get(line_index) {
                let mut handler = handler.borrow_mut();
                if glyph_offset > 0 {
                    handler.add_offset(glyph_offset.into(), byte_offset.into());
                }
                apply_secret_replacements(
                    &mut text,
                    glyph_offset,
                    handler.secret_replacement.as_slice(),
                );
            }

            // Indent should only be considered with left text alignment. This matches the behavior
            // of other text editors (Google Docs).
            let should_layout_with_indent = line_type != LineType::FormattedLine
                && matches!(self.alignment, TextAlignment::Left);

            let text_frame_width = if self.disable_text_wrapping && self.clip_config.is_none() {
                // Use a very large width to prevent text wrapping.
                f32::MAX
            } else if should_layout_with_indent {
                (max_width - font_size * indent as f32).max(0.)
            } else {
                max_width
            };

            // If clip_config is set we layout a line, otherwise we use layout_text which uses a TextFrame for soft-wrapping.
            let text_frame = if let Some(clip_config) = self.clip_config {
                let line = ctx.text_layout_cache.layout_line(
                    &text,
                    LineStyle {
                        font_size,
                        line_height_ratio: self.line_height_ratio,
                        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                        fixed_width_tab_size: matches!(line, FormattedTextLine::CodeBlock(_))
                            .then_some(4),
                        skip_caret_positions: false,
                    },
                    styles.as_slice(),
                    text_frame_width,
                    clip_config,
                    &app.font_cache().text_layout_system(),
                );

                Arc::new(TextFrame::new(
                    vec1![(*line).clone()],
                    text_frame_width,
                    self.alignment,
                ))
            } else {
                ctx.text_layout_cache.layout_text(
                    &text,
                    LineStyle {
                        font_size,
                        line_height_ratio: self.line_height_ratio,
                        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                        fixed_width_tab_size: matches!(line, FormattedTextLine::CodeBlock(_))
                            .then_some(4),
                        skip_caret_positions: false,
                    },
                    styles.as_slice(),
                    text_frame_width,
                    max_height,
                    self.alignment,
                    None,
                    &app.font_cache().text_layout_system(),
                )
            };

            // Adjust frame width if there is an indent.
            frame_width = frame_width.max(if should_layout_with_indent {
                text_frame.max_width() + font_size * indent as f32
            } else {
                text_frame.max_width()
            });

            // Give certain lines additional breathing room underneath. This padding is only added
            // if there's more formatted text below, and omitted for the last line of text.
            let bottom_padding = if let Some((_, next_line)) = lines.peek() {
                match line {
                    FormattedTextLine::Heading(_) => match next_line {
                        // If the heading is followed by a line break, don't add any
                        // additional padding (as the line break will take care of it).
                        FormattedTextLine::LineBreak => 0.,
                        _ => LINE_BREAK_HEIGHT,
                    },
                    FormattedTextLine::OrderedList(_) | FormattedTextLine::UnorderedList(_)
                        if !next_line_is_list_item =>
                    {
                        LINE_BREAK_HEIGHT
                    }
                    FormattedTextLine::CodeBlock(_) => LINE_BREAK_HEIGHT,
                    _ => 0.,
                }
            } else {
                0.
            };

            // The saved_glyph_positions vector should be relatively small, so the performance should be acceptable.
            self.saved_glyph_positions
                .iter_mut()
                .for_each(|saved_glyph_position| {
                    if let SavedGlyphPosition::FormattedTextLinePosition(pos) =
                        saved_glyph_position.position
                    {
                        if pos.frame_index != line_index {
                            return;
                        }

                        let mut row_index = 0;
                        let mut glyph_accum = 0;
                        for row in text_frame.lines() {
                            if row.end_index() > (pos.glyph_index + glyph_offset) - glyph_accum {
                                break;
                            }
                            row_index += 1;
                            glyph_accum += row.end_index();
                        }

                        saved_glyph_position.position =
                            SavedGlyphPosition::LaidOutTextFramePosition(
                                FormattedTextSelectionLocation {
                                    frame_index,
                                    row_index,
                                    glyph_index: pos.glyph_index + glyph_offset,
                                },
                            );
                    }
                });

            let laid_out_frame = match line_type {
                LineType::FormattedLine => LaidOutTextFrame::Text {
                    text_frame,
                    frame_bounds: RectF::default(),
                    bottom_padding,
                    raw_text: text,
                    mouse_handlers: self
                        .text_frame_mouse_handlers
                        .get(line_index)
                        .cloned()
                        .unwrap_or(Rc::new(RefCell::new(FrameMouseHandlers::default()))),
                },
                LineType::CodeBlock => LaidOutTextFrame::CodeBlock {
                    text_frame,
                    frame_bounds: RectF::default(),
                    bottom_padding,
                    raw_text: text,
                },
                LineType::OrderedList | LineType::UnorderedList => LaidOutTextFrame::Indented {
                    text_frame,
                    indent,
                    frame_bounds: RectF::default(),
                    top_padding: FRAME_SPACER_HEIGHT,
                    bottom_padding,
                    left_padding: font_size * indent as f32,
                    raw_text: text,
                    mouse_handlers: self
                        .text_frame_mouse_handlers
                        .get(line_index)
                        .cloned()
                        .unwrap_or(Rc::new(RefCell::new(FrameMouseHandlers::default()))),
                },
                LineType::LineBreak => LaidOutTextFrame::LineBreak {
                    frame_bounds: RectF::default(),
                },
            };

            frame_height += laid_out_frame.calculate_frame_height();
            self.laid_out_text.push(laid_out_frame);

            last_line_was_list_item =
                matches!(line_type, LineType::OrderedList | LineType::UnorderedList);

            frame_index += 1;
        }

        let mut size = vec2f(frame_width, frame_height);
        if should_expand_to_max_width {
            if max_width.is_infinite() {
                panic!("The max width was infinite when stretch set to true");
            }
            size.set_x(max_width);
        }
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let mut mut_origin = origin;
        let size = self.size().expect("Expected size to not be none");

        for saved_glyph_position in self.saved_glyph_positions.iter() {
            let SavedGlyphPosition::LaidOutTextFramePosition(pos) = saved_glyph_position.position
            else {
                continue;
            };
            let Some(laid_out_frame) = self.laid_out_text.get(pos.frame_index) else {
                continue;
            };

            let frame = match laid_out_frame {
                LaidOutTextFrame::Text { text_frame, .. }
                | LaidOutTextFrame::Indented { text_frame, .. }
                | LaidOutTextFrame::CodeBlock { text_frame, .. } => text_frame,
                _ => continue,
            };
            let Some(line) = frame.lines().get(pos.row_index) else {
                continue;
            };

            let Some(glyph_width) = line.width_for_index(pos.glyph_index) else {
                continue;
            };

            let x_offset = if let LaidOutTextFrame::Indented { left_padding, .. } = laid_out_frame {
                *left_padding
            } else {
                0.
            } + line.x_for_index(pos.glyph_index);

            let y_offset = self
                .laid_out_text
                .iter()
                .take(pos.frame_index)
                .map(|frame| frame.calculate_frame_height())
                .sum::<f32>()
                + frame
                    .lines()
                    .iter()
                    .take(pos.row_index)
                    .map(|line| line.height())
                    .sum::<f32>();

            ctx.position_cache.cache_position_indefinitely(
                saved_glyph_position.position_id.clone(),
                RectF::new(
                    origin + vec2f(x_offset, y_offset),
                    vec2f(glyph_width, line.height()),
                ),
            );
        }

        // Add indent by moving origin point of the frame.
        for laid_out_frame in &mut self.laid_out_text {
            // Get the x-offset for this laid out frame.
            let x_offset = laid_out_frame.calculate_x_offset(
                self.font_size,
                mut_origin.x(),
                self.alignment,
                size.x(),
            );
            let frame_height = laid_out_frame.calculate_frame_height();

            let (frame, frame_height, bounds) = match laid_out_frame {
                LaidOutTextFrame::Text {
                    text_frame,
                    frame_bounds,
                    bottom_padding,
                    ..
                } => {
                    let curr_origin = vec2f(x_offset, mut_origin.y());
                    *frame_bounds = RectF::new(
                        curr_origin,
                        Vector2F::new(size.x(), frame_height - *bottom_padding),
                    );
                    (Some(text_frame), frame_height, *frame_bounds)
                }
                LaidOutTextFrame::Indented {
                    text_frame,
                    frame_bounds,
                    top_padding,
                    bottom_padding,
                    ..
                } => {
                    let curr_origin = vec2f(x_offset, mut_origin.y() + *top_padding);
                    *frame_bounds = RectF::new(
                        curr_origin,
                        Vector2F::new(size.x(), frame_height - *top_padding - *bottom_padding),
                    );
                    (Some(text_frame), frame_height, *frame_bounds)
                }
                LaidOutTextFrame::CodeBlock {
                    text_frame,
                    frame_bounds,
                    bottom_padding,
                    ..
                } => {
                    let curr_origin = vec2f(x_offset, mut_origin.y());

                    #[cfg(debug_assertions)]
                    ctx.scene
                        .set_location_for_panic_logging(self.constructor_location);

                    // Draw code block rectangle.
                    ctx.scene
                        .draw_rect_with_hit_recording(RectF::new(
                            curr_origin,
                            vec2f(size.x(), frame_height - *bottom_padding),
                        ))
                        .with_background(Fill::Solid(ColorU::from_u32(CODE_BLOCK_BACKGROUND)))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)));

                    *frame_bounds = RectF::new(
                        curr_origin,
                        Vector2F::new(size.x(), frame_height - *bottom_padding),
                    );
                    (Some(text_frame), frame_height, *frame_bounds)
                }
                LaidOutTextFrame::LineBreak { frame_bounds } => {
                    let curr_origin = vec2f(x_offset, mut_origin.y());
                    *frame_bounds = RectF::new(curr_origin, Vector2F::new(size.x(), frame_height));
                    (None, LINE_BREAK_HEIGHT, *frame_bounds)
                }
            };

            if let Some(frame) = frame {
                frame.paint(
                    bounds,
                    &Default::default(),
                    self.text_color,
                    ctx.scene,
                    ctx.font_cache,
                );
            }
            mut_origin += vec2f(0., frame_height);
        }

        // Draw selection if there is one
        if let Some(point_ranges) = self.calculate_point_ranges(ctx.current_selection) {
            for (start, end) in point_ranges {
                self.draw_selection(start, end, ctx);
            }
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let Some(z_index) = self.z_index() else {
            return false;
        };
        match event.at_z_index(z_index, ctx) {
            Some(Event::MouseMoved { position, .. }) => {
                let link_pos_before = self
                    .hyperlink_support
                    .highlighted_hyperlink
                    .lock()
                    .expect("Failed to acquire lock on highlighted_hyperlink")
                    .clone();
                let result = self.handle_mouse_moved(*position, z_index, ctx, app);
                let link_pos_after = self
                    .hyperlink_support
                    .highlighted_hyperlink
                    .lock()
                    .expect("Failed to acquire lock on highlighted_hyperlink")
                    .clone();

                if link_pos_before != link_pos_after {
                    ctx.notify();
                }
                result
            }
            Some(Event::LeftMouseDown {
                position,
                modifiers,
                ..
            }) => {
                self.handle_mouse_down(*position, z_index, modifiers, ctx, app);
                false
            }
            _ => false,
        }

        // false
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        if self.is_selectable {
            Some(self)
        } else {
            None
        }
    }
}

impl SelectableElement for FormattedTextElement {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        let mut start_bound =
            self.position_for_point(selection_start, SnappingPolicy::default().snap_on_gap())?;
        let mut end_bound =
            self.position_for_point(selection_end, SnappingPolicy::default().snap_on_gap())?;

        // If the start and end are at the same position, selection is empty.
        if start_bound.frame_index == end_bound.frame_index
            && start_bound.glyph_index == end_bound.glyph_index
        {
            return None;
        }

        // Ensure start comes before end
        let (selection_start, selection_end) = if start_bound.frame_index > end_bound.frame_index
            || (start_bound.frame_index == end_bound.frame_index
                && start_bound.glyph_index > end_bound.glyph_index)
        {
            std::mem::swap(&mut start_bound, &mut end_bound);
            (selection_end, selection_start)
        } else {
            (selection_start, selection_end)
        };

        let text = match is_rect {
            IsRect::True => {
                let selection_bounds = self.compute_rect_selection_bounds(
                    start_bound,
                    end_bound,
                    selection_start.x(),
                    selection_end.x(),
                )?;
                selection_bounds
                    .into_iter()
                    .filter_map(|(start_bound, end_bound)| {
                        let frame = self.laid_out_text.get(start_bound.frame_index)?;
                        char_slice(
                            frame.get_raw_text(),
                            start_bound.glyph_index,
                            end_bound.glyph_index,
                        )
                    })
                    .join("\n")
            }
            IsRect::False => self.build_regular_selection_text(start_bound, end_bound)?,
        };

        Some(vec![SelectionFragment {
            text,
            origin: self.origin?,
        }])
    }

    fn expand_selection(
        &self,
        absolute_point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        if matches!(unit, SelectionType::Simple | SelectionType::Rect) {
            return None;
        }

        let bound = self.bounds()?;

        // Handle points above/below bounds
        if absolute_point.y() < bound.min_y() {
            return match direction {
                SelectionDirection::Backward => Some(bound.origin()),
                SelectionDirection::Forward => None,
            };
        }
        if absolute_point.y() > bound.max_y() {
            return match direction {
                SelectionDirection::Backward => None,
                SelectionDirection::Forward => Some(absolute_point),
            };
        }

        match unit {
            SelectionType::Simple | SelectionType::Rect => None,
            SelectionType::Semantic => {
                let text_selection_bound = self
                    .position_for_point(absolute_point, SnappingPolicy::default().snap_on_gap())?;
                let frame = self.laid_out_text.get(text_selection_bound.frame_index)?;

                match frame {
                    LaidOutTextFrame::Text {
                        text_frame,
                        raw_text,
                        ..
                    }
                    | LaidOutTextFrame::CodeBlock {
                        text_frame,
                        raw_text,
                        ..
                    }
                    | LaidOutTextFrame::Indented {
                        text_frame,
                        raw_text,
                        ..
                    } => {
                        let frame_bounds = frame.get_frame_bounds();
                        let inner_point = if matches!(direction, SelectionDirection::Backward) {
                            raw_text.word_starts_backward_from_offset_exclusive(CharOffset::from(
                                text_selection_bound.glyph_index,
                            ))
                        } else {
                            raw_text.word_ends_from_offset_exclusive(CharOffset::from(
                                text_selection_bound.glyph_index,
                            ))
                        }
                        .ok()?
                        .with_policy(word_boundaries_policy)
                        .next()?;

                        let offset = raw_text.to_offset(inner_point).ok()?.as_usize();

                        let line = text_frame.lines().get(text_selection_bound.row_index)?;
                        let first_glyph = line.first_glyph()?;
                        let last_glyph = line.last_glyph()?;
                        let relative_x =
                            if first_glyph.index <= offset && offset <= last_glyph.index + 1 {
                                line.x_for_index(offset)
                            } else if matches!(direction, SelectionDirection::Backward) {
                                0.
                            } else {
                                line.width
                            };
                        let absolute_x =
                            frame_bounds.min_x() + text_frame.line_x_offset(line) + relative_x;

                        Some(vec2f(absolute_x, absolute_point.y()))
                    }
                    LaidOutTextFrame::LineBreak { .. } => None,
                }
            }
            SelectionType::Lines => {
                let text_selection_bound =
                    self.position_for_point(absolute_point, SnappingPolicy::default())?;
                let snap_to = self.translate_selection_bound_to_line_bound(
                    text_selection_bound,
                    matches!(direction, SelectionDirection::Backward),
                )?;
                Some(snap_to)
            }
        }
    }

    fn is_point_semantically_before(
        &self,
        absolute_point_1: Vector2F,
        absolute_point_2: Vector2F,
    ) -> Option<bool> {
        let bounds = self.bounds()?;
        match (
            bounds.contains_point(absolute_point_1),
            bounds.contains_point(absolute_point_2),
        ) {
            (false, false) => None,
            (false, true) => {
                if absolute_point_1.y() < bounds.min_y() {
                    Some(true)
                } else if absolute_point_1.y() > bounds.max_y() {
                    Some(false)
                } else {
                    Some(absolute_point_1.x() < bounds.min_x())
                }
            }
            (true, false) => {
                if absolute_point_2.y() < bounds.min_y() {
                    Some(false)
                } else if absolute_point_2.y() > bounds.max_y() {
                    Some(true)
                } else {
                    Some(absolute_point_2.x() > bounds.max_x())
                }
            }
            (true, true) => {
                let point_1 = self.position_for_point(
                    absolute_point_1,
                    SnappingPolicy::default().snap_on_gap(),
                )?;
                let point_2 = self.position_for_point(
                    absolute_point_2,
                    SnappingPolicy::default().snap_on_gap(),
                )?;
                Some(
                    (
                        point_1.frame_index,
                        point_1.row_index,
                        point_1.glyph_index,
                        absolute_point_1.x(),
                    ) < (
                        point_2.frame_index,
                        point_2.row_index,
                        point_2.glyph_index,
                        absolute_point_2.x(),
                    ),
                )
            }
        }
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        smart_select_fn: SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        let bound = self.bounds()?;
        if absolute_point.y() < bound.min_y() || absolute_point.y() > bound.max_y() {
            return None;
        }
        if absolute_point.x() < bound.min_x() || absolute_point.x() > bound.max_x() {
            return None;
        }

        let text_selection_bound =
            self.position_for_point(absolute_point, SnappingPolicy::default())?;
        let frame = self.laid_out_text.get(text_selection_bound.frame_index)?;

        match frame {
            LaidOutTextFrame::Text {
                text_frame,
                raw_text,
                ..
            }
            | LaidOutTextFrame::CodeBlock {
                text_frame,
                raw_text,
                ..
            }
            | LaidOutTextFrame::Indented {
                text_frame,
                raw_text,
                ..
            } => {
                // Get row that we initially clicked on.
                // We need to do this because the same char offset in the raw text buffer
                // could be either the end of one line or the start of the next line.
                let line = text_frame.lines().get(text_selection_bound.row_index)?;
                let first_glyph = line.first_glyph()?;
                let last_glyph = line.last_glyph()?;
                // If we clicked to the right of a line, the text_selection_bound's glyph index would be one larger than the last glyph's index.
                // Snap it within the line's index range so we do smart selection as if we clicked on the last glyph in the line.
                let char_offset = text_selection_bound
                    .glyph_index
                    .clamp(first_glyph.index, last_glyph.index);
                let byte_offset = raw_text.char_indices().nth(char_offset)?.0.into();

                let smart_select_range = smart_select_fn(raw_text, byte_offset)?;
                // convert the byte offset to glyph index
                let smart_select_start =
                    count_chars_up_to_byte(raw_text, smart_select_range.start)?.as_usize();
                let smart_select_end =
                    count_chars_up_to_byte(raw_text, smart_select_range.end)?.as_usize();
                // After smart selection, the start and end offsets can be on different lines if a word wrapped.
                // Find the lines for the start and end offsets.
                let start_row = text_frame.row_within_frame(smart_select_start, false);
                let end_row = text_frame.row_within_frame(smart_select_end, true);
                let start_line = text_frame.lines().get(start_row)?;
                let end_line = text_frame.lines().get(end_row)?;

                let start_relative_x = start_line.x_for_index(smart_select_start);
                let end_relative_x = end_line.x_for_index(smart_select_end);
                // We subtract half the line's height to put the position in the center of the line.
                let start_relative_y =
                    text_frame.height_up_to_row(start_row) - start_line.height() / 2.;
                let end_relative_y = text_frame.height_up_to_row(end_row) - end_line.height() / 2.;
                let frame_bounds = frame.get_frame_bounds();

                Some((
                    vec2f(
                        frame_bounds.min_x()
                            + text_frame.line_x_offset(start_line)
                            + start_relative_x,
                        frame_bounds.min_y() + start_relative_y,
                    ),
                    vec2f(
                        frame_bounds.min_x() + text_frame.line_x_offset(end_line) + end_relative_x,
                        frame_bounds.min_y() + end_relative_y,
                    ),
                ))
            }
            LaidOutTextFrame::LineBreak { .. } => None,
        }
    }

    fn calculate_clickable_bounds(&self, current_selection: Option<Selection>) -> Vec<RectF> {
        if self.origin.is_none() {
            return vec![];
        }

        self.calculate_point_ranges(current_selection)
            .unwrap_or_default()
            .iter()
            .flat_map(|(start_point, end_point)| {
                self.calculate_selection_bounds(*start_point, *end_point)
            })
            .collect()
    }
}

/// A policy that determines how the snapping should behave when converting a point position
/// to a position of a glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnappingPolicy {
    /// If true, the snapping will snap to the beginning of the next frame if the point is in
    /// the gap between two frames; otherwise, it will return `None`.
    should_snap_on_gap: bool,
    /// If true, the snapping will snap to the beginning or the end of a frame if the point is
    /// outside the frame bounds; otherwise, it will return `None`.
    should_snap_to_ends: bool,
    /// If true, the returned result will not exceed the last char index. The last caret index
    /// will simply be reduced to the last char index.
    should_adjust_to_char_indices: bool,
}

impl SnappingPolicy {
    fn snap_on_gap(mut self) -> Self {
        self.should_snap_on_gap = true;
        self
    }

    fn precise_char_range() -> Self {
        Self {
            should_snap_on_gap: false,
            should_snap_to_ends: false,
            should_adjust_to_char_indices: true,
        }
    }
}

impl Default for SnappingPolicy {
    fn default() -> Self {
        Self {
            should_snap_on_gap: false,
            should_snap_to_ends: true,
            should_adjust_to_char_indices: false,
        }
    }
}

#[cfg(test)]
#[path = "formatted_text_element_tests.rs"]
mod tests;
