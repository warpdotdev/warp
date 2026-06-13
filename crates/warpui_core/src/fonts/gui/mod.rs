//! GUI-backend extensions to the font [`Cache`].

mod text_layout_system;

use anyhow::{Error, Result};
use dashmap::mapref::entry::Entry;
use ordered_float::OrderedFloat;
use pathfinder_geometry::rect::RectI;
use pathfinder_geometry::vector::Vector2F;
pub use text_layout_system::TextLayoutSystem;

use super::{canvas, Cache, RasterizedGlyph, SubpixelAlignment};
use crate::rendering;
use crate::scene::GlyphKey;

pub(super) type RasterBoundsKey = (GlyphKey, (OrderedFloat<f32>, OrderedFloat<f32>));

impl Cache {
    /// Returns the [`TextLayoutSystem`], which can be used to layout text either on the main thread
    /// or in the background.
    pub fn text_layout_system(&self) -> TextLayoutSystem<'_> {
        TextLayoutSystem {
            platform: self.font_db().text_layout_system(),
            cache: &self.font_fallback_cache,
        }
    }

    pub fn glyph_raster_bounds(
        &self,
        glyph_key: GlyphKey,
        scale: Vector2F,
        glyph_config: &rendering::GlyphConfig,
    ) -> Result<RectI> {
        let entry = self
            .raster_bounds
            .entry((glyph_key, (scale.x().into(), scale.y().into())));
        let bounds = match entry {
            Entry::Occupied(entry) => entry.into_ref(),
            Entry::Vacant(entry) => entry.insert(self.platform.glyph_raster_bounds(
                glyph_key.font_id,
                glyph_key.font_size.into(),
                glyph_key.glyph_id,
                scale,
                glyph_config,
            )),
        };
        match bounds.value() {
            Ok(bounds) => Ok(*bounds),
            Err(error) => Err(Error::msg(error.to_string())),
        }
    }

    pub fn rasterized_glyph(
        &self,
        glyph_key: GlyphKey,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &rendering::GlyphConfig,
        format: canvas::RasterFormat,
    ) -> Result<RasterizedGlyph> {
        self.platform.rasterize_glyph(
            glyph_key.font_id,
            glyph_key.font_size.into(),
            glyph_key.glyph_id,
            scale,
            subpixel_alignment,
            glyph_config,
            format,
        )
    }
}
