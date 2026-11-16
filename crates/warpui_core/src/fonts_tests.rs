use anyhow::{Result, anyhow};
use pathfinder_geometry::rect::RectI;
use pathfinder_geometry::vector::{Vector2F, Vector2I, vec2i};

use super::*;
use crate::platform::{self, TextLayoutSystem};

#[test]
fn test_subpixel_alignment_computation() {
    {
        // Default case - a non-fractional offset should have an alignment
        // value of zero.
        let pos = vec2f(0., 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 0);
    }
    {
        // 0.1 rounds down to 0 (not up to 0.33 -> 1)
        let pos = vec2f(0.1, 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 0);
    }
    {
        // y-position doesn't affect computation
        let pos = vec2f(0.1, 0.33);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 0);
    }
    {
        // 0.2 rounds up to 0.33 -> 1
        let pos = vec2f(0.2, 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 1);
    }
    {
        // 0.66 doesn't round, and converts to 2
        let pos = vec2f(0.66, 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 2);
    }
    {
        // 0.9 rounds up to 1.0 -> 0
        let pos = vec2f(0.9, 0.);
        let alignment = SubpixelAlignment::new(pos);
        assert_eq!(alignment.0, 0);
    }
}

struct EmWidthFontDB {
    bounds: Option<RectI>,
    advance: Option<Vector2I>,
}

impl EmWidthFontDB {
    fn cache(bounds: Option<RectI>, advance: Option<Vector2I>) -> Cache {
        Cache::new(Box::new(Self { bounds, advance }))
    }
}

impl platform::FontDB for EmWidthFontDB {
    fn load_from_bytes(&mut self, _name: &str, _bytes: Vec<Vec<u8>>) -> Result<FamilyId> {
        unimplemented!()
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_from_system(&mut self, _font_family: &str) -> Result<FamilyId> {
        unimplemented!()
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_all_system_fonts(
        &self,
    ) -> futures::future::BoxFuture<'static, Box<dyn platform::LoadedSystemFonts>> {
        unimplemented!()
    }

    #[cfg(not(target_family = "wasm"))]
    fn process_loaded_system_fonts(
        &mut self,
        _loaded_system_fonts: Box<dyn platform::LoadedSystemFonts>,
    ) -> Vec<(Option<FamilyId>, FontInfo)> {
        unimplemented!()
    }

    fn fallback_fonts(&self, _ch: char, _font_id: FontId) -> Vec<FontId> {
        unimplemented!()
    }

    fn select_font(&self, _family_id: FamilyId, _properties: Properties) -> FontId {
        FontId(0)
    }

    fn font_metrics(&self, _font_id: FontId) -> Metrics {
        Metrics {
            units_per_em: 16,
            ascent: 12,
            descent: -4,
            line_gap: 0,
        }
    }

    fn glyph_advance(&self, _font_id: FontId, _glyph_id: GlyphId) -> Result<Vector2I> {
        self.advance.ok_or_else(|| anyhow!("No advance for glyph"))
    }

    fn load_family_name_from_id(&self, _id: FamilyId) -> Option<String> {
        unimplemented!()
    }

    fn glyph_raster_bounds(
        &self,
        _font_id: FontId,
        _size: f32,
        _glyph_id: GlyphId,
        _scale: Vector2F,
        _glyph_config: &crate::rendering::GlyphConfig,
    ) -> Result<RectI> {
        unimplemented!()
    }

    fn glyph_typographic_bounds(&self, _font_id: FontId, glyph_id: GlyphId) -> Result<RectI> {
        self.bounds
            .ok_or_else(|| anyhow!("No bounding box for glyph id {glyph_id}"))
    }

    fn rasterize_glyph(
        &self,
        _font_id: FontId,
        _size: f32,
        _glyph_id: GlyphId,
        _scale: Vector2F,
        _subpixel_alignment: SubpixelAlignment,
        _glyph_config: &crate::rendering::GlyphConfig,
        _format: canvas::RasterFormat,
    ) -> Result<RasterizedGlyph> {
        unimplemented!()
    }

    fn glyph_for_char(&self, _font_id: FontId, _char: char) -> Option<GlyphId> {
        Some(0)
    }

    fn family_id_for_name(&self, _name: &str) -> Option<FamilyId> {
        unimplemented!()
    }

    fn text_layout_system(&self) -> &dyn TextLayoutSystem {
        unimplemented!()
    }
}

#[test]
fn em_width_uses_typographic_bounds_width_when_available() {
    let cache = EmWidthFontDB::cache(
        Some(RectI::new(vec2i(0, 0), vec2i(10, 12))),
        Some(vec2i(20, 0)),
    );

    assert_eq!(cache.em_width(FamilyId(0), 16.0), 10.0);
}

#[test]
fn em_width_uses_horizontal_advance_when_typographic_bounds_are_missing() {
    let cache = EmWidthFontDB::cache(None, Some(vec2i(20, 0)));

    assert_eq!(cache.em_width(FamilyId(0), 16.0), 20.0);
}

#[test]
fn em_width_does_not_panic_when_bounds_and_advance_are_missing() {
    let cache = EmWidthFontDB::cache(None, None);

    assert_eq!(cache.em_width(FamilyId(0), 16.0), 16.0);
}

#[test]
fn em_width_falls_back_when_horizontal_advance_is_zero() {
    let cache = EmWidthFontDB::cache(None, Some(vec2i(0, 0)));

    assert_eq!(cache.em_width(FamilyId(0), 16.0), 16.0);
}
