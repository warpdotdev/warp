use crate::{
    cloud_object::{
        model::{generic_string_model::StringModel, json_model::JsonModel},
        GenericStringObjectFormat, GenericStringObjectUniqueKey, JsonObjectType, Revision,
        UniquePer,
    },
    server::sync_queue::QueueItem,
};
pub use cloud_object_models::{CloudPreference, CloudPreferenceModel, Platform, Preference};

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
