use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

define_settings_group!(ScrollSettings, settings: [
    mouse_scroll_multiplier: MouseScrollMultiplier {
        type: f32,
        default: 3.0,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "general.mouse_scroll_multiplier",
        description: "The scroll speed multiplier for mouse scroll events.",
    },
    smooth_scrolling: SmoothScrolling {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "general.smooth_scrolling",
        description: "Animate mouse-wheel scrolling instead of jumping line-by-line.",
    },
]);
