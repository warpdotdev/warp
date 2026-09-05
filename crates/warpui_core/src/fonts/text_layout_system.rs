use std::ops::Range;

use crate::fonts::{FontFallbackCache, RequestedFallbackFontSource};
use crate::platform;
use crate::platform::LineStyle;
use crate::text_layout::{
    ClipConfig, Line, StyleAndFont, TextAlignment, TextFrame, strip_leading_unicode_bom,
};

/// Struct to layout text, updating cached font fallback state as needed.
/// See [fonts::Cache::text_layout_system].
pub struct TextLayoutSystem<'a> {
    pub(super) platform: &'a dyn platform::TextLayoutSystem,
    pub(super) cache: &'a FontFallbackCache,
}

impl TextLayoutSystem<'_> {
    /// Checks if the application specified a fallback font for the given char.
    /// If yes, the UI framework will lazy load the fallback font and trigger
    /// a re-render of the window.
    pub(crate) fn request_fallback_font_for_char(
        &self,
        ch: char,
        source: RequestedFallbackFontSource,
    ) {
        self.cache.request_fallback_font_for_char(ch, source)
    }

    pub fn layout_line(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        clip_config: ClipConfig,
    ) -> Line {
        self.platform
            .layout_line(text, line_style, style_runs, max_width, clip_config)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn layout_text(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        max_height: f32,
        alignment: TextAlignment,
        first_line_head_indent: Option<f32>,
    ) -> TextFrame {
        self.platform.layout_text(
            text,
            line_style,
            style_runs,
            max_width,
            max_height,
            alignment,
            first_line_head_indent,
        )
    }

    /// Lays out a text frame without retaining it in a [`crate::text_layout::LayoutCache`].
    #[allow(clippy::too_many_arguments)]
    pub fn layout_text_uncached(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        max_height: f32,
        alignment: TextAlignment,
        first_line_head_indent: Option<f32>,
    ) -> TextFrame {
        let (text, adjusted_style_runs) = strip_leading_unicode_bom(text, style_runs);
        let style_runs = adjusted_style_runs
            .as_ref()
            .map_or(style_runs, Vec::as_slice);
        let text_frame = self.layout_text(
            text,
            line_style,
            style_runs,
            max_width,
            max_height,
            alignment,
            first_line_head_indent,
        );
        for line in text_frame.lines() {
            self.request_fallback_fonts(&line.chars_with_missing_glyphs);
        }
        text_frame
    }

    fn request_fallback_fonts(&self, missing_glyphs: &[char]) {
        for ch in missing_glyphs {
            self.request_fallback_font_for_char(*ch, RequestedFallbackFontSource::UncachedText);
        }
    }
}
