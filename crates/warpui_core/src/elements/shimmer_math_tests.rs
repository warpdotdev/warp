use std::time::Duration;

use super::{intensity_at, shimmer_center, shimmer_color_at, ShimmerConfig};
use crate::color::ColorU;

fn config() -> ShimmerConfig {
    ShimmerConfig {
        period: Duration::from_secs(2),
        shimmer_radius: 6,
        padding: 8,
    }
}

#[test]
fn center_starts_before_the_text_and_sweeps_across_it() {
    // At t=0 the center sits `padding` glyphs before the first glyph.
    assert_eq!(shimmer_center(7, Duration::ZERO, &config()), -8.0);
    // Half a period in, the center is half way along the padded track:
    // 0.5 * ((7 - 1) + 2 * 8) - 8 = 3.
    assert_eq!(shimmer_center(7, Duration::from_secs(1), &config()), 3.0);
    // The animation loops every period.
    assert_eq!(shimmer_center(7, Duration::from_secs(2), &config()), -8.0);
}

#[test]
fn single_glyph_text_keeps_the_center_at_zero() {
    assert_eq!(
        shimmer_center(1, Duration::from_millis(500), &config()),
        0.0
    );
    assert_eq!(
        shimmer_center(0, Duration::from_millis(500), &config()),
        0.0
    );
}

#[test]
fn intensity_peaks_at_the_center_and_fades_to_zero_at_the_radius() {
    let config = config();
    assert_eq!(intensity_at(3, 3.0, &config), 1.0);
    // A glyph exactly `shimmer_radius` away has no intensity.
    assert_eq!(intensity_at(9, 3.0, &config), 0.0);
    // Intensity decreases monotonically with distance from the center.
    let near = intensity_at(4, 3.0, &config);
    let far = intensity_at(5, 3.0, &config);
    assert!(near > far);
    assert!(far > 0.0);
}

#[test]
fn color_lerp_hits_both_endpoints() {
    let base = ColorU::new(254, 253, 194, 255);
    let shimmer = ColorU::new(254, 255, 255, 255);
    assert_eq!(shimmer_color_at(base, shimmer, 0.0), base);
    assert_eq!(shimmer_color_at(base, shimmer, 1.0), shimmer);
}
