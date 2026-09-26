//! This module defines CellGlyphCache, a struct which manages the caching of glyph values for cells
//! when rendering Grids within Warp.
use std::collections::HashMap;

use warpui::PaintContext;
use warpui::elements::DEFAULT_LINE_HEIGHT_RATIO;
use warpui::fonts::{Cache as FontCache, FamilyId, FontId, GlyphId, Properties};
use warpui::geometry::vector::Vector2F;
use warpui::platform::LineStyle;
use warpui::text_layout::{DEFAULT_TOP_BOTTOM_RATIO, StyleAndFont};

/// A glyph plus its baseline-relative position from the start of the laid-out string.
/// We carry the position so combining marks (e.g. Thai ั / ี) land at the correct visual
/// offset from the base consonant — drawing every glyph at the same origin makes marks
/// pile up wherever the font happens to put them, which is rarely above the right base.
pub(super) type PositionedGlyph = (GlyphId, FontId, Vector2F);

/// Stores cached glyph values for characters/strings. Note that we normally only need to look up
/// characters - we only look up strings in the case of zerowidth characters (which act as modifiers
/// to the first character e.g. emoji variant selectors). We have 2 separate caches internally for
/// performance reasons (avoid allocating strings when we don't need to!).
#[derive(Default)]
pub struct CellGlyphCache {
    glyph_cache: HashMap<(char, FontId), Option<(GlyphId, FontId)>>,
    // Keyed on font_size bits too: the cached positions are baseline-relative
    // layout offsets that change with the font size, so different sizes must not
    // share an entry (otherwise zooming reuses stale mark offsets).
    string_cache: HashMap<(String, FontId, u32), Vec<PositionedGlyph>>,
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
    pub(super) fn glyphs_for_string(
        &mut self,
        string: &str,
        font_id: FontId,
        font_cache: &FontCache,
        font_family: FamilyId,
        font_size: f32,
        properties: Properties,
        ctx: &mut PaintContext,
    ) -> Vec<PositionedGlyph> {
        let cached = self
            .string_cache
            .entry((string.to_owned(), font_id, font_size.to_bits()))
            .or_insert_with(|| {
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
                let Some(run) = line.runs.first() else {
                    return vec![];
                };
                // If the shaper produced any missing glyph (id 0 / .notdef), treat this as
                // a miss and return empty so the per-character fallback below runs instead
                // of caching tofu.
                if run.glyphs.iter().any(|g| g.id == 0) {
                    return vec![];
                }
                run.glyphs
                    .iter()
                    .map(|g| (g.id, run.font_id, g.position_along_baseline))
                    .collect()
            })
            .clone();

        if cached.is_empty() {
            // layout_line may return empty when the primary font has no glyphs for the script
            // (e.g. Thai with a Latin terminal font). Fall back to per-character lookup, but
            // anchor on the *first* character: it is the base consonant, and its fallback font
            // is the script font (e.g. Thai) that also contains every following combining mark.
            // Looking up combining marks like ั / ี standalone via DirectWrite returns nothing
            // — they have no script context on their own — so we MUST reuse the base font here.
            #[cfg(debug_assertions)]
            log::warn!("Falling back to per-character glyph lookup for: {string:?}");
            let mut chars = string.chars();
            let Some(first_char) = chars.next() else {
                return vec![];
            };
            let Some((first_glyph, base_font_id)) =
                self.glyph_for_char(first_char, font_id, font_cache)
            else {
                return vec![];
            };
            // Without HarfBuzz shaping we have no real positions; stack everything on the cell
            // origin. This is suboptimal for combining marks, but better than dropping them.
            let mut fallback: Vec<PositionedGlyph> =
                vec![(first_glyph, base_font_id, Vector2F::zero())];
            for c in chars {
                if let Some((glyph_id, _)) = font_cache.glyph_for_char(base_font_id, c, false) {
                    fallback.push((glyph_id, base_font_id, Vector2F::zero()));
                }
            }
            self.string_cache.insert(
                (string.to_owned(), font_id, font_size.to_bits()),
                fallback.clone(),
            );
            return fallback;
        }
        cached
    }
}
