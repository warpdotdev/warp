//! GUI-only pieces of the test platform delegate.

use anyhow::Result;
use pathfinder_geometry::vector::{vec2i, Vector2F};

use super::FontDB;
use crate::platform;
use crate::text_layout::TextAlignment;

impl platform::FontDBExt for FontDB {
    fn glyph_raster_bounds(
        &self,
        _font_id: crate::fonts::FontId,
        _size: f32,
        _glyph_id: crate::fonts::GlyphId,
        _scale: Vector2F,
        _glyph_config: &crate::rendering::GlyphConfig,
    ) -> Result<pathfinder_geometry::rect::RectI> {
        Ok(pathfinder_geometry::rect::RectI::default())
    }

    fn rasterize_glyph(
        &self,
        _font_id: crate::fonts::FontId,
        _size: f32,
        _glyph_id: crate::fonts::GlyphId,
        _scale: Vector2F,
        _subpixel_alignment: crate::fonts::SubpixelAlignment,
        _glyph_config: &crate::rendering::GlyphConfig,
        _format: crate::fonts::canvas::RasterFormat,
    ) -> Result<crate::fonts::RasterizedGlyph> {
        Ok(crate::fonts::RasterizedGlyph {
            canvas: crate::fonts::canvas::Canvas {
                pixels: vec![],
                size: vec2i(0, 0),
                row_stride: 0,
                format: crate::fonts::canvas::RasterFormat::Rgba32,
            },
            is_emoji: false,
        })
    }

    fn text_layout_system(&self) -> &dyn platform::TextLayoutSystem {
        self
    }
}

impl platform::TextLayoutSystem for FontDB {
    fn layout_line(
        &self,
        _text: &str,
        line_style: platform::LineStyle,
        _style_runs: &[(std::ops::Range<usize>, crate::text_layout::StyleAndFont)],
        _max_width: f32,
        _clip_config: crate::text_layout::ClipConfig,
    ) -> crate::text_layout::Line {
        crate::text_layout::Line::empty(line_style.font_size, line_style.line_height_ratio, 0)
    }

    fn layout_text(
        &self,
        _text: &str,
        line_style: platform::LineStyle,
        _style_runs: &[(std::ops::Range<usize>, crate::text_layout::StyleAndFont)],
        _max_width: f32,
        _max_height: f32,
        _alignment: TextAlignment,
        _first_line_head_indent: Option<f32>,
    ) -> crate::text_layout::TextFrame {
        crate::text_layout::TextFrame::empty(line_style.font_size, line_style.line_height_ratio)
    }
}
