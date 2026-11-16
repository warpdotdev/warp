use warpui_core::color::ColorU;

pub trait Blend<Rhs = Self> {
    type Output;
    fn blend(&self, rhs: &Rhs) -> Self::Output;
}

impl Blend for ColorU {
    type Output = ColorU;

    /// Color blending computation.
    /// This function calculates the color, assuming that "self" is a background, and "other" is
    /// a new color on top.
    /// It simply calculates a weighted sum of each channel, and averages out the opacity.
    /// Note that due to rounding errors, the result of computation maybe slightly different than
    /// what comes out of figma (ie. 181818 instead of 191918) - differences shouldn't be
    /// noticeable though, due to nature of the rounding error.
    fn blend(&self, other: &ColorU) -> ColorU {
        // Helper function that computes a weighted sum using the overlay color's opacity as weight.
        fn add_channels(c1: u8, c2: u8, ratio: f32) -> u8 {
            ((c1 as f32 * (1. - ratio)) + (c2 as f32 * ratio)) as u8
        }

        // background not visible, lets return other
        if self.is_fully_transparent() || other.a == super::OPAQUE {
            return *other;
        }
        // other not visible, self it is.
        if other.is_fully_transparent() {
            return *self;
        }
        // alpha value for new color, opaque if  background is opaque already, otherwise simple avg
        let alpha = if self.is_opaque() {
            super::OPAQUE
        } else {
            // doing type conversion, since adding two arbitrary alphas may result in u8 overflow
            ((self.a as f32 + other.a as f32) / 2.) as u8
        };
        // basically overlay color's opacity expressed as %, rounded to 2 digits after decimal
        let ratio = ((other.a as f32 / 255.) * 100.).ceil() / 100.;
        ColorU::new(
            add_channels(self.r, other.r, ratio),
            add_channels(self.g, other.g, ratio),
            add_channels(self.b, other.b, ratio),
            alpha,
        )
    }
}
