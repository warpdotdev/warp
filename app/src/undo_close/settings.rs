use std::time::Duration;

use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

define_settings_group!(UndoCloseSettings, settings: [
    enabled: UndoCloseEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "general.undo_close.enabled",
        description_key: "settings.schema.general.undo_close.enabled.description",
    },
    grace_period: UndoCloseGracePeriod {
        type: Duration,
        default: Duration::from_secs(60),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "general.undo_close.grace_period",
        description_key: "settings.schema.general.undo_close.grace_period.description",
    },
]);
