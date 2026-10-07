use std::str::FromStr;

use settings::macros::define_settings_group;
use settings::{Setting as _, SupportedPlatforms, SyncToCloud};
use warp_core::ui::theme::{ColorScheme, WarpTheme};
#[cfg(feature = "tui")]
use warpui_core::runtime::BackgroundLuminance;

#[cfg(feature = "tui")]
use crate::themes::default_themes::{dark_theme, light_theme};

/// The color theme selection used by Warp Agent CLI.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "The color theme used by Warp Agent CLI.",
    rename_all = "snake_case"
)]
pub enum TuiTheme {
    #[default]
    Auto,
    Light,
    Dark,
}

impl TuiTheme {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    #[cfg(feature = "tui")]
    pub fn resolve_for_background(self, background_luminance: BackgroundLuminance) -> WarpTheme {
        match self {
            Self::Auto => match background_luminance {
                BackgroundLuminance::Light => light_theme(),
                BackgroundLuminance::Dark | BackgroundLuminance::Unknown => dark_theme(),
            },
            Self::Light => light_theme(),
            Self::Dark => dark_theme(),
        }
    }
}

impl FromStr for TuiTheme {
    type Err = strum::ParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("auto") {
            Ok(Self::Auto)
        } else if value.eq_ignore_ascii_case("light") {
            Ok(Self::Light)
        } else if value.eq_ignore_ascii_case("dark") {
            Ok(Self::Dark)
        } else {
            Err(strum::ParseError::VariantNotFound)
        }
    }
}

impl From<&WarpTheme> for TuiTheme {
    fn from(theme: &WarpTheme) -> Self {
        match theme.inferred_color_scheme() {
            ColorScheme::DarkOnLight => Self::Light,
            ColorScheme::LightOnDark => Self::Dark,
        }
    }
}

define_settings_group!(TuiThemeSettings, settings: [
    theme: TuiThemeSetting {
        type: TuiTheme,
        default: TuiTheme::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::TUI,
        private: false,
        toml_path: "appearance.theme",
        description: "The Warp Agent CLI color theme. Auto matches the host terminal background.",
    },
]);

impl TuiThemeSettings {
    pub fn selected_theme(&self) -> TuiTheme {
        *self.theme.value()
    }
}

#[cfg(test)]
#[path = "tui_theme_tests.rs"]
mod tests;
