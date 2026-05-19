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

/// Defines the data model for a cloud-synced user preference.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Preference {
    pub storage_key: String,
    pub value: Value,
    pub platform: Platform,
}

impl Preference {
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
