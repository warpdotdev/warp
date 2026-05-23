use std::fmt::{Display, Formatter};
use std::ops::Neg;

use derive_more::{AddAssign, SubAssign};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

/// Newtype representing a position in line coordinates. See `to_pixel` to convert to pixel
/// coordinates.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    AddAssign,
    SubAssign,
    num_derive::FromPrimitive,
    num_derive::ToPrimitive,
    num_derive::NumCast,
    num_derive::NumOps,
    num_derive::Zero,
    num_derive::One,
    num_derive::Num,
    num_derive::Float,
)]
pub struct Lines(OrderedFloat<f64>);

impl Lines {
    /// The epsilon value used for approximate equality checks.
    const APPROX_EQ_EPSILON: f64 = 0.000001;

    pub const fn new(lines: f64) -> Self {
        Lines(OrderedFloat(lines))
    }

    pub fn to_pixels(self, line_height: impl Into<Pixels>) -> Pixels {
        let line_height = line_height.into();
        Pixels(self.as_f64() as f32 * line_height.0)
    }

    pub const fn zero() -> Self {
        Self::new(0.)
    }

    pub fn fract(&self) -> Lines {
        self.0.fract().into_lines()
    }

    pub fn as_f64(&self) -> f64 {
        self.0 .0
    }

    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }
}

impl Neg for Lines {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(self.0.neg())
    }
}

#[cfg(any(test, feature = "test-util"))]
impl std::ops::Add<f64> for Lines {
    type Output = Self;

    fn add(self, rhs: f64) -> Self::Output {
        Self(self.0 + rhs)
    }
}

#[cfg(any(test, feature = "test-util"))]
impl std::ops::Sub<f64> for Lines {
    type Output = Self;

    fn sub(self, rhs: f64) -> Self::Output {
        Self(self.0 - rhs)
    }
}

impl Display for Lines {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl float_cmp::ApproxEq for Lines {
    type Margin = float_cmp::F64Margin;

    fn approx_eq<M: Into<Self::Margin>>(self, other: Self, margin: M) -> bool {
        let margin: Self::Margin = margin.into();
        let margin = Self::Margin {
            // Make sure we use an epsilon value that's at least APPROX_EQ_EPSILON.
            epsilon: margin.epsilon.max(Self::APPROX_EQ_EPSILON),
            ulps: margin.ulps,
        };
        self.as_f64().approx_eq(other.as_f64(), margin)
    }
}

/// Newtype representing a position in pixel coordinates. See `to_lines` to convert to line
/// coordinates.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialOrd,
    PartialEq,
    AddAssign,
    SubAssign,
    Serialize,
    Deserialize,
    num_derive::FromPrimitive,
    num_derive::ToPrimitive,
    num_derive::NumCast,
    num_derive::NumOps,
    num_derive::Zero,
    num_derive::One,
    num_derive::Num,
    num_derive::Float,
)]
#[cfg_attr(feature = "schema_gen", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema_gen", schemars(description = "A value in pixels."))]
#[cfg_attr(feature = "settings_value", derive(settings_value::SettingsValue))]
pub struct Pixels(f32);

impl Pixels {
    pub const fn new(pixels: f32) -> Self {
        Pixels(pixels)
    }

    pub fn to_lines(self, line_height: Pixels) -> Lines {
        Lines(OrderedFloat(self.0 as f64 / line_height.0 as f64))
    }

    pub fn fract(&self) -> Pixels {
        self.0.fract().into_pixels()
    }

    pub fn zero() -> Self {
        Pixels(0.)
    }

    pub fn as_f32(&self) -> f32 {
        self.0
    }

    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }
}

impl Neg for Pixels {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(self.0.neg())
    }
}

impl Display for Pixels {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl float_cmp::ApproxEq for Pixels {
    type Margin = float_cmp::F32Margin;

    fn approx_eq<M: Into<Self::Margin>>(self, other: Self, margin: M) -> bool {
        self.as_f32().approx_eq(other.as_f32(), margin)
    }
}

/// Trait to convert an arbitrary type to `Pixels`.
pub trait IntoPixels {
    fn into_pixels(self) -> Pixels;
}

/// Trait to convert an arbitrary type to `Lines`.
pub trait IntoLines {
    fn into_lines(self) -> Lines;
}

impl IntoLines for Lines {
    fn into_lines(self) -> Lines {
        self
    }
}

macro_rules! impl_into_pixels {
    ($($t:ident)*) => ($(impl IntoPixels for $t {
        fn into_pixels(self) -> Pixels {
            Pixels(self as f32)
        }
    })*)
}

macro_rules! impl_into_lines {
    ($($t:ident)*) => ($(impl IntoLines for $t {
        fn into_lines(self) -> Lines {
            Lines(OrderedFloat(self as f64))
        }
    })*)
}

impl_into_pixels! { usize f32 f64 }
impl_into_lines! { usize i32 u32 u64 f32 f64 }
