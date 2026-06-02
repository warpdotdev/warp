use warpui_core::color::ColorU;

use super::blend::Blend;
use super::{coloru_with_opacity, Rgb};

/// Offset to the relative luminance when computing the contrast ratio per the formula defined in
/// the [W3C Spec](https://www.w3.org/TR/WCAG20-TECHS/G17.html). This offset is included to
/// compensate for contrast ratios that occur when a value is at or near zero, and for ambient light
/// effects. See <https://juicystudio.com/article/luminositycontrastratioalgorithm.php> for more
/// details.
const LUMINANCE_OFFSET_FOR_CONTRAST_RATIO: f32 = 0.05;

/// Returns a new foreground color that when rendered against `background_color` would have a
/// contrast of at least `minimum_allowed_contrast`. NOTE the `background_color` must be fully
/// opaque in order to perform proper contrast checking.
///
/// If `foreground_color` already meets the minimum contrast, it is returned unchanged.
///
/// Color shifting is performed by computing the color that would produce the max contrast against
/// the `background_color` and then binary searching across all opacities to find an opacity that
/// would produce a color with at least the `minimum_allowed_contrast` when blended with the
/// `foreground_color`.
///
/// This is _heavily_ inspired by Chromium's approach to color shifting. See
/// <https://source.chromium.org/chromium/chromium/src/+/main:ui/gfx/color_utils.cc;l=634;drc=9f7b5c10efd74425f135fd5aad2076a7cc78607a>.
pub fn foreground_color_with_minimum_contrast(
    foreground_color: ColorU,
    background_color: Rgb,
    minimum_allowed_contrast: MinimumAllowedContrast,
) -> ColorU {
    // Convert the `RGB` into a fully opaque `ColorU` so that we can use existing blending functions
    // that rely on `ColorU`s.
    let background_color = ColorU::from(background_color);
    let foreground_color = background_color.blend(&foreground_color);
    if high_enough_contrast(foreground_color, background_color, minimum_allowed_contrast) {
        return foreground_color;
    }

    // Determine the color that would have the maximum contrast against the background. Contrast
    // is determined by the formula C = (L1 + 0.05) /(L2 + 0.05) where L1 is the relative luminance
    // of the lighter color and L2 is the relative luminance of the darker color. Since black has a
    // luminance of 0, and white has a luminance of 1, we know that white or black must produce the
    // color with most contrast against the background. In other words, if the background is
    // "light", then a luminance of 1 (black) in the denominator would produce the maximum possible
    // contrast. Alternately, if the background is a "dark" color, then a value of 0 (white) in the
    // numerator would produce the maximum possible contrast.
    let color_with_max_contrast =
        pick_constrasting_color(background_color, ColorU::white(), ColorU::black());

    // Perform binary search across all possible opacities (0,100) to find the best color that meets
    // the minimum allowed contrast. The returned color is computed by blending the current alpha
    // with the target foreground color and foreground color.
    let mut low_opacity = 0;

    let mut high_opacity = 101;
    let mut best_color = foreground_color;

    while low_opacity < high_opacity {
        let opacity = (low_opacity + high_opacity) / 2;

        let color = foreground_color.blend(&coloru_with_opacity(color_with_max_contrast, opacity));
        let contrast = contrast_ratio(color, background_color);

        if contrast >= minimum_allowed_contrast.get() {
            best_color = color;
            high_opacity = opacity;
        } else {
            low_opacity = opacity + 1;
        }
    }

    best_color
}

fn relative_luminance_for_channel(channel: u8) -> f32 {
    let srgb_channel = channel as f32 / 255.;
    if srgb_channel <= 0.03928 {
        srgb_channel / 12.92
    } else {
        ((srgb_channel + 0.055) / 1.055).powf(2.4)
    }
}

/// Computed based on the WCAG recommendations:
/// https://www.w3.org/TR/WCAG20/#relativeluminancedef
pub fn relative_luminance(color: ColorU) -> f32 {
    let r = relative_luminance_for_channel(color.r);
    let g = relative_luminance_for_channel(color.g);
    let b = relative_luminance_for_channel(color.b);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// More on calculating contrast ration here:
/// https://medium.muz.li/the-science-of-color-contrast-an-expert-designers-guide-33e84c41d156
fn contrast_ratio(color1: ColorU, color2: ColorU) -> f32 {
    let luminance1 = relative_luminance(color1) + LUMINANCE_OFFSET_FOR_CONTRAST_RATIO;
    let luminance2 = relative_luminance(color2) + LUMINANCE_OFFSET_FOR_CONTRAST_RATIO;
    // dividend here is supposed to be a lighter color than the divisor
    if luminance1 > luminance2 {
        return luminance1 / luminance2;
    }
    luminance2 / luminance1
}

/// This method picks the color option (option1 or option2) that has the highest contrast relative
/// to background color.
pub(super) fn pick_constrasting_color(
    background: ColorU,
    option1: ColorU,
    option2: ColorU,
) -> ColorU {
    let contrast_option1 = contrast_ratio(background, option1);
    let contrast_option2 = contrast_ratio(background, option2);
    if contrast_option1 > contrast_option2 {
        return option1;
    }
    option2
}

/// Enum that species the desired contrast ratio based on the type of content in the foreground.
#[derive(Copy, Clone, Debug)]
pub enum MinimumAllowedContrast {
    /// Text is on the foreground.
    Text,
    /// A non-text element (such as an icon or a UI component) is on the foreground.
    NonText,
}

impl MinimumAllowedContrast {
    /// Returns the minimum acceptable contrast ratio per the [WCAG (Web Content Accessibility
    /// Guidelines)](https://www.w3.org/WAI/WCAG21/Understanding/contrast-minimum.html) of a
    /// foreground color against a background color.
    fn get(&self) -> f32 {
        match self {
            MinimumAllowedContrast::Text => {
                // Normal sized text should have a contrast of at least 4.5:1. Source:
                // https://www.w3.org/WAI/WCAG21/Understanding/contrast-minimum.html
                4.5
            }
            MinimumAllowedContrast::NonText => {
                // Graphical elements should have a contrast of at least 3:1. Source:
                // https://www.w3.org/WAI/WCAG21/Techniques/general/G207
                3.0
            }
        }
    }
}

/// This method determines what font color should be used based on the background color it's
/// written on.
/// Most of the time, we juggle between background and foreground colors, assuming one of them
/// is dark, and the other is bright. If that's not the case and the contrast between both
/// background and foreground against provided color is not high enough, we simply fallback to white and
/// black for base font colors.
pub fn pick_best_foreground_color(
    bg: ColorU,
    option1: ColorU,
    option2: ColorU,
    minimum_allowed_contrast: MinimumAllowedContrast,
) -> ColorU {
    let contrasting_color = pick_constrasting_color(bg, option1, option2);
    if high_enough_contrast(bg, contrasting_color, minimum_allowed_contrast) {
        return contrasting_color;
    }

    // if the above didn't have enough contrast, we fallback to using black or white.
    // we assume that since luminance for black is 0 and 1 for white, we will always pick a
    // color that has high enough contrast.
    pick_constrasting_color(bg, ColorU::black(), ColorU::white())
}

/// Returns whether `color1` has a contrast of at least `minimum_allowed_contrast` when rendered
/// against `color2`.
pub fn high_enough_contrast(
    color1: ColorU,
    color2: ColorU,
    minimum_allowed_contrast: MinimumAllowedContrast,
) -> bool {
    contrast_ratio(color1, color2) > minimum_allowed_contrast.get()
}

#[cfg(test)]
#[path = "contrast_tests.rs"]
mod tests;
