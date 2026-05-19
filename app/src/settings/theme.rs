use warpui::{platform::SystemTheme, AppContext};

use crate::themes::theme::{RespectSystemTheme, SelectedSystemThemes, ThemeKind};
use settings::{
    macros::define_settings_group, RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud,
};

// Settings group for themes related settings.
// Note that we store just the information needed to derive the current
// theme state, which boils down to:
// ThemeKind: the theme to use when the system theme is off.
// UseSystemTheme: whether to respect the system theme.
// SelectedSystemThemes: the themes to use when the system theme is on.
define_settings_group!(ThemeSettings, settings: [
    theme_kind: Theme {
        type: ThemeKind,
        // Note that for new users, we now override this default value in SettingsInitializer
        // to set the default theme to Phenomenon.
        default: ThemeKind::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.themes.theme",
        max_table_depth: 0,
        description: "The color theme.",
    },
    use_system_theme: UseSystemTheme {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "SystemTheme",
        toml_path: "appearance.themes.system_theme",
        description: "Whether to match the system light/dark theme.",
    },
    selected_system_themes: SystemThemes {
        type: SelectedSystemThemes,
        default: SelectedSystemThemes::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "SelectedSystemThemes",
        toml_path: "appearance.themes.selected_system_themes",
        max_table_depth: 0,
        description: "The themes to use for system light and dark modes.",
    },
]);

impl Theme {
    fn current_value_is_syncable(&self) -> bool {
        self.value().is_custom_theme_reference_syncable()
    }
}

impl SystemThemes {
    fn current_value_is_syncable(&self) -> bool {
        let selected = self.value();
        selected.light.is_custom_theme_reference_syncable()
            && selected.dark.is_custom_theme_reference_syncable()
    }
}

/// Returns a derived value for whether to respect the system theme based on
/// the current theme settings.
pub fn respect_system_theme(theme_settings: &ThemeSettings) -> RespectSystemTheme {
    if *theme_settings.use_system_theme.value() {
        RespectSystemTheme::On(theme_settings.selected_system_themes.value().clone())
    } else {
        RespectSystemTheme::Off
    }
}

/// Returns the current theme kind based on the theme settings and the system theme.
pub fn derived_theme_kind(theme_settings: &ThemeSettings, system_theme: SystemTheme) -> ThemeKind {
    let respect_system_theme = respect_system_theme(theme_settings);
    match respect_system_theme {
        RespectSystemTheme::On(selected_system_themes) => match system_theme {
            SystemTheme::Light => selected_system_themes.light.clone(),
            SystemTheme::Dark => selected_system_themes.dark.clone(),
        },
        RespectSystemTheme::Off => theme_settings.theme_kind.value().clone(),
    }
}

/// Return the current theme kind based on the theme settings and active app context.
pub fn active_theme_kind(theme_settings: &ThemeSettings, app: &AppContext) -> ThemeKind {
    derived_theme_kind(theme_settings, app.system_theme())
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
