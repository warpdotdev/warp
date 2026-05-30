use anyhow::{Result, anyhow};
use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType},
    ids::GenericStringObjectId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use settings::SyncToCloud;

use crate::{JsonModel, JsonSerializer};

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
            Err(err) => Err(anyhow!("Failed to parse preference value {err}")),
        }
    }
}

impl JsonModel for Preference {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::Preference
    }
}

pub type CloudPreference = GenericCloudObject<GenericStringObjectId, CloudPreferenceModel>;
pub type CloudPreferenceModel = GenericStringModel<Preference, JsonSerializer>;
pub type ServerPreference = GenericServerObject<GenericStringObjectId, CloudPreferenceModel>;
