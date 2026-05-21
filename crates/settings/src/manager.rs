use std::collections::HashMap;

use std::ops::Deref;

use anyhow::{Result, anyhow};
use warp_features::FeatureFlag;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};
use warpui_extras::user_preferences::UserPreferences;

use super::PrivatePreferences;

use super::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

type UpdateFn = Box<dyn FnMut(String, bool, &mut AppContext) -> Result<()>>;

type ClearFn = Box<dyn FnMut(&mut AppContext) -> Result<()>>;

/// Loads a value into memory without persisting. Parameters: (serialized_value, explicitly_set, ctx).
type LoadFn = Box<dyn FnMut(String, bool, &mut AppContext) -> Result<()>>;

type EqualsFn = Box<dyn Fn(&str, &str) -> Result<bool>>;

type IsSyncableFn = Box<dyn Fn(&AppContext) -> bool>;

/// Intermediate data collected for each setting during reload, before
/// calling the mutable `load_fns`.
struct SettingReloadEntry {
    storage_key: String,
    read_value: Option<String>,
    serialized_default: String,
    toml_key: &'static str,
    hierarchy: Option<&'static str>,
}

#[derive(Debug)]
struct SettingsInfo {
    sync_to_cloud: SyncToCloud,
    supported_platforms: SupportedPlatforms,
    serialized_default_value: String,
    /// The default value serialized using the `SettingsValue` trait
    /// for the settings file.
    file_serialized_default_value: String,
    hierarchy: Option<&'static str>,
    /// The key used in the TOML settings file (last segment of `toml_path`).
    /// For settings without a `toml_path`, this equals the storage key.
    toml_key: &'static str,
    /// The maximum number of TOML section-table levels to use when rendering
    /// this setting's value in the settings file. `None` means unlimited.
    max_table_depth: Option<u32>,
    /// Whether this setting is private (not shown in the user-visible settings file).
    is_private: bool,
}

/// Provides an interface for listening for settings events based on
/// storage key and also for updating settings based on storage key.
///
/// Practically speaking this struct is used for keeping local and
/// cloud preferences in sync with each other without creating a direct
/// dependency between the define_settings_group macros and the
/// cloud preferences syncing machinery.
#[derive(Default)]
pub struct SettingsManager {
    /// Settings info by storage key
    settings: HashMap<String, SettingsInfo>,

    /// Functions for updating settings by storage key
    update_fns: HashMap<String, UpdateFn>,

    /// Functions for clearing settings from local storage (which also effectively resets them to their default value)
    clear_fns: HashMap<String, ClearFn>,

    /// Functions for loading a value into memory without persisting to storage.
    /// Used during hot-reload to avoid write-back loops with the file watcher.
    load_fns: HashMap<String, LoadFn>,

    /// Functions for checking whether two serialized settings
    /// with the same storage key have equal values.  Note that
    /// we need this because we can't just compare the deserialized
    /// or raw json values for equality. This fails because things
    /// like HashSet serialize to ordered json arrays, but don't have
    /// a defined order.
    equals_fns: HashMap<String, EqualsFn>,

    /// Functions for checking whether a setting is currently syncable
    /// based on its value. Settings that want custom logic here should define
    /// the current_value_is_syncable method.
    is_syncable_fns: HashMap<String, IsSyncableFn>,
}

pub enum SettingsEvent {
    LocalPreferencesUpdated {
        storage_key: String,
        sync_to_cloud: SyncToCloud,
    },
}

impl SettingsManager {
    /// Registers a function that updates a setting with the given storage key
    /// to have a new value. Also tracks whether that storage key is for a cloud-synced
    /// setting and what platforms it's supported on.
    #[allow(clippy::too_many_arguments)]
    pub fn register_setting(
        &mut self,
        storage_key: &str,
        sync_to_cloud: SyncToCloud,
        supported_platforms: SupportedPlatforms,
        serialized_default_value: String,
        file_serialized_default_value: String,
        hierarchy: Option<&'static str>,
        toml_key: &'static str,
        max_table_depth: Option<u32>,
        is_private: bool,
        update_fn: impl FnMut(String, bool, &mut AppContext) -> Result<()> + 'static,
        clear_fn: impl FnMut(&mut AppContext) -> Result<()> + 'static,
        load_fn: impl FnMut(String, bool, &mut AppContext) -> Result<()> + 'static,
        equals_fn: impl Fn(&str, &str) -> Result<bool> + 'static,
        is_syncable_fn: impl Fn(&AppContext) -> bool + 'static,
    ) {
        self.update_fns
            .insert(storage_key.to_owned(), Box::new(update_fn));
        self.clear_fns
            .insert(storage_key.to_owned(), Box::new(clear_fn));
        self.load_fns
            .insert(storage_key.to_owned(), Box::new(load_fn));
        self.equals_fns
            .insert(storage_key.to_owned(), Box::new(equals_fn));
        self.is_syncable_fns
            .insert(storage_key.to_owned(), Box::new(is_syncable_fn));
        self.settings.insert(
            storage_key.to_owned(),
            SettingsInfo {
                supported_platforms,
                sync_to_cloud,
                serialized_default_value,
                file_serialized_default_value,
                hierarchy,
                toml_key,
                max_table_depth,
                is_private,
            },
        );
    }

    /// Clears all cloud synced settings from the user defaults. Does not affect their cloud state.
    /// Typically called when a user logs out. Note that the caller is responsible for ensuring that
    /// cloud preferences are enabled before calling this.
    pub fn clear_cloud_settings_local_state(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Vec<anyhow::Error> {
        self.clear_fns
            .values_mut()
            .filter_map(|clear_fn| clear_fn(ctx).err())
            .collect::<Vec<anyhow::Error>>()
    }

    /// Returns all registered storage keys.
    pub fn all_storage_keys(&self) -> impl Iterator<Item = &String> {
        self.settings.keys()
    }

    /// Returns the storage keys for all public (non-private) settings.
    pub fn public_storage_keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.settings
            .iter()
            .filter(|(_, info)| !info.is_private)
            .map(|(key, _)| key.as_str())
    }

    /// Returns whether the setting with the given storage key should be synced even if the
    /// user has disabled syncing.
    pub fn sync_regardless_of_users_syncing_setting(&self, storage_key: &str) -> bool {
        self.settings
            .get(storage_key)
            .map(|info| {
                matches!(
                    info.sync_to_cloud,
                    SyncToCloud::Globally(RespectUserSyncSetting::No)
                        | SyncToCloud::PerPlatform(RespectUserSyncSetting::No)
                )
            })
            .unwrap_or(false)
    }

    /// Returns whether the setting with the given storage key has a value that is currently
    /// syncable to the cloud.
    pub fn is_current_value_syncable(&self, storage_key: &str, app: &AppContext) -> Result<bool> {
        self.is_syncable_fns
            .get(storage_key)
            .map(|cb| Ok(cb(app)))
            .unwrap_or_else(|| {
                Err(anyhow!(
                    "no is_syncable fn registered for storage key {}",
                    storage_key
                ))
            })
    }

    /// Returns the cloud_syncing_mode for the given storage key.
    pub fn cloud_syncing_mode_for_storage_key(&self, storage_key: &str) -> Option<SyncToCloud> {
        self.settings
            .get(storage_key)
            .map(|info| info.sync_to_cloud)
    }

    /// Returns the supported platforms for this storage key.
    pub fn supported_platforms_for_storage_key(
        &self,
        storage_key: &str,
    ) -> Option<&SupportedPlatforms> {
        self.settings
            .get(storage_key)
            .map(|info| &info.supported_platforms)
    }

    /// Returns whether the setting with the given storage key is private.
    pub fn is_private_for_storage_key(&self, storage_key: &str) -> bool {
        self.settings
            .get(storage_key)
            .map(|info| info.is_private)
            .unwrap_or(false)
    }

    /// Reads a setting's current local value from the correct preferences
    /// backend, routing private settings to the private store and public
    /// settings to the main (potentially TOML-backed) store.
    pub fn read_local_setting_value(
        &self,
        storage_key: &str,
        ctx: &AppContext,
    ) -> Result<Option<String>> {
        let private: &dyn UserPreferences =
            <PrivatePreferences as SingletonEntity>::as_ref(ctx).deref();
        let prefs: &dyn UserPreferences = if self.is_private_for_storage_key(storage_key) {
            private
        } else if FeatureFlag::SettingsFile.is_enabled() {
            <super::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences()
        } else {
            // When the settings file is disabled, fall back to the private
            // backend so both paths share a single instance.
            private
        };
        let info = self.settings.get(storage_key);
        let key = if prefs.is_settings_file() {
            info.map_or(storage_key, |i| i.toml_key)
        } else {
            storage_key
        };
        let hierarchy = info.and_then(|i| i.hierarchy);
        prefs
            .read_value_with_hierarchy(key, hierarchy)
            .map_err(|e| anyhow!("failed to read setting {storage_key}: {e}"))
    }

    /// Updates the setting with the given storage key to a new value, returning
    /// a result indicating whether the update was successful.
    pub fn update_setting_with_storage_key(
        &mut self,
        storage_key: &str,
        new_value: String,
        from_cloud_sync: bool,
        ctx: &mut AppContext,
    ) -> Result<()> {
        self.update_fns
            .get_mut(storage_key)
            .map(|update_fn| update_fn(new_value, from_cloud_sync, ctx))
            .unwrap_or_else(|| {
                Err(anyhow!(
                    "no update fn registered for storage key {}",
                    storage_key
                ))
            })
    }

    /// Returns whether the two serialized settings with the given storage key
    /// have equal values.  This isn't a direct string comparison, or even a comparison
    /// of JSON values, but a comparison using the Setting.value()'s equality method.
    pub fn are_equal_settings(&self, storage_key: &str, left: &str, right: &str) -> Result<bool> {
        self.equals_fns
            .get(storage_key)
            .map(|equality_fn| equality_fn(left, right))
            .unwrap_or_else(|| {
                Err(anyhow!(
                    "no equals fn registered for storage key {}",
                    storage_key
                ))
            })
    }

    pub fn default_values(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.settings
            .iter()
            .map(|(key, info)| (key.clone(), info.serialized_default_value.clone()))
    }

    /// Loads a setting value into memory without persisting to storage.
    ///
    /// `explicitly_set` indicates whether the value came from the file (`true`)
    /// or is a default for an absent key (`false`).
    pub fn load_setting(
        &mut self,
        storage_key: &str,
        value: String,
        explicitly_set: bool,
        ctx: &mut AppContext,
    ) -> Result<()> {
        self.load_fns
            .get_mut(storage_key)
            .map(|load_fn| load_fn(value, explicitly_set, ctx))
            .unwrap_or_else(|| {
                Err(anyhow!(
                    "no load fn registered for storage key {}",
                    storage_key
                ))
            })
    }

    /// Reloads all public (non-private) settings from the preferences backend.
    ///
    /// Call this after the backing store has been refreshed from disk (e.g.
    /// via [`UserPreferences::reload_from_disk`]) so that every in-memory
    /// setting picks up the new values.
    ///
    /// Uses [`load_setting`](Self::load_setting) to update in-memory values
    /// without writing back to the preferences backend, avoiding write-back
    /// loops with the file watcher. Keys present in the file are loaded with
    /// `explicitly_set = true`; absent keys are reset to their default with
    /// `explicitly_set = false`.
    /// Returns the storage keys of any settings that failed to load.
    pub fn reload_all_public_settings(&mut self, ctx: &mut AppContext) -> Vec<String> {
        let prefs = <super::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();

        // Read every non-private setting from the (now-reloaded) preferences,
        // collecting them up-front to release the immutable borrow on
        // `self.settings` before calling the mutable `load_fns`.
        let updates: Vec<SettingReloadEntry> = self
            .settings
            .iter()
            .filter(|(_, info)| !info.is_private)
            .map(|(key, info)| {
                let read_value =
                    match prefs.read_value_with_hierarchy(info.toml_key, info.hierarchy) {
                        Ok(v) => v,
                        Err(err) => {
                            log::warn!("Failed to read setting {key} during reload: {err}");
                            None
                        }
                    };
                SettingReloadEntry {
                    storage_key: key.clone(),
                    read_value,
                    serialized_default: info.serialized_default_value.clone(),
                    toml_key: info.toml_key,
                    hierarchy: info.hierarchy,
                }
            })
            .collect();

        let mut failed_keys = Vec::new();
        for entry in updates {
            let (effective_value, explicitly_set) = match entry.read_value {
                Some(v) => (v, true),
                None => (entry.serialized_default, false),
            };
            if let Err(err) =
                self.load_setting(&entry.storage_key, effective_value, explicitly_set, ctx)
            {
                log::warn!("Failed to reload setting {}: {err}", entry.storage_key);
                // Re-inhibit this key so writes don't overwrite the
                // user's broken-but-fixable value in the file.
                let prefs =
                    <super::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();
                prefs.inhibit_writes_for_key(entry.toml_key, entry.hierarchy);
                failed_keys.push(entry.toml_key.to_string());
            }
        }
        failed_keys
    }

    /// Validates all public settings by reading each from the preferences
    /// backend and attempting deserialization. Returns the storage keys of
    /// any settings whose stored value cannot be deserialized.
    ///
    /// This is a read-only check — it does not modify in-memory state.
    /// Call after [`register_all_settings`] on startup to detect invalid
    /// values in the settings file.
    pub fn validate_all_public_settings(&self, ctx: &AppContext) -> Vec<String> {
        let prefs = <super::PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences();

        self.settings
            .iter()
            .filter(|(_, info)| !info.is_private)
            .filter_map(|(key, info)| {
                let value = prefs
                    .read_value_with_hierarchy(info.toml_key, info.hierarchy)
                    .ok()
                    .flatten()?;

                // Try deserializing through the equals_fn — if serde_json
                // can't parse both sides, the value is invalid.
                if let Some(equals_fn) = self.equals_fns.get(key)
                    && equals_fn(&value, &value).is_err()
                {
                    return Some(info.toml_key.to_string());
                }
                None
            })
            .collect()
    }

    /// Returns all registered settings with their toml key, serialized
    /// default value (in file format), hierarchy path, and max table depth,
    /// for use when writing the user-visible settings file.
    pub fn default_values_for_settings_file(
        &self,
    ) -> impl Iterator<Item = (&str, &str, Option<&'static str>, Option<u32>)> + '_ {
        self.settings
            .iter()
            .filter(|(_, info)| !info.is_private)
            .map(|(_, info)| {
                (
                    info.toml_key,
                    info.file_serialized_default_value.as_str(),
                    info.hierarchy,
                    info.max_table_depth,
                )
            })
    }
}

impl Entity for SettingsManager {
    type Event = SettingsEvent;
}

/// Mark SettingsManager as global application state.
impl SingletonEntity for SettingsManager {}
