pub mod color;
pub mod phenomenon;

use std::path::PathBuf;

use dirs::home_dir;
use serde::{Deserialize, Serialize};
use warpui_core::assets::asset_cache::AssetSource;
use warpui_core::color::ColorU;
use warpui_core::geometry::vector::vec2f;

use self::color::CustomDetails;
use super::color::blend::Blend;
use super::color::contrast::{pick_best_foreground_color, MinimumAllowedContrast};
use super::color::{coloru_with_opacity, hex_color, mid_coloru, ContrastingColor, Opacity, OPAQUE};
use crate::paths::themes_dir;
// Import relative_luminance from contrast module for brightness calculation
use crate::ui::color::contrast::relative_luminance;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub source: AssetSource,
    pub opacity: Opacity,
}

/// This is a helper struct used for deserialization.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
struct SerializedBackgroundThemeImage {
    path: String,
    #[serde(default = "default_image_opacity")]
    pub opacity: Opacity,
}

impl Serialize for Image {
    // We only serialize Images that are sourced from local files. Currently,
    // there is no need in our app to serialize a theme that contains a bundled image.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let AssetSource::LocalFile { path } = self.source.clone() else {
            return Err(serde::ser::Error::custom(
                "image path was serialized but it's not a local file",
            ));
        };

        let serialized = SerializedBackgroundThemeImage {
            path,
            opacity: self.opacity,
        };

        serialized.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Image {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value: SerializedBackgroundThemeImage =
            SerializedBackgroundThemeImage::deserialize(deserializer)?;

        // The user is allowed to specify a relative path. It's our responsibility to
        // deserialize this into an absolute path.
        let path = {
            let expanded_path = expand_tilde(value.path.into());
            if expanded_path.is_absolute() {
                expanded_path
            } else {
                themes_dir().join(expanded_path)
            }
        };

        Ok(Image {
            source: AssetSource::LocalFile {
                path: path.to_str().unwrap_or_default().to_owned(),
            },
            opacity: value.opacity,
        })
    }
}

/// Returns the default opacity for serde to use for an [`Image`] if one is not
/// specified.
fn default_image_opacity() -> Opacity {
    100
}

/// Performs tilde expansion to expand a _leading_ tilde to the user's home dir. Any intermediate
/// tildes are not expanded. If the path does not begin with a tilde, then the existing path is
/// returned unchanged.
fn expand_tilde(path: PathBuf) -> PathBuf {
    let home_dir = match home_dir() {
        Some(home_dir) => home_dir,
        None => return path,
    };

    match path.strip_prefix("~") {
        Ok(stripped) => home_dir.join(stripped),
        Err(_) => path,
    }
}

impl Image {
    pub fn source(&self) -> AssetSource {
        self.source.clone()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AnsiColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
impl From<AnsiColor> for ColorU {
    fn from(color: AnsiColor) -> Self {
        ColorU {
            r: color.r,
            g: color.g,
            b: color.b,
            a: OPAQUE, // ansi colors are at full opacity
        }
    }
}

impl From<ColorU> for AnsiColor {
    fn from(color: ColorU) -> Self {
        AnsiColor {
            r: color.r,
            g: color.g,
            b: color.b,
        }
    }
}

impl From<AnsiColor> for Fill {
    fn from(color: AnsiColor) -> Fill {
        Fill::Solid(color.into())
    }
}

impl AnsiColor {
    pub const fn from_u32(color: u32) -> Self {
        AnsiColor {
            r: (color >> 24) as u8,
            g: ((color >> 16) & 0xff) as u8,
            b: ((color >> 8) & 0xff) as u8,
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct VerticalGradient {
    #[serde(with = "hex_color")]
    top: ColorU,
    #[serde(with = "hex_color")]
    bottom: ColorU,
}

impl VerticalGradient {
    pub fn new(top: ColorU, bottom: ColorU) -> Self {
        VerticalGradient { top, bottom }
    }

    fn midcolor(&self) -> ColorU {
        mid_coloru(self.top, self.bottom)
    }

    pub fn get_most_opaque(&self) -> ColorU {
        if self.top.a > self.bottom.a {
            self.top
        } else {
            self.bottom
        }
    }
}
impl Blend for VerticalGradient {
    type Output = VerticalGradient;
    fn blend(&self, other: &VerticalGradient) -> VerticalGradient {
        VerticalGradient::new(self.top.blend(&other.top), self.bottom.blend(&other.bottom))
    }
}

impl ContrastingColor<ColorU> for VerticalGradient {
    type Output = VerticalGradient;
    fn on_background(
        self,
        background: ColorU,
        minimum_allowed_contrast: MinimumAllowedContrast,
    ) -> VerticalGradient {
        VerticalGradient::new(
            self.top.on_background(background, minimum_allowed_contrast),
            self.bottom
                .on_background(background, minimum_allowed_contrast),
        )
    }
}

#[derive(Serialize, Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct HorizontalGradient {
    #[serde(with = "hex_color")]
    left: ColorU,
    #[serde(with = "hex_color")]
    right: ColorU,
}

impl HorizontalGradient {
    pub fn new(left: ColorU, right: ColorU) -> Self {
        HorizontalGradient { left, right }
    }

    fn midcolor(&self) -> ColorU {
        mid_coloru(self.left, self.right)
    }

    pub fn get_most_opaque(&self) -> ColorU {
        if self.left.a > self.right.a {
            self.left
        } else {
            self.right
        }
    }
}

impl Blend for HorizontalGradient {
    type Output = HorizontalGradient;
    fn blend(&self, other: &HorizontalGradient) -> HorizontalGradient {
        HorizontalGradient::new(self.left.blend(&other.left), self.right.blend(&other.right))
    }
}

impl ContrastingColor<ColorU> for HorizontalGradient {
    type Output = HorizontalGradient;
    fn on_background(
        self,
        background: ColorU,
        minimum_allowed_contrast: MinimumAllowedContrast,
    ) -> HorizontalGradient {
        HorizontalGradient::new(
            self.left
                .on_background(background, minimum_allowed_contrast),
            self.right
                .on_background(background, minimum_allowed_contrast),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorScheme {
    /// Light foreground colors on a dark background (light mode).
    LightOnDark,
    /// Dark foreground colors on a light background (dark mode).
    DarkOnLight,
}

impl ColorScheme {
    fn infer_from_foreground_color(foreground_color: ColorU) -> Self {
        // We actually are picking whether the foreground color is most visible
        // on a light or dark _background_, despite the helper function name.
        if pick_best_foreground_color(
            foreground_color,
            ColorU::white(),
            ColorU::black(),
            MinimumAllowedContrast::Text,
        ) == ColorU::white()
        {
            ColorScheme::DarkOnLight
        } else {
            ColorScheme::LightOnDark
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged, rename_all = "lowercase")]
pub enum Fill {
    #[serde(with = "hex_color")]
    Solid(ColorU),
    VerticalGradient(VerticalGradient),
    HorizontalGradient(HorizontalGradient),
}

impl Fill {
    pub fn black() -> Fill {
        Fill::Solid(ColorU::from_u32(0x000000ff))
    }

    pub fn white() -> Fill {
        Fill::Solid(ColorU::from_u32(0xffffffff))
    }

    pub fn warn() -> Fill {
        Fill::Solid(ColorU::from_u32(0xC28000FF))
    }

    pub fn error() -> Fill {
        Fill::Solid(ColorU::new(188, 54, 42, 255))
    }

    // Translucent black used for blur backdrop
    pub fn blur() -> Fill {
        Fill::Solid(ColorU::new(0, 0, 0, 179))
    }

    /// Green color used for elements that show success status.
    pub fn success() -> Fill {
        Fill::Solid(ColorU::new(0, 142, 65, 255))
    }

    pub fn with_opacity(&self, opacity: Opacity) -> Self {
        match self {
            Fill::Solid(c) => Fill::Solid(coloru_with_opacity(*c, opacity)),
            Fill::VerticalGradient(g) => Fill::VerticalGradient(VerticalGradient::new(
                coloru_with_opacity(g.top, opacity),
                coloru_with_opacity(g.bottom, opacity),
            )),
            Fill::HorizontalGradient(g) => Fill::HorizontalGradient(HorizontalGradient::new(
                coloru_with_opacity(g.left, opacity),
                coloru_with_opacity(g.right, opacity),
            )),
        }
    }

    /// Convert this fill into a solid color, taking the midpoint color for gradients
    pub fn into_solid(self) -> ColorU {
        match self {
            Fill::Solid(c) => c,
            Fill::VerticalGradient(g) => g.midcolor(),
            Fill::HorizontalGradient(g) => g.midcolor(),
        }
    }

    /// Convert this Fill into a solid color, taking the top color for vertical gradients and the
    /// midpoint color for horizontal gradients.
    pub fn into_solid_bias_top_color(self) -> ColorU {
        match self {
            Fill::Solid(c) => c,
            Fill::VerticalGradient(g) => g.top,
            Fill::HorizontalGradient(g) => g.midcolor(),
        }
    }

    /// Convert this Fill into a solid color, taking the right color for horizontal gradients and
    /// the midpoint color for vertical gradients.
    pub fn into_solid_bias_right_color(self) -> ColorU {
        match self {
            Self::Solid(c) => c,
            Self::HorizontalGradient(g) => g.right,
            Self::VerticalGradient(g) => g.midcolor(),
        }
    }

    /// Convert this Fill into a version of itself whose color is adaptively faded based on the
    /// background brightness. Uses less aggressive fading on light backgrounds and more aggressive
    /// fading on dark backgrounds to maintain optimal contrast.
    pub fn fade_into_background(self, background_color: &Self) -> Self {
        let background_luminance = relative_luminance(background_color.into_solid());

        // Threshold for determining if background is "light" vs "dark"
        // 0.5 is approximately middle gray in terms of perceived brightness
        let is_light_background = background_luminance > 0.2;

        // Use different opacity levels based on background brightness:
        // - Light backgrounds: Use higher opacity (85%) to maintain contrast with focused diffs
        // - Dark backgrounds: Use lower opacity (65%) since the contrast is naturally better
        let fade_opacity = if is_light_background {
            85 // More aggressive fading on light backgrounds
        } else {
            65 // Less aggressive fading on dark backgrounds
        };

        self.blend(&background_color.with_opacity(fade_opacity))
    }
}

impl Blend for Fill {
    type Output = Fill;
    fn blend(&self, other: &Fill) -> Fill {
        match (self, other) {
            (Fill::Solid(c1), Fill::Solid(c2)) => Fill::Solid(c1.blend(c2)),
            (Fill::VerticalGradient(g), Fill::Solid(c)) => {
                Fill::VerticalGradient(VerticalGradient::new(g.top.blend(c), g.bottom.blend(c)))
            }
            (Fill::Solid(c), Fill::VerticalGradient(g)) => {
                Fill::VerticalGradient(VerticalGradient::new(c.blend(&g.top), c.blend(&g.bottom)))
            }
            (Fill::HorizontalGradient(g), Fill::Solid(c)) => {
                Fill::HorizontalGradient(HorizontalGradient::new(g.left.blend(c), g.right.blend(c)))
            }
            (Fill::Solid(c), Fill::HorizontalGradient(g)) => Fill::HorizontalGradient(
                HorizontalGradient::new(c.blend(&g.left), c.blend(&g.right)),
            ),
            (Fill::VerticalGradient(g1), Fill::VerticalGradient(g2)) => {
                Fill::VerticalGradient(g1.blend(g2))
            }
            (Fill::HorizontalGradient(g1), Fill::HorizontalGradient(g2)) => {
                Fill::HorizontalGradient(g1.blend(g2))
            }
            (Fill::HorizontalGradient(g1), Fill::VerticalGradient(g2)) => {
                Fill::VerticalGradient(VerticalGradient::new(
                    g1.midcolor().blend(&g2.top),
                    g1.midcolor().blend(&g2.bottom),
                ))
            }
            (Fill::VerticalGradient(g1), Fill::HorizontalGradient(g2)) => {
                Fill::HorizontalGradient(HorizontalGradient::new(
                    g1.midcolor().blend(&g2.left),
                    g1.midcolor().blend(&g2.right),
                ))
            }
        }
    }
}

impl ContrastingColor for Fill {
    type Output = Fill;
    fn on_background(
        self,
        background: Fill,
        minimum_allowed_contrast: MinimumAllowedContrast,
    ) -> Fill {
        match self {
            Fill::Solid(c) => {
                Fill::Solid(c.on_background(background.into(), minimum_allowed_contrast))
            }
            Fill::HorizontalGradient(g) => Fill::HorizontalGradient(
                g.on_background(background.into(), minimum_allowed_contrast),
            ),
            Fill::VerticalGradient(g) => {
                Fill::VerticalGradient(g.on_background(background.into(), minimum_allowed_contrast))
            }
        }
    }
}

impl From<Fill> for warpui_core::elements::Fill {
    fn from(theme: Fill) -> Self {
        match theme {
            Fill::Solid(c) => warpui_core::elements::Fill::Solid(c),
            Fill::HorizontalGradient(g) => warpui_core::elements::Fill::Gradient {
                start: vec2f(0.0, 0.0),
                end: vec2f(1.0, 0.0),
                start_color: g.left,
                end_color: g.right,
            },
            Fill::VerticalGradient(g) => warpui_core::elements::Fill::Gradient {
                start: vec2f(0.0, 0.0),
                end: vec2f(0.0, 1.0),
                start_color: g.top,
                end_color: g.bottom,
            },
        }
    }
}

impl From<Fill> for ColorU {
    fn from(color: Fill) -> Self {
        match color {
            Fill::Solid(c) => c,
            Fill::VerticalGradient(g) => g.midcolor(),
            Fill::HorizontalGradient(g) => g.midcolor(),
        }
    }
}

impl From<ColorU> for Fill {
    fn from(color: ColorU) -> Fill {
        Fill::Solid(color)
    }
}

#[derive(Serialize, Copy, Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AnsiColors {
    #[serde(with = "hex_color")]
    pub black: AnsiColor,
    #[serde(with = "hex_color")]
    pub red: AnsiColor,
    #[serde(with = "hex_color")]
    pub green: AnsiColor,
    #[serde(with = "hex_color")]
    pub yellow: AnsiColor,
    #[serde(with = "hex_color")]
    pub blue: AnsiColor,
    #[serde(with = "hex_color")]
    pub magenta: AnsiColor,
    #[serde(with = "hex_color")]
    pub cyan: AnsiColor,
    #[serde(with = "hex_color")]
    pub white: AnsiColor,
}

impl AnsiColors {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        black: AnsiColor,
        red: AnsiColor,
        green: AnsiColor,
        yellow: AnsiColor,
        blue: AnsiColor,
        magenta: AnsiColor,
        cyan: AnsiColor,
        white: AnsiColor,
    ) -> Self {
        AnsiColors {
            black,
            red,
            green,
            yellow,
            blue,
            magenta,
            cyan,
            white,
        }
    }
}

#[derive(
    Serialize,
    Copy,
    Clone,
    Debug,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[schemars(
    description = "One of the eight standard ANSI terminal colors.",
    rename_all = "snake_case"
)]
#[serde(rename_all = "lowercase")]
#[strum(ascii_case_insensitive)]
pub enum AnsiColorIdentifier {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

impl AnsiColorIdentifier {
    pub fn to_ansi_color(self, colors: &AnsiColors) -> AnsiColor {
        match self {
            Self::Black => colors.black,
            Self::Red => colors.red,
            Self::Green => colors.green,
            Self::Yellow => colors.yellow,
            Self::Blue => colors.blue,
            Self::Magenta => colors.magenta,
            Self::Cyan => colors.cyan,
            Self::White => colors.white,
        }
    }
}

#[derive(Serialize, Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Details {
    Darker,
    Lighter,
    Custom(CustomDetails),
}

#[derive(Serialize, Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TerminalColors {
    pub normal: AnsiColors,
    pub bright: AnsiColors,
}

impl TerminalColors {
    pub fn new(normal: AnsiColors, bright: AnsiColors) -> Self {
        TerminalColors { normal, bright }
    }
}

#[derive(Serialize, Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct WarpTheme {
    background: Fill,
    accent: Fill,
    #[serde(with = "hex_color")]
    foreground: ColorU,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor: Option<Fill>,

    #[serde(skip_serializing_if = "Option::is_none")]
    background_image: Option<Image>,

    details: Details,
    terminal_colors: TerminalColors,
    // If name is None, we construct the name by processing the theme .yaml file name
    name: Option<String>,
}

impl WarpTheme {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bg: Fill,
        foreground: ColorU,
        accent: Fill,
        cursor: Option<Fill>,
        details: Option<Details>,
        terminal_colors: TerminalColors,
        background_image: Option<Image>,
        name: Option<String>,
    ) -> Self {
        WarpTheme {
            background: bg,
            foreground,
            accent,
            cursor,
            details: details.unwrap_or_else(|| Details::Custom(CustomDetails::default())),
            terminal_colors,
            background_image,
            name,
        }
    }

    pub fn name(&self) -> Option<String> {
        self.name.clone()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = Some(name);
    }

    pub fn details(&self) -> CustomDetails {
        match self.details {
            Details::Darker => CustomDetails::darker_details(),
            Details::Lighter => CustomDetails::lighter_details(),
            Details::Custom(details) => details,
        }
    }

    pub fn inferred_color_scheme(&self) -> ColorScheme {
        ColorScheme::infer_from_foreground_color(self.foreground)
    }

    pub fn background_image(&self) -> Option<Image> {
        self.background_image.clone()
    }
}

#[cfg(any(test, feature = "test-util"))]
pub fn mock_terminal_colors() -> TerminalColors {
    TerminalColors::new(
        AnsiColors::new(
            AnsiColor::from_u32(0x616161FF),
            AnsiColor::from_u32(0xFF8272FF),
            AnsiColor::from_u32(0xB4FA72FF),
            AnsiColor::from_u32(0xFEFDC2FF),
            AnsiColor::from_u32(0xA5D5FEFF),
            AnsiColor::from_u32(0xFF8FFDFF),
            AnsiColor::from_u32(0xD0D1FEFF),
            AnsiColor::from_u32(0xF1F1F1FF),
        ),
        AnsiColors::new(
            AnsiColor::from_u32(0x8E8E8EFF),
            AnsiColor::from_u32(0xFFC4BDFF),
            AnsiColor::from_u32(0xD6FCB9FF),
            AnsiColor::from_u32(0xFEFDD5FF),
            AnsiColor::from_u32(0xC1E3FEFF),
            AnsiColor::from_u32(0xFFB1FEFF),
            AnsiColor::from_u32(0xE5E6FEFF),
            AnsiColor::from_u32(0xFEFFFFFF),
        ),
    )
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
