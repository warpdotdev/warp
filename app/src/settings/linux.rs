use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};
use warpui::platform::linux;

define_settings_group!(LinuxAppConfiguration,
    settings: [
        force_x11: ForceX11 {
            type: bool,
            // Default to true on WSL and false on all other platforms.
            default: !linux::is_wsl(),
            supported_platforms: SupportedPlatforms::LINUX,
            sync_to_cloud: SyncToCloud::Never,
            surface: settings::SettingSurfaces::GUI,
            private: false,
            toml_path: "system.force_x11",
            description: "Whether to force X11 instead of Wayland on Linux.",
        },
    ]
);
