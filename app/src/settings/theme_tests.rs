use super::*;
use crate::{themes::theme::CustomTheme, user_config};
use std::path::PathBuf;

fn custom(path: PathBuf) -> ThemeKind {
    ThemeKind::Custom(CustomTheme::new("Custom".to_string(), path))
}

fn custom_base16(path: PathBuf) -> ThemeKind {
    ThemeKind::CustomBase16(CustomTheme::new("Base16 Custom".to_string(), path))
}

#[test]
fn theme_kind_syncs_custom_theme_under_theme_root() {
    let setting = Theme::new(Some(custom(user_config::themes_dir().join("custom.yml"))));

    assert!(setting.current_value_is_syncable());
}

#[test]
fn theme_kind_does_not_sync_custom_theme_outside_theme_root() {
    let setting = Theme::new(Some(custom(std::env::temp_dir().join("custom.yml"))));

    assert!(!setting.current_value_is_syncable());
}

#[test]
fn theme_kind_syncs_custom_base16_theme_under_theme_root() {
    let setting = Theme::new(Some(custom_base16(
        user_config::themes_dir().join("base16/custom.yml"),
    )));

    assert!(setting.current_value_is_syncable());
}

#[test]
fn selected_system_themes_sync_when_custom_paths_are_under_theme_root() {
    let setting = SystemThemes::new(Some(SelectedSystemThemes {
        light: custom(user_config::themes_dir().join("light.yml")),
        dark: custom_base16(user_config::themes_dir().join("dark.yml")),
    }));

    assert!(setting.current_value_is_syncable());
}

#[test]
fn selected_system_themes_do_not_sync_when_any_custom_path_is_outside_theme_root() {
    let setting = SystemThemes::new(Some(SelectedSystemThemes {
        light: custom(user_config::themes_dir().join("light.yml")),
        dark: custom(std::env::temp_dir().join("dark.yml")),
    }));

    assert!(!setting.current_value_is_syncable());
}

#[test]
fn built_in_theme_settings_remain_syncable() {
    let theme = Theme::new(Some(ThemeKind::Dark));
    let system_themes = SystemThemes::new(Some(SelectedSystemThemes {
        light: ThemeKind::Light,
        dark: ThemeKind::Dark,
    }));

    assert!(theme.current_value_is_syncable());
    assert!(system_themes.current_value_is_syncable());
}
