mod gpu_info;
pub mod texture_cache;
pub use gpu_info::{GPUBackend, GPUDeviceInfo, GPUDeviceType, OnGPUDeviceSelected};
use serde::{Deserialize, Serialize};

use crate::platform::GraphicsBackend;

/// Circumstances under which glyphs should be rasterized with thin strokes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema_gen", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema_gen",
    schemars(
        description = "When to render text with thinner strokes for a lighter appearance.",
        rename_all = "snake_case"
    )
)]
#[cfg_attr(feature = "settings_value", derive(settings_value::SettingsValue))]
pub enum ThinStrokes {
    /// Never render glyphs using thin strokes.
    Never,
    /// Render glyphs using thin strokes when rendering on a low-DPI display.
    OnLowDpiDisplays,
    /// Render glyphs using thin strokes when rendering on a high-DPI display.
    #[default]
    OnHighDpiDisplays,
    /// Always render glyphs using thin strokes.
    Always,
}

impl ThinStrokes {
    /// The minimum scale factor for which we'll consider a display to be high-DPI.
    const HIGH_DPI_SCALE_FACTOR: f32 = 1.5;

    pub fn enabled_for_scale_factor(&self, scale_factor: f32) -> bool {
        match self {
            Self::Never => false,
            Self::OnLowDpiDisplays => scale_factor < Self::HIGH_DPI_SCALE_FACTOR,
            Self::OnHighDpiDisplays => scale_factor >= Self::HIGH_DPI_SCALE_FACTOR,
            Self::Always => true,
        }
    }
}

/// Options for configuring rendering of glyphs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphConfig {
    /// Whether to render glyphs using thin strokes.
    pub use_thin_strokes: ThinStrokes,
}

/// Power preference for GPU for rendering.
///
/// Relevant for machines with multiple GPUs (typically a discrete high-performance GPU and an
/// integrated low-power-usage GPU).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GPUPowerPreference {
    LowPower,
    #[default]
    HighPerformance,
}

/// Options for configuring rendering at the application level. These options
/// will apply for the entirety of a frame, but may change between frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Configuration options relating to glyph rendering.
    pub glyphs: GlyphConfig,

    /// Power preference for GPU used for rendering; this is applicable on dual GPU machines where
    /// there's a choice between a discrete high-performance GPU and a more power-efficient
    /// integrated GPU.
    pub gpu_power_preference: GPUPowerPreference,

    pub backend_preference: Option<GraphicsBackend>,
}

#[derive(Clone, Debug, Default)]
pub struct CornerRadius {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_left: f32,
    pub bottom_right: f32,
}

impl CornerRadius {
    pub fn from_ui_corner_radius(
        corner_radius: crate::scene::CornerRadius,
        scale_factor: f32,
        min_dimension: f32,
    ) -> Self {
        let top_left = match corner_radius.get_top_left() {
            crate::scene::Radius::Pixels(px) => px * scale_factor,
            crate::scene::Radius::Percentage(percent) => percent / 100. * min_dimension,
        };
        let top_right = match corner_radius.get_top_right() {
            crate::scene::Radius::Pixels(px) => px * scale_factor,
            crate::scene::Radius::Percentage(percent) => percent / 100. * min_dimension,
        };
        let bottom_left = match corner_radius.get_bottom_left() {
            crate::scene::Radius::Pixels(px) => px * scale_factor,
            crate::scene::Radius::Percentage(percent) => percent / 100. * min_dimension,
        };
        let bottom_right = match corner_radius.get_bottom_right() {
            crate::scene::Radius::Pixels(px) => px * scale_factor,
            crate::scene::Radius::Percentage(percent) => percent / 100. * min_dimension,
        };
        Self {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        }
    }
}
