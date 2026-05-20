use super::{
    AfterLayoutContext, AppContext, Axis, ClickableCharRange, Element, EventContext, Fill,
    HoverableCharRange, LayoutContext, MouseStateHandle, PaintContext, PartialClickableElement,
    Point, RectF, SecretRange, SelectableElement, Selection, SelectionFragment, SizeConstraint,
    SELECTED_HIGHLIGHT_COLOR,
};

use crate::event::ModifiersState;
use crate::platform::{Cursor, LineStyle};
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{IsRect, SelectionDirection, SelectionType, TextBuffer};
use crate::text_layout::{
    ClipConfig, ComputeBaselinePositionFn, Line, StyleAndFont, TextFrame, TextStyle,
    DEFAULT_TOP_BOTTOM_RATIO,
};
use crate::text_selection_utils::{
    calculate_tick_width, create_newline_tick_rect, selection_crosses_newline_row_based,
    NewlineTickParams,
};
use crate::Event;
use crate::{
    event::DispatchedEvent,
    fonts::{Cache as FontCache, FamilyId, Properties},
    Scene,
};
use itertools::Itertools;
use pathfinder_color::ColorU;
use pathfinder_geometry::util::EPSILON;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use std::borrow::Cow;
use std::mem::swap;
use std::{borrow::Borrow, ops::Range, sync::Arc};
use string_offset::CharOffset;

pub const DEFAULT_UI_LINE_HEIGHT_RATIO: f32 = 1.2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd)]
pub struct TextSelectionBound {
    row: usize,
    glyph_index: usize,
}

#[derive(Clone)]
enum LaidOutText {
    // The text hasn't been laid out or doesn't fit given the size constraints.
    None,
    Line(Arc<Line>),
    Frame(Arc<TextFrame>),
}

impl LaidOutText {
    fn width(&self) -> f32 {
        match self {
            LaidOutText::Line(line) => line.width,
            LaidOutText::Frame(frame) => frame.max_width(),
            LaidOutText::None => 0.,
        }
    }

    fn height(&self) -> f32 {
        match self {
            LaidOutText::Line(line) => line.height(),
            LaidOutText::Frame(frame) => frame.height(),
            LaidOutText::None => 0.,
        }
    }

    fn paint(
        &self,
        bounds: RectF,
        default_color: ColorU,
        scene: &mut Scene,
        font_cache: &FontCache,
        compute_baseline_position_fn: Option<&ComputeBaselinePositionFn>,
    ) {
        match self {
            LaidOutText::Line(line) => {
                if let Some(compute_baseline_position_fn) = compute_baseline_position_fn {
                    line.paint_with_baseline_position(
                        bounds,
                        &Default::default(),
                        default_color,
                        font_cache,
                        scene,
                        compute_baseline_position_fn,
                    )
                } else {
                    line.paint(
                        bounds,
                        &Default::default(),
                        default_color,
                        font_cache,
                        scene,
                    )
                }
            }
            LaidOutText::Frame(frame) => {
                if let Some(compute_baseline_position_fn) = compute_baseline_position_fn {
                    frame.paint_with_baseline_position(
                        bounds,
                        &Default::default(),
                        default_color,
                        scene,
                        font_cache,
                        compute_baseline_position_fn,
                    )
                } else {
                    frame.paint(
                        bounds,
                        &Default::default(),
                        default_color,
                        scene,
                        font_cache,
                    )
                }
            }
            LaidOutText::None => {}
        }
    }
}

#[derive()]
pub struct Text {
    text: Cow<'static, str>,
    family_id: FamilyId,
    font_properties: Properties,
    font_size: f32,
    line_height_ratio: f32,
    styles: Vec<Styles>,
    laid_out_text: LaidOutText,
    text_color: ColorU,
    text_selection_color: ColorU,
    size: Option<Vector2F>,
    origin: Option<Point>,
    soft_wrap: bool,
    autosize_text: Option<f32>,
    /// Sets the desired clip configuration used if the single line text gets clipped
    clip_config: ClipConfig,
    /// Contains the clickable char ranges and the corresponding click
    /// handler for each char range
    click_handlers: Vec<ClickableCharRange>,
    /// Contains the hoverable char ranges and the corresponding hover
    /// handler for each char range
    hover_handlers: Vec<HoverableCharRange>,
    saved_char_positions: Vec<SavedCharPositionIds>,
    /// Optional override on the baseline position computation.
    compute_baseline_position_fn: Option<ComputeBaselinePositionFn>,
    /// Whether the text is selectable when rendered as a descendant of a [`SelectableArea`].
    is_selectable: bool,
    #[cfg(debug_assertions)]
    /// Captures the location of the constructor call site. This is used for debugging purposes.
    constructor_location: Option<&'static std::panic::Location<'static>>,
}

/// Contains a char index for the text that we want to save position_id for.
/// This can be used to position other elements relative to a char in the text element.
struct SavedCharPositionIds {
    char_index: usize,
    position_id: String,
}

pub struct Styles {
    indices: Vec<usize>,
    font_properties: Properties,
    styles: TextStyle,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlight {
    properties: Properties,
    pub(crate) text_style: TextStyle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HighlightedRange {
    pub highlight: Highlight,
    pub highlight_indices: Vec<usize>,
}

impl HighlightedRange {
    pub fn merge_overlapping_ranges(mut ranges: Vec<HighlightedRange>) -> Vec<HighlightedRange> {
        if ranges.is_empty() {
            return ranges;
        }

        // Sort the ranges by the start index of the range.
        ranges.sort_by_key(|range| range.highlight_indices[0]);

        let mut merged_ranges = Vec::new();
        let mut current_range = ranges[0].clone();

        for range in ranges.into_iter().skip(1) {
            let current_end = *current_range
                .highlight_indices
                .last()
                .expect("Expected non-empty range");
            let next_start = range.highlight_indices[0];

            // Check if the current range overlaps or is contiguous with the next range.
            if next_start <= current_end + 1 {
                // Extend the current range to include the next range, avoiding duplicates.
                let new_end = *range
                    .highlight_indices
                    .last()
                    .expect("Expected non-empty range");
                current_range
                    .highlight_indices
                    .extend((current_end + 1..=new_end).filter(|&i| i <= new_end));
            } else {
                // Push the current range to the merged list and start a new range.
                merged_ranges.push(current_range);
                current_range = range;
            }
        }

        // Push the final range.
        merged_ranges.push(current_range);

        merged_ranges
    }
}

impl Highlight {
    pub fn new() -> Self {
        Highlight {
            properties: Default::default(),
            text_style: Default::default(),
        }
    }

    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.properties = properties;
        self
    }

    pub fn with_text_style(mut self, text_style: TextStyle) -> Self {
        self.text_style = text_style;
        self
    }

    pub fn with_foreground_color(mut self, color: ColorU) -> Self {
        self.text_style = TextStyle::new().with_foreground_color(color);
        self
    }

    pub fn text_style(&self) -> &TextStyle {
        &self.text_style
    }

    pub fn properties(&self) -> Properties {
        self.properties
    }
}

impl Text {
    /// We've changed [`Text::new`] to default to enabling soft-wrap. All usages of [`Text::new_inline`] have not been audited.
    /// Consider [`new_inline`](`Text::new_inline`) as deprecated and use [`new`](`Text::new`) instead, with [`soft_warp(false)`](`Text::soft_wrap`) if needed.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn new_inline(
        text: impl Into<Cow<'static, str>>,
        family_id: FamilyId,
        font_size: f32,
    ) -> Self {
        Self {
            soft_wrap: false,
            ..Self::new(text, family_id, font_size)
        }
    }

    /// **Note!!** Text now defaults to `soft_wrap: true`. If you want to disable soft wrapping,
    /// use [`soft_warp(false)`](`Text::soft_wrap`) after creating the text element.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn new(text: impl Into<Cow<'static, str>>, family_id: FamilyId, font_size: f32) -> Self {
        Self {
            text: text.into(),
            soft_wrap: true,
            family_id,
            font_properties: Properties::default(),
            font_size,
            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
            styles: vec![],
            laid_out_text: LaidOutText::None,
            text_color: ColorU::white(),
            text_selection_color: *SELECTED_HIGHLIGHT_COLOR,
            size: None,
            origin: None,
            autosize_text: None,
            clip_config: ClipConfig::default(),
            click_handlers: vec![],
            hover_handlers: vec![],
            saved_char_positions: vec![],
            compute_baseline_position_fn: None,
            is_selectable: true,
            #[cfg(debug_assertions)]
            constructor_location: Some(std::panic::Location::caller()),
        }
    }

    /// Set how to clip the text with both direction and style
    pub fn with_clip(mut self, clip_config: ClipConfig) -> Self {
        self.clip_config = clip_config;
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

    pub fn with_line_height_ratio(mut self, line_height_ratio: f32) -> Self {
        self.line_height_ratio = line_height_ratio;
        self
    }

    /// Set optional override for the baseline position computation function, to be used when
    /// painting Lines within the Text.
    pub fn with_compute_baseline_position_fn(
        mut self,
        compute_baseline_position_fn: ComputeBaselinePositionFn,
    ) -> Self {
        self.compute_baseline_position_fn = Some(compute_baseline_position_fn);
        self
    }

    /// Save a position_id in the position cache for a given char_index in the text.
    /// This can be used to position other elements relative to a char in the text element.
    pub fn with_saved_char_position(mut self, char_index: usize, position_id: String) -> Self {
        self.saved_char_positions.push(SavedCharPositionIds {
            char_index,
            position_id,
        });
        self
    }

    /// Returns a text in which characters at indices are highlighted.
    /// Note that indices are char indices.
    pub fn with_single_highlight(mut self, highlight: Highlight, indices: Vec<usize>) -> Self {
        self.styles.push(Styles {
            indices,
            font_properties: highlight.properties,
            styles: highlight.text_style,
        });

        self
    }

    pub fn with_highlights(
        mut self,
        sorted_highlights: impl IntoIterator<Item = HighlightedRange>,
    ) -> Self {
        self.styles = sorted_highlights
            .into_iter()
            .map(|highlighted_range| Styles {
                indices: highlighted_range.highlight_indices,
                font_properties: highlighted_range.highlight.properties,
                styles: highlighted_range.highlight.text_style,
            })
            .collect();

        self
    }

    // Registers a callback that is called when a character in the given hoverable_char_range
    // is hovered or unhovered.
    pub fn with_hoverable_char_range<F>(
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

    /// Replace the text in the given byte range with the replacement text.
    pub fn replace_byte_range(&mut self, range: Range<usize>, replacement: &str) {
        if let Cow::Owned(ref mut owned_string) = self.text {
            owned_string.replace_range(range, replacement);
        } else {
            // If the text is borrowed, convert it to owned before modifying.
            let mut owned_string = self.text.clone().into_owned();
            owned_string.replace_range(range, replacement);
            self.text = Cow::Owned(owned_string);
        }
    }

    fn handle_mouse_down(
        &mut self,
        clicked_pos: &Vector2F,
        modifiers: &ModifiersState,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let is_covered = ctx.is_covered(Point::from_vec2f(
            *clicked_pos,
            self.z_index()
                .expect("z index should be set before dispatching"),
        ));
        if is_covered {
            return false;
        }
        let mut handled = false;
        // Note that these are all char indices!
        if let Some(clicked_char_idx) = self.get_char_index(clicked_pos) {
            self.click_handlers.iter_mut().for_each(|clickable_range| {
                if clickable_range.char_range.contains(&clicked_char_idx) {
                    let handler = clickable_range.click_handler.as_mut();
                    handler(modifiers, ctx, app);
                    handled = true;
                }
            })
        }
        handled
    }

    /// Given a position, returns the index of the character the position is over.
    /// Returns None if the position is not over any character.
    fn get_char_index(&self, position: &Vector2F) -> Option<usize> {
        let origin = self.origin?;
        let distance_x = position.x() - origin.x();
        match self.laid_out_text.clone() {
            LaidOutText::Frame(text_frame) => {
                let origin_y = origin.y();
                let mut line_height_from_origin = 0.;
                for line in text_frame.lines() {
                    line_height_from_origin += line.height();
                    let distance_y = position.y() - origin_y;
                    if distance_y >= 0. && distance_y <= line_height_from_origin {
                        return line.index_for_x(distance_x);
                    }
                }
                None
            }
            LaidOutText::Line(line) => {
                let distance_y = position.y() - origin.y();
                if distance_y < 0. || distance_y > line.height() {
                    return None;
                }
                line.index_for_x(distance_x)
            }
            _ => None,
        }
    }

    // Returns the bounding box of the character at the given index.
    fn get_char_bounding_box(&self, char_index: usize) -> Option<RectF> {
        let origin = self.origin?.xy();
        match &self.laid_out_text {
            LaidOutText::None => None,
            LaidOutText::Line(line) => {
                let glyph_width = line.width_for_index(char_index)?;
                let relative_x = line.x_for_index(char_index);
                Some(RectF::new(
                    origin + vec2f(relative_x, 0.),
                    vec2f(glyph_width, line.height()),
                ))
            }
            LaidOutText::Frame(frame) => {
                let mut relative_y = 0.;
                for line in frame.lines() {
                    let Some(glyph_width) = line.width_for_index(char_index) else {
                        relative_y += line.height();
                        continue;
                    };
                    let relative_x = line.x_for_index(char_index);
                    return Some(RectF::new(
                        origin + vec2f(relative_x, relative_y),
                        vec2f(glyph_width, line.height()),
                    ));
                }
                None
            }
        }
    }

    fn handle_mouse_moved(
        &mut self,
        mouse_pos: &Vector2F,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let is_covered = ctx.is_covered(Point::from_vec2f(
            *mouse_pos,
            self.z_index()
                .expect("z index should be set before dispatching"),
        ));
        let mut handled = false;
        let hovered_char_index = self.get_char_index(mouse_pos);
        let Some(z_index) = self.z_index() else {
            return false;
        };
        // Note that these are all char indices!
        self.hover_handlers.iter_mut().for_each(|hoverable_range| {
            let was_hovered = hoverable_range.mouse_state().is_hovered();
            let is_hovered = !is_covered
                && hovered_char_index.is_some_and(|hovered_char_index| {
                    hoverable_range.char_range.contains(&hovered_char_index)
                });
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
        });
        handled
    }

    // Autosize the text so that it fits within the bounds if the text doesn't fit
    // with the given font size. If the text does not fit with the minimum_font_size,
    // the text is not rendered.
    pub fn autosize_text(mut self, minimum_font_size: f32) -> Self {
        self.autosize_text = Some(minimum_font_size);
        self
    }

    pub fn add_text_with_highlights(
        &mut self,
        text: impl AsRef<str>,
        color: ColorU,
        font_properties: Properties,
    ) {
        let text = text.as_ref();
        let text_len = self.text.chars().count();

        self.styles.push(Styles {
            indices: (text_len..text_len + text.chars().count()).collect_vec(),
            font_properties,
            styles: TextStyle::new().with_foreground_color(color),
        });

        match &mut self.text {
            Cow::Borrowed(inner) => {
                let mut temp = inner.to_owned();
                temp.push_str(text);
                self.text = temp.into();
            }
            Cow::Owned(inner) => inner.push_str(text),
        }
    }

    pub fn with_style(mut self, font_properties: Properties) -> Self {
        self.font_properties = font_properties;
        self
    }

    /// Whether to soft wrap the text. If the text is soft-wrapped it will wrap onto a new line
    /// if there is not enough horizontal space to fit the text. If not soft-wrapped, the text will
    /// be positioned as if there were unlimited horizontal space.
    pub fn soft_wrap(mut self, soft_wrap: bool) -> Self {
        self.soft_wrap = soft_wrap;
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    fn line_height(&self) -> f32 {
        self.font_size * self.line_height_ratio
    }

    /// Determines rendering boundaries for drawing the given selection.
    /// Assumes that [`selection_start`] comes before [`selection_end`].
    fn calculate_selection_bounds(
        &self,
        content_origin: Vector2F,
        selection_start: TextSelectionBound,
        selection_end: TextSelectionBound,
    ) -> Vec<RectF> {
        let is_selection_empty = selection_start == selection_end;
        if is_selection_empty {
            return vec![];
        }

        let line_height = self.line_height();
        let mut selection_bounds = Vec::new();

        for row in selection_start.row..=selection_end.row {
            let line = match &self.laid_out_text {
                LaidOutText::None => return selection_bounds,
                LaidOutText::Line(line) => Some(line.borrow()),
                LaidOutText::Frame(frame) => frame.lines().get(row),
            };

            let Some(line) = line else {
                return selection_bounds;
            };

            let start_x = if row == selection_start.row {
                line.x_for_index(selection_start.glyph_index)
            } else {
                0.
            };
            let end_x = if row == selection_end.row {
                line.x_for_index(selection_end.glyph_index)
            } else {
                line.width
            };

            let rect_origin = content_origin + vec2f(start_x, row as f32 * line_height);
            let rect_size = vec2f(end_x - start_x, line_height);
            selection_bounds.push(RectF::new(rect_origin, rect_size));

            let is_last_line = match &self.laid_out_text {
                LaidOutText::Line(_) => row == 0,
                LaidOutText::Frame(frame) => row == frame.lines().len() - 1,
                LaidOutText::None => true,
            };
            let selection_crosses_newline = selection_crosses_newline_row_based(
                row,
                is_last_line,
                selection_start.row,
                selection_end.row,
                selection_end.glyph_index,
                line.last_index(),
            );
            if selection_crosses_newline {
                let tick_width = calculate_tick_width(self.font_size);
                let tick_origin = content_origin + vec2f(line.width, row as f32 * line_height);
                selection_bounds.push(create_newline_tick_rect(NewlineTickParams {
                    tick_origin,
                    tick_width,
                    tick_height: line_height,
                }));
            }
        }
        selection_bounds
    }

    /// Assumes that [`selection_start`] comes before [`selection_end`].
    fn draw_selection(
        &self,
        content_origin: Vector2F,
        selection_start: TextSelectionBound,
        selection_end: TextSelectionBound,
        ctx: &mut PaintContext,
    ) {
        if !self.is_selectable {
            return;
        }

        #[cfg(debug_assertions)]
        ctx.scene
            .set_location_for_panic_logging(self.constructor_location);

        for rect in self.calculate_selection_bounds(content_origin, selection_start, selection_end)
        {
            ctx.scene
                .draw_rect_without_hit_recording(rect)
                .with_background(Fill::Solid(self.text_selection_color));
        }
    }

    fn y_bound_to_row_index(&self, y_bound: f32) -> usize {
        (y_bound / (self.line_height_ratio * self.font_size)) as usize
    }

    /// Guarantees that any returned ranges are non-inverted (i.e. start before end)
    fn calculate_point_ranges(
        &self,
        current_selection: Option<Selection>,
    ) -> Option<Vec<(TextSelectionBound, TextSelectionBound)>> {
        let current_selection = current_selection?;
        let is_rect = current_selection.is_rect;
        let mut point_ranges = match is_rect {
            IsRect::False => {
                let start_point = self.position_for_point(current_selection.start);
                let end_point = self.position_for_point(current_selection.end);
                start_point.zip(end_point).map(|tuple| vec![tuple])
            }
            IsRect::True => self.calculate_row_bounds_for_rect_selection(
                current_selection.start,
                current_selection.end,
            ),
        };

        if let Some(ranges) = &mut point_ranges {
            for (start_point, end_point) in ranges {
                if end_point.glyph_index < start_point.glyph_index {
                    swap(start_point, end_point);
                }
            }
        }
        point_ranges
    }

    /// Given an absolute start and end position, calculate the bounds for each row in the text
    /// element for rect selection.
    fn calculate_row_bounds_for_rect_selection(
        &self,
        start: Vector2F,
        end: Vector2F,
    ) -> Option<Vec<(TextSelectionBound, TextSelectionBound)>> {
        let origin = self.origin()?;
        let size = self.size()?;

        let relative_start = start - origin.xy;
        let relative_end = end - origin.xy;

        // Early return if the selection range does not overlap with the element bounds.
        if relative_end.y() < 0. || relative_start.y() > size.y() {
            return None;
        }

        match &self.laid_out_text {
            LaidOutText::None => None,
            LaidOutText::Line(line) => {
                let start_bound = TextSelectionBound {
                    row: 0,
                    glyph_index: line.caret_index_for_x_unbounded(relative_start.x()),
                };
                let end_bound = TextSelectionBound {
                    row: 0,
                    glyph_index: line.caret_index_for_x_unbounded(relative_end.x()),
                };
                Some(vec![(start_bound, end_bound)])
            }
            LaidOutText::Frame(frame) => {
                let start_index = self.y_bound_to_row_index(relative_start.y());
                let end_index = self.y_bound_to_row_index(relative_end.y());

                let mut rows = Vec::new();
                let lines = frame.lines();

                // Construct the start and end text selection bounds for each row.
                for index in start_index..=end_index {
                    let bounds = lines.get(index).map(|line| {
                        let start_bound = TextSelectionBound {
                            row: index,
                            glyph_index: line.caret_index_for_x_unbounded(relative_start.x()),
                        };
                        let end_bound = TextSelectionBound {
                            row: index,
                            glyph_index: line.caret_index_for_x_unbounded(relative_end.x()),
                        };
                        (start_bound, end_bound)
                    });

                    match bounds {
                        Some(bounds) => rows.push(bounds),
                        None => break,
                    };
                }

                Some(rows)
            }
        }
    }

    /// Given an absolute point, returns the row and glyph index that makes the most sense for a
    /// caret position. For example, with example text "here is example text",
    /// if the mouse point is over |i|, the index could be either 5 or 6
    /// depending on which side of the |i| the mouse is closest to.
    /// This snaps the point to somewhere in bounds.
    fn position_for_point(&self, absolute_point: Vector2F) -> Option<TextSelectionBound> {
        let (Some(origin), Some(size)) = (self.origin(), self.size()) else {
            return None;
        };

        let relative_point = absolute_point - origin.xy;

        // Snap to the first or last character if we are above or below the text
        if relative_point.y() < 0. {
            (!matches!(&self.laid_out_text, LaidOutText::None)).then_some(TextSelectionBound {
                row: 0,
                glyph_index: 0,
            })
        } else if relative_point.y() > size.y() {
            let row_and_glyph_index = match &self.laid_out_text {
                LaidOutText::None => None,
                LaidOutText::Line(line) => Some((0usize, line.end_index())),
                LaidOutText::Frame(frame) => frame
                    .lines()
                    .last()
                    .map(|line| (frame.lines().len() - 1usize, line.end_index())),
            };
            row_and_glyph_index.map(|(row, glyph_index)| TextSelectionBound { row, glyph_index })
        } else {
            let (row_index, line) = match &self.laid_out_text {
                LaidOutText::None => None,
                LaidOutText::Line(line) => Some((0, line.borrow())),
                LaidOutText::Frame(frame) => {
                    let row_index = self.y_bound_to_row_index(relative_point.y());
                    frame.lines().get(row_index).map(|line| (row_index, line))
                }
            }?;

            Some(TextSelectionBound {
                row: row_index,
                glyph_index: line.caret_index_for_x_unbounded(relative_point.x()),
            })
        }
    }

    /// Converts the text selection bound to the start of the line if line_start is true,
    /// and the end of the line otherwise.
    fn translate_selection_bound_to_line_bound(
        &self,
        bound: TextSelectionBound,
        line_start: bool,
    ) -> Option<Vector2F> {
        let line_height = self.line_height();
        let origin = self.origin()?;
        let line = match &self.laid_out_text {
            LaidOutText::None => return None,
            LaidOutText::Line(line) => Some(line.borrow()),
            LaidOutText::Frame(frame) => frame.lines().get(bound.row),
        }?;

        let relative_x = if line_start { 0. } else { line.width };

        Some(origin.xy + vec2f(relative_x, bound.row as f32 * line_height))
    }

    pub fn with_selectable(mut self, is_selectable: bool) -> Self {
        self.is_selectable = is_selectable;
        self
    }

    /// Return the substring of text from start_glyph_index to end_glyph_index (end exclusive).
    fn text_for_index_range(
        &self,
        mut start_glyph_index: usize,
        mut end_glyph_index: usize,
    ) -> Option<&str> {
        if start_glyph_index == end_glyph_index {
            return None;
        }

        if start_glyph_index > end_glyph_index {
            // Ensure the start point is before the end point.
            std::mem::swap(&mut start_glyph_index, &mut end_glyph_index);
        }

        let text = self.text();
        // Convert the `TextSelectionBound`'s glyph_index's to byte indices to slice the text by
        // byte offsets. glyph_index is the index of the character in the string, while byte index
        // is in terms of bytes, which matters since UTF-8 glyphs have variable byte widths (for
        // instance a simple ASCII char is a single byte while an emoji is multiple bytes).
        let (start_byte_index, _) = text.char_indices().nth(start_glyph_index)?;
        if end_glyph_index == text.char_indices().count() {
            Some(&text[start_byte_index..])
        } else {
            let end_byte_index = text
                .char_indices()
                .nth(end_glyph_index)
                .map(|(offset, _)| offset)?;
            Some(&text[start_byte_index..end_byte_index])
        }
    }
}

impl Element for Text {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let text_len = self.text.chars().count();
        let mut styles = vec![];
        let mut prev_index = 0;

        for style in &self.styles {
            let mut pending_style: Option<(Range<usize>, TextStyle)> = None;

            for ix in &style.indices {
                if let Some((style_range, text_style)) = pending_style.as_mut() {
                    // Extend the range if the current index is consecutive from the last.
                    if *ix == style_range.end {
                        style_range.end += 1;
                    } else {
                        // If the current index is not consecutive from the last,
                        // we push the pending highlight to styles and colors and fill
                        // the gap with default font id and color.
                        styles.push((
                            style_range.clone(),
                            StyleAndFont::new(self.family_id, style.font_properties, *text_style),
                        ));
                        styles.push((
                            style_range.end..*ix,
                            StyleAndFont::new(
                                self.family_id,
                                self.font_properties,
                                TextStyle::new(),
                            ),
                        ));
                        *style_range = *ix..*ix + 1;
                    }
                } else {
                    // Fill the gap between highlights with default font id and color.
                    styles.push((
                        prev_index..*ix,
                        StyleAndFont::new(self.family_id, self.font_properties, TextStyle::new()),
                    ));
                    pending_style = Some((*ix..*ix + 1, style.styles));
                }
                prev_index = *ix + 1;
            }

            if let Some((style_range, text_style)) = pending_style.as_mut() {
                styles.push((
                    style_range.clone(),
                    StyleAndFont::new(self.family_id, style.font_properties, *text_style),
                ));
            } else {
                styles.push((
                    0..prev_index,
                    StyleAndFont::new(self.family_id, self.font_properties, TextStyle::new()),
                ));
            }
        }

        if text_len > prev_index {
            styles.push((
                prev_index..text_len,
                StyleAndFont::new(self.family_id, self.font_properties, TextStyle::new()),
            ));
        }

        let max_width = constraint.max_along(Axis::Horizontal);
        let max_height = constraint.max_along(Axis::Vertical);

        let text_frame = if self.soft_wrap {
            let frame = ctx.text_layout_cache.layout_text(
                &self.text,
                LineStyle {
                    font_size: self.font_size,
                    line_height_ratio: self.line_height_ratio,
                    baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                    fixed_width_tab_size: None,
                },
                styles.as_slice(),
                max_width,
                max_height,
                Default::default(),
                None,
                &app.font_cache().text_layout_system(),
            );
            LaidOutText::Frame(frame)
        } else {
            let single_line = ctx.text_layout_cache.layout_line(
                &self.text,
                LineStyle {
                    font_size: self.font_size,
                    line_height_ratio: self.line_height_ratio,
                    baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                    fixed_width_tab_size: None,
                },
                styles.as_slice(),
                max_width,
                self.clip_config,
                &app.font_cache().text_layout_system(),
            );

            // Don't render the line if it won't fit.
            // Here we are using EPSILON from pathfinder because the margin of error
            // in the floating point calculation in pathfinder is larger than standard
            // f32. For example, adding 15.756032 to 12 in Vector2F will result in 27.756031
            // -- a 1e-6 error which is larger than f32::EPSILON (1e-7). The reason for
            // this problem in pathfinder could be because internally Vector2F is representing
            // f32 using u64 instead of the native type.
            if (single_line.height() - max_height) > EPSILON {
                let mut state = LaidOutText::None;

                if let Some(minimum_size) = self.autosize_text {
                    let mut size = minimum_size;
                    while size < self.font_size {
                        let line = ctx.text_layout_cache.layout_line(
                            &self.text,
                            LineStyle {
                                font_size: size,
                                line_height_ratio: self.line_height_ratio,
                                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                                fixed_width_tab_size: None,
                            },
                            styles.as_slice(),
                            max_width,
                            self.clip_config,
                            &app.font_cache().text_layout_system(),
                        );

                        if (line.height() - max_height) > EPSILON {
                            break;
                        } else {
                            state = LaidOutText::Line(line);
                        }

                        size += 1.;
                    }
                }

                state
            } else {
                LaidOutText::Line(single_line)
            }
        };

        let size = vec2f(
            text_frame
                .width()
                .max(constraint.min.x())
                .min(constraint.max.x()),
            text_frame.height(),
        );

        self.laid_out_text = text_frame;
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let bounds = RectF::from_points(
            origin,
            origin
                + self
                    .size()
                    .expect("layout() should have been called before paint()"),
        );
        for saved_char_position in &self.saved_char_positions {
            let Some(char_bounding_box) =
                self.get_char_bounding_box(saved_char_position.char_index)
            else {
                continue;
            };
            ctx.position_cache.cache_position_indefinitely(
                saved_char_position.position_id.clone(),
                char_bounding_box,
            );
        }
        self.laid_out_text.paint(
            bounds,
            self.text_color,
            ctx.scene,
            app.font_cache(),
            self.compute_baseline_position_fn.as_ref(),
        );

        if let Some(ranges) = self.calculate_point_ranges(ctx.current_selection) {
            for (start_point, end_point) in ranges {
                self.draw_selection(origin, start_point, end_point, ctx);
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
            Some(Event::LeftMouseDown {
                position,
                modifiers,
                click_count,
                ..
            }) => {
                if *click_count == 1 {
                    self.handle_mouse_down(position, modifiers, ctx, app);
                }
            }
            Some(Event::MouseMoved { position, .. }) => {
                self.handle_mouse_moved(position, ctx, app);
            }
            Some(Event::LeftMouseDragged { position, .. }) => {
                self.handle_mouse_moved(position, ctx, app);
            }
            _ => (),
        };

        // Always propagate events to parent, since for now this is the only behavior we want.
        // We may need to make this configurable in the future.
        false
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        if self.is_selectable {
            Some(self as &dyn SelectableElement)
        } else {
            None
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        Some(self.text.to_string())
    }
}

impl SelectableElement for Text {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        let text = match is_rect {
            // If the active selection is not a rect selection, directly return the substring of the text from selection_start
            // to selection_end.
            IsRect::False => {
                let start_glyph_index = self.position_for_point(selection_start)?.glyph_index;
                let end_glyph_index = self.position_for_point(selection_end)?.glyph_index;

                self.text_for_index_range(start_glyph_index, end_glyph_index)?
                    .to_owned()
            }
            // If the active selection is a rect selection, first calculate the rows covered by this selection. Then for each
            // selection, find its corresponding substring. They are then concatenated together in the end.
            // Note that since we are already joining each fragment with \n, we should strip away any active trailing newline
            // in each text fragment.
            IsRect::True => {
                let selection_bounds =
                    self.calculate_row_bounds_for_rect_selection(selection_start, selection_end)?;
                selection_bounds
                    .into_iter()
                    .filter_map(|(start, end)| {
                        self.text_for_index_range(start.glyph_index, end.glyph_index)
                            .map(|s| s.trim_end_matches('\n'))
                    })
                    .join("\n")
            }
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
        // We never need to do expansion for Char units.
        if matches!(unit, SelectionType::Simple | SelectionType::Rect) {
            return None;
        }
        let bound = self.bounds()?;
        // If we're above the bound and expanding backward, expand to the origin.
        // This is needed to render the selection to the beginning of the element
        // if the mouse goes above the element.
        if absolute_point.y() < bound.min_y() {
            return match direction {
                SelectionDirection::Backward => Some(bound.origin()),
                SelectionDirection::Forward => None,
            };
        }
        // If we're below the bound and expanding forward, we really want
        // to expand to the lower right corner of the bound,
        // but selection rendering is not inclusive of the lower right corner,
        // so we return the original point below the bound instead.
        if absolute_point.y() > bound.max_y() {
            return match direction {
                SelectionDirection::Backward => None,
                SelectionDirection::Forward => Some(absolute_point),
            };
        }
        // If we're outside the x bounds, return None. This prevents cells in the same row
        // (which share Y bounds) from all responding to a double-click meant for one cell.
        if absolute_point.x() < bound.min_x() || absolute_point.x() > bound.max_x() {
            return None;
        }
        match unit {
            SelectionType::Simple | SelectionType::Rect => None,
            SelectionType::Semantic => {
                let text = self.text();
                // TODO (roland): if the mouse is over a character, position_for_point could put us to the left or
                // right of the character depending which side it's closer to. For semantic selections, if we're expanding to start
                // we always want it to the right of the character, and if we're expanding to the end we want it on the left.
                // For instance, given some text "first |m|iddl|e| second",
                // if we double click anywhere on "m" or "e", we should expand to select "middle".
                // Currently if we double click on the left side of "m", all of "first middle" would be selected,
                // and if we double click on the right side of "e" all of "middle second" would be selected.
                let text_selection_bound = self.position_for_point(absolute_point)?;
                let inner_point = if matches!(direction, SelectionDirection::Backward) {
                    text.word_starts_backward_from_offset_exclusive(CharOffset::from(
                        text_selection_bound.glyph_index,
                    ))
                    .ok()?
                    .with_policy(word_boundaries_policy)
                    .next()?
                } else {
                    text.word_ends_from_offset_exclusive(CharOffset::from(
                        text_selection_bound.glyph_index,
                    ))
                    .ok()?
                    .with_policy(word_boundaries_policy)
                    .next()?
                };

                let offset = text.to_offset(inner_point).ok()?.as_usize();
                let origin = self.origin()?.xy;
                match &self.laid_out_text {
                    LaidOutText::None => None,
                    LaidOutText::Line(line) => {
                        let relative_x = line.x_for_index(offset);
                        Some(vec2f(origin.x() + relative_x, absolute_point.y()))
                    }
                    LaidOutText::Frame(frame) => {
                        // Get row that we initially clicked on. Semantic selection should only
                        // expand within this row.
                        // We need to do this because the same char offset in the raw text buffer
                        // could be either the end of one line or the start of the next line.
                        let line = frame.lines().get(text_selection_bound.row)?;
                        let first_glyph = line.first_glyph()?;
                        let last_glyph = line.last_glyph()?;

                        // If we're expanding to the end of the line, we can have offset == last_glyph.index + 1.
                        let relative_x =
                            if first_glyph.index <= offset && offset <= last_glyph.index + 1 {
                                line.x_for_index(offset)
                            } else if matches!(direction, SelectionDirection::Backward) {
                                0.
                            } else {
                                line.width
                            };
                        Some(vec2f(origin.x() + relative_x, absolute_point.y()))
                    }
                }
            }
            SelectionType::Lines => {
                let text_selection_bound = self.position_for_point(absolute_point)?;
                let mut snap_to = self.translate_selection_bound_to_line_bound(
                    text_selection_bound,
                    matches!(direction, SelectionDirection::Backward),
                )?;
                snap_to.set_y(absolute_point.y());
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
            // If neither point is within the bounds, we don't have enough information
            // to tell which point is semantically before the other. This is because y value
            // alone is not sufficient, since a range of y values can map to the same line.
            (false, false) => None,
            (false, true) => {
                if absolute_point_1.y() < bounds.min_y() {
                    return Some(true);
                } else if absolute_point_1.y() > bounds.max_y() {
                    return Some(false);
                }
                Some(absolute_point_1.x() < bounds.min_x())
            }
            (true, false) => {
                if absolute_point_2.y() < bounds.min_y() {
                    return Some(false);
                } else if absolute_point_2.y() > bounds.max_y() {
                    return Some(true);
                }
                Some(absolute_point_2.x() > bounds.max_x())
            }
            (true, true) => {
                let point_1 = self.position_for_point(absolute_point_1)?;
                let point_2 = self.position_for_point(absolute_point_2)?;
                Some(
                    point_1.row < point_2.row
                        || (point_1.row == point_2.row
                            && point_1.glyph_index < point_2.glyph_index)
                        || (point_1.row == point_2.row
                            && point_1.glyph_index == point_2.glyph_index
                            && absolute_point_1.x() < absolute_point_2.x()),
                )
            }
        }
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        smart_select_fn: super::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        let bound = self.bounds()?;
        if bound.min_y() > absolute_point.y() || absolute_point.y() > bound.max_y() {
            return None;
        }
        if bound.min_x() > absolute_point.x() || absolute_point.x() > bound.max_x() {
            return None;
        }
        let origin = self.origin()?.xy;
        let text = self.text();
        let text_selection_bound = self.position_for_point(absolute_point)?;
        match &self.laid_out_text {
            LaidOutText::None => None,
            LaidOutText::Line(line) => {
                let char_offset = text_selection_bound.glyph_index;
                let byte_offset = text.char_indices().nth(char_offset)?.0.into();

                let smart_select_range = smart_select_fn(text, byte_offset)?;
                // convert to glyph (char) index
                let smart_select_start =
                    text[..smart_select_range.start.as_usize()].chars().count();
                let smart_select_end = text[..smart_select_range.end.as_usize()].chars().count();

                let start_relative_x = line.x_for_index(smart_select_start);
                let end_relative_x = line.x_for_index(smart_select_end);
                Some((
                    vec2f(origin.x() + start_relative_x, absolute_point.y()),
                    vec2f(origin.x() + end_relative_x, absolute_point.y()),
                ))
            }
            LaidOutText::Frame(frame) => {
                // Get row that we initially clicked on.
                // We need to do this because the same char offset in the raw text buffer
                // could be either the end of one line or the start of the next line.
                let line = frame.lines().get(text_selection_bound.row)?;
                let first_glyph = line.first_glyph()?;
                let last_glyph = line.last_glyph()?;
                // If we clicked to the right of a line, the text_selection_bound's glyph index would be one larger than the last glyph's index.
                // Snap it within the line's index range so we do smart selection as if we clicked on the last glyph in the line.
                let char_offset = text_selection_bound
                    .glyph_index
                    .min(last_glyph.index)
                    .max(first_glyph.index);
                let byte_offset = text.char_indices().nth(char_offset)?.0.into();

                let smart_select_range = smart_select_fn(text, byte_offset)?;
                // convert to glyph (char) index
                let smart_select_start =
                    text[..smart_select_range.start.as_usize()].chars().count();
                let smart_select_end = text[..smart_select_range.end.as_usize()].chars().count();
                // After smart selection, the start and end offsets can be on different lines if a word wrapped.
                // Find the lines for the start and end offsets.
                let start_row = frame.row_within_frame(smart_select_start, false);
                let end_row = frame.row_within_frame(smart_select_end, true);
                let start_line = frame.lines().get(start_row)?;
                let end_line = frame.lines().get(end_row)?;

                let start_relative_x = start_line.x_for_index(smart_select_start);
                let end_relative_x = end_line.x_for_index(smart_select_end);
                // We subtract half the line's height to put the position in the center of the line.
                let start_relative_y = frame.height_up_to_row(start_row) - start_line.height() / 2.;
                let end_relative_y = frame.height_up_to_row(end_row) - end_line.height() / 2.;
                Some((
                    vec2f(origin.x() + start_relative_x, origin.y() + start_relative_y),
                    vec2f(origin.x() + end_relative_x, origin.y() + end_relative_y),
                ))
            }
        }
    }

    fn calculate_clickable_bounds(&self, current_selection: Option<Selection>) -> Vec<RectF> {
        let Some(content_origin) = self.origin else {
            return vec![];
        };

        self.calculate_point_ranges(current_selection)
            .unwrap_or_default()
            .iter()
            .flat_map(|(start_point, end_point)| {
                self.calculate_selection_bounds(content_origin.xy(), *start_point, *end_point)
            })
            .collect()
    }
}

impl PartialClickableElement for Text {
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
        self.replace_byte_range(range.byte_range, &replacement);
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
