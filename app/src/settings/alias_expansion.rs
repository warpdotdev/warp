use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

define_settings_group!(AliasExpansionSettings, settings: [
    alias_expansion_enabled: AliasExpansionEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "terminal.input.alias_expansion_enabled",
        description: "Whether shell alias expansion is enabled in the input.",
    },
]);
