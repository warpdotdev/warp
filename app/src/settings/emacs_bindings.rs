use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

use crate::banner::BannerState;

// This isn't exactly a setting, but rather a record of a
// user action that should be persisted the same way we would a setting.
//
// When a Linux user chooses Emacs bindings,
// we want to remember that they did so.
// That way, we skip displaying it in the future
// and prevent it from becoming an annoyance.
define_settings_group!(EmacsBindingsSettings, settings: [
    emacs_bindings_banner_state: EmacsBindingsBannerState {
        type: BannerState,
        default: BannerState::NotDismissed,
        supported_platforms: SupportedPlatforms::LINUX,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
]);
