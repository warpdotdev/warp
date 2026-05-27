use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};
use warp_core::features::FeatureFlag;

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
    focus_panes_on_hover: FocusPaneOnHover {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.panes.focus_pane_on_hover",
        description: "Whether panes are focused when hovered over.",
    },
    pane_specific_font_size: PaneSpecificFontSize {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.panes.pane_specific_font_size",
        description: "Whether font size adjustments apply only to the focused pane.",
    }
]);

impl PaneSettings {
    /// Whether per-pane monospace font-size overrides are active. On channels where
    /// `FeatureFlag::PerPaneFontSize` is on, the feature is force-enabled (no toggle
    /// shown). On other channels, users opt in via the `pane_specific_font_size` setting.
    pub fn is_pane_specific_font_size_enabled(&self) -> bool {
        FeatureFlag::PerPaneFontSize.is_enabled() || *self.pane_specific_font_size
    }
}
