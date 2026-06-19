//! This module defines CellGlyphCache, a struct which manages the caching of glyph values for cells
//! when rendering Grids within Warp.
use std::collections::HashMap;

use warpui::elements::DEFAULT_LINE_HEIGHT_RATIO;
use warpui::fonts::{Cache as FontCache, FamilyId, FontId, GlyphId, Properties};
use warpui::platform::LineStyle;
use warpui::text_layout::{StyleAndFont, DEFAULT_TOP_BOTTOM_RATIO};
use warpui::PaintContext;

/// Stores cached glyph values for characters/strings. Note that we normally only need to look up
/// characters - we only look up strings in the case of zerowidth characters (which act as modifiers
/// to the first character e.g. emoji variant selectors). We have 2 separate caches internally for
/// performance reasons (avoid allocating strings when we don't need to!).
#[derive(Default)]
pub struct CellGlyphCache {
    glyph_cache: HashMap<(char, FontId), Option<(GlyphId, FontId)>>,
    string_cache: HashMap<(String, FontId), Option<(GlyphId, FontId)>>,
}

impl CellGlyphCache {
    pub(super) fn glyph_for_char(
        &mut self,
        char: char,
        font_id: FontId,
        font_cache: &FontCache,
    ) -> Option<(GlyphId, FontId)> {
        *self
            .glyph_cache
            .entry((char, font_id))
            .or_insert_with(|| font_cache.glyph_for_char(font_id, char, true))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn glyph_for_string(
        &mut self,
        string: &str,
        font_id: FontId,
        font_cache: &FontCache,
        font_family: FamilyId,
        font_size: f32,
        properties: Properties,
        ctx: &mut PaintContext,
    ) -> Option<(GlyphId, FontId)> {
        let glyph = *self
            .string_cache
            .entry((string.to_owned(), font_id))
            .or_insert_with(|| {
                // Calculate the length of total characters in the string.
                let run_length_chars = string.chars().count();
                let line = ctx.text_layout_cache.layout_line(
                    string,
                    LineStyle {
                        font_size,
                        // Note that we DO NOT paint the `Line` in this particular instance. As such,
                        // the line height ratio and baseline ratio are both NOT used. Hence, we arbitrarily
                        // set them to the default values.
                        line_height_ratio: DEFAULT_LINE_HEIGHT_RATIO,
                        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                        fixed_width_tab_size: None,
                        skip_caret_positions: false,
                    },
                    &[(
                        (0..run_length_chars),
                        StyleAndFont {
                            font_family,
                            properties,
                            style: Default::default(),
                        },
                    )],
                    f32::MAX,
                    Default::default(),
                    &font_cache.text_layout_system(),
                );
                let run = line.runs.first()?;
                if run.glyphs.len() > 1 {
                    // If we have more than one glyph, something has gone wrong.
                    return None;
                }
                run.glyphs.first().map(|glyph| (glyph.id, run.font_id))
            });

        glyph.or_else(|| {
            #[cfg(debug_assertions)]
            log::warn!("Falling back to glyph for first character of string, could not get glyph for entire string: {string:?}");
            let first_char = string.chars().next()?;
            let glyph = self.glyph_for_char(first_char, font_id, font_cache);
            // Make sure we update the cache with the fallback, so we don't
            // recompute it again.
            self.string_cache.insert((string.to_owned(), font_id), glyph);
            glyph
        })
    }
}
