//! Reading and writing setting values in a preferences backend.
//!
//! The storage methods of the [`Setting`] trait delegate to these functions. They are generic over
//! at most the value type, so settings that share a value type share one compiled copy instead of
//! each setting compiling its own.

use std::fmt::Debug;
use std::ops::Deref;

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use settings_value::SettingsValue;
use warp_errors::report_error;
use warp_features::FeatureFlag;
use warpui_core::{AppContext, SingletonEntity};
use warpui_extras::user_preferences::UserPreferences;

use crate::{PrivatePreferences, PublicPreferences, Setting};

/// Where a setting is stored, and the name it is logged under.
#[derive(Clone, Copy)]
pub(crate) struct StorageLocation {
    setting_name: &'static str,
    storage_key: &'static str,
    toml_key: &'static str,
    hierarchy: Option<&'static str>,
    max_table_depth: Option<u32>,
}

impl StorageLocation {
    pub(crate) fn of<S: Setting + ?Sized>() -> Self {
        Self {
            setting_name: S::setting_name(),
            storage_key: S::storage_key(),
            toml_key: S::toml_key(),
            hierarchy: S::hierarchy(),
            max_table_depth: S::max_table_depth(),
        }
    }

    /// Returns the key the setting is stored under in the given backend.
    fn key_in(&self, preferences: &dyn UserPreferences) -> &'static str {
        if preferences.is_settings_file() {
            self.toml_key
        } else {
            self.storage_key
        }
    }
}

/// Returns the preferences backend that holds private or public settings.
pub(crate) fn preferences_for(is_private: bool, ctx: &AppContext) -> &dyn UserPreferences {
    if is_private {
        <PrivatePreferences as SingletonEntity>::as_ref(ctx).deref()
    } else if FeatureFlag::SettingsFile.is_enabled() {
        <PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences()
    } else {
        // When the settings file is disabled, fall back to the private
        // backend so both paths share a single instance.
        <PrivatePreferences as SingletonEntity>::as_ref(ctx).deref()
    }
}

/// Reads a setting's value from the given preferences, returning `None` if the value is not set
/// or could not be parsed.
pub(crate) fn read_stored_value<V>(
    preferences: &dyn UserPreferences,
    location: &StorageLocation,
) -> Option<V>
where
    V: DeserializeOwned + Debug + SettingsValue,
{
    let value = preferences
        .read_value_with_hierarchy(location.key_in(preferences), location.hierarchy)
        .unwrap_or_default()?;

    // For the settings file, use the SettingsValue trait.
    if preferences.is_settings_file() {
        let json_value = match serde_json::from_str::<serde_json::Value>(&value)
            .context("Failed to parse JSON for setting")
        {
            Ok(v) => v,
            Err(err) => {
                report_error!(err, extra: { "setting" => location.storage_key });
                preferences.inhibit_writes_for_key(location.toml_key, location.hierarchy);
                return None;
            }
        };
        match V::from_file_value(&json_value) {
            Some(val) => {
                log::debug!(
                    "Loaded {} from settings file; value: {:?}",
                    location.setting_name,
                    val
                );
                return Some(val);
            }
            None => {
                report_error!(
                    "Failed to parse file value for setting",
                    extra: { "setting" => location.storage_key }
                );
                preferences.inhibit_writes_for_key(location.toml_key, location.hierarchy);
                return None;
            }
        }
    }

    match serde_json::from_str(&value).context("Failed to parse stored value for setting") {
        Ok(val) => {
            log::debug!(
                "Loaded {} from user defaults; value: {:?}",
                location.setting_name,
                val
            );
            Some(val)
        }
        Err(err) => {
            report_error!(err, extra: { "setting" => location.storage_key });
            None
        }
    }
}

/// Writes a setting's value to the given preferences. Returns whether the value was changed from
/// what was stored.
pub(crate) fn write_stored_value<V>(
    new_value: &V,
    preferences: &dyn UserPreferences,
    location: &StorageLocation,
) -> Result<bool>
where
    V: Serialize + DeserializeOwned + PartialEq + Debug + SettingsValue,
{
    let key = location.key_in(preferences);

    // For the settings file, use the SettingsValue trait.
    let value = if preferences.is_settings_file() {
        let file_value = new_value.to_file_value();
        serde_json::to_string(&file_value).context(format!(
            "Failed to write {} to storage",
            location.storage_key
        ))?
    } else {
        serde_json::to_string(new_value).context(format!(
            "Failed to write {} to storage",
            location.storage_key
        ))?
    };

    // Compare semantically by deserializing the stored value back into
    // the typed value rather than comparing JSON strings. This avoids
    // spurious writes caused by serialization differences (key ordering,
    // null-vs-missing fields, formatting) that don't represent actual
    // value changes.
    let stored_value_matches = preferences
        .read_value_with_hierarchy(key, location.hierarchy)?
        .as_deref()
        .and_then(|stored| {
            if preferences.is_settings_file() {
                let json_value = serde_json::from_str::<serde_json::Value>(stored).ok()?;
                return V::from_file_value(&json_value);
            }
            serde_json::from_str::<V>(stored).ok()
        })
        .is_some_and(|stored_val| &stored_val == new_value);

    if !stored_value_matches {
        log::debug!(
            "Writing new value of {} to storage; key: {}; value: {:?}",
            location.setting_name,
            key,
            value
        );
        let _ = preferences.write_value_with_hierarchy(
            key,
            value,
            location.hierarchy,
            location.max_table_depth,
        );
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Removes a setting's value from the given preferences.
pub(crate) fn clear_stored_value(
    preferences: &dyn UserPreferences,
    location: &StorageLocation,
) -> Result<()> {
    let key = location.key_in(preferences);
    log::debug!(
        "Clearing setting {} with key {} from preferences",
        location.setting_name,
        key,
    );
    preferences.remove_value_with_hierarchy(key, location.hierarchy)?;
    Ok(())
}
