//! GUI-backend platform items.

use std::ops::Range;

use anyhow::Result;
use pathfinder_geometry::rect::RectI;
use pathfinder_geometry::vector::Vector2F;

use super::LineStyle;
use crate::fonts::canvas::RasterFormat;
use crate::fonts::{FontId, GlyphId, RasterizedGlyph, SubpixelAlignment};
use crate::rendering;
use crate::text_layout::{ClipConfig, Line, StyleAndFont, TextAlignment, TextFrame};

/// GUI-only extension of [`FontDB`](super::FontDB): glyph rasterization and
/// text layout/shaping. The neutral `FontDB` trait carries this as a routed
/// supertrait bound (`FontDB: FontDBExt`), so the GUI build requires these
/// methods of every font database while the TUI build substitutes an empty
/// marker trait (see the `tui` sibling module).
pub trait FontDBExt {
    /// Computes the size of the canvas needed to rasterize the glyph.
    fn glyph_raster_bounds(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        glyph_config: &rendering::GlyphConfig,
    ) -> Result<RectI>;

    /// Rasterizes a single glyph so it can be rendered to the screen.
    #[allow(clippy::too_many_arguments)]
    fn rasterize_glyph(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &rendering::GlyphConfig,
        format: RasterFormat,
    ) -> Result<RasterizedGlyph>;

    fn text_layout_system(&self) -> &dyn TextLayoutSystem;
}

/// Trait that implements text layout. Implementors must be [`Send`] and
/// [`Sync`] so that text can be laid out in a background thread.
pub trait TextLayoutSystem: 'static + Send + Sync {
    /// Lays out a single line of text.
    fn layout_line(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        clip_config: ClipConfig,
    ) -> Line;

    /// Lays out text into a series of lines that fit within the bounding box
    /// defined by `max_width` and `max_height`.
    #[allow(clippy::too_many_arguments)]
    fn layout_text(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        max_height: f32,
        alignment: TextAlignment,
        first_line_head_indent: Option<f32>,
    ) -> TextFrame;
}
