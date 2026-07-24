use settings::schema::SettingSchemaEntry;
use settings::{Setting, SettingSurfaces, SettingsMode, SyncToCloud};
use settings_value::SettingsValue;

use super::{TuiTheme, TuiThemeSetting};

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
