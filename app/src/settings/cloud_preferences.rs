use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    cloud_object::{
        model::{
            generic_string_model::{GenericStringModel, GenericStringObjectId, StringModel},
            json_model::{JsonModel, JsonSerializer},
        },
        GenericCloudObject, GenericStringObjectFormat, GenericStringObjectUniqueKey,
        JsonObjectType, Revision, UniquePer,
    },
    server::sync_queue::QueueItem,
};

use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};
define_settings_group!(CloudPreferencesSettings, settings: [
   settings_sync_enabled: IsSettingsSyncEnabled {
       type: bool,
       default: false,
       supported_platforms: SupportedPlatforms::ALL,
       sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
       private: false,
       toml_path: "account.is_settings_sync_enabled",
       description: "Whether settings are synced across devices via the cloud.",
   },
]);

pub type CloudPreference = GenericCloudObject<GenericStringObjectId, CloudPreferenceModel>;
pub type CloudPreferenceModel = GenericStringModel<Preference, JsonSerializer>;

/// Defines the platform that a preference was set on.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Platform {
    Mac,
    Linux,
    Windows,
    Web,

    /// This implies the preference applies on all supported platforms
    Global,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mac => write!(f, "Mac"),
            Self::Linux => write!(f, "Linux"),
            Self::Windows => write!(f, "Windows"),
            Self::Web => write!(f, "Web"),
            Self::Global => write!(f, "Global"),
        }
    }
}

impl Platform {
    pub fn applies_to_current_platform(&self) -> bool {
        *self == Platform::current_platform() || *self == Platform::Global
    }
}

impl Platform {
    pub fn current_platform() -> Self {
        if cfg!(all(not(target_family = "wasm"), target_os = "macos")) {
            return Self::Mac;
        }

        if cfg!(all(
            not(target_family = "wasm"),
            any(target_os = "linux", target_os = "freebsd")
        )) {
            return Self::Linux;
        }

        if cfg!(all(not(target_family = "wasm"), target_os = "windows")) {
            return Self::Windows;
        }
        if cfg!(target_family = "wasm") {
            return Self::Web;
        }
        panic!("Unsupported platform");
    }
}

/// Defines the data model for a cloud synced user preference.
///
/// The expected usage is that each storage key is modeled as its own cloud preference object.
/// This allows users to edit individual cloud preferences with less fear of an offline
/// collision (e.g. if I change one preference on one machine and then update another while
/// offline on another machine, modeling them individually allows for both changes to be applied).
///
/// Note that I considered adding a concept of "preference group" as a higher level namespace
/// for preferences (in case users want to create groups of them), but decided to hold off on
/// this until we actually support that feature.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Preference {
    /// The storage key (unique identifier for this preference).
    pub storage_key: String,

    /// The value of the preference, which can be any JSON value.
    pub value: Value,

    /// The platform that this preference was set on.
    /// If the preference is global, this will be set to Platform::Global.
    pub platform: Platform,
}

impl Preference {
    /// Creates a new preference object with the given storage key and value and the appropriate
    /// platform key for the given syncing mode.
    /// Used when creating a new preference the first time.  For preferences synced from the
    /// cloud they will desererialize directly from JSON.
    pub fn new(storage_key: String, value: &str, syncing_mode: SyncToCloud) -> Result<Self> {
        let platform = match syncing_mode {
            SyncToCloud::PerPlatform(_) => Platform::current_platform(),
            SyncToCloud::Globally(_) => Platform::Global,
            SyncToCloud::Never => Err(anyhow!(
                "Cannot create a preference with SyncToCloud::Never"
            ))?,
        };
        match serde_json::from_str(value) {
            Ok(value) => Ok(Self {
                storage_key,
                value,
                platform,
            }),
            Err(err) => Err(anyhow!("Failed to parse preference value {}", err)),
        }
    }
}

/// Defines a based model for syncing cloud preferences.
impl StringModel for Preference {
    type CloudObjectType = CloudPreference;

    fn model_type_name(&self) -> &'static str {
        "Preference"
    }

    fn should_enforce_revisions() -> bool {
        // Last write wins for cloud prefs
        false
    }

    fn should_show_activity_toasts() -> bool {
        // No update toasts for cloud prefs
        false
    }

    fn warn_if_unsaved_at_quit() -> bool {
        // Don't block quitting on unsaved cloud prefs changes
        false
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(Self::json_object_type())
    }

    fn display_name(&self) -> String {
        self.model_type_name().to_owned()
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &CloudPreference,
    ) -> QueueItem {
        QueueItem::UpdateCloudPreferences {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn should_clear_on_unique_key_conflict(&self) -> bool {
        true
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        Some(GenericStringObjectUniqueKey {
            key: format!("{}_{}", self.platform, self.storage_key),
            unique_per: UniquePer::User,
        })
    }
}

impl JsonModel for Preference {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::Preference
    }
}
