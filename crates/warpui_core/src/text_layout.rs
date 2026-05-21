use crate::elements::{Fill, DEFAULT_UI_LINE_HEIGHT_RATIO};
use crate::fonts::{
    Cache as FontCache, FamilyId, Properties, RequestedFallbackFontSource, TextLayoutSystem,
};
use crate::geometry::rect::RectF;
use crate::geometry::vector::vec2f;
use crate::platform::LineStyle;
use crate::scene::{Border, CornerRadius, Dash};
use crate::{
    fonts::{FontId, GlyphId},
    scene::GlyphFade,
    Scene,
};
use itertools::Itertools;
use ordered_float::OrderedFloat;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use rangemap::RangeMap;
use smallvec::SmallVec;
use std::{
    borrow::Borrow,
    collections::HashMap,
    hash::{Hash, Hasher},
    ops::Range,
    sync::Arc,
};
use vec1::{vec1, Vec1};

type StyleRun = (Range<usize>, StyleAndFont);

/// The maximum width of the fade applied to text that overflows its
/// container's maximum width.
const LINE_FADE_MAX_PIXELS: f32 = 25.0;
/// The scaling factor applied to the overflow amount to determine the fade
/// width.
const LINE_FADE_SCALE_FACTOR: f32 = 3.0;
/// Minimum overflow threshold before applying clipping effects.
/// This prevents jitter or odd behavior when text is very close to overflowing.
const MIN_OVERFLOW_FOR_CLIPPING: f32 = 0.1;

// How far below the origin the baseline should fall.
// This means that within the em-box for the line, 80% is above the baseline and 20% is below the baseline.
pub const DEFAULT_TOP_BOTTOM_RATIO: f32 = 0.8;

pub const UNDERLINE_THICKNESS: f32 = 2.;
pub const STRIKETHROUGH_THICKNESSS: f32 = 2.;
pub const UNDERLINE_BOTTOM_PADDING: f32 = 2.;
// TODO: Ideally, we would use DEFAULT_MONOSPACE_FONT_SIZE here, however that
// should stay app crate-specific. Hence, we're using 13.0 as a magic number
// for the purposes of scaling underline padding correctly.
const DEFAULT_FONT_SIZE: f32 = 13.;

// The offset for where on the text glyph the strikethrough should be drawn.
const STRIKETHROUGH_FONT_OFFSET: f32 = 2.5;

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

struct TextCache<T> {
    prev_frame: Mutex<HashMap<CacheKeyValue, Arc<T>>>,
    curr_frame: RwLock<HashMap<CacheKeyValue, Arc<T>>>,
}

impl<T> TextCache<T> {
    pub fn new() -> Self {
        Self {
            prev_frame: Mutex::new(HashMap::new()),
            curr_frame: RwLock::new(HashMap::new()),
        }
    }

    pub fn finish_frame(&self) {
        let mut prev_frame = self.prev_frame.lock();
        let mut curr_frame = self.curr_frame.write();
        std::mem::swap(&mut *prev_frame, &mut *curr_frame);
        curr_frame.clear();
    }

    pub fn get(&self, key: &dyn CacheKey) -> Option<Arc<T>> {
        let curr_frame = self.curr_frame.upgradable_read();
        if let Some(val) = curr_frame.get(key) {
            return Some(val.clone());
        }

        let mut curr_frame = RwLockUpgradableReadGuard::upgrade(curr_frame);
        if let Some((key, val)) = self.prev_frame.lock().remove_entry(key) {
            curr_frame.insert(key, val.clone());
            Some(val)
        } else {
            None
        }
    }

    pub fn insert(&self, key: CacheKeyValue, val: Arc<T>) {
        let mut curr_frame = self.curr_frame.write();
        curr_frame.insert(key, val);
    }

    pub fn remove(&self, key: &dyn CacheKey) {
        self.prev_frame.lock().remove(key);
        self.curr_frame.write().remove(key);
    }
}

pub struct LayoutCache {
    line_cache: TextCache<Line>,
    text_frame_cache: TextCache<TextFrame>,
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutCache {
    pub fn new() -> Self {
        Self {
            line_cache: TextCache::new(),
            text_frame_cache: TextCache::new(),
        }
    }

    pub fn finish_frame(&self) {
        self.line_cache.finish_frame();
        self.text_frame_cache.finish_frame();
    }

    pub fn remove_line(&self, key: &dyn CacheKey) {
        self.line_cache.remove(key);
    }

    pub fn remove_text_frame(&self, key: &dyn CacheKey) {
        self.text_frame_cache.remove(key);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn layout_text<'a>(
        &'a self,
        text: &'a str,
        line_style: LineStyle,
        styles: &'a [StyleRun],
        max_width: f32,
        max_height: f32,
        alignment: TextAlignment,
        first_line_head_indent: Option<f32>,
        text_layout_system: &'a TextLayoutSystem<'a>,
    ) -> Arc<TextFrame> {
        let (text, adjusted_styles) = strip_leading_unicode_bom(text, styles);
        let styles = adjusted_styles
            .as_ref()
            .map_or(styles, |adjusted_styles| adjusted_styles.as_slice());
        let key = &CacheKeyRef {
            text,
            font_size: OrderedFloat(line_style.font_size),
            line_height_ratio: line_style.line_height_ratio.into(),
            fixed_width_tab_size: line_style.fixed_width_tab_size,
            style_runs: styles,
            max_width: OrderedFloat(max_width),
            max_height: Some(max_height.into()),
            alignment,
            first_line_head_indent: first_line_head_indent
                .map(|first_line_head_indent_value| first_line_head_indent_value.into()),
            clip_config: None,
        } as &dyn CacheKey;
        if let Some(text_frame) = self.text_frame_cache.get(key) {
            text_frame
        } else {
            let text_frame = Arc::new(text_layout_system.layout_text(
                text,
                line_style,
                styles,
                max_width,
                max_height,
                alignment,
                first_line_head_indent,
            ));
            let key = CacheKeyValue {
                text: text.into(),
                font_size: line_style.font_size.into(),
                line_height_ratio: line_style.line_height_ratio.into(),
                fixed_width_tab_size: line_style.fixed_width_tab_size,
                style_runs: styles.into(),
                max_width: max_width.into(),
                max_height: Some(max_height.into()),
                alignment,
                first_line_head_indent: first_line_head_indent
                    .map(|first_line_head_indent_value| first_line_head_indent_value.into()),
                clip_config: None,
            };
            for line in text_frame.lines() {
                for ch in &line.chars_with_missing_glyphs {
                    text_layout_system.request_fallback_font_for_char(
                        *ch,
                        RequestedFallbackFontSource::TextFrame(key.clone()),
                    );
                }
            }
            self.text_frame_cache.insert(key, text_frame.clone());
            text_frame
        }
    }

    pub fn layout_line<'a>(
        &'a self,
        text: &'a str,
        line_style: LineStyle,
        style_runs: &'a [StyleRun],
        max_width: f32,
        clip_config: ClipConfig,
        text_layout_system: &TextLayoutSystem<'a>,
    ) -> Arc<Line> {
        let (text, adjusted_style_runs) = strip_leading_unicode_bom(text, style_runs);
        let style_runs = adjusted_style_runs
            .as_ref()
            .map_or(style_runs, |adjusted_style_runs| {
                adjusted_style_runs.as_slice()
            });
        let key = &CacheKeyRef {
            text,
            font_size: line_style.font_size.into(),
            line_height_ratio: line_style.line_height_ratio.into(),
            fixed_width_tab_size: line_style.fixed_width_tab_size,
            style_runs,
            max_width: max_width.into(),
            max_height: None,
            clip_config: Some(clip_config),
            alignment: Default::default(),
            first_line_head_indent: None,
        } as &dyn CacheKey;

        if let Some(line) = self.line_cache.get(key) {
            line
        } else {
            let line = Arc::new(text_layout_system.layout_line(
                text,
                line_style,
                style_runs,
                max_width,
                clip_config,
            ));

            let key = CacheKeyValue {
                text: text.into(),
                font_size: line_style.font_size.into(),
                line_height_ratio: line_style.line_height_ratio.into(),
                fixed_width_tab_size: line_style.fixed_width_tab_size,
                style_runs: style_runs.into(),
                max_width: max_width.into(),
                max_height: None,
                alignment: Default::default(),
                first_line_head_indent: None,
                clip_config: Some(clip_config),
            };
            for ch in &line.chars_with_missing_glyphs {
                text_layout_system.request_fallback_font_for_char(
                    *ch,
                    RequestedFallbackFontSource::Line(key.clone()),
                );
            }
            self.line_cache.insert(key, line.clone());
            line
        }
    }
}

/// Removes a leading UTF-8 BOM from the text and adjusts the style run offsets accordingly.
/// We throw away the styling of the BOM character.
fn strip_leading_unicode_bom<'a>(
    text: &'a str,
    style_runs: &'a [(Range<usize>, StyleAndFont)],
) -> (&'a str, Option<Vec<StyleRun>>) {
    let bom = '\u{FEFF}';
    if text
        .chars()
        .next()
        .is_none_or(|first_character| first_character != bom)
    {
        // There is no leading BOM.
        return (text, None);
    }

    let mut style_runs = style_runs.to_vec();
    for style_run in style_runs.iter_mut() {
        let range = (style_run.0.start.saturating_sub(1))..(style_run.0.end.saturating_sub(1));
        style_run.0 = range;
    }
    let text = text.get(bom.len_utf8()..).unwrap_or_else(|| {
        log::warn!("Unable to get the a substring of the text without a leading BOM");
        text
    });
    (text, Some(style_runs))
}

pub trait CacheKey {
    fn key(&self) -> CacheKeyRef<'_>;
}

impl PartialEq for dyn CacheKey + '_ {
    fn eq(&self, other: &dyn CacheKey) -> bool {
        self.key() == other.key()
    }
}

impl Eq for dyn CacheKey + '_ {}

impl Hash for dyn CacheKey + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state)
    }
}

#[derive(Clone, Eq)]
pub struct CacheKeyValue {
    text: String,
    font_size: OrderedFloat<f32>,
    line_height_ratio: OrderedFloat<f32>,
    fixed_width_tab_size: Option<u8>,
    style_runs: SmallVec<[(Range<usize>, StyleAndFont); 1]>,
    max_width: OrderedFloat<f32>,
    max_height: Option<OrderedFloat<f32>>,
    alignment: TextAlignment,
    first_line_head_indent: Option<OrderedFloat<f32>>,
    clip_config: Option<ClipConfig>,
}

impl PartialEq for CacheKeyValue {
    fn eq(&self, other: &Self) -> bool {
        self.key().eq(&other.key())
    }
}

impl CacheKey for CacheKeyValue {
    fn key(&self) -> CacheKeyRef<'_> {
        CacheKeyRef {
            text: self.text.as_str(),
            font_size: self.font_size,
            line_height_ratio: self.line_height_ratio,
            fixed_width_tab_size: self.fixed_width_tab_size,
            style_runs: self.style_runs.as_slice(),
            max_width: self.max_width,
            max_height: self.max_height,
            alignment: self.alignment,
            first_line_head_indent: self.first_line_head_indent,
            clip_config: self.clip_config,
        }
    }
}

impl Hash for CacheKeyValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

impl<'a> Borrow<dyn CacheKey + 'a> for CacheKeyValue {
    fn borrow(&self) -> &(dyn CacheKey + 'a) {
        self as &dyn CacheKey
    }
}

/// A style override that is applied on paint time without relaying out the text line/frame.
///
/// This should be used with caution since the overrides is applied on the character-level
/// but the character indices might not map to the laid out glyphs 1-1 when there is presence of
/// ligatures. To give an example, if a text frame has the content "[fi]nal" with fi laid out one
/// ligature, trying to apply color on only the first character "f" will cause "fi" to be colored.
#[derive(Default)]
pub struct PaintStyleOverride {
    color: RangeMap<usize, ColorU>,
    underline: RangeMap<usize, ColorU>,
}

impl PaintStyleOverride {
    pub fn with_color(mut self, color_override: RangeMap<usize, ColorU>) -> Self {
        self.color = color_override;
        self
    }

    pub fn with_underline(mut self, underline_override: RangeMap<usize, ColorU>) -> Self {
        self.underline = underline_override;
        self
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct CacheKeyRef<'a> {
    text: &'a str,
    font_size: OrderedFloat<f32>,
    line_height_ratio: OrderedFloat<f32>,
    fixed_width_tab_size: Option<u8>,
    style_runs: &'a [(Range<usize>, StyleAndFont)],
    max_width: OrderedFloat<f32>,
    max_height: Option<OrderedFloat<f32>>,
    alignment: TextAlignment,
    first_line_head_indent: Option<OrderedFloat<f32>>,
    clip_config: Option<ClipConfig>,
}

impl CacheKey for CacheKeyRef<'_> {
    fn key(&self) -> CacheKeyRef<'_> {
        *self
    }
}

/// Enum describing which way to clip the text.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, Hash)]
pub enum ClipDirection {
    /// Clip at the end of the text (default)
    #[default]
    End,
    /// Clip at the front of the text
    Start,
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, Hash)]
pub enum ClipStyle {
    /// Fade out the clipped text (default)
    #[default]
    Fade,
    /// Show an ellipsis (…) at the clipped edge
    Ellipsis,
}

/// Configuration for clipping text that overflows the available width.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClipConfig {
    pub direction: ClipDirection,
    pub style: ClipStyle,
}

impl Default for ClipConfig {
    fn default() -> Self {
        Self {
            direction: ClipDirection::End,
            style: ClipStyle::Fade,
        }
    }
}

impl ClipConfig {
    pub fn end() -> Self {
        Self {
            direction: ClipDirection::End,
            style: ClipStyle::Fade,
        }
    }

    pub fn start() -> Self {
        Self {
            direction: ClipDirection::Start,
            style: ClipStyle::Fade,
        }
    }

    pub fn ellipsis() -> Self {
        Self {
            direction: ClipDirection::End,
            style: ClipStyle::Ellipsis,
        }
    }
}

pub struct ComputeBaselinePositionArgs<'a> {
    pub font_cache: &'a FontCache,
    pub font_size: f32,
    /// Defines how tall a line should be, used in conjunction with font size. The height of a line is defined to be
    /// line height ratio * font size (if we are not using glyph-based metrics e.g. ascent/descent).
    pub line_height_ratio: f32,
    /// Baseline ratio defines how far below the origin the baseline should fall.
    /// For example, with a ratio of 0.8, it means that within the em-box for the line, 80% is above the baseline
    /// and 20% is below the baseline.
    pub baseline_ratio: f32,
    /// Ascent measures the distance from the baseline to the top of the em-box.
    pub ascent: f32,
    /// Descent measures the distance from the baseline to the bottom of the em-box.
    pub descent: f32,
}

/// Closure to compute baseline position from given arguments. Note that this concept of "baseline position"
/// is distinct from Core Text's concept of "baseline offset". Specifically, this is the position of the baseline
/// used to render glyphs. This position is relative to the top of a given Line (see paint() method in Line for
/// further details on usage).
pub type ComputeBaselinePositionFn =
    Box<dyn Fn(ComputeBaselinePositionArgs) -> f32 + 'static + Send + Sync>;

#[derive(Default, Debug, Clone)]
pub struct Line {
    pub width: f32,
    pub trailing_whitespace_width: f32,
    pub runs: Vec<Run>,
    pub font_size: f32,
    pub line_height_ratio: f32,
    pub baseline_ratio: f32,
    pub clip_config: Option<ClipConfig>,

    pub ascent: f32,
    pub descent: f32,

    /// Caret positions represent locations the cursor and selection endpoints
    /// can snap to when selecting text.
    /// On MacOS, CoreText gives us one caret position per visible glyphs,
    /// meaning that ligatures will have a single caret position.
    /// On winit platforms, cosmic-text gives us one caret position per
    /// codepoint, meaning ligatures will have multiple caret positions.
    pub caret_positions: Vec<CaretPosition>,
    pub chars_with_missing_glyphs: Vec<char>,
}

/// Default baseline offset calculation for Lines.
pub fn default_compute_baseline_position(
    font_size: f32,
    line_height_ratio: f32,
    ascent: f32,
    descent: f32,
) -> f32 {
    let line_height = font_size * line_height_ratio;
    // Text height is the distance from top of em-box to
    // baseline + distance from baseline to bottom of em-box.
    let text_height = ascent + descent;
    // We want the text to be vertically centered within the line.
    let padding_top = (line_height - text_height) / 2.0;
    // Baseline position is the distance from top of line to top
    // of em-box + distance from top of em-box to baseline.
    padding_top + ascent
}

/// Returns closure computing the default baseline offset for given font metrics.
pub fn default_compute_baseline_position_fn() -> ComputeBaselinePositionFn {
    Box::new(|font_metrics| {
        default_compute_baseline_position(
            font_metrics.font_size,
            font_metrics.line_height_ratio,
            font_metrics.ascent,
            font_metrics.descent,
        )
    })
}

/// A caret position within a line. Generally, there is a caret position after
/// every grapheme. This does not always correspond to glyphs (for example, a
/// ligature is one glyph but contains multiple caret positions). It also does
/// not necessarily correspond to characters (many emoji are represented by
/// multiple Unicode scalar values, but only produce a single caret position).
#[derive(Debug, Default, Clone)]
pub struct CaretPosition {
    /// The x-position of this caret location, relative to the line's origin.
    pub position_in_line: f32,
    /// The starting character index corresponding to this location in the input string.
    /// In the case of RTL text, this may not correspond to `position_in_line`.
    /// That is, a caret position that's visually to the right of another may
    /// actually have a _lower_ `start_offset`.
    pub start_offset: usize,
    /// The index of the last character in this caret position. This is _inclusive_.
    pub last_offset: usize,
}

#[derive(Debug, Default, Copy, Clone, Hash, PartialEq, Eq)]
pub struct TextStyle {
    pub foreground_color: Option<ColorU>,
    // syntax_color is similar to foreground_color except it isn't inheritable.
    pub syntax_color: Option<ColorU>,
    pub background_color: Option<ColorU>,
    pub border: Option<TextBorder>,
    pub error_underline_color: Option<ColorU>,
    pub show_strikethrough: bool,
    // Underline color (of either text underline or hyperlink).
    pub underline_color: Option<ColorU>,
    // Unique id for each hyperlink in a frame, used to group parts of a hyperlink together if a hyperlink is soft-wrapped.
    pub hyperlink_id: Option<i32>,
}

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct TextBorder {
    pub color: ColorU,
    pub radius: u8,
    pub width: u8,
    // The line height ratio override to determine the size of the border and the background color (if there is one).
    // By default, the border will fit the entire line height.
    pub line_height_ratio_override: Option<u8>,
}

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct StyleAndFont {
    pub font_family: FamilyId,
    pub properties: Properties,
    pub style: TextStyle,
}

impl StyleAndFont {
    pub fn new(font_family: FamilyId, properties: Properties, style: TextStyle) -> Self {
        StyleAndFont {
            font_family,
            properties,
            style,
        }
    }
}

impl TextStyle {
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns a new TextStyle containing only the inheritable styles from
    /// the current TextStyle (self). Note that this is the source of truth
    /// for which text styles are inheritable.
    pub fn filter_inheritable_styles(self) -> Self {
        TextStyle {
            foreground_color: self.foreground_color,
            syntax_color: None,
            background_color: self.background_color,
            error_underline_color: None,
            border: None,
            show_strikethrough: false,
            underline_color: None,
            hyperlink_id: None,
        }
    }

    pub fn with_foreground_color(mut self, foreground_color: ColorU) -> Self {
        self.foreground_color = Some(foreground_color);
        self
    }

    pub fn with_syntax_color(mut self, syntax_color: ColorU) -> Self {
        self.syntax_color = Some(syntax_color);
        self
    }

    pub fn with_border(mut self, border: TextBorder) -> Self {
        self.border = Some(border);
        self
    }

    pub fn with_background_color(mut self, background_color: ColorU) -> Self {
        self.background_color = Some(background_color);
        self
    }

    pub fn with_error_underline_color(mut self, error_underline_color: ColorU) -> Self {
        self.error_underline_color = Some(error_underline_color);
        self
    }

    pub fn with_show_strikethrough(mut self, show_strikethrough: bool) -> Self {
        self.show_strikethrough = show_strikethrough;
        self
    }

    pub fn with_underline_color(mut self, underline_color: ColorU) -> Self {
        self.underline_color = Some(underline_color);
        self
    }

    pub fn with_hyperlink_id(mut self, hyperlink_id: i32) -> Self {
        self.hyperlink_id = Some(hyperlink_id);
        self
    }
}

/// A series of consecutive glyphs within a run that have the same styles.
#[derive(Debug, Clone)]
pub struct Run {
    pub font_id: FontId,
    pub glyphs: Vec<Glyph>,
    pub styles: TextStyle,
    pub width: f32,
}

#[derive(Debug, Clone)]
pub struct Glyph {
    pub id: GlyphId,
    /// Position of the glyph on its baseline.
    pub position_along_baseline: Vector2F,
    /// The starting index of the character in the original string where this glyph starts.
    pub index: usize,
    /// The width of the glyph (its advance), in pixels.
    pub width: f32,
}

/// On MacOS, CoreText includes line separators in the TextFrame's lines.
/// On winit, cosmic-text strips line separators, so they do not have their
/// own glyphs in the TextFrame's lines.
#[derive(Default, Debug)]
pub struct TextFrame {
    lines: Vec1<Line>,
    /// The max width of any line in the text frame.
    max_width: f32,
    alignment: TextAlignment,
}

impl TextFrame {
    pub fn new(lines: Vec1<Line>, max_width: f32, alignment: TextAlignment) -> Self {
        TextFrame {
            lines,
            max_width,
            alignment,
        }
    }

    /// A text frame with no text. It has a single line with no runs which
    /// is created by `Line#empty`.
    ///
    /// System APIs may return text frames with all sorts of different values so
    /// this API helps standardize these values for our application logic.
    pub fn empty(font_size: f32, line_height_ratio: f32) -> Self {
        TextFrame {
            lines: vec1![Line::empty(font_size, line_height_ratio, 0)],
            max_width: 0.0,
            alignment: Default::default(),
        }
    }

    // Returns the absolute bounds of all hyperlinks in the frame. The position of each link is offset by the provided `origin.`
    pub fn hyperlink_bounds(&self, origin: Vector2F) -> Vec<Vec<RectF>> {
        let mut positions: Vec<Vec<RectF>> = Vec::new();
        let mut height = origin.y();
        let mut prev_hyperlink_id: Option<i32> = None;

        // If the current rectangle is part of the previous hyperlink frame, just push it in a running vector. Otherwise create a new entry.
        for line in &self.lines {
            let mut width = origin.x() + self.line_x_offset(line);
            for run in &line.runs {
                if let Some(curr_hyperlink_id) = run.styles.hyperlink_id {
                    let curr_rectangle = RectF::new(
                        Vector2F::new(width, height),
                        Vector2F::new(run.width, line.font_size * line.line_height_ratio),
                    );

                    let mut soft_wrapped = false;
                    if let Some(prev_id) = prev_hyperlink_id {
                        if prev_id == curr_hyperlink_id {
                            positions
                                .last_mut()
                                .expect("Positions should be non-empty")
                                .push(curr_rectangle);
                            soft_wrapped = true;
                        }
                    }

                    if !soft_wrapped {
                        positions.push(vec![curr_rectangle]);
                    }

                    prev_hyperlink_id = Some(curr_hyperlink_id);
                }
                width += run.width;
            }
            height += line.font_size * line.line_height_ratio;
        }

        positions
    }

    /// We can't mark this as cfg(test) because we need this in the warp crate tests.
    pub fn mock(text: &str) -> Self {
        let mut acc = 0;
        let lines = text
            .split('\n')
            .map(|line| {
                let glyphs: Vec<_> = line
                    .chars()
                    .enumerate()
                    .map(|(index, _)| Glyph {
                        id: Default::default(),
                        position_along_baseline: Default::default(),
                        index: index + acc,
                        width: 10.0, // dummy width
                    })
                    .collect();
                acc += glyphs.len();
                let runs = vec![Run {
                    font_id: FontId(0),
                    glyphs,
                    styles: TextStyle::new(),
                    width: Default::default(),
                }];
                Line::mock(runs)
            })
            .collect();

        match Vec1::try_from_vec(lines) {
            Ok(lines) => TextFrame {
                lines,
                max_width: Default::default(),
                alignment: Default::default(),
            },
            Err(_) => TextFrame::empty(Default::default(), Default::default()),
        }
    }

    pub fn lines(&self) -> &Vec<Line> {
        self.lines.as_ref()
    }

    pub fn max_width(&self) -> f32 {
        self.max_width
    }

    pub fn height(&self) -> f32 {
        self.lines()
            .iter()
            .fold(0., |prev, line| prev + line.height())
    }

    /// Returns the height of the frame up to the row, inclusive
    pub fn height_up_to_row(&self, row: usize) -> f32 {
        self.lines()
            .iter()
            .take(row + 1)
            .fold(0., |prev, line| prev + line.height())
    }

    /// Given an index, returns the row the corresponding glyph is in the text frame.
    /// If the index is beyond the bounds of the text frame, we return the last row
    /// in the text frame.
    ///
    /// `clamp_above` is used to disambiguate whether the index is the last index
    /// of a line or the first index of the next line. If `clamp_above` is true,
    /// it's the last index of the above line and if it's falase, it's the first
    /// index of the below line.
    pub fn row_within_frame(&self, index: usize, clamp_above: bool) -> usize {
        for (i, line) in self.lines.iter().enumerate() {
            if let Some(last_glyph) = line.last_glyph() {
                let last_index_in_row = match clamp_above {
                    true => last_glyph.index + 1,
                    false => last_glyph.index,
                };
                if last_index_in_row >= index {
                    return i;
                }
            }
        }
        self.lines.len() - 1
    }

    pub fn paint(
        &self,
        bounds: RectF,
        style_overrides: &PaintStyleOverride,
        default_color: ColorU,
        scene: &mut Scene,
        font_cache: &FontCache,
    ) {
        for (index, line) in self.lines.iter().enumerate() {
            let origin =
                bounds.origin() + vec2f(self.line_x_offset(line), index as f32 * line.height());
            let bounds = RectF::from_points(origin, bounds.lower_right());
            line.paint(bounds, style_overrides, default_color, font_cache, scene);
        }
    }

    // The x offset relative to the text frame origin to paint the line.
    pub(crate) fn line_x_offset(&self, line: &Line) -> f32 {
        let line_width = line.width;

        match self.alignment {
            TextAlignment::Left => 0.,
            TextAlignment::Right => self.max_width - line_width,
            TextAlignment::Center => (self.max_width - line_width) / 2.,
        }
    }

    pub fn paint_with_baseline_position(
        &self,
        bounds: RectF,
        style_overrides: &PaintStyleOverride,
        default_color: ColorU,
        scene: &mut Scene,
        font_cache: &FontCache,
        baseline_position_fn: &ComputeBaselinePositionFn,
    ) {
        for (index, line) in self.lines.iter().enumerate() {
            let origin = bounds.origin()
                + vec2f(
                    0.,
                    index as f32 * font_cache.line_height(line.font_size, line.line_height_ratio),
                );
            let bounds = RectF::from_points(origin, bounds.lower_right());
            line.paint_with_baseline_position(
                bounds,
                style_overrides,
                default_color,
                font_cache,
                scene,
                baseline_position_fn,
            );
        }
    }
}

impl Line {
    /// A line with no text. It has zero width and a height equivalent to the
    /// line height.
    ///
    /// System APIs may return empty lines with all sorts of different values for
    /// width, height, etc. so we can use this API to help standardize these
    /// values for our application logic.
    pub fn empty(font_size: f32, line_height_ratio: f32, glyph_index: usize) -> Self {
        Line {
            width: 0.0,
            trailing_whitespace_width: 0.0,
            runs: vec![],
            font_size,
            line_height_ratio,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            ascent: font_size * DEFAULT_TOP_BOTTOM_RATIO,
            descent: font_size * (1. - DEFAULT_TOP_BOTTOM_RATIO),
            clip_config: None,
            caret_positions: vec![CaretPosition {
                position_in_line: 0.0,
                start_offset: glyph_index,
                last_offset: glyph_index,
            }],
            chars_with_missing_glyphs: vec![],
        }
    }

    /// We can't mark this as cfg(test) because we need this in the warp crate tests.
    pub fn mock(runs: Vec<Run>) -> Self {
        Line {
            width: Default::default(),
            trailing_whitespace_width: Default::default(),
            runs,
            font_size: DEFAULT_FONT_SIZE,
            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            ascent: DEFAULT_FONT_SIZE * DEFAULT_TOP_BOTTOM_RATIO,
            descent: DEFAULT_FONT_SIZE * (1. - DEFAULT_TOP_BOTTOM_RATIO),
            clip_config: None,
            caret_positions: Default::default(),
            chars_with_missing_glyphs: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn mock_from_str(line: &str) -> Self {
        assert!(!line.contains('\n'));
        let glyphs: Vec<_> = line
            .chars()
            .enumerate()
            .map(|(index, _)| Glyph {
                id: Default::default(),
                position_along_baseline: Default::default(),
                index,
                width: 10.0, // dummy width
            })
            .collect();
        let runs = vec![Run {
            font_id: FontId(0),
            glyphs,
            styles: TextStyle::new(),
            width: Default::default(),
        }];
        Line::mock(runs)
    }

    pub fn height(&self) -> f32 {
        self.line_height_ratio * self.font_size
    }

    pub fn first_glyph(&self) -> Option<&Glyph> {
        let first_run = self.runs.first()?;
        first_run.glyphs.first()
    }

    pub fn last_glyph(&self) -> Option<&Glyph> {
        let last_run = self.runs.last()?;
        last_run.glyphs.last()
    }

    pub fn x_for_index(&self, index: usize) -> f32 {
        for run in &self.runs {
            for glyph in &run.glyphs {
                if glyph.index == index {
                    return glyph.position_along_baseline.x();
                }
            }
        }

        self.width
    }

    /// The width in pixels of the glyph at this index. Returns None if the index is invalid.
    pub fn width_for_index(&self, index: usize) -> Option<f32> {
        let mut prev_glyph = self.runs.first().and_then(|run| run.glyphs.first())?;
        for run in &self.runs {
            for glyph in &run.glyphs {
                if glyph.index == index + 1 {
                    return Some(
                        glyph.position_along_baseline.x() - prev_glyph.position_along_baseline.x(),
                    );
                }
                prev_glyph = glyph;
            }
        }

        if index == self.last_index() {
            return Some(self.width - prev_glyph.position_along_baseline.x());
        }

        None
    }

    /// Finds the nearest caret position for a character index. This is similar
    /// to [`Self::x_for_index`], but accounts for multi-character glyphs such
    /// as ligatures and many emojis.
    pub fn caret_position_for_index(&self, index: usize) -> f32 {
        for caret in self.caret_positions.iter() {
            if caret.contains_index(index) {
                return caret.position_in_line;
            }
        }

        // If `index` is out of bounds or at the extremes of the line, clamp to
        // either 0 or the line width. Which we choose depends on whether `index`
        // is before or after the line's range.
        if self
            .caret_positions
            .first()
            .is_some_and(|caret| index < caret.start_offset)
        {
            0.
        } else {
            self.width
        }
    }

    fn is_x_in_bound(&self, x: f32) -> bool {
        x >= 0. && x < self.width
    }

    /// Returns the character index for the glyph best corresponding to `x`.
    pub fn index_for_x(&self, x: f32) -> Option<usize> {
        if !self.is_x_in_bound(x) {
            None
        } else {
            for run in self.runs.iter().rev() {
                for glyph in run.glyphs.iter().rev() {
                    if glyph.position_along_baseline.x() <= x {
                        return Some(glyph.index);
                    }
                }
            }

            Some(0)
        }
    }

    /// Returns the caret index closest to the (relative) `x` position,
    /// but returns the first or end index if the `x` position is out of bounds.
    pub fn caret_index_for_x_unbounded(&self, x: f32) -> usize {
        let max_line_x = self.x_for_index(self.end_index());

        if !self.is_x_in_bound(x) {
            // max_line_x should be smaller than self.width, but we check both just in case.
            return if x >= max_line_x || x >= self.width {
                self.end_index()
            } else {
                self.first_index()
            };
        }

        // Handle special case where x is on the second half of the last glyph: `caret_index_for_x`
        // checks against glyph boundaries, but there's no glyph to the right when x lies past the
        // midpoint of the last glyph, so it'll always return the start of the last glyph instead of
        // the end. We need to handle this case separately and return the end of the last glyph.
        let tail_caret_position = match self.caret_positions.last() {
            Some(caret) => caret.position_in_line,
            None => return self.first_index(),
        };
        // Note that if the text has been truncated, `max_line_x` could potentially be smaller than
        // `tail_caret_position`. In such cases, the below math for handling this special case becomes
        // incorrect, so we skip the check.
        let is_text_truncated = max_line_x <= tail_caret_position;
        if !is_text_truncated && (x - max_line_x).abs() < (x - tail_caret_position).abs() {
            return self.end_index();
        }

        // Default case - we can unwrap safely since we've covered the edge cases resulting in `None` above.
        self.caret_index_for_x(x)
            .expect("None conditions should be already checked & handled")
    }

    /// Returns the starting character index for the caret position best corresponding to `x`.
    /// Returns `None` if `x` is out of bounds.
    /// Max return value is `self.last_index()` (`self.end_index() - 1`).
    ///
    /// *Important*: if you change the condition for returning `None`, make sure to update the
    /// checks in `caret_index_for_x_unbounded` as well.
    pub fn caret_index_for_x(&self, x: f32) -> Option<usize> {
        if !self.is_x_in_bound(x) {
            None
        } else {
            // Iterate backwards through the list of caret positions, and bias to the start of the
            // line if the search fails. Equivalently, we could iterate forwards and bias to the
            // end of the line.
            for (right, left) in self.caret_positions.iter().rev().tuple_windows() {
                // We want to find the two caret positions adjacent to x, and then chose the closest.
                // This is the first window from the back where the left caret position starts before x.
                if left.position_in_line <= x {
                    if (left.position_in_line - x).abs() < (right.position_in_line - x).abs() {
                        return Some(left.start_offset);
                    } else {
                        return Some(right.start_offset);
                    }
                }
            }

            Some(0)
        }
    }

    /// The first character index that's within this line.
    pub fn first_index(&self) -> usize {
        self.caret_positions
            .first()
            .map_or(0, |caret| caret.start_offset)
    }

    /// The last character index that's within this line (inclusive).
    pub fn last_index(&self) -> usize {
        self.caret_positions
            .last()
            .map_or(0, |caret| caret.last_offset)
    }

    /// The first character index that's after this line.
    pub fn end_index(&self) -> usize {
        self.last_index() + 1
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_run_decorations(
        &self,
        glyph_color: ColorU,
        run: &Run,
        origin: Vector2F,
        visible_bounds: RectF,
        font_cache: &FontCache,
        scene: &mut Scene,
        baseline_position_fn: &ComputeBaselinePositionFn,
    ) {
        if let Some(first_glyph) = run.glyphs.first() {
            // We only need to draw text background if
            // 1) there is at least one glyph in the run
            // 2) either the border or the background color is present
            if run.styles.border.is_some() || run.styles.background_color.is_some() {
                // Scale the block padding based on the font size.
                let block_padding = if run.styles.border.is_some() {
                    self.font_size / 10.
                } else {
                    0.
                };

                // If the border has a line height ratio override, convert it from u8 to f32.
                // Else use the current text's line height ratio.
                let line_height_ratio = run
                    .styles
                    .border
                    .and_then(|border| {
                        border
                            .line_height_ratio_override
                            .map(|val| val as f32 / 100.)
                    })
                    .unwrap_or(self.line_height_ratio);

                // Compute the origin of where the first glyph was rendered. The position reported
                // by the glyph is along it's baseline, so we need to offset it by the baseline
                // offset to get back to the top of the glyph.
                // We also need to shift the position horizontally to account for kerning when bordering
                // is turned on.
                let rect_origin = origin + first_glyph.position_along_baseline
                    - vec2f(
                        2. * block_padding,
                        (baseline_position_fn)(ComputeBaselinePositionArgs {
                            font_cache,
                            font_size: self.font_size,
                            line_height_ratio,
                            baseline_ratio: self.baseline_ratio,
                            ascent: self.ascent,
                            descent: self.descent,
                        }) + 2. * block_padding,
                    );
                let text_rect = RectF::new(
                    rect_origin,
                    vec2f(
                        run.width + 2. * block_padding,
                        font_cache.line_height(self.font_size, line_height_ratio)
                            + 2. * block_padding,
                    ),
                );

                let Some(clipped_rect) = text_rect.intersection(visible_bounds) else {
                    // If there is no intersection, there's no need to paint the rect
                    // (as it won't be visible).
                    return;
                };
                let rendered_background = scene.draw_rect_with_hit_recording(clipped_rect);

                if let Some(border) = run.styles.border {
                    rendered_background
                        .with_corner_radius(CornerRadius::with_all(crate::scene::Radius::Pixels(
                            border.radius as f32,
                        )))
                        .with_border(
                            Border::all(border.width as f32).with_border_color(border.color),
                        );
                }

                if let Some(color) = run.styles.background_color {
                    rendered_background.with_background(Fill::Solid(color));
                }
            }
        }

        if let Some((error_underline_color, first_glyph)) =
            run.styles.error_underline_color.zip(run.glyphs.first())
        {
            // We draw the error underline at the baseline.
            let underline_origin = origin + first_glyph.position_along_baseline;

            let scaled_underline_bottom_padding =
                UNDERLINE_BOTTOM_PADDING * (self.font_size / DEFAULT_FONT_SIZE);

            let dash = Dash {
                dash_length: 4.,
                gap_length: 3.,
                // We don't want to adjust the gap width based on the length of
                // the run, as that would cause the dashes to wiggle as the run
                // changes.
                force_consistent_gap_length: true,
            };

            let underline_rect = RectF::new(
                underline_origin,
                vec2f(
                    run.width,
                    scaled_underline_bottom_padding + UNDERLINE_THICKNESS,
                ),
            );

            if let Some(clipped_rect) = underline_rect.intersection(visible_bounds) {
                scene
                    .draw_rect_without_hit_recording(clipped_rect)
                    .with_border(
                        Border::bottom(UNDERLINE_THICKNESS)
                            .with_dashed_border(dash)
                            .with_border_color(error_underline_color),
                    );
            }
        }

        // Draw a strikethrough through text if boolean flag is set.
        if run.styles.show_strikethrough {
            if let Some(first_glyph) = run.glyphs.first() {
                let mut strikethrough_origin = origin + first_glyph.position_along_baseline;
                strikethrough_origin
                    .set_y(strikethrough_origin.y() - self.font_size / STRIKETHROUGH_FONT_OFFSET);
                let strikethrough_rect = RectF::new(
                    strikethrough_origin,
                    vec2f(run.width, STRIKETHROUGH_THICKNESSS),
                );

                if let Some(clipped_rect) = strikethrough_rect.intersection(visible_bounds) {
                    scene
                        .draw_rect_without_hit_recording(clipped_rect)
                        .with_background(Fill::Solid(glyph_color));
                }
            }
        }
    }

    /// Paints the line of text using given parameters. Uses default baseline offset calculation for a Line.
    pub fn paint(
        &self,
        bounds: RectF,
        style_overrides: &PaintStyleOverride,
        default_color: ColorU,
        font_cache: &FontCache,
        scene: &mut Scene,
    ) {
        self.paint_internal(
            bounds,
            style_overrides,
            default_color,
            font_cache,
            scene,
            &default_compute_baseline_position_fn(),
        )
    }

    /// Paints the line of text using given parameters. Note that the caller can provide a custom
    /// closure to compute the baseline position used for the text.
    pub fn paint_with_baseline_position(
        &self,
        bounds: RectF,
        style_overrides: &PaintStyleOverride,
        default_color: ColorU,
        font_cache: &FontCache,
        scene: &mut Scene,
        baseline_position_fn: &ComputeBaselinePositionFn,
    ) {
        self.paint_internal(
            bounds,
            style_overrides,
            default_color,
            font_cache,
            scene,
            baseline_position_fn,
        )
    }

    fn paint_internal(
        &self,
        bounds: RectF,
        style_overrides: &PaintStyleOverride,
        default_color: ColorU,
        font_cache: &FontCache,
        scene: &mut Scene,
        baseline_position_fn: &ComputeBaselinePositionFn,
    ) {
        let origin = bounds.origin();
        let available_width = bounds.width();

        // Fade out the line if the text in it has been clipped to max_width
        let width_without_trailing_whitespace = self.width - self.trailing_whitespace_width;
        let overflow = width_without_trailing_whitespace - available_width;

        let (clip_direction, clip_style) = self
            .clip_config
            .map(|config| (config.direction, config.style))
            .unwrap_or_default();

        let ellipsis_glyph: Option<(GlyphId, FontId, f32)> =
            if clip_style == ClipStyle::Ellipsis && overflow > MIN_OVERFLOW_FOR_CLIPPING {
                let ellipsis_run = match clip_direction {
                    ClipDirection::Start => self.runs.last(),
                    ClipDirection::End => self.runs.first(),
                };

                ellipsis_run.and_then(|run| {
                    font_cache.glyph_for_char(run.font_id, '…', false).and_then(
                        |(glyph_id, font_id)| {
                            font_cache
                                .glyph_advance(font_id, self.font_size, glyph_id)
                                .ok()
                                .map(|advance| (glyph_id, font_id, advance.x()))
                        },
                    )
                })
            } else {
                None
            };
        let ellipsis_width = ellipsis_glyph
            .as_ref()
            .map(|(_, _, width)| *width)
            .unwrap_or_default();

        // Set the length of the fade based on how much text is overflowing.
        let fade_width = LINE_FADE_MAX_PIXELS.min(overflow * LINE_FADE_SCALE_FACTOR);
        let fade = if overflow < MIN_OVERFLOW_FOR_CLIPPING
            || clip_style == ClipStyle::Ellipsis
            || self.clip_config.is_none()
        {
            None
        } else {
            match clip_direction {
                ClipDirection::End => {
                    let fade_end = bounds.upper_right().x();
                    let fade_start = fade_end - fade_width;
                    Some(GlyphFade::horizontal(fade_start, fade_end))
                }
                ClipDirection::Start => {
                    let fade_end = bounds.origin().x();
                    let fade_start = fade_end + fade_width;
                    Some(GlyphFade::horizontal(fade_start, fade_end))
                }
            }
        };

        // Adjust the origin to be the baseline of the line, not the top of
        // the line. Note that the baseline position is consistent across the entire line,
        // even if we have different fonts on a single line.
        let baseline_position = (baseline_position_fn)(ComputeBaselinePositionArgs {
            font_cache,
            font_size: self.font_size,
            line_height_ratio: self.line_height_ratio,
            baseline_ratio: self.baseline_ratio,
            ascent: self.ascent,
            descent: self.descent,
        });

        let line_origin = origin + vec2f(0., baseline_position);
        let is_start_clipping =
            self.clip_config.is_some() && clip_direction == ClipDirection::Start;
        let run_iter = if is_start_clipping {
            itertools::Either::Left(self.runs.iter().rev())
        } else {
            itertools::Either::Right(self.runs.iter())
        };

        let mut remaining_width = match clip_style {
            // For ellipsis, reserve space on the side where we will draw the ellipsis.
            ClipStyle::Ellipsis if ellipsis_width > 0. => match clip_direction {
                ClipDirection::End => (available_width - ellipsis_width).max(0.),
                ClipDirection::Start => (available_width - ellipsis_width).max(0.),
            },
            _ => available_width,
        };

        // When start-clipping with an ellipsis we reserved `ellipsis_width` of
        // space at the LEFT for the ellipsis glyph. Visible glyphs need to be
        // offset by that amount so they stay flush with the right edge and do
        // not overlap the ellipsis. This is constant for the entire paint, so
        // hoist it out of the per-glyph loop.
        let start_ellipsis_offset = if is_start_clipping && ellipsis_width > 0. {
            ellipsis_width
        } else {
            0.
        };

        'runs: for run in run_iter {
            let mut glyph_color = default_color;
            // We define foreground_color to overwrite syntax_color since the
            // foreground color was likely set explicitly somewhere (by the user
            // or system), whereas the syntax color is automatically added.
            if let Some(syntax_color) = run.styles.syntax_color {
                glyph_color = syntax_color;
            }
            if let Some(foreground_color) = run.styles.foreground_color {
                glyph_color = foreground_color;
            }

            let glyph_iter = if is_start_clipping {
                itertools::Either::Left(run.glyphs.iter().rev())
            } else {
                itertools::Either::Right(run.glyphs.iter())
            };
            let mut should_stop_after_run = false;
            for glyph in glyph_iter {
                let index = glyph.index;
                let override_color = style_overrides.color.get(&index).cloned();

                // If we've started truncating in ellipsis mode, draw the ellipsis and stop painting glyphs.
                if clip_style == ClipStyle::Ellipsis
                    && ellipsis_width > 0.
                    && remaining_width < glyph.width
                {
                    if let Some((glyph_id, font_id, _)) = ellipsis_glyph {
                        let ellipsis_x = match clip_direction {
                            ClipDirection::End => {
                                (available_width - ellipsis_width - remaining_width).max(0.)
                            }
                            ClipDirection::Start => remaining_width,
                        };
                        let ellipsis_origin = line_origin + vec2f(ellipsis_x, 0.);

                        scene.draw_glyph(
                            ellipsis_origin,
                            glyph_id,
                            font_id,
                            self.font_size,
                            default_color,
                        );
                    }
                    break 'runs;
                }

                // If there is not enough space to paint even part of the glyph,
                // stop painting glyphs but still paint run decorations
                // (so that the decorations are still visible even if the glyphs are partially hidden).
                if remaining_width <= 0. {
                    should_stop_after_run = true;
                    break;
                }
                remaining_width -= glyph.width;

                let glyph_origin = if is_start_clipping {
                    line_origin
                        + vec2f(
                            remaining_width + start_ellipsis_offset,
                            glyph.position_along_baseline.y(),
                        )
                } else {
                    line_origin + glyph.position_along_baseline
                };

                scene
                    .draw_glyph(
                        glyph_origin,
                        glyph.id,
                        run.font_id,
                        self.font_size,
                        override_color.unwrap_or(glyph_color),
                    )
                    .with_fade(fade);

                // Draw the underline under the tag (e.g. for a hyperlink).
                if let Some(underline_color) = style_overrides
                    .underline
                    .get(&index)
                    .copied()
                    .or(run.styles.underline_color)
                {
                    let scaled_underline_bottom_padding =
                        UNDERLINE_BOTTOM_PADDING * (self.font_size / DEFAULT_FONT_SIZE);
                    let underline_origin = line_origin
                        + glyph.position_along_baseline
                        + vec2f(0., scaled_underline_bottom_padding);

                    scene
                        .draw_rect_without_hit_recording(RectF::new(
                            underline_origin,
                            vec2f(glyph.width, UNDERLINE_THICKNESS),
                        ))
                        .with_background(Fill::Solid(underline_color));
                }
            }

            self.paint_run_decorations(
                glyph_color,
                run,
                line_origin,
                bounds,
                font_cache,
                scene,
                baseline_position_fn,
            );

            if should_stop_after_run {
                break;
            }
        }
    }
}

impl CaretPosition {
    /// The number of characters covered by this caret position.
    pub fn char_count(&self) -> usize {
        self.last_offset - self.start_offset + 1
    }

    /// Whether or not a given character offset is within this caret position.
    pub fn contains_index(&self, index: usize) -> bool {
        index >= self.start_offset && index <= self.last_offset
    }
}

#[cfg(test)]
#[path = "text_layout_tests.rs"]
mod tests;
