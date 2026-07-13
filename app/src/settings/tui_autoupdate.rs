use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

define_settings_group!(TuiAutoupdateSettings, settings: [
    // Whether the `warp-tui` background auto-updater is enabled.
    //
    // TUI-only (`surface: Tui`): the GUI has its own autoupdater and update
    // preferences, so this key only appears in (and is read from) the TUI's
    // settings file. Read once at TUI startup; the `WARP_TUI_DISABLE_AUTOUPDATE`
    // environment variable also disables updates for a single launch.
    autoupdate_enabled: TuiAutoupdateEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::TUI,
        private: false,
        toml_path: "general.autoupdate_enabled",
        description: "Whether warp-tui automatically installs updates in the background.",
    },
]);
