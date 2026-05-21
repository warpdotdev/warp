#[macro_use]
pub mod macros;
pub mod manager;
pub mod schema;

// Re-export commonly used types and traits
pub use macros::SettingSection;
pub use manager::SettingsManager;

// Re-export crates used by macro expansions in downstream crates.
#[doc(hidden)]
pub use inventory as _inventory;
#[doc(hidden)]
pub use schemars as _schemars;
#[doc(hidden)]
pub use settings_value as _settings_value;
pub use settings_value::SettingsValue;

use std::fmt::Debug;
use std::ops::Deref;

/// Extracts the storage key (last segment after the final `.`) from a toml_path.
///
/// # Examples
/// - `"appearance.text.font_name"` → `"font_name"`
/// - `"font_name"` → `"font_name"`
pub const fn toml_path_storage_key(path: &str) -> &str {
    let bytes = path.as_bytes();
    let mut i = path.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'.' {
            let (_, suffix) = path.split_at(i + 1);
            return suffix;
        }
    }
    path
}

/// Extracts the hierarchy (everything before the final `.`) from a toml_path.
///
/// Returns `None` when the path contains no dot (the path is just a key).
///
/// # Examples
/// - `"appearance.text.font_name"` → `Some("appearance.text")`
/// - `"font_name"` → `None`
pub const fn toml_path_hierarchy(path: &str) -> Option<&str> {
    let bytes = path.as_bytes();
    let mut i = path.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'.' {
            let (prefix, _) = path.split_at(i);
            return Some(prefix);
        }
    }
    None
}

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use warp_features::FeatureFlag;
use warpui::{AppContext, Entity, ModelContext};
use warpui_extras::user_preferences::UserPreferences;

/// A newtype wrapper for the public preferences backend.
///
/// Public settings (those marked `private: false` in `define_settings_group!`)
/// are stored in the user-visible settings file (TOML) when the `SettingsFile`
/// feature flag is enabled, otherwise in the platform-native store.
///
/// The inner field is private and only accessible within the settings crate via
/// [`as_preferences`](Self::as_preferences). This prevents external code from
/// bypassing the settings macros to read/write public preferences directly.
pub struct PublicPreferences(Box<dyn UserPreferences>);

impl PublicPreferences {
    pub fn new(prefs: Box<dyn UserPreferences>) -> Self {
        Self(prefs)
    }

    /// Returns the underlying preferences backend.
    ///
    /// This is intentionally `pub(crate)` so that only the settings
    /// infrastructure (macros, `Setting` trait, `SettingsManager`) can access
    /// the raw backend. External code must go through typed settings groups
    /// produced by `define_settings_group!`.
    pub(crate) fn as_preferences(&self) -> &dyn UserPreferences {
        self.0.as_ref()
    }

    /// Returns whether this backend is the user-visible settings file.
    pub fn is_settings_file(&self) -> bool {
        self.0.is_settings_file()
    }

    /// Reloads the backing store from disk.
    pub fn reload_from_disk(&self) -> Result<(), warpui_extras::user_preferences::Error> {
        self.0.reload_from_disk()
    }
}

impl warpui::Entity for PublicPreferences {
    type Event = ();
}

impl warpui::SingletonEntity for PublicPreferences {}

/// A newtype wrapper for the private preferences backend.
///
/// Private settings (those marked `private: true` in `define_settings_group!`)
/// are stored here instead of in the user-visible settings file. This always
/// uses the platform-native store (e.g. UserDefaults on macOS, JSON file on
/// Linux, registry on Windows).
pub struct PrivatePreferences(Box<dyn UserPreferences>);

impl PrivatePreferences {
    pub fn new(prefs: Box<dyn UserPreferences>) -> Self {
        Self(prefs)
    }
}

impl Deref for PrivatePreferences {
    type Target = dyn UserPreferences;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl warpui::Entity for PrivatePreferences {
    type Event = ();
}

impl warpui::SingletonEntity for PrivatePreferences {}

/// An enum representing the different platforms a setting could apply to.
#[derive(Debug, Clone)]
pub enum SupportedPlatforms {
    ALL,
    DESKTOP, /* Refers to running on device, not web-based, such as Mac, Linux, and Windows */
    MAC,
    LINUX,
    WINDOWS,
    WEB,
    OR(Box<SupportedPlatforms>, Box<SupportedPlatforms>),
}

/// An enum representing the different ways a setting can be synced to the cloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncToCloud {
    /// The setting is synced to the cloud as a single global value that applies to on all supported platforms.
    Globally(RespectUserSyncSetting),

    /// The setting is synced to the cloud as a value that is unique to each platform.
    PerPlatform(RespectUserSyncSetting),

    /// The setting is not synced to the cloud.
    Never,
}

/// Whether for this setting we respect the user toggle for settings sync.
/// There are some cases we want to sync settings regardless of the user setting,
/// such as for the value of whether cloud syncing is enabled, whether telemetry is enabled, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RespectUserSyncSetting {
    /// Only sync if the user has settings sync enabled
    Yes,

    /// Sync regardless of the user's setting
    No,
}

impl SupportedPlatforms {
    pub fn matches_current_platform(&self) -> bool {
        match self {
            SupportedPlatforms::ALL => true,
            SupportedPlatforms::DESKTOP => {
                cfg!(not(target_family = "wasm"))
            }
            SupportedPlatforms::MAC => {
                cfg!(all(not(target_family = "wasm"), target_os = "macos"))
            }
            SupportedPlatforms::LINUX => {
                cfg!(all(
                    not(target_family = "wasm"),
                    any(target_os = "linux", target_os = "freebsd")
                ))
            }
            SupportedPlatforms::WINDOWS => {
                cfg!(all(not(target_family = "wasm"), target_os = "windows"))
            }
            SupportedPlatforms::WEB => {
                cfg!(target_family = "wasm")
            }
            SupportedPlatforms::OR(first, second) => {
                first.matches_current_platform() || second.matches_current_platform()
            }
        }
    }
}

/// An enum representing the reason for a change event.
#[derive(Debug, Clone, Copy)]
pub enum ChangeEventReason {
    /// The change was initiated from a cloud sync
    CloudSync,

    /// The change was initiated from a local setting change
    LocalChange,

    /// The change was initiated from a clear operation
    Clear,
}

/// A representation of a setting which can be loaded from and persisted to some
/// sort of durable storage.
pub trait Setting {
    type Group: Entity;
    type Value: Serialize + DeserializeOwned + PartialEq + Debug + SettingsValue;

    /// Constructs this setting object with the given initial value.
    /// If value is None, uses the default value and marks as not explicitly set.
    /// If value is Some, uses that value and marks as explicitly set.
    fn new(value: Option<Self::Value>) -> Self
    where
        Self: Sized;

    /// Returns the name of the setting.
    fn setting_name() -> &'static str;

    /// Returns the key underwhich this setting should be stored.  Should be
    /// distinct from all other settings, and should not change over time.
    fn storage_key() -> &'static str;

    /// Returns the full TOML path for this setting, if any.
    ///
    /// The toml_path is a dot-separated path that includes both the hierarchy
    /// (section) and the storage key as the last segment. For example,
    /// `"appearance.text.font_name"` means the setting lives under
    /// `[appearance.text]` with key `font_name`.
    fn toml_path() -> Option<&'static str> {
        None
    }

    /// Returns the key used in the TOML settings file.
    ///
    /// For settings with a `toml_path`, this is the last segment (e.g.
    /// `"font_name"` from `"appearance.text.font_name"`). For settings
    /// without a `toml_path`, falls back to `storage_key()`.
    fn toml_key() -> &'static str {
        Self::storage_key()
    }

    /// Returns the hierarchy path for this setting, if any.
    ///
    /// When set, hierarchy-aware preferences backends use this to organize
    /// settings into logical groups. For example, a hierarchy of `"font"`
    /// places the setting under a `font` section in the backing store.
    fn hierarchy() -> Option<&'static str> {
        None
    }

    /// Returns the maximum number of TOML section-table levels to use when
    /// rendering this setting's value in the settings file.
    ///
    /// - `None` (default) — unlimited depth; nested objects become section
    ///   tables (`[section.subsection]`) all the way down.
    /// - `Some(0)` — the value itself is rendered as an inline table
    ///   (`key = { ... }`). Used for enum settings whose shape changes between
    ///   variants.
    /// - `Some(1)` — the setting gets its own section header, but any nested
    ///   objects within it are rendered inline. Useful for struct settings
    ///   that contain maps or nested structs.
    fn max_table_depth() -> Option<u32> {
        None
    }

    /// Returns the platforms that this setting is supported on.
    fn supported_platforms() -> SupportedPlatforms;

    /// Returns whether and how this setting is synced to the cloud via Warp Drive.
    fn sync_to_cloud() -> SyncToCloud;

    /// Returns whether this setting is private (not shown in the user-visible settings file).
    ///
    /// Private settings are persisted to the platform-native store (e.g. UserDefaults on
    /// macOS) rather than the TOML settings file, ensuring they never appear in the
    /// user-editable file.
    fn is_private() -> bool;

    /// Returns whether the current value of this setting should be synced.
    /// Only applies if sync_to_cloud() returns a value other than SyncToCloud::Never.
    /// Specific settings can implement this to filter which values should be synced.
    fn current_value_is_syncable(&self) -> bool {
        true
    }

    /// Returns whether the current value of this setting is syncable on the current platform,
    /// given the user's settings sync preference.
    fn is_setting_syncable_on_current_platform(&self, settings_sync_enabled: bool) -> bool {
        if !self.current_value_is_syncable() {
            return false;
        }
        match (Self::sync_to_cloud(), settings_sync_enabled) {
            (SyncToCloud::Never, _) => false,
            (SyncToCloud::Globally(RespectUserSyncSetting::No), _) => true,
            (SyncToCloud::Globally(RespectUserSyncSetting::Yes), true) => true,
            (SyncToCloud::Globally(RespectUserSyncSetting::Yes), false) => false,
            (SyncToCloud::PerPlatform(RespectUserSyncSetting::No), _) => {
                self.is_supported_on_current_platform()
            }
            (SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes), true) => {
                self.is_supported_on_current_platform()
            }
            (SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes), false) => false,
        }
    }

    /// Returns the current value of the setting.  This may be different from
    /// the value persisted in storage.
    fn value(&self) -> &Self::Value;

    /// Validates whether the new value is valid and returns a valid value to
    /// use.  If the provided value is valid, the expectation is that this will
    /// return the provided value.  If not, it is up to the implementation
    /// whether the current value of the setting is returned or whether some
    /// other value is returned.
    fn validate(&self, new_value: Self::Value) -> Self::Value {
        new_value
    }

    /// Clears the value of the setting from persistent storage and fires a change event
    /// indicating that the value was cleared.
    fn clear_value(&mut self, ctx: &mut ModelContext<Self::Group>) -> anyhow::Result<()>;

    /// Loads a value into memory without persisting it to storage.
    ///
    /// Used during hot-reload to sync in-memory state with the file on disk.
    /// Unlike [`set_value`](Self::set_value), this never writes to the
    /// preferences backend, avoiding write-back loops with the file watcher.
    fn load_value(
        &mut self,
        new_value: Self::Value,
        explicitly_set: bool,
        ctx: &mut ModelContext<Self::Group>,
    ) -> anyhow::Result<()>;

    /// Sets the value of the setting persisting it to storage. The change event indicates
    /// that the update was initiated from a cloud sync.
    fn set_value_from_cloud_sync(
        &mut self,
        new_value: Self::Value,
        ctx: &mut warpui::ModelContext<Self::Group>,
    ) -> anyhow::Result<()>;

    /// Sets the value of the setting persisting it to storage.
    fn set_value(
        &mut self,
        new_value: Self::Value,
        ctx: &mut ModelContext<Self::Group>,
    ) -> Result<()>;

    /// Returns the default value of the setting.
    fn default_value() -> Self::Value;

    /// Sets the value of the setting to its default and persists it to storage.
    fn set_value_to_default(
        &mut self,
        ctx: &mut warpui::ModelContext<Self::Group>,
    ) -> anyhow::Result<()> {
        self.set_value(Self::default_value(), ctx)
    }

    /// Returns the appropriate preferences backend for this setting.
    ///
    /// Private settings use the platform-native store; public settings use
    /// the main preferences backend (which may be the TOML settings file).
    fn preferences_for_setting(ctx: &AppContext) -> &dyn UserPreferences {
        use warpui::SingletonEntity;

        if Self::is_private() {
            <PrivatePreferences as SingletonEntity>::as_ref(ctx).deref()
        } else if FeatureFlag::SettingsFile.is_enabled() {
            <PublicPreferences as SingletonEntity>::as_ref(ctx).as_preferences()
        } else {
            // When the settings file is disabled, fall back to the private
            // backend so both paths share a single instance.
            <PrivatePreferences as SingletonEntity>::as_ref(ctx).deref()
        }
    }

    /// Constructs a new instance of the setting, populating its initial value
    /// based on any previously-stored value, falling back to
    /// `Self::default_value()` if no value was stored or it could not be parsed
    /// successfully.
    fn new_from_storage(ctx: &mut AppContext) -> Self
    where
        Self: Sized,
    {
        Self::new(Self::read_from_preferences(Self::preferences_for_setting(
            ctx,
        )))
    }

    /// Reads the setting's value from the provided preferences, returning None
    /// if the value is not set or could not be parsed successfully.
    fn read_from_preferences(preferences: &dyn UserPreferences) -> Option<Self::Value> {
        let key = if preferences.is_settings_file() {
            Self::toml_key()
        } else {
            Self::storage_key()
        };
        let value = preferences
            .read_value_with_hierarchy(key, Self::hierarchy())
            .unwrap_or_default()?;

        // For the settings file, use the SettingsValue trait.
        if preferences.is_settings_file() {
            let json_value = match serde_json::from_str::<serde_json::Value>(&value) {
                Ok(v) => v,
                Err(err) => {
                    log::error!(
                        "Failed to parse JSON for setting {}: {err:?}",
                        Self::storage_key()
                    );
                    preferences.inhibit_writes_for_key(Self::toml_key(), Self::hierarchy());
                    return None;
                }
            };
            match <Self::Value as SettingsValue>::from_file_value(&json_value) {
                Some(val) => {
                    log::debug!(
                        "Loaded {} from settings file; value: {:?}",
                        Self::setting_name(),
                        val
                    );
                    return Some(val);
                }
                None => {
                    log::error!(
                        "Failed to parse file value for setting {}",
                        Self::storage_key()
                    );
                    preferences.inhibit_writes_for_key(Self::toml_key(), Self::hierarchy());
                    return None;
                }
            }
        }

        match serde_json::from_str(&value) {
            Ok(val) => {
                log::debug!(
                    "Loaded {} from user defaults; value: {:?}",
                    Self::setting_name(),
                    val
                );
                Some(val)
            }
            Err(err) => {
                log::error!(
                    "Failed to parse stored value for setting {}: {err:?}",
                    Self::storage_key()
                );
                None
            }
        }
    }

    /// Persists the current value of the setting in some form of durable
    /// storage. Returns whether the value was changed from what was currently
    /// stored.
    fn write_to_preferences(
        new_value: &Self::Value,
        preferences: &dyn UserPreferences,
    ) -> Result<bool> {
        let key = if preferences.is_settings_file() {
            Self::toml_key()
        } else {
            Self::storage_key()
        };

        // For the settings file, use the SettingsValue trait.
        let value = if preferences.is_settings_file() {
            let file_value = <Self::Value as SettingsValue>::to_file_value(new_value);
            serde_json::to_string(&file_value).context(format!(
                "Failed to write {} to storage",
                Self::storage_key()
            ))?
        } else {
            serde_json::to_string(new_value).context(format!(
                "Failed to write {} to storage",
                Self::storage_key()
            ))?
        };

        // Compare semantically by deserializing the stored value back into
        // the typed value rather than comparing JSON strings. This avoids
        // spurious writes caused by serialization differences (key ordering,
        // null-vs-missing fields, formatting) that don't represent actual
        // value changes.
        let stored_value_matches = preferences
            .read_value_with_hierarchy(key, Self::hierarchy())?
            .as_deref()
            .and_then(|stored| {
                if preferences.is_settings_file() {
                    let json_value = serde_json::from_str::<serde_json::Value>(stored).ok()?;
                    return <Self::Value as SettingsValue>::from_file_value(&json_value);
                }
                serde_json::from_str::<Self::Value>(stored).ok()
            })
            .is_some_and(|stored_val| &stored_val == new_value);

        if !stored_value_matches {
            log::debug!(
                "Writing new value of {} to storage; key: {}; value: {:?}",
                Self::setting_name(),
                key,
                value
            );
            let _ = preferences.write_value_with_hierarchy(
                key,
                value,
                Self::hierarchy(),
                Self::max_table_depth(),
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Clears the setting from the given durable storage and returns whether the setting was cleared.
    fn clear_from_preferences(preferences: &dyn UserPreferences) -> Result<()> {
        let key = if preferences.is_settings_file() {
            Self::toml_key()
        } else {
            Self::storage_key()
        };
        log::debug!(
            "Clearing setting {} with key {} from preferences",
            Self::setting_name(),
            key,
        );
        preferences.remove_value_with_hierarchy(key, Self::hierarchy())?;
        Ok(())
    }

    /// Returns true if this setting is supported on the current platform (e.g., Web, Linux, Mac). For example,
    /// Background opacity is supported on Mac and Linux, not Web.
    fn is_supported_on_current_platform(&self) -> bool;

    /// Returns true if this setting was explicitly set by the user (i.e., not using the default value).
    fn is_value_explicitly_set(&self) -> bool;
}

/// A trait for settings that can be toggled between two values.
pub trait ToggleableSetting: Setting {
    /// Toggles the value of the setting and persists it to storage, returning
    /// the new value upon success.
    fn toggle_and_save_value(
        &mut self,
        ctx: &mut ModelContext<<Self as Setting>::Group>,
    ) -> Result<<Self as Setting>::Value>;
}

impl<T, S> ToggleableSetting for S
where
    T: std::ops::Not<Output = T> + Copy + Debug,
    S: Setting<Value = T>,
{
    fn toggle_and_save_value(
        &mut self,
        ctx: &mut ModelContext<<Self as Setting>::Group>,
    ) -> Result<<Self as Setting>::Value> {
        let current_value = *self.value();
        let new_value = !current_value;
        log::debug!(
            "Toggling value of {} from {:?} to {:?}",
            Self::setting_name(),
            current_value,
            new_value
        );
        self.set_value(new_value, ctx)?;
        Ok(new_value)
    }
}

#[cfg(test)]
#[path = "toml_path_tests.rs"]
mod toml_path_tests;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
