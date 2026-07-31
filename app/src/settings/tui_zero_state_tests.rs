use std::path::PathBuf;

use serde::Deserialize as _;
use settings::schema::SettingSchemaEntry;
use settings::{Setting, SettingSurfaces, SettingsMode, SyncToCloud};
use settings_value::SettingsValue;

use super::{
    MAX_TUI_ZERO_STATE_EXTRUSION_DEPTH, MAX_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS,
    MIN_TUI_ZERO_STATE_EXTRUSION_DEPTH, MIN_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS,
    TuiZeroStateExtrusionDepth, TuiZeroStateExtrusionDepthSetting, TuiZeroStateObject,
    TuiZeroStateObjectSetting, TuiZeroStateRotationPeriodSeconds,
    TuiZeroStateRotationPeriodSecondsSetting,
};

#[test]
fn object_source_uses_tagged_file_representation() {
    let value = serde_json::json!({
        "type": "ascii_file",
        "path": "logos/rocket.txt",
    });

    assert_eq!(
        TuiZeroStateObject::from_file_value(&value),
        Some(TuiZeroStateObject::AsciiFile {
            path: PathBuf::from("logos/rocket.txt"),
        })
    );
    assert_eq!(
        TuiZeroStateObject::BuiltIn.to_file_value(),
        serde_json::json!({ "type": "built_in" })
    );
}

#[test]
fn numeric_settings_accept_inclusive_bounds() {
    for value in [
        MIN_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS,
        MAX_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS,
    ] {
        let parsed =
            serde_json::from_value::<TuiZeroStateRotationPeriodSeconds>(serde_json::json!(value))
                .unwrap();
        assert_eq!(parsed.get(), value);
    }
    for value in [
        MIN_TUI_ZERO_STATE_EXTRUSION_DEPTH,
        MAX_TUI_ZERO_STATE_EXTRUSION_DEPTH,
    ] {
        let parsed =
            serde_json::from_value::<TuiZeroStateExtrusionDepth>(serde_json::json!(value)).unwrap();
        assert_eq!(parsed.get(), value);
    }
}

#[test]
fn numeric_settings_reject_out_of_range_values() {
    for value in [
        MIN_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS - 0.1,
        MAX_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS + 0.1,
    ] {
        assert!(
            serde_json::from_value::<TuiZeroStateRotationPeriodSeconds>(serde_json::json!(value))
                .is_err()
        );
    }
    for value in [
        MIN_TUI_ZERO_STATE_EXTRUSION_DEPTH - 0.01,
        MAX_TUI_ZERO_STATE_EXTRUSION_DEPTH + 0.01,
    ] {
        assert!(
            serde_json::from_value::<TuiZeroStateExtrusionDepth>(serde_json::json!(value)).is_err()
        );
    }
    let nan = serde::de::value::F64Deserializer::<serde::de::value::Error>::new(f64::NAN);
    assert!(TuiZeroStateRotationPeriodSeconds::deserialize(nan).is_err());
    let infinity = serde::de::value::F64Deserializer::<serde::de::value::Error>::new(f64::INFINITY);
    assert!(TuiZeroStateExtrusionDepth::deserialize(infinity).is_err());
}

#[test]
fn zero_state_settings_are_tui_local_file_settings() {
    assert_eq!(
        TuiZeroStateObjectSetting::toml_path(),
        Some("appearance.zero_state.object")
    );
    assert_eq!(
        TuiZeroStateRotationPeriodSecondsSetting::toml_path(),
        Some("appearance.zero_state.rotation_period_seconds")
    );
    assert_eq!(
        TuiZeroStateExtrusionDepthSetting::toml_path(),
        Some("appearance.zero_state.extrusion_depth")
    );
    assert_eq!(
        TuiZeroStateObjectSetting::sync_to_cloud(),
        SyncToCloud::Never
    );
    assert_eq!(
        TuiZeroStateRotationPeriodSecondsSetting::sync_to_cloud(),
        SyncToCloud::Never
    );
    assert_eq!(
        TuiZeroStateExtrusionDepthSetting::sync_to_cloud(),
        SyncToCloud::Never
    );
    assert_eq!(TuiZeroStateObjectSetting::max_table_depth(), Some(0));
}

#[test]
fn zero_state_schema_entries_are_tui_only() {
    let zero_state_entries = inventory::iter::<SettingSchemaEntry>
        .into_iter()
        .filter(|entry| entry.hierarchy == Some("appearance.zero_state"))
        .collect::<Vec<_>>();

    assert_eq!(zero_state_entries.len(), 3);
    for entry in zero_state_entries {
        assert!(entry.description.contains("Warp Agent CLI"));
        assert!(!entry.description.contains("TUI"));
        let surfaces: SettingSurfaces = (entry.surfaces_fn)();
        assert!(surfaces.includes(SettingsMode::Tui));
        assert!(!surfaces.includes(SettingsMode::Gui));
    }
}
