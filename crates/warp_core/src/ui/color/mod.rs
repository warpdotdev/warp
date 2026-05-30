use warpui::color::ColorU;

use self::contrast::{high_enough_contrast, pick_constrasting_color, MinimumAllowedContrast};

pub mod blend;
pub mod contrast;
pub mod hex_color;

/// Opacity of the given color expressed as %. Allowing for range 0..100 inclusive.
/// TODO: use a bounded type instead
pub type Opacity = u8;

pub const OPAQUE: u8 = 255;

/// Claude brand orange color (#E8704E)
pub const CLAUDE_ORANGE: ColorU = ColorU {
    r: 232,
    g: 112,
    b: 78,
    a: OPAQUE,
};

/// Simple type representing a color _without_ an alpha channel.
pub struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

impl From<Rgb> for ColorU {
    fn from(rgb: Rgb) -> Self {
        Self {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
            a: OPAQUE,
        }
    }
}

impl From<ColorU> for Rgb {
    fn from(color: ColorU) -> Self {
        Self {
            r: color.r,
            g: color.g,
            b: color.b,
        }
    }
}

pub fn coloru_with_opacity(color: ColorU, opacity: Opacity) -> ColorU {
    let new_alpha: u8 = (color.a as f32 * (opacity as f32 / 100.)) as u8;
    ColorU::new(color.r, color.g, color.b, new_alpha)
}

/// mid_coloru determines a color 'in-between' the 2 colors (or simply, an average of 2 colors).
/// Currently used to figure the midpoint color for gradients (which is then needed for the font
/// color computation etc.).
pub fn mid_coloru(c1: ColorU, c2: ColorU) -> ColorU {
    let r = (c1.r as f32 + c2.r as f32) / 2.;
    let g = (c1.g as f32 + c2.g as f32) / 2.;
    let b = (c1.b as f32 + c2.b as f32) / 2.;
    ColorU::new(r as u8, g as u8, b as u8, OPAQUE)
}

/// "those are kinda arbitrary" -- Agata. We could tweak these factors.
const DARKEN_COLORU_SHADE_FACTOR: f32 = 0.52;
const LIGHTEN_COLORU_SHADE_FACTOR: f32 = 0.5;

/// Finds a darker version of the given color using DARKEN_COLORU_SHADE_FACTOR form factor.
pub fn darken(c: ColorU) -> ColorU {
    let shade_factor = 1. - DARKEN_COLORU_SHADE_FACTOR;
    let r = ((c.r as f32) * shade_factor).ceil() as u8;
    let g = ((c.g as f32) * shade_factor).ceil() as u8;
    let b = ((c.b as f32) * shade_factor).ceil() as u8;
    ColorU::new(r, g, b, c.a)
}

/// Finds a ligher version of the given color using LIGHTEN_COLORU_SHADE_FACTOR form factor.
pub fn lighten(c: ColorU) -> ColorU {
    // applying the shade factor only to the difference between 255 and channel
    // (doing so to the actual channel value could produce incorrect results
    // since channels are capped at 255 value).
    let r = ((OPAQUE - c.r) as f32 * LIGHTEN_COLORU_SHADE_FACTOR).ceil() as u8;
    let g = ((OPAQUE - c.g) as f32 * LIGHTEN_COLORU_SHADE_FACTOR).ceil() as u8;
    let b = ((OPAQUE - c.b) as f32 * LIGHTEN_COLORU_SHADE_FACTOR).ceil() as u8;
    // in the result, we add the computed value to the current channel value
    // to get the actual lighter color.
    ColorU::new(r + c.r, g + c.g, b + c.b, c.a)
}

pub fn pick_foreground_color(background: ColorU) -> ColorU {
    pick_constrasting_color(background, ColorU::black(), ColorU::white())
}

pub trait ContrastingColor<Rhs = Self> {
    type Output;
    fn on_background(
        self,
        background: Rhs,
        minimum_allowed_contrast: MinimumAllowedContrast,
    ) -> Self::Output;
}

impl ContrastingColor for ColorU {
    type Output = ColorU;
    fn on_background(
        self,
        background: ColorU,
        minimum_allowed_contrast: MinimumAllowedContrast,
    ) -> ColorU {
        if !high_enough_contrast(background, self, minimum_allowed_contrast) {
            return contrast::foreground_color_with_minimum_contrast(
                self,
                background.into(),
                minimum_allowed_contrast,
            );
        }
        self
    }
}

#[cfg(test)]
#[path = "color_tests.rs"]
mod tests;
