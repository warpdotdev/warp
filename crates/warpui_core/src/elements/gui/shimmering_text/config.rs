use std::time::Duration;

/// Configuration for the ShimmeringText element.
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
