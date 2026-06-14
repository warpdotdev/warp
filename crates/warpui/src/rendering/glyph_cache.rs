use std::collections::HashMap;

use anyhow::Result;
use ordered_float::OrderedFloat;
use pathfinder_geometry::rect::{RectF, RectI};
use pathfinder_geometry::vector::{Vector2F, Vector2I};

use crate::fonts::{canvas, RasterizedGlyph, SubpixelAlignment};
use crate::rendering;
use crate::rendering::atlas::{self, AllocatedRegion, TextureId};
use crate::scene::GlyphKey;

const ATLAS_SIZE: usize = 1024;

/// Callback to create a texture at a given size.
type CreateTextureCallback<'a, T> = dyn Fn(usize) -> T + 'a;

/// Callback to insert [`RasterizedGlyph`] at a region identified by [`AllocatedRegion`] into a
/// texture, `T`.
type InsertIntoTextureCallback<'a, T> = dyn Fn(AllocatedRegion, &RasterizedGlyph, &mut T) + 'a;

/// Callback to compute the bounds of a glyph when rasterized.
pub(crate) type GlyphRasterBoundsFn<'a> =
    dyn Fn(GlyphKey, Vector2F, &rendering::GlyphConfig) -> Result<RectI> + 'a;

/// Callback to rasterize a glyph.
pub(crate) type RasterizeGlyphFn<'a> = dyn Fn(
        GlyphKey,
        Vector2F,
        SubpixelAlignment,
        &rendering::GlyphConfig,
        canvas::RasterFormat,
    ) -> Result<RasterizedGlyph>
    + 'a;

/// A cache that caches glyphs in a texture atlas.  
pub struct GlyphCache<Texture> {
    textures: Vec<Texture>,
    cache: HashMap<GlyphCacheKey, Option<GlyphTextureOffset>>,
    glyph_config: rendering::GlyphConfig,
    atlas_manager: atlas::Manager,
}

#[derive(Hash, PartialEq, Eq)]
struct GlyphCacheKey {
    glyph_key: GlyphKey,
    scale_factor: OrderedFloat<f32>,
    subpixel_alignment: SubpixelAlignment,
}

impl GlyphCacheKey {
    fn new(glyph_key: GlyphKey, scale_factor: f32, subpixel_alignment: SubpixelAlignment) -> Self {
        GlyphCacheKey {
            glyph_key,
            scale_factor: scale_factor.into(),
            subpixel_alignment,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use anyhow::anyhow;
    use pathfinder_geometry::{
        rect::RectI,
        vector::{vec2f, vec2i},
    };
    use warpui_core::{
        fonts::{
            canvas::{Canvas, RasterFormat},
            FontId, RasterizedGlyph, SubpixelAlignment,
        },
        rendering::GlyphConfig,
        scene::GlyphKey,
    };

    use super::*;

    fn glyph_key(glyph_id: u32) -> GlyphKey {
        GlyphKey {
            glyph_id,
            font_id: FontId(0),
            font_size: 12.0.into(),
        }
    }

    fn rasterized_glyph() -> RasterizedGlyph {
        RasterizedGlyph {
            canvas: Canvas {
                pixels: vec![255; 16],
                size: vec2i(2, 2),
                row_stride: 8,
                format: RasterFormat::Rgba32,
            },
            is_emoji: false,
        }
    }

    #[test]
    fn raster_bounds_errors_are_cached_as_missing_glyphs() {
        let mut glyph_cache = GlyphCache::new(GlyphConfig::default());
        let glyph_key = glyph_key(1);
        let raster_bounds_calls = Cell::new(0);
        let rasterize_calls = Cell::new(0);

        for _ in 0..2 {
            let result = glyph_cache.get(
                glyph_key,
                1.0,
                SubpixelAlignment::new(vec2f(0.0, 0.0)),
                &|_| (),
                &|_, _, _| {},
                &|_, _, _| {
                    raster_bounds_calls.set(raster_bounds_calls.get() + 1);
                    Err(anyhow!("failed to get raster image"))
                },
                &|_, _, _, _, _| {
                    rasterize_calls.set(rasterize_calls.get() + 1);
                    Ok(rasterized_glyph())
                },
            );

            assert!(result.unwrap().is_none());
        }

        assert_eq!(raster_bounds_calls.get(), 1);
        assert_eq!(rasterize_calls.get(), 0);
    }

    #[test]
    fn rasterize_errors_are_cached_as_missing_glyphs() {
        let mut glyph_cache = GlyphCache::new(GlyphConfig::default());
        let glyph_key = glyph_key(1);
        let raster_bounds_calls = Cell::new(0);
        let rasterize_calls = Cell::new(0);

        for _ in 0..2 {
            let result = glyph_cache.get(
                glyph_key,
                1.0,
                SubpixelAlignment::new(vec2f(0.0, 0.0)),
                &|_| (),
                &|_, _, _| {},
                &|_, _, _| {
                    raster_bounds_calls.set(raster_bounds_calls.get() + 1);
                    Ok(RectI::new(vec2i(0, 0), vec2i(2, 2)))
                },
                &|_, _, _, _, _| {
                    rasterize_calls.set(rasterize_calls.get() + 1);
                    Err(anyhow!("failed to get raster image"))
                },
            );

            assert!(result.unwrap().is_none());
        }

        assert_eq!(raster_bounds_calls.get(), 1);
        assert_eq!(rasterize_calls.get(), 1);
    }

    #[test]
    fn cached_missing_glyphs_do_not_prevent_other_glyphs_from_rendering() {
        let mut glyph_cache = GlyphCache::new(GlyphConfig::default());
        let missing_glyph_key = glyph_key(1);
        let renderable_glyph_key = glyph_key(2);

        let missing_result = glyph_cache
            .get(
                missing_glyph_key,
                1.0,
                SubpixelAlignment::new(vec2f(0.0, 0.0)),
                &|_| (),
                &|_, _, _| {},
                &|_, _, _| Err(anyhow!("failed to get raster image")),
                &|_, _, _, _, _| Ok(rasterized_glyph()),
            )
            .unwrap();
        assert!(missing_result.is_none());

        let renderable_result = glyph_cache
            .get(
                renderable_glyph_key,
                1.0,
                SubpixelAlignment::new(vec2f(0.0, 0.0)),
                &|_| (),
                &|_, _, _| {},
                &|_, _, _| Ok(RectI::new(vec2i(0, 0), vec2i(2, 2))),
                &|_, _, _, _, _| Ok(rasterized_glyph()),
            )
            .unwrap();

        assert!(renderable_result.is_some());
    }
}

/// A glyph within a texture atlas.
#[derive(Copy, Debug, Clone)]
pub(crate) struct GlyphTextureOffset {
    pub texture_id: TextureId,
    pub allocated_region: AllocatedRegion,
    pub raster_bounds: RectF,
    pub is_emoji: bool,
}

impl<Texture> GlyphCache<Texture> {
    pub(crate) fn new(glyph_config: rendering::GlyphConfig) -> Self {
        GlyphCache {
            textures: Vec::new(),
            cache: HashMap::new(),
            glyph_config,
            atlas_manager: atlas::Manager::new(ATLAS_SIZE),
        }
    }

    pub(crate) fn update_config(&mut self, glyph_config: &rendering::GlyphConfig) {
        // If the glyph rendering configuration has changed, blow away the cache
        // and replace ourself with a new one.
        if *glyph_config != self.glyph_config {
            *self = GlyphCache::new(*glyph_config);
        }
    }

    /// Returns the texture identified by [`TextureId`].
    pub(crate) fn texture(&self, texture_id: &TextureId) -> Option<&Texture> {
        self.textures.get(texture_id.as_usize())
    }

    /// Returns a [`GlyphTextureOffset`] identified by [`GlyphKey`]. If the [`GlyphKey`] has not
    /// been previously cached, the glyph is rasterized and inserted into the texture via the
    /// `insert_into_texture` callback. If a new texture needs to be created (since a previous
    /// texture is now fill), the `create_texture` callback is called to construct a new texture
    /// atlas.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn get(
        &mut self,
        glyph_key: GlyphKey,
        scale_factor: f32,
        subpixel_alignment: SubpixelAlignment,
        create_texture: &CreateTextureCallback<'_, Texture>,
        insert_into_texture: &InsertIntoTextureCallback<'_, Texture>,
        raster_bounds_fn: &GlyphRasterBoundsFn<'_>,
        rasterize_glyph_fn: &RasterizeGlyphFn<'_>,
    ) -> Result<Option<GlyphTextureOffset>> {
        let cache_key = GlyphCacheKey::new(glyph_key, scale_factor, subpixel_alignment);

        match self.cache.get(&cache_key) {
            None => {
                let bounds = match raster_bounds_fn(
                    glyph_key,
                    Vector2F::splat(scale_factor),
                    &self.glyph_config,
                ) {
                    Ok(bounds) => bounds,
                    Err(err) => {
                        log::warn!("Unable to get glyph raster bounds: {err:?}, {glyph_key:?}");
                        self.cache.insert(cache_key, None);
                        return Ok(None);
                    }
                };

                if bounds.size() == Vector2I::zero() {
                    self.cache.insert(cache_key, None);
                    return Ok(None);
                }

                let rasterized_glyph = match rasterize_glyph_fn(
                    glyph_key,
                    Vector2F::splat(scale_factor),
                    subpixel_alignment,
                    &self.glyph_config,
                    crate::fonts::canvas::RasterFormat::Rgba32,
                ) {
                    Ok(rasterized_glyph) => rasterized_glyph,
                    Err(err) => {
                        log::warn!("Unable to rasterize glyph: {err:?}, {glyph_key:?}");
                        self.cache.insert(cache_key, None);
                        return Ok(None);
                    }
                };

                let texture_offset = self.atlas_manager.insert(rasterized_glyph.canvas.size)?;
                let idx = texture_offset.texture_id.as_usize();
                if idx >= self.textures.len() {
                    self.textures
                        .resize_with(idx + 1, || create_texture(ATLAS_SIZE));
                }
                let texture = &mut self.textures[idx];
                insert_into_texture(texture_offset.allocated_region, &rasterized_glyph, texture);

                let glyph_texture_offset = GlyphTextureOffset {
                    texture_id: texture_offset.texture_id,
                    raster_bounds: bounds.to_f32(),
                    is_emoji: rasterized_glyph.is_emoji,
                    allocated_region: texture_offset.allocated_region,
                };

                self.cache.insert(cache_key, Some(glyph_texture_offset));
                Ok(Some(glyph_texture_offset))
            }
            Some(gto) => Ok(*gto),
        }
    }
}
