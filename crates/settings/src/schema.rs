//! Schema metadata for settings, used by the JSON Schema generator.

use schemars::{Schema, SchemaGenerator};

use crate::SupportedPlatforms;

/// Metadata about a single setting, collected via `inventory` for schema generation.
///
/// Each setting registered with `define_setting!` or `implement_setting_for_enum!`
/// emits an `inventory::submit!` call that registers one of these entries. The
/// generator binary iterates all entries to produce a JSON Schema document.
pub struct SettingSchemaEntry {
    /// The storage key for this setting (last segment of toml_path).
    pub storage_key: &'static str,

    /// User-facing description of what this setting does.
    pub description: &'static str,

    /// The TOML section path (everything before the last segment of toml_path).
    pub hierarchy: Option<&'static str>,

    /// Whether this setting is private (excluded from user-facing schema).
    pub is_private: bool,

    /// Feature flag gating this setting.
    /// If Some, the setting is only included in the schema when the flag
    /// is active for the target build channel.
    pub feature_flag: Option<warp_features::FeatureFlag>,

    /// Returns which platforms this setting applies to.
    pub supported_platforms_fn: fn() -> SupportedPlatforms,

    /// Returns the default value serialized as JSON.
    pub default_value_fn: fn() -> String,

    /// Returns the JSON Schema for this setting's value type.
    pub schema_fn: fn(&mut SchemaGenerator) -> Schema,

    /// Returns the default value serialized using `SettingsValue::to_file_value`.
    pub file_default_value_fn: fn() -> String,

    /// The maximum number of TOML section-table levels to use when rendering
    /// this setting's value in the settings file. Mirrors `Setting::max_table_depth`.
    /// `None` means unlimited depth.
    pub max_table_depth: Option<u32>,
}

inventory::collect!(SettingSchemaEntry);

/// Submits a [`SettingSchemaEntry`] to the `inventory` registry.
#[macro_export]
macro_rules! submit_schema_entry {
    (
        private: $private:expr,
        description: $desc:expr,
        toml_path_value: $toml_path:expr,
        fallback_storage_key: $fallback_key:expr,
        supported_platforms: $plat:expr,
        feature_flag: $flag:expr,
        max_table_depth: $mtd:expr,
        default: $default:tt,
        value_type: $type:ty $(,)?
    ) => {
        $crate::_inventory::submit! {
            $crate::schema::SettingSchemaEntry {
                storage_key: {
                    const KEY: &str = match $toml_path {
                        Some(path) => $crate::toml_path_storage_key(path),
                        None => $fallback_key,
                    };
                    KEY
                },
                description: $desc,
                hierarchy: {
                    const HIER: Option<&str> = match $toml_path {
                        Some(path) => $crate::toml_path_hierarchy(path),
                        None => None,
                    };
                    HIER
                },
                is_private: $private,
                feature_flag: $flag,
                supported_platforms_fn: || $plat,
                default_value_fn: || {
                    let val: $type = $default;
                    serde_json::to_string(&val).expect("default value should serialize")
                },
                schema_fn: <$type as $crate::_settings_value::SettingsValue>::file_schema,
                file_default_value_fn: || {
                    use $crate::_settings_value::SettingsValue as _;
                    let val: $type = $default;
                    let file_value = val.to_file_value();
                    serde_json::to_string(&file_value).expect("default file value should serialize")
                },
                max_table_depth: $mtd,
            }
        }
    };
}

/// Helper: produces a `&'static str` description, defaulting to `""` when omitted.
#[macro_export]
macro_rules! _schema_default_description {
    () => {
        ""
    };
    ($desc:literal) => {
        $desc
    };
}

/// Helper: produces `Option<FeatureFlag>` for a feature flag, defaulting to `None`.
#[macro_export]
macro_rules! _schema_default_flag {
    () => {
        None
    };
    ($flag:path) => {
        Some($flag)
    };
}

/// Helper: produces `Option<u32>` for a max-table-depth literal, defaulting to `None`.
#[macro_export]
macro_rules! _schema_default_max_table_depth {
    () => {
        None
    };
    ($mtd:literal) => {
        Some($mtd)
    };
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
