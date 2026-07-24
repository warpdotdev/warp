use settings::schema::SettingSchemaEntry;
use settings::{Setting, SettingSurfaces, SettingsMode, SyncToCloud};
use settings_value::SettingsValue;
#[cfg(feature = "tui")]
use warpui_core::runtime::BackgroundLuminance;

use super::{TuiTheme, TuiThemeSetting};

#[test]
fn theme_names_parse_case_insensitively() {
    assert_eq!("auto".parse(), Ok(TuiTheme::Auto));
    assert_eq!("LIGHT".parse(), Ok(TuiTheme::Light));
    assert_eq!("dark".parse(), Ok(TuiTheme::Dark));
    assert!("sepia".parse::<TuiTheme>().is_err());
}

#[test]
#[cfg(feature = "tui")]
fn automatic_theme_follows_the_background_luminance() {
    let light = TuiTheme::Auto.resolve_for_background(BackgroundLuminance::Light);
    let dark = TuiTheme::Auto.resolve_for_background(BackgroundLuminance::Dark);
    let unknown = TuiTheme::Auto.resolve_for_background(BackgroundLuminance::Unknown);

    assert_eq!(TuiTheme::from(&light), TuiTheme::Light);
    assert_eq!(TuiTheme::from(&dark), TuiTheme::Dark);
    assert_eq!(TuiTheme::from(&unknown), TuiTheme::Dark);
}

#[test]
#[cfg(feature = "tui")]
fn explicit_theme_overrides_the_background_luminance() {
    let dark = TuiTheme::Dark.resolve_for_background(BackgroundLuminance::Light);

    assert_eq!(TuiTheme::from(&dark), TuiTheme::Dark);
}
#[test]
fn theme_values_use_lowercase_file_representation() {
    assert_eq!(TuiTheme::Auto.to_file_value(), serde_json::json!("auto"));
    assert_eq!(TuiTheme::Light.to_file_value(), serde_json::json!("light"));
    assert_eq!(TuiTheme::Dark.to_file_value(), serde_json::json!("dark"));
    assert_eq!(
        TuiTheme::from_file_value(&serde_json::json!("light")),
        Some(TuiTheme::Light)
    );
    assert_eq!(
        TuiTheme::from_file_value(&serde_json::json!("auto")),
        Some(TuiTheme::Auto)
    );
}

#[test]
fn theme_setting_is_tui_local_and_defaults_to_automatic_detection() {
    let setting = TuiThemeSetting::new(None);

    assert_eq!(TuiThemeSetting::toml_path(), Some("appearance.theme"));
    assert_eq!(TuiThemeSetting::sync_to_cloud(), SyncToCloud::Never);
    assert_eq!(setting.value(), &TuiTheme::Auto);
    assert!(!setting.is_value_explicitly_set());
}

#[test]
fn theme_schema_entry_is_tui_only() {
    let entry = inventory::iter::<SettingSchemaEntry>
        .into_iter()
        .find(|entry| entry.hierarchy == Some("appearance") && entry.storage_key == "theme")
        .expect("expected Warp Agent CLI theme schema entry");
    let surfaces: SettingSurfaces = (entry.surfaces_fn)();

    assert!(surfaces.includes(SettingsMode::Tui));
    assert!(!surfaces.includes(SettingsMode::Gui));
}
