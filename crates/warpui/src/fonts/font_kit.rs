//! A text rasterizer backed by [`font_kit`] that supports rasterizing at subpixel offsets

use std::sync::Arc;

use anyhow::Result;
use dashmap::DashMap;
use font_kit::canvas::{AntialiasingStrategy, Canvas, RasterizationOptions};
use font_kit::font::Font;
use font_kit::hinting::HintingOptions;
use pathfinder_geometry::rect::RectI;
use pathfinder_geometry::transform2d::Transform2F;
use pathfinder_geometry::vector::{Vector2F, Vector2I, vec2i};
use warpui_core::fonts::canvas::RasterFormat;
use warpui_core::fonts::{
    FontId, GlyphId, Properties, RasterizedGlyph, Style, SubpixelAlignment, Weight,
};
use warpui_core::rendering;

#[cfg(target_os = "macos")]
use crate::platform::mac::AutoreleasePoolGuard;

/// A simpler rasterizer backed by font-kit.
pub(crate) struct Rasterizer {
    fonts: DashMap<FontId, Arc<Font>>,
}

impl Rasterizer {
    pub fn new() -> Self {
        Self {
            fonts: Default::default(),
        }
    }

    pub fn insert(&self, font_id: FontId, font: Arc<Font>) {
        self.fonts.insert(font_id, font);
    }

    pub fn font_for_id(&self, font_id: FontId) -> Arc<Font> {
        self.fonts.get(&font_id).expect("Font must exist").clone()
    }

    pub fn glyph_raster_bounds(
        &self,
        font_id: FontId,
        point_size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        glyph_config: &rendering::GlyphConfig,
    ) -> Result<RectI> {
        let raw_raster_bounds = self.font_for_id(font_id).raster_bounds(
            glyph_id,
            point_size,
            Transform2F::from_scale(scale),
            HintingOptions::None,
            RasterizationOptions {
                antialiasing_strategy: AntialiasingStrategy::GrayscaleAa,
                use_thin_strokes: glyph_config
                    .use_thin_strokes
                    .enabled_for_scale_factor(scale.x()),
            },
        )?;
        if raw_raster_bounds.size() == Vector2I::zero() {
            // Don't adjust the size of a glyph with a default size of zero.
            return Ok(raw_raster_bounds);
        }
        // The default raster bounds provided by font-kit sometimes clip pixels
        // off of anti-aliased glyphs; add one pixel to the glyph bounds to
        // compensate.  We only adjust the origin vertically because the extra
        // pixel of height changes the baseline; the extra pixel on the right
        // side doesn't change positioning (as the origin is on the left edge of
        // the glyph).
        let fudge_factor = vec2i(1, 1);
        let offset = vec2i(0, 1);
        Ok(RectI::new(
            raw_raster_bounds.origin() - offset,
            raw_raster_bounds.size() + fudge_factor,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_glyph(
        &self,
        font_id: FontId,
        point_size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &rendering::GlyphConfig,
        format: RasterFormat,
    ) -> Result<RasterizedGlyph> {
        // On macOS, this function calls into Core Graphics and Core Text
        // (`CGBitmapContextCreate` per glyph plus `raster_bounds` reading font
        // metadata), each of which leaves transient bookkeeping objects in the
        // thread's autorelease pool. Because this is invoked during Metal
        // frame rendering on the main thread, hundreds of those objects can
        // accumulate between run-loop turns before AppKit's outer pool
        // drains. A local pool bounds that peak without relying on the outer
        // pool. The guard drains on `Drop`, covering the error paths from `?`
        // below and any panics from `font_kit`.
        #[cfg(target_os = "macos")]
        let _pool = AutoreleasePoolGuard::new();

        let bounds =
            self.glyph_raster_bounds(font_id, point_size, glyph_id, scale, glyph_config)?;
        let mut canvas = Canvas::new(bounds.size(), raster_format_to_font_kit(format));

        let base_transform = Transform2F::from_scale(scale).translate(-bounds.origin().to_f32());
        let aligned_transform = base_transform.translate(subpixel_alignment.to_offset());

        self.font_for_id(font_id).rasterize_glyph(
            &mut canvas,
            glyph_id,
            point_size,
            aligned_transform,
            HintingOptions::None,
            RasterizationOptions {
                antialiasing_strategy: AntialiasingStrategy::GrayscaleAa,
                use_thin_strokes: glyph_config
                    .use_thin_strokes
                    .enabled_for_scale_factor(scale.x()),
            },
        )?;

        Ok(RasterizedGlyph {
            canvas: canvas.into(),
            // TODO(alokedesai): Properly support colored glyphs on Windows.
            is_emoji: self.font_for_id(font_id).is_colored() && !cfg!(windows),
        })
    }
}

pub fn properties_to_font_kit(properties: Properties) -> font_kit::properties::Properties {
    font_kit::properties::Properties {
        style: style_to_font_kit(properties.style),
        weight: weight_to_font_kit(properties.weight),
        stretch: Default::default(),
    }
}

fn raster_format_to_font_kit(format: RasterFormat) -> font_kit::canvas::Format {
    use font_kit::canvas::Format as FKFormat;
    match format {
        RasterFormat::Rgba32 => FKFormat::Rgba32,
        RasterFormat::Rgb24 => FKFormat::Rgb24,
        RasterFormat::A8 => FKFormat::Rgb24,
    }
}

fn weight_to_font_kit(weight: Weight) -> font_kit::properties::Weight {
    match weight {
        Weight::Thin => font_kit::properties::Weight::THIN,
        Weight::ExtraLight => font_kit::properties::Weight::EXTRA_LIGHT,
        Weight::Light => font_kit::properties::Weight::LIGHT,
        Weight::Normal => font_kit::properties::Weight::NORMAL,
        Weight::Medium => font_kit::properties::Weight::MEDIUM,
        Weight::Semibold => font_kit::properties::Weight::SEMIBOLD,
        Weight::Bold => font_kit::properties::Weight::BOLD,
        Weight::ExtraBold => font_kit::properties::Weight::EXTRA_BOLD,
        Weight::Black => font_kit::properties::Weight::BLACK,
    }
}

fn style_to_font_kit(value: Style) -> font_kit::properties::Style {
    match value {
        Style::Normal => font_kit::properties::Style::Normal,
        Style::Italic => font_kit::properties::Style::Italic,
    }
}
