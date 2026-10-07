//! This module defines helper functions pertaining to the size/position of items in a Grid,
//! such as the dimensions of a grid cell and the baseline position of text within a cell.
use num_traits::Zero;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_errors::report_error;
use warpui::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
use warpui::fonts::{Cache as FontCache, FamilyId};
use warpui::text_layout::ComputeBaselinePositionFn;

/// Computes the grid cell size given the font and size at which the grid should
/// be rendered. We use a similar algorithm to Alacritty to do this, where the
/// cell width is the average advance of a glyph to its next (scaled up to pixel
/// coordinates instead of font coordinates) and the height is the line height.
pub fn grid_cell_dimensions(
    font_cache: &FontCache,
    font_family: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
) -> Vector2F {
    let default_font_id = font_cache.select_font(font_family, Default::default());
    // Use the `m` character as a proxy for the average advance between two
    // characters in the font. It's safe to unwrap here as we expect the `m`
    // char to be present when we load the font.
    let (m_glyph, _) = font_cache
        .glyph_for_char(default_font_id, 'm', false)
        .expect("font must contain a glyph for the 'm' character");
    let advance = font_cache.glyph_advance(default_font_id, font_size, m_glyph);

    // Get the font metrics that make up the total line height for the font.
    let ascent = font_cache.ascent(default_font_id, font_size);
    let descent = font_cache.descent(default_font_id, font_size);
    let leading = font_cache.leading(default_font_id, font_size);
    // Using a ratio, compared to the default line height, to appropriately scale the cell's Height.
    let height = ((ascent - descent + leading)
        * (line_height_ratio / DEFAULT_UI_LINE_HEIGHT_RATIO))
        .ceil()
        .max(1.);

    match advance {
        Ok(advance) => {
            // The horizontal advance may be zero if the font is only meant to be rendered
            // vertically. In that case, default to an advance of 1.
            let horizontal_advance = if advance.x().is_zero() {
                1.
            } else {
                advance.x().round().max(1.)
            };
            Vector2F::new(horizontal_advance, height)
        }
        Err(error) => {
            // Return a default advance width if we couldn't load the advance from the glyph.
            report_error!(
                error.context("could not obtain advance for m glyph computing cell dimensions")
            );
            Vector2F::new(1.0, height)
        }
    }
}

/// Calculates the baseline position using Grid-style calculations, specifically using font metrics
/// such as leading/descent properties for the given font ID and font size.
pub fn calculate_grid_baseline_position(
    font_cache: &FontCache,
    font_family: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
    cell_size_y: f32,
) -> Vector2F {
    let default_font_id = font_cache.select_font(font_family, Default::default());
    // Leading is additional spacing between lines.
    // Descent is the general guideline for how far the font will go below the baseline (for single spacing).
    let leading = font_cache.leading(default_font_id, font_size).floor();
    let descent = font_cache.descent(default_font_id, font_size).floor();
    // TODO(advait): Consider having this simply return a f32 instead of a vec2f (need to change entire callstack).
    // Adjust descent to be scaled up by the line height ratio to move up the baseline
    // and approximately maintain the same position within a line.
    vec2f(
        0.,
        cell_size_y - leading
            + (descent * (line_height_ratio / DEFAULT_UI_LINE_HEIGHT_RATIO).min(1.0)),
    )
}

/// Returns a closure that computes the baseline position in a Grid-style computation for the given font family.
pub fn grid_compute_baseline_position_fn(font_family: FamilyId) -> ComputeBaselinePositionFn {
    Box::new(move |font_metrics| {
        let cell_size_y = grid_cell_dimensions(
            font_metrics.font_cache,
            font_family,
            font_metrics.font_size,
            font_metrics.line_height_ratio,
        )
        .y();
        calculate_grid_baseline_position(
            font_metrics.font_cache,
            font_family,
            font_metrics.font_size,
            font_metrics.line_height_ratio,
            cell_size_y,
        )
        .y()
    })
}
