use std::ops::Range;
use std::sync::{Arc, Mutex};

use super::*;
use crate::fonts::{ExternalFontFamily, FamilyId, FontFallbackCache, Properties};
use crate::text_layout::{StyleAndFont, TextStyle};

#[derive(Default)]
struct RecordingLayoutSystem {
    line_input: Mutex<Option<(String, Vec<Range<usize>>)>>,
    text_input: Mutex<Option<(String, Vec<Range<usize>>)>>,
    missing_glyph: Option<char>,
}

impl platform::TextLayoutSystem for RecordingLayoutSystem {
    fn layout_line(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        _max_width: f32,
        _clip_config: ClipConfig,
    ) -> Line {
        *self.line_input.lock().unwrap() = Some((
            text.to_owned(),
            style_runs.iter().map(|(range, _)| range.clone()).collect(),
        ));
        let mut line = Line::empty(line_style.font_size, line_style.line_height_ratio, 0);
        line.chars_with_missing_glyphs.extend(self.missing_glyph);
        line
    }

    fn layout_text(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        _max_width: f32,
        _max_height: f32,
        _alignment: TextAlignment,
        _first_line_head_indent: Option<f32>,
    ) -> TextFrame {
        *self.text_input.lock().unwrap() = Some((
            text.to_owned(),
            style_runs.iter().map(|(range, _)| range.clone()).collect(),
        ));
        TextFrame::empty(line_style.font_size, line_style.line_height_ratio)
    }
}

fn style() -> StyleAndFont {
    StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::default())
}

fn line_style() -> LineStyle {
    LineStyle {
        font_size: 13.,
        line_height_ratio: 1.2,
        baseline_ratio: 0.8,
        fixed_width_tab_size: None,
    }
}

#[test]
fn uncached_line_layout_strips_a_leading_bom_and_adjusts_styles() {
    let platform = RecordingLayoutSystem::default();
    let cache = FontFallbackCache::default();
    let system = TextLayoutSystem {
        platform: &platform,
        cache: &cache,
    };

    system.layout_line_uncached(
        "\u{feff}hello",
        line_style(),
        &[(0..1, style()), (1..6, style())],
        100.,
        ClipConfig::end(),
    );

    assert_eq!(
        platform.line_input.lock().unwrap().as_ref(),
        Some(&("hello".to_owned(), vec![0..0, 0..5]))
    );
}

#[test]
fn uncached_text_layout_strips_a_leading_bom_and_adjusts_styles() {
    let platform = RecordingLayoutSystem::default();
    let cache = FontFallbackCache::default();
    let system = TextLayoutSystem {
        platform: &platform,
        cache: &cache,
    };

    system.layout_text_uncached(
        "\u{feff}hello",
        line_style(),
        &[(0..1, style()), (1..6, style())],
        100.,
        100.,
        TextAlignment::Left,
        None,
    );

    assert_eq!(
        platform.text_input.lock().unwrap().as_ref(),
        Some(&("hello".to_owned(), vec![0..0, 0..5]))
    );
}

#[test]
fn uncached_layout_requests_fallback_fonts_for_missing_glyphs() {
    let missing_glyph = '🦀';
    let platform = RecordingLayoutSystem {
        missing_glyph: Some(missing_glyph),
        ..Default::default()
    };
    let cache = FontFallbackCache {
        fallback_font_fn: Some(Box::new(move |ch| {
            (ch == missing_glyph).then(|| ExternalFontFamily {
                font_urls: Arc::new(vec!["https://example.com/fallback.ttf".to_owned()]),
                name: "Test fallback",
            })
        })),
        ..Default::default()
    };
    let system = TextLayoutSystem {
        platform: &platform,
        cache: &cache,
    };

    system.layout_line_uncached(
        "🦀",
        line_style(),
        &[(0..1, style())],
        100.,
        ClipConfig::end(),
    );

    let requested = cache
        .requested_fallback_families
        .iter()
        .next()
        .expect("fallback font should be requested");
    assert_eq!(requested.key().name, "Test fallback");
    assert!(matches!(
        requested.value().as_slice(),
        [RequestedFallbackFontSource::UncachedText]
    ));
}
