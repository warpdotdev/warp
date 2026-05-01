use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};

define_settings_group!(ShellHistorySyncSettings, settings: [
    live_sync_os_shell_history: LiveSyncOsShellHistoryEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.live_sync_os_shell_history",
        description: "When enabled, Warp watches the active shell's history file (~/.zsh_history, \
                      ~/.bash_history, fish, PSReadLine) for changes made by other terminals and \
                      merges new commands into Warp's autocomplete in real time. Off by default. \
                      Tracks GH-3422.",
    },
]);
