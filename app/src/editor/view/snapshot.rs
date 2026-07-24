use core::f32;
use std::borrow::Cow;
use std::cmp::{self};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use instant::Instant;
use parking_lot::Mutex;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use rayon::prelude::*;
use string_offset::ByteOffset;
use warp_completer::completer::Description;
use warp_errors::report_error;
use warpui::fonts::{Cache as FontCache, FamilyId, Properties};
use warpui::platform::LineStyle;
use warpui::text::point::Point;
use warpui::text_layout::{
    self, ClipConfig, ComputeBaselinePositionFn, DEFAULT_TOP_BOTTOM_RATIO, LayoutCache,
    StyleAndFont, TextAlignment, TextStyle, default_compute_baseline_position_fn,
};
use warpui::{AppContext, EntityId, ModelHandle};

use super::model::EditorModel;
use super::{
    AutosuggestionLocation, AutosuggestionState, AutosuggestionType,
    BaselinePositionComputationMethod, Bias, DisplayPoint, DrawableSelection, ScrollState,
    ToBufferOffset, ToCharOffset, ToDisplayPoint, ToPoint,
};
use crate::editor::soft_wrap::FrameLayouts;
#[cfg(feature = "voice_input")]
use crate::editor::view::voice::VoiceInputState;
use crate::terminal::grid_size_util::grid_compute_baseline_position_fn;

/// Ratio to calculate font size of cursor avatar.
/// Found experimentally to scale the best proportionally with
/// current font size and the avatar's size.
pub const CURSOR_AVATAR_FONT_RATIO: f32 = 0.8;
/// Offset to calculate size of cursor avatar.
/// Found experimentally to look the best with current font size.
pub const CURSOR_AVATAR_IMAGE_OFFSET: f32 = 4.;

/// Fudge factor to make the voice input icon slightly wider than it is tall.
const VOICE_INPUT_ICON_IMAGE_OFFSET_X: f32 = 5.;

/// Minimum size of voice input icon.
const MIN_VOICE_INPUT_ICON_SIZE: f32 = 16.;

/// Gap between voice input icon's botton and cursor's top.
pub const VOICE_INPUT_ICON_CURSOR_GAP: f32 = 2.;

/// The amount of time the editor height must have remained shrunken
/// before we actually shrink the height. This is to prevent jittering
/// before an autosuggestion is computed on keypress, if the autosuggestion would wrap
/// and cause the editor height to grow.
const EDITOR_HEIGHT_SHRINK_DELAY: Duration = Duration::from_millis(25);

/// A read-only snapshot of the [`EditorView`] that is needed
/// to render the [`EditorElement`].
pub struct ViewSnapshot {
    pub view_id: EntityId,
    pub is_focused: bool,
    pub editor_model: ModelHandle<EditorModel>,

    pub can_select: bool,

    pub font_size: f32,
    pub font_family: FamilyId,
    pub placeholder_font_family: FamilyId,
    pub font_properties: Properties,
    pub line_height: f32,
    pub line_height_ratio: f32,
    pub em_width: f32,

    pub autogrow: bool,
    pub is_empty: bool,

    /// Map from prefix to placeholder text. The view can have multiple active prefixes that when
    /// exactly matched in the buffer cause additional placeholder ghost text to be displayed. The
    /// empty string prefix "" is the default placeholder (shown when buffer is empty).
    pub placeholder_texts: Arc<HashMap<String, String>>,

    pub autosuggestion_state: Option<Arc<AutosuggestionState>>,
    pub command_xray: Option<Arc<Description>>,

    pub cached_buffer_points: HashMap<Cow<'static, str>, Point>,

    pub baseline_position_computation_method: BaselinePositionComputationMethod,

    #[cfg(feature = "voice_input")]
    pub voice_input_state: VoiceInputState,

    pub editor_height_shrink_delay: Arc<Mutex<EditorHeightShrinkDelay>>,
}

/// A struct to hold the editor height before it was shrunk and the time it was first shrunk.
/// This is used to delay shrinking the editor height until after it's remained shrunken for EDITOR_HEIGHT_SHRINK_DELAY.
pub struct EditorHeightShrinkDelay {
    pub editor_height_before_shrink: f32,
    pub editor_height_shrink_start: Option<Instant>,
}

impl ViewSnapshot {
    /// Returns the editor height with the EDITOR_HEIGHT_SHRINK_DELAY applied, and updates internal state.
    pub fn get_editor_height_with_shrink_delay(&self, editor_height: f32) -> f32 {
        let mut editor_height_shrink_delay = self.editor_height_shrink_delay.lock();
        // If the editor_height stayed the same or grew, use editor_height and reset the shrink time.
        if editor_height >= editor_height_shrink_delay.editor_height_before_shrink {
            editor_height_shrink_delay.editor_height_before_shrink = editor_height;
            editor_height_shrink_delay.editor_height_shrink_start = None;
            return editor_height;
        }
        // From here we know the editor height shrank.
        // If there's no start time recorded, this is the first render where the editor height shrunk.
        // Record the current time but use the height before shrinking.
        let Some(editor_height_shrink_start) =
            editor_height_shrink_delay.editor_height_shrink_start
        else {
            editor_height_shrink_delay.editor_height_shrink_start = Some(Instant::now());
            return editor_height_shrink_delay.editor_height_before_shrink;
        };

        // If the height has been shrunken for a while, use the latest shrunken height.
        // It is our new baseline.
        if editor_height_shrink_start.elapsed() > EDITOR_HEIGHT_SHRINK_DELAY {
            editor_height_shrink_delay.editor_height_before_shrink = editor_height;
            editor_height_shrink_delay.editor_height_shrink_start = None;
            editor_height
        } else {
            editor_height_shrink_delay.editor_height_before_shrink
        }
    }

    /// Returns the time at which the editor height should be repainted with the shrunken editor height.
    pub fn get_editor_repaint_at(&self) -> Option<Instant> {
        self.editor_height_shrink_delay
            .lock()
            .editor_height_shrink_start
            .map(|start| start + EDITOR_HEIGHT_SHRINK_DELAY)
    }

    pub fn baseline_position_fn(&self) -> ComputeBaselinePositionFn {
        // Copy the font family ID so that it can be moved into the closure.
        let font_family = self.font_family;
        match self.baseline_position_computation_method {
            BaselinePositionComputationMethod::Grid => {
                grid_compute_baseline_position_fn(font_family)
            }
            BaselinePositionComputationMethod::Default => default_compute_baseline_position_fn(),
        }
    }

    /// Returns the placeholder text for the prefix that matches the current buffer content.
    pub fn matching_placeholder_text(&self, buffer_text: &str) -> Option<String> {
        // Find exact match - buffer content must equal the prefix exactly
        self.placeholder_texts
            .iter()
            .find(|(prefix, _)| buffer_text == prefix.as_str())
            .map(|(_, text)| text.clone())
    }

    pub fn placeholder_text_exists(&self) -> bool {
        // Only consider the default placeholder (empty prefix) for "exists" checks.
        // This is used when laying out the input and determining input height.
        self.placeholder_texts.contains_key("")
    }

    pub fn is_selecting(&self, app: &AppContext) -> bool {
        self.editor_model.as_ref(app).is_selecting(app)
    }

    pub fn rightmost_point(&self, app: &AppContext) -> DisplayPoint {
        self.editor_model
            .as_ref(app)
            .display_map(app)
            .rightmost_point()
    }

    pub fn max_point(&self, app: &AppContext) -> DisplayPoint {
        self.editor_model.as_ref(app).max_point(app)
    }

    pub fn autosuggestion_location(&self) -> Option<AutosuggestionLocation> {
        self.autosuggestion_state
            .as_ref()
            .map(|state| state.location)
    }

    pub fn active_autosuggestion(&self) -> bool {
        self.autosuggestion_state
            .as_ref()
            .is_some_and(|s| s.is_active())
    }

    pub fn active_next_command_suggestion(&self) -> bool {
        self.autosuggestion_state.as_ref().is_some_and(|s| {
            s.is_active()
                && matches!(
                    s.autosuggestion_type,
                    AutosuggestionType::Command {
                        was_intelligent_autosuggestion: true
                    }
                )
        })
    }

    /// Lays out the given ghosted text, which can be a placeholder or autosuggestion.
    fn layout_ghosted_text(
        &self,
        text: &str,
        size: &Vector2F,
        soft_wrap: bool,
        first_line_head_indent: Option<f32>,
        font_cache: &FontCache,
        layout_cache: &LayoutCache,
    ) -> Vec<Arc<text_layout::Line>> {
        let font_size = self.font_size;

        if soft_wrap {
            let text_frame = layout_cache.layout_text(
                text,
                LineStyle {
                    font_size: self.font_size,
                    line_height_ratio: self.line_height_ratio,
                    baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                    fixed_width_tab_size: None,
                },
                &[(
                    0..text.chars().count(),
                    StyleAndFont::new(
                        self.placeholder_font_family,
                        self.font_properties,
                        TextStyle::new(),
                    ),
                )],
                size.x(),
                f32::MAX,
                Default::default(),
                first_line_head_indent,
                &font_cache.text_layout_system(),
            );

            // Return a vec of lines to be backward compatible with laying out placeholder
            // without soft_wrapping and autosuggestion. In the future we may want to
            // return a TextFrame here so we don't have to clone the text_frame lines
            text_frame
                .lines()
                .iter()
                .map(|l| Arc::new(l.clone()))
                .collect()
        } else {
            text.lines()
                .map(|line| {
                    layout_cache.layout_line(
                        line,
                        LineStyle {
                            font_size,
                            line_height_ratio: self.line_height_ratio,
                            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                            fixed_width_tab_size: None,
                        },
                        &[(
                            0..line.chars().count(),
                            StyleAndFont::new(
                                self.placeholder_font_family,
                                self.font_properties,
                                TextStyle::new(),
                            ),
                        )],
                        f32::MAX,
                        ClipConfig::default(),
                        &font_cache.text_layout_system(),
                    )
                })
                .collect()
        }
    }

    pub fn layout_autosuggestion(
        &self,
        preceding_text_width: f32,
        font_cache: &FontCache,
        layout_cache: &LayoutCache,
        size: &Vector2F,
        soft_wrap: bool,
    ) -> Vec<Arc<text_layout::Line>> {
        // The autosuggestion is laid out on the same line as editor text and then soft wraps.
        // This means the first line of the autosuggestion has preceding_text_width less width to use before it needs to wrap.
        self.autosuggestion_state
            .as_ref()
            .and_then(|state| state.current_autosuggestion_text.as_ref())
            .map(|current_autosuggestion_text| {
                self.layout_ghosted_text(
                    current_autosuggestion_text,
                    size,
                    soft_wrap,
                    Some(preceding_text_width),
                    font_cache,
                    layout_cache,
                )
            })
            .unwrap_or_default()
    }

    /// Layout placeholder text with an optional indent for the first line.
    pub fn layout_placeholder_text(
        &self,
        placeholder_text: &str,
        first_line_indent: f32,
        font_cache: &FontCache,
        layout_cache: &LayoutCache,
        size: &Vector2F,
        soft_wrap: bool,
    ) -> Vec<Arc<text_layout::Line>> {
        self.layout_ghosted_text(
            placeholder_text,
            size,
            soft_wrap,
            if first_line_indent > 0. {
                Some(first_line_indent)
            } else {
                None
            },
            font_cache,
            layout_cache,
        )
    }

    pub(super) fn clamp_scroll_left(&self, scroll_state: &ScrollState, max: f32) {
        let mut scroll_position = scroll_state.scroll_position.lock();
        let scroll_left = scroll_position.x();
        scroll_position.set_x(scroll_left.min(max));
    }

    pub fn max_scroll_top(total_lines: f32, visible_lines: f32) -> f32 {
        (total_lines - visible_lines).max(0.)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn autoscroll_horizontally(
        &self,
        scroll_state: &ScrollState,
        start_row: u32,
        viewport_width: f32,
        scroll_width: f32,
        max_glyph_width: f32,
        layouts: Vec<&text_layout::Line>,
        app: &AppContext,
    ) {
        let map = self.editor_model.as_ref(app).display_map(app);

        let mut target_left = f32::INFINITY;
        let mut target_right = 0.0_f32;
        for selection in self.editor_model.as_ref(app).selections(app) {
            let head = selection.head().to_display_point(map, app).unwrap();
            let start_column = head.column().saturating_sub(3);
            let end_column = cmp::min(map.line_len(head.row(), app).unwrap(), head.column() + 3);

            if let Some(line) = layouts.get((head.row() - start_row) as usize) {
                target_left = target_left.min(line.x_for_index(start_column as usize));
            }
            if let Some(line) = layouts.get((head.row() - start_row) as usize) {
                target_right =
                    target_right.max(line.x_for_index(end_column as usize) + max_glyph_width);
            }
        }
        target_right = target_right.min(scroll_width);

        if target_right - target_left > viewport_width {
            return;
        }

        let mut scroll_position = scroll_state.scroll_position.lock();
        let scroll_left = scroll_position.x() * max_glyph_width;
        let scroll_right = scroll_left + viewport_width;

        if target_left < scroll_left {
            scroll_position.set_x(target_left / max_glyph_width);
        } else if target_right > scroll_right {
            scroll_position.set_x((target_right - viewport_width) / max_glyph_width);
        }
    }

    pub(super) fn autoscroll_vertically(
        &self,
        scroll_state: &ScrollState,
        total_lines: f32,
        visible_lines: f32,
        top_section_height_lines: f32,
        frame_layouts: &FrameLayouts,
        app: &AppContext,
    ) -> bool {
        let mut scroll_position = scroll_state.scroll_position.lock();
        let scroll_top = scroll_position.y();
        scroll_position.set_y(scroll_top.min(total_lines - visible_lines).max(0.));

        let mut autoscroll_requested = scroll_state.autoscroll_requested.lock();
        if *autoscroll_requested {
            *autoscroll_requested = false;
        } else {
            return false;
        }

        let map = self.editor_model.as_ref(app).display_map(app);

        let first_selection = self.editor_model.as_ref(app).first_selection(app);
        let first_selection_clamp_direction = first_selection.clamp_direction;
        let first_cursor = first_selection.head().to_display_point(map, app);

        let first_cursor_top = match first_cursor {
            Ok(first_cursor) => {
                match frame_layouts
                    .to_soft_wrap_point(first_cursor, first_selection_clamp_direction)
                {
                    Some(point) => point.row() as f32 + top_section_height_lines,
                    None => {
                        report_error!("Failed to get softwrapped point from display point");
                        return false;
                    }
                }
            }
            Err(err) => {
                report_error!(err.context("Error trying to turn selection into display point"));
                return false;
            }
        };

        let last_selection = self.editor_model.as_ref(app).last_selection(app);
        let last_selection_clamp_direction = last_selection.clamp_direction;
        let last_cursor = last_selection.head().to_display_point(map, app);

        let last_cursor_bottom = match last_cursor {
            Ok(last_cursor) => {
                match frame_layouts.to_soft_wrap_point(last_cursor, last_selection_clamp_direction)
                {
                    Some(point) => point.row() as f32 + 1.0 + top_section_height_lines,
                    None => {
                        report_error!("Failed to get softwrapped point from display point");
                        return false;
                    }
                }
            }
            Err(err) => {
                report_error!(err.context("Error trying to turn selection into display point"));
                return false;
            }
        };

        let margin = ((visible_lines - (last_cursor_bottom - first_cursor_top)) / 2.0)
            .floor()
            .min(3.0);
        if margin < 0.0 {
            return false;
        }

        let target_top = (first_cursor_top - margin).max(0.0);
        let target_bottom = last_cursor_bottom + margin;
        let start_row = scroll_position.y();
        let end_row = start_row + visible_lines;

        if target_top < start_row {
            scroll_position.set_y(target_top.min(Self::max_scroll_top(total_lines, visible_lines)));
        } else if target_bottom >= end_row {
            scroll_position.set_y(
                (target_bottom - visible_lines)
                    .min(Self::max_scroll_top(total_lines, visible_lines)),
            );
        }
        true
    }

    /// If the range of rows passed does not exist, we just layout the rows that do exist
    /// within the provided range.
    pub fn layout_text_frames(
        &self,
        mut rows: Range<u32>,
        layout_cache: &LayoutCache,
        max_width: f32,
        left_notch_width_px: f32,
        app: &AppContext,
    ) -> Result<Vec<Arc<text_layout::TextFrame>>> {
        let display_map = self.editor_model.as_ref(app).display_map(app);

        rows.end = cmp::min(rows.end, display_map.max_point(app).row() + 1);
        if rows.start >= rows.end {
            return Ok(Vec::new());
        }

        let family_id = self.font_family;
        let properties = self.font_properties;
        let font_size = self.font_size;

        // Collect style information for lines
        let stylized_lines = rows
            .map(|row| {
                display_map
                    .chars_with_styles_at(DisplayPoint::new(row, 0), app)
                    .unwrap()
                    // Stop when we hit the end of the row.
                    .take_while(|c| c.char() != '\n')
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let font_cache = app.font_cache();

        let line_height_ratio = self.line_height_ratio;

        // Use rayon parallel iterators to efficiently lay out all of the lines.
        let text_layout_system = font_cache.text_layout_system();
        let layouts = stylized_lines
            .into_par_iter()
            .enumerate()
            .map(|(cur_line_idx, stylized_chars)| {
                let mut line = String::with_capacity(stylized_chars.len());
                let mut style_runs = Vec::with_capacity(stylized_chars.len());
                for (idx, stylized_char) in stylized_chars.into_iter().enumerate() {
                    // Accumulate both style information and the characters to lay out.
                    style_runs.push((
                        idx..idx + 1,
                        StyleAndFont::new(family_id, properties, stylized_char.style()),
                    ));
                    line.push(stylized_char.char());
                }

                layout_cache.layout_text(
                    &line,
                    LineStyle {
                        font_size,
                        line_height_ratio,
                        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                        fixed_width_tab_size: None,
                    },
                    &style_runs,
                    max_width,
                    f32::MAX,
                    TextAlignment::default(),
                    // Push over text by left notch width, if we're on the first line. Note notch width will be 0 if same line prompt is disabled.
                    (cur_line_idx == 0).then_some(left_notch_width_px),
                    &text_layout_system,
                )
            })
            .collect();

        Ok(layouts)
    }

    pub fn line(&self, display_row: u32, app: &AppContext) -> Result<String> {
        self.editor_model
            .as_ref(app)
            .display_map(app)
            .line(display_row, app)
    }

    pub fn all_drawable_selections_intersecting_range<'a>(
        &'a self,
        range: Range<DisplayPoint>,
        app: &'a AppContext,
    ) -> impl 'a + Iterator<Item = DrawableSelection> {
        self.editor_model
            .as_ref(app)
            .all_drawable_selections_intersecting_range(range, app)
    }

    // todo: dedup this with EditorView. it's really: given an editor model + row, get the line len
    pub fn line_len(&self, display_row: u32, app: &AppContext) -> Result<u32> {
        self.editor_model.as_ref(app).line_len(display_row, app)
    }

    pub fn layout_line(
        &self,
        row: u32,
        text_layout_cache: &LayoutCache,
        app: &AppContext,
    ) -> Result<Arc<text_layout::Line>> {
        let font_cache = app.font_cache();
        let line = self.line(row, app)?;

        Ok(text_layout_cache.layout_line(
            &line,
            LineStyle {
                font_size: self.font_size,
                line_height_ratio: self.line_height_ratio,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                fixed_width_tab_size: None,
            },
            &[(
                0..self.line_len(row, app)? as usize,
                StyleAndFont::new(self.font_family, Default::default(), TextStyle::new()),
            )],
            f32::MAX,
            ClipConfig::default(),
            &font_cache.text_layout_system(),
        ))
    }

    /// Finds the byte offset directly under the given point
    pub fn byte_offset_at_point(
        &self,
        point: &DisplayPoint,
        app: &AppContext,
    ) -> Option<ByteOffset> {
        let model = self.editor_model.as_ref(app);
        let buffer = model.buffer(app);
        let map = model.display_map(app);
        point
            .to_buffer_point(map, Bias::Left, app)
            .and_then(|point| point.to_byte_offset(buffer))
            .ok()
    }

    pub fn display_point_at_byte_offset(
        &self,
        byte_offset: &ByteOffset,
        app: &AppContext,
    ) -> Option<DisplayPoint> {
        let model = self.editor_model.as_ref(app);
        let buffer = model.buffer(app);
        let map = model.display_map(app);
        let char_offset = byte_offset.to_char_offset(buffer).ok()?;
        let point = char_offset.to_point(buffer).ok()?;
        point.to_display_point(map, app).ok()
    }

    pub fn vim_visual_tails<'a>(
        &self,
        app: &'a AppContext,
    ) -> impl Iterator<Item = DisplayPoint> + 'a + use<'a> {
        let editor_model = self.editor_model.as_ref(app);
        let map = editor_model.display_map(app);
        editor_model
            .vim_visual_tails()
            .iter()
            .filter_map(|anchor| anchor.to_display_point(map, app).ok())
    }

    /// Returns the font size for a cursor avatar. Value is based on the snapshot's
    /// font size and an avatar-specific ratio.
    pub fn cursor_avatar_font_size(&self) -> f32 {
        self.font_size * CURSOR_AVATAR_FONT_RATIO
    }

    /// Returns the size (diameter) for a cursor avatar. Value is based on the snapshot's
    /// font size and an avatar-specific offset.
    pub fn cursor_avatar_size(&self) -> f32 {
        self.font_size + CURSOR_AVATAR_IMAGE_OFFSET
    }

    /// Returns the size to render the voice input icon at, scaled by the font size.
    pub fn voice_input_icon_size(&self) -> Vector2F {
        let scaled_size = (self.font_size + 1.).max(MIN_VOICE_INPUT_ICON_SIZE);
        vec2f(scaled_size + VOICE_INPUT_ICON_IMAGE_OFFSET_X, scaled_size)
    }
}
