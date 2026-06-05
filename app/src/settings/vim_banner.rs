use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};
use warp_core::define_settings_group;

use crate::banner::BannerState;

// This isn't exactly a setting, but rather a record of a
// user action that should be persisted the same way we would a setting.
//
// When a user dismisses the Vim keybindings banner,
// we want to remember that they did so.
// That way, we skip displaying it in the future
// and prevent it from becoming an annoyance.
define_settings_group!(VimBannerSettings, settings: [
    vim_keybindings_banner_state: VimKeybindingsBannerState {
        type: BannerState,
        default: BannerState::NotDismissed,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
]);
