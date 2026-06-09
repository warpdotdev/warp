//! Monospace font + text-layout system for the TUI backend.
//!
//! Exposes a [`FontDB`] implementing both [`warpui_core::platform::FontDB`] and
//! [`warpui_core::platform::TextLayoutSystem`]. The model is: 1 terminal cell ==
//! 1 WarpUI "pixel". Every character maps to an identity glyph id
//! (`glyph_for_char(c) == c as u32`) so the rasterizer can recover the `char`,
//! and text is laid out one entry per character advancing by its terminal
//! display width (1 for normal characters, 2 for wide CJK/emoji). At
//! `font_size == 1.0` a character advances exactly 1.0 horizontally and a line
//! is 1.0 tall, so the scene -> cell mapping is a plain `floor()`.
//!
//! Glyph indices and caret offsets are character offsets into the input
//! string, matching the style-run ranges and the other platform backends.

use std::any::Any;
use std::ops::Range;

use anyhow::Result;
use pathfinder_geometry::rect::RectI;
use pathfinder_geometry::vector::{vec2f, vec2i, Vector2F, Vector2I};
use unicode_width::UnicodeWidthChar;
use vec1::Vec1;
use warpui_core::fonts::canvas::{Canvas, RasterFormat};
use warpui_core::fonts::{
    FamilyId, FontId, GlyphId, Metrics, Properties, RasterizedGlyph, SubpixelAlignment,
};
use warpui_core::platform::{self, LineStyle, TextLayoutSystem};
use warpui_core::rendering::GlyphConfig;
use warpui_core::text_layout::{
    CaretPosition, ClipConfig, Glyph, Line, Run, StyleAndFont, TextAlignment, TextFrame, TextStyle,
};

#[cfg(not(target_family = "wasm"))]
use futures::future::BoxFuture;
#[cfg(not(target_family = "wasm"))]
use futures::FutureExt;
#[cfg(not(target_family = "wasm"))]
use warpui_core::fonts::FontInfo;

/// Font units per em. Chosen so ascent/descent fall on tidy 0.8/0.2 fractions.
const UNITS_PER_EM: u32 = 1000;
/// Ascent in font units (0.8 em), placing the baseline 80% down each line.
const ASCENT: i16 = 800;
/// Descent in font units (-0.2 em); negative per the OpenType convention.
const DESCENT: i16 = -200;
const LINE_GAP: i16 = 0;

/// A fully fixed-width font + text-layout system that rasterizes to terminal
/// cells.
pub(super) struct FontDB;

impl FontDB {
    pub(super) fn new() -> Self {
        Self
    }
}

/// Ascent and descent in pixels for `font_size`, derived from the monospace
/// metrics. With the default 0.8/0.2 split this matches [`Line::empty`].
fn scaled_ascent_descent(font_size: f32) -> (f32, f32) {
    let scale = font_size / UNITS_PER_EM as f32;
    (ASCENT as f32 * scale, (DESCENT as f32).abs() * scale)
}

/// Returns the [`TextStyle`] that applies to the character at `char_index`,
/// falling back to the default style for any uncovered position.
fn style_for_char(style_runs: &[(Range<usize>, StyleAndFont)], char_index: usize) -> TextStyle {
    style_runs
        .iter()
        .find(|(range, _)| range.contains(&char_index))
        .map(|(_, style_and_font)| style_and_font.style)
        .unwrap_or_default()
}

/// Lays out a single line of `text` into a [`Line`].
///
/// `char_offset` is the character position of this line within the larger
/// input, so style-run lookup and glyph/caret indices (all char-indexed) stay
/// correct for multi-line frames.
fn build_line(
    text: &str,
    line_style: LineStyle,
    style_runs: &[(Range<usize>, StyleAndFont)],
    clip_config: Option<ClipConfig>,
    char_offset: usize,
) -> Line {
    if text.is_empty() {
        return Line::empty(
            line_style.font_size,
            line_style.line_height_ratio,
            char_offset,
        );
    }

    let (ascent, descent) = scaled_ascent_descent(line_style.font_size);

    let mut runs: Vec<Run> = Vec::new();
    let mut caret_positions: Vec<CaretPosition> = Vec::new();
    // Running x position, measured in whole display columns (== cells).
    let mut column = 0.0_f32;

    for (char_index, ch) in text.chars().enumerate() {
        let columns = UnicodeWidthChar::width(ch).unwrap_or(0) as f32;
        // Char index into the whole input; matches style-run ranges and the
        // other backends, keeping style spans and selection correct.
        let index = char_offset + char_index;
        let style = style_for_char(style_runs, index);

        let glyph = Glyph {
            // Identity mapping: GlyphId is a u32 codepoint the renderer decodes.
            id: ch as GlyphId,
            position_along_baseline: vec2f(column, 0.0),
            index,
            width: columns,
        };

        // Coalesce consecutive characters sharing a style into one run so the
        // painter applies per-segment colors; all glyphs use the single font.
        match runs.last_mut() {
            Some(run) if run.styles == style => {
                run.glyphs.push(glyph);
                run.width += columns;
            }
            _ => runs.push(Run {
                font_id: FontId(0),
                glyphs: vec![glyph],
                styles: style,
                width: columns,
            }),
        }

        caret_positions.push(CaretPosition {
            position_in_line: column,
            start_offset: index,
            last_offset: index,
        });

        column += columns;
    }

    let trailing_whitespace_width: f32 = text
        .chars()
        .rev()
        .take_while(|ch| ch.is_whitespace())
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0) as f32)
        .sum();

    Line {
        width: column,
        trailing_whitespace_width,
        runs,
        font_size: line_style.font_size,
        line_height_ratio: line_style.line_height_ratio,
        baseline_ratio: line_style.baseline_ratio,
        clip_config,
        ascent,
        descent,
        caret_positions,
        chars_with_missing_glyphs: Vec::new(),
    }
}

impl platform::FontDB for FontDB {
    fn load_from_bytes(&mut self, _name: &str, _bytes: Vec<Vec<u8>>) -> Result<FamilyId> {
        Ok(FamilyId(0))
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_from_system(&mut self, _font_family: &str) -> Result<FamilyId> {
        Ok(FamilyId(0))
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_all_system_fonts(&self) -> BoxFuture<'static, Box<dyn platform::LoadedSystemFonts>> {
        futures::future::ready(Box::new(LoadedSystemFonts) as Box<dyn platform::LoadedSystemFonts>)
            .boxed()
    }

    #[cfg(not(target_family = "wasm"))]
    fn process_loaded_system_fonts(
        &mut self,
        loaded_system_fonts: Box<dyn platform::LoadedSystemFonts>,
    ) -> Vec<(Option<FamilyId>, FontInfo)> {
        let _loaded_system_fonts: Box<LoadedSystemFonts> = loaded_system_fonts
            .as_any()
            .downcast()
            .expect("should not fail to downcast to concrete type");
        vec![]
    }

    fn fallback_fonts(&self, _character: char, _font_id: FontId) -> Vec<FontId> {
        vec![]
    }

    fn select_font(&self, _family_id: FamilyId, _properties: Properties) -> FontId {
        FontId(0)
    }

    fn font_metrics(&self, _font_id: FontId) -> Metrics {
        Metrics {
            units_per_em: UNITS_PER_EM,
            ascent: ASCENT,
            descent: DESCENT,
            line_gap: LINE_GAP,
        }
    }

    fn glyph_advance(&self, _font_id: FontId, _glyph_id: GlyphId) -> Result<Vector2I> {
        // One em wide; per-column widths (incl. wide chars) are handled in layout.
        Ok(vec2i(UNITS_PER_EM as i32, 0))
    }

    fn load_family_name_from_id(&self, _id: FamilyId) -> Option<String> {
        None
    }

    fn glyph_raster_bounds(
        &self,
        _font_id: FontId,
        _size: f32,
        _glyph_id: GlyphId,
        _scale: Vector2F,
        _glyph_config: &GlyphConfig,
    ) -> Result<RectI> {
        Ok(RectI::default())
    }

    fn glyph_typographic_bounds(&self, _font_id: FontId, _glyph_id: GlyphId) -> Result<RectI> {
        Ok(RectI::default())
    }

    #[allow(clippy::too_many_arguments)]
    fn rasterize_glyph(
        &self,
        _font_id: FontId,
        _size: f32,
        _glyph_id: GlyphId,
        _scale: Vector2F,
        _subpixel_alignment: SubpixelAlignment,
        _glyph_config: &GlyphConfig,
        _format: RasterFormat,
    ) -> Result<RasterizedGlyph> {
        // The TUI never rasterizes glyphs to pixels; it writes characters.
        Ok(RasterizedGlyph {
            canvas: Canvas {
                pixels: vec![],
                size: vec2i(0, 0),
                row_stride: 0,
                format: RasterFormat::Rgba32,
            },
            is_emoji: false,
        })
    }

    fn glyph_for_char(&self, _font_id: FontId, ch: char) -> Option<GlyphId> {
        Some(ch as GlyphId)
    }

    fn family_id_for_name(&self, _name: &str) -> Option<FamilyId> {
        None
    }

    fn text_layout_system(&self) -> &dyn TextLayoutSystem {
        self
    }
}

impl TextLayoutSystem for FontDB {
    fn layout_line(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        _max_width: f32,
        clip_config: ClipConfig,
    ) -> Line {
        build_line(text, line_style, style_runs, Some(clip_config), 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_text(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        _max_width: f32,
        _max_height: f32,
        alignment: TextAlignment,
        _first_line_head_indent: Option<f32>,
    ) -> TextFrame {
        if text.is_empty() {
            return TextFrame::empty(line_style.font_size, line_style.line_height_ratio);
        }

        let mut lines: Vec<Line> = Vec::new();
        let mut char_offset = 0;
        let mut max_width = 0.0_f32;

        for line_text in text.split('\n') {
            let line = build_line(line_text, line_style, style_runs, None, char_offset);
            max_width = max_width.max(line.width);
            lines.push(line);
            // Advance past the line and its '\n' separator (one character).
            char_offset += line_text.chars().count() + 1;
        }

        match Vec1::try_from_vec(lines) {
            Ok(lines) => TextFrame::new(lines, max_width, alignment),
            Err(_) => TextFrame::empty(line_style.font_size, line_style.line_height_ratio),
        }
    }
}

/// Marker returned by [`FontDB::load_all_system_fonts`]; the TUI has no system
/// fonts to enumerate.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct LoadedSystemFonts;

impl platform::LoadedSystemFonts for LoadedSystemFonts {
    fn as_any(self: Box<Self>) -> Box<dyn Any> {
        self as Box<dyn Any>
    }
}
