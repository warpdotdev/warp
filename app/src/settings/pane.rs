use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

define_settings_group!(PaneSettings, settings: [
    should_dim_inactive_panes: ShouldDimInactivePanes {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.panes.should_dim_inactive_panes",
        description_key: "settings.schema.appearance.panes.should_dim_inactive_panes.description",
    },
    focus_panes_on_hover: FocusPaneOnHover {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.panes.focus_pane_on_hover",
        description_key: "settings.schema.appearance.panes.focus_pane_on_hover.description",
    }
]);
