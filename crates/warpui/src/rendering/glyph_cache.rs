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
    cache: HashMap<GlyphCacheKey, GlyphTextureOffset>,
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
                let bounds =
                    raster_bounds_fn(glyph_key, Vector2F::splat(scale_factor), &self.glyph_config)?;

                if bounds.size() == Vector2I::zero() {
                    return Ok(None);
                }

                let rasterized_glyph = rasterize_glyph_fn(
                    glyph_key,
                    Vector2F::splat(scale_factor),
                    subpixel_alignment,
                    &self.glyph_config,
                    crate::fonts::canvas::RasterFormat::Rgba32,
                )?;

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

                self.cache.insert(cache_key, glyph_texture_offset);
                Ok(Some(glyph_texture_offset))
            }
            Some(gto) => Ok(Some(*gto)),
        }
    }
}
