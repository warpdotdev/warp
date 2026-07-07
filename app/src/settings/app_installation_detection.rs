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
    description = "Whether the desktop app installation has been detected.",
    rename_all = "snake_case"
)]
pub enum UserAppInstallStatus {
    #[default]
    NotDetected,
    Detected,
}

define_settings_group!(UserAppInstallDetectionSettings, settings: [
    user_app_installation_detected: UserAppInstallationDetected {
        type: UserAppInstallStatus,
        default: UserAppInstallStatus::default(),
        supported_platforms: SupportedPlatforms::WEB,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
        storage_key: "UserAppInstallStatus",
    }
]);
