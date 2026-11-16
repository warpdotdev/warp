use serde::{Deserialize, Serialize};
use serde_json::json;
use settings_value::SettingsValue;

// -- struct-level #[serde(default)] -------------------------------------------

#[derive(Debug, Default, PartialEq, Serialize, Deserialize, SettingsValue)]
#[serde(default)]
struct StructWithContainerDefault {
    name: String,
    count: u32,
}

#[test]
fn container_default_missing_field_uses_default() {
    let json = json!({"name": "hello"});
    let result = StructWithContainerDefault::from_file_value(&json).unwrap();
    assert_eq!(result.name, "hello");
    assert_eq!(result.count, 0);
}

#[test]
fn container_default_empty_object_uses_all_defaults() {
    let json = json!({});
    let result = StructWithContainerDefault::from_file_value(&json).unwrap();
    assert_eq!(result, StructWithContainerDefault::default());
}

#[test]
fn container_default_all_fields_present() {
    let json = json!({"name": "hello", "count": 42});
    let result = StructWithContainerDefault::from_file_value(&json).unwrap();
    assert_eq!(result.name, "hello");
    assert_eq!(result.count, 42);
}

#[test]
fn container_default_round_trip() {
    let val = StructWithContainerDefault {
        name: "test".into(),
        count: 7,
    };
    let file_val = val.to_file_value();
    let back = StructWithContainerDefault::from_file_value(&file_val).unwrap();
    assert_eq!(back, val);
}

// -- struct-level default with non-Default field type -------------------------

/// An enum that does NOT implement Default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, SettingsValue)]
enum Mode {
    Off,
    On,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, SettingsValue)]
#[serde(default)]
struct StructWithNonDefaultField {
    mode: Mode,
    label: String,
}

impl Default for StructWithNonDefaultField {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            label: String::new(),
        }
    }
}

#[test]
fn container_default_non_default_field_uses_struct_default() {
    let json = json!({"label": "hello"});
    let result = StructWithNonDefaultField::from_file_value(&json).unwrap();
    assert_eq!(result.mode, Mode::Off);
    assert_eq!(result.label, "hello");
}

#[test]
fn container_default_non_default_field_present() {
    // SettingsValue derive converts variant names to snake_case.
    let json = json!({"mode": "on", "label": "hello"});
    let result = StructWithNonDefaultField::from_file_value(&json).unwrap();
    assert_eq!(result.mode, Mode::On);
    assert_eq!(result.label, "hello");
}

// -- without struct-level default, missing fields should fail -----------------

#[derive(Debug, PartialEq, Serialize, Deserialize, SettingsValue)]
struct StructWithoutDefault {
    name: String,
    count: u32,
}

#[test]
fn no_container_default_missing_field_returns_none() {
    let json = json!({"name": "hello"});
    assert!(StructWithoutDefault::from_file_value(&json).is_none());
}

#[test]
fn no_container_default_all_present_succeeds() {
    let json = json!({"name": "hello", "count": 42});
    let result = StructWithoutDefault::from_file_value(&json).unwrap();
    assert_eq!(result.name, "hello");
    assert_eq!(result.count, 42);
}
