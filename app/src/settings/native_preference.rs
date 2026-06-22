use serde::{Deserialize, Serialize};
use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Preference for using the native desktop app or the web app.",
    rename_all = "snake_case"
)]
pub enum UserNativePreference {
    #[default]
    NotSelected,
    Web,
    Desktop,
}

define_settings_group!(NativePreferenceSettings, settings: [
    user_native_redirect_preference: UserNativeRedirectPreference {
        type: UserNativePreference,
        default: UserNativePreference::default(),
        supported_platforms: SupportedPlatforms::WEB,
        // Once setting sync is enabled we should sync this to the cloud
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        storage_key: "UserNativePreference",
        toml_path: "general.user_native_preference",
        description: "Whether to prefer the native desktop app or the web app.",
    },
    preference_dialog_dismissed: UserNativePreferenceDialogDismissed {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::WEB,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    },
]);
