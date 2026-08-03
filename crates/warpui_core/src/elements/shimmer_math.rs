//! Backend-agnostic math for the "shimmer" animation: a highlight band that
//! sweeps across a run of glyphs, lerping each glyph from a base color toward
//! a shimmer color based on its distance from the band's center.
//!
//! Both the GUI [`ShimmeringTextElement`](crate::elements::shimmering_text)
//! and the TUI shimmering text render with this math; only glyph mapping and
//! painting differ per backend.

use std::f32::consts::PI;
use std::time::Duration;

use crate::color::ColorU;

/// Configuration for a shimmering text animation.
///
/// The shimmer moves through each glyph in the text with a configurable number of "padding"
/// surrounding the text to ensure the shimmer moves smoothly into and out of the text.
///
/// For example: Consider if the text is "foo" with the following configuration options:
///     * `period`: 2s
///     * `shimmer_radius`: 6
///     * `padding`: 8
/// This would mean that the shimmer would travel 19 glyphs (the
/// padding + the 3 characters in the text) over the course of 2 seconds. Any glyph within 6 glyphs
/// of the center will be considered part of the shimmer.
/// NOTE this means that part of the shimmer would span a glyph range that isn't visible to the user.
/// This is purposeful so that the shimmer smoothly moves into and out of the text range.
#[derive(Clone, Copy, Debug)]
pub struct ShimmerConfig {
    /// How long the shimmer should take from the start to the end of the track.
    pub period: Duration,
    /// The radius of the shimmer in fractional glyphs. Any glyph more than this distance away from
    /// the center  of the shimmer is displayed with no intensity.
    pub shimmer_radius: usize,
    /// Any extra padding of the shimmer, in fractional glyphs. Padding is added around the overall
    /// laid out glyphs to ensure the shimmer is smooth as it enters and exits the text.
    pub padding: usize,
}

impl Default for ShimmerConfig {
    fn default() -> Self {
        Self {
            period: Duration::from_secs(3),
            shimmer_radius: 6,
            padding: 8,
        }
    }
}

/// Returns the center of the shimmer band as a fractional glyph index along the track,
/// `elapsed` into the animation. The center starts before glyph 0 (negative) and travels
/// past the last glyph so the band sweeps smoothly into and out of the text.
pub fn shimmer_center(number_of_glyphs: usize, elapsed: Duration, config: &ShimmerConfig) -> f32 {
    if number_of_glyphs <= 1 {
        return 0.0;
    }

    let period_s = config.period.as_secs_f32();
    // Get the percent of the way through we are of the current loop.
    let progress = (elapsed.as_secs_f32() / period_s).fract();

    // Compute the total number of glyphs the band needs to travel.
    let span = (number_of_glyphs as f32 - 1.0) + (2.0 * config.padding as f32);
    // Get the fractional glyph index for the center of the band, factoring in that the center
    // can be negative (before any of the text)
    (progress * span) - config.padding as f32
}

/// Returns how strong the shimmer effect should be for the glyph at `glyph_index`, in `[0, 1]`,
/// based on how far it is from the shimmer band's `center`.
pub fn intensity_at(glyph_index: usize, center: f32, config: &ShimmerConfig) -> f32 {
    let dist = (glyph_index as f32 - center).abs();
    // If the distance is greater than the size of the band, there's no intensity.
    if dist >= config.shimmer_radius as f32 {
        return 0.0;
    }
    // Use a cosine wave to generate the intensity otherwise and normalize it to [0,1].
    let theta = (dist / config.shimmer_radius as f32) * PI;
    (theta.cos() + 1.0) * 0.5
}

/// Lerps a glyph's color from `base` toward `shimmer` by the given `intensity` in `[0, 1]`.
pub fn shimmer_color_at(base: ColorU, shimmer: ColorU, intensity: f32) -> ColorU {
    base.to_f32().lerp(shimmer.to_f32(), intensity).to_u8()
}

#[cfg(test)]
#[path = "shimmer_math_tests.rs"]
mod tests;
