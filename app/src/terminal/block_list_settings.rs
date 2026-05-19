use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};

// Settings for controlling the behavior of the block list.
define_settings_group!(BlockListSettings, settings: [
   show_jump_to_bottom_of_block_button: ShowJumpToBottomOfBlockButton {
       type: bool,
       default: true,
       supported_platforms: SupportedPlatforms::ALL,
       sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
       private: false,
       toml_path: "appearance.blocks.show_jump_to_bottom_of_block_button",
       description: "Whether to show the jump-to-bottom button in long command output.",
   },
   snackbar_enabled: SnackbarEnabled {
       type: bool,
       default: true,
       supported_platforms: SupportedPlatforms::ALL,
       sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
       private: false,
       toml_path: "general.snackbar_enabled",
       description: "Whether to show snackbar notifications.",
   }
   // When enabled, the input box retains focus when selecting a block in shell mode
   // (useful for quickly attaching context). When disabled, selecting a block focuses
   // the terminal so blocklist navigation with arrow keys continues to work.
   preserve_input_focus_on_block_selection: PreserveInputFocusOnBlockSelection {
       type: bool,
       default: false,
       supported_platforms: SupportedPlatforms::ALL,
       sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
       private: false,
       toml_path: "general.preserve_input_focus_on_block_selection",
       description: "Whether to preserve input box focus when selecting a block.",
   }
   show_block_dividers: ShowBlockDividers {
       type: bool,
       default: true,
       supported_platforms: SupportedPlatforms::ALL,
       sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
       private: false,
       toml_path: "appearance.blocks.show_block_dividers",
       description: "Whether to show dividers between terminal blocks.",
   }
]);
