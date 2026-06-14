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
        description: "Whether inactive panes are visually dimmed.",
    },
    inactive_pane_dim_strength: InactivePaneDimStrength {
        type: u8,
        default: 35,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.panes.inactive_pane_dim_strength",
        description: "How strongly inactive panes are dimmed, from 5 to 80 percent.",
    },
    focus_panes_on_hover: FocusPaneOnHover {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.panes.focus_pane_on_hover",
        description: "Whether panes are focused when hovered over.",
    }
]);

impl InactivePaneDimStrength {
    /// Minimum dim strength, expressed as a percentage. Kept above zero so the slider
    /// always produces a visible effect when dimming is enabled.
    pub const MIN: u8 = 5;
    /// Maximum dim strength, expressed as a percentage. Capped below full opacity so the
    /// inactive pane's contents remain at least faintly visible.
    pub const MAX: u8 = 80;

    fn validate(&self, new_value: u8) -> u8 {
        new_value.clamp(Self::MIN, Self::MAX)
    }
}
