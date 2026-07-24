use settings::macros::define_settings_group;
use settings::{Setting as _, SupportedPlatforms, SyncToCloud};

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
