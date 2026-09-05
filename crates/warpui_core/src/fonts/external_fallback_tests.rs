use std::sync::Arc;

use vec1::vec1;

use super::*;
use crate::fonts::TextLayoutSystem;
use crate::platform::{self, LineStyle};
use crate::text_layout::{ClipConfig, LayoutCache, Line, StyleAndFont, TextAlignment, TextFrame};

struct MissingGlyphTextLayout;

impl platform::TextLayoutSystem for MissingGlyphTextLayout {
    fn layout_line(
        &self,
        _text: &str,
        line_style: LineStyle,
        _style_runs: &[(std::ops::Range<usize>, StyleAndFont)],
        _max_width: f32,
        _clip_config: ClipConfig,
    ) -> Line {
        Line::empty(line_style.font_size, line_style.line_height_ratio, 0)
    }

    fn layout_text(
        &self,
        _text: &str,
        line_style: LineStyle,
        _style_runs: &[(std::ops::Range<usize>, StyleAndFont)],
        _max_width: f32,
        _max_height: f32,
        alignment: TextAlignment,
        _first_line_head_indent: Option<f32>,
    ) -> TextFrame {
        let mut line = Line::empty(line_style.font_size, line_style.line_height_ratio, 0);
        line.chars_with_missing_glyphs.push('⌘');
        TextFrame::new(vec1![line], 0.0, alignment)
    }
}

#[test]
fn sequential_redraw_only_layouts_retain_one_keyless_source() {
    let cache = FontFallbackCache {
        fallback_font_fn: Some(Box::new(|_| {
            Some(ExternalFontFamily {
                font_urls: Arc::new(vec!["fallback".to_string()]),
                name: "fallback",
            })
        })),
        ..Default::default()
    };
    let platform = MissingGlyphTextLayout;
    let text_layout_system = TextLayoutSystem {
        platform: &platform,
        cache: &cache,
    };
    let line_style = LineStyle {
        font_size: 12.0,
        line_height_ratio: 1.0,
        baseline_ratio: 0.8,
        fixed_width_tab_size: None,
    };

    for prefix_chars in 1..=1_000 {
        let throwaway_cache = LayoutCache::new();
        throwaway_cache.layout_text_redraw_only(
            &"x".repeat(prefix_chars),
            line_style,
            &[],
            f32::MAX,
            f32::MAX,
            TextAlignment::Left,
            None,
            &text_layout_system,
        );
    }

    let requests = cache
        .requested_fallback_families
        .iter()
        .next()
        .expect("fallback family should be requested");
    assert_eq!(requests.value().len(), 1);
    assert!(matches!(
        requests.value().as_slice(),
        [RequestedFallbackFontSource::RedrawOnly]
    ));
    assert_eq!(cache.requested_redraw_only_families.len(), 1);
}
