use settings::macros::define_settings_group;
use settings::{Setting, SupportedPlatforms, SyncToCloud};

// Debug mode settings.
//
// If "shell debug mode" is enabled, the `WARP_SHELL_DEBUG_MODE` environment variable is
// set in subsequently spawned terminal sessions.
//
// If `are_in_band_generators_for_all_sessions_enabled` is `true`, then all new sessions employ
// in-band generators for powering completions and syntax highlighting.
//
// If `are_in_band_generators_disabled` is `true`, then in-band generators are _never_ used to
// power completions/syntax highlighting in _any_ new session.  For sessions that have no
// alternative completions method (e.g. remote non-SSH subshells), completions and syntax
// highlighting are broken. This setting takes precedence over
// `are_in_band_generators_for_all_sessions_enabled`. This is only offered as a setting as a sort
// of 'kill-switch' for in-band generators, should a particularly bad bug appear during or shortly
// after launch.
//
// The recording mode setting can be turned on to start by using the "recording_mode" feature
// and can subsequently be turned on and off via the "Toggle Recording Mode" App->Debug mac menu.
define_settings_group!(DebugSettings, settings: [
    is_shell_debug_mode_enabled: IsShellDebugModeEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    are_in_band_generators_for_all_sessions_enabled: AreInBandGeneratorsForAllSessionsEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    force_disable_in_band_generators: ForceDisableInBandGenerators {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
        storage_key: "DisableInBandCommands",
    },
    recording_mode: RecordingModeEnabled {
        type: bool,
        default: cfg!(feature = "recording_mode"),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    show_memory_stats: ShowMemoryStats {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    }
]);

impl DebugSettings {
    pub fn should_show_memory_stats(&self) -> bool {
        // We only want to show memory stats in dogfood and not in tests.
        *self.show_memory_stats.value()
            && warp_core::channel::ChannelState::enable_debug_features()
            && !cfg!(test)
    }
}
