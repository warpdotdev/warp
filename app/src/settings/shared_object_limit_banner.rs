use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};
use warp_core::define_settings_group;

use crate::banner::BannerState;

// These aren't exactly settings, but rather a record of a user action that
// should be persisted the same way we would a setting.
//
// When a user dismisses the "you've run out of <object>s on your plan" banner
// shown in the Warp Drive sidebar, we want to remember that they did so and
// avoid showing the same prompt again. The state is tracked per object type so
// dismissing the notebook banner doesn't hide a future workflow banner.
define_settings_group!(SharedObjectLimitBannerSettings, settings: [
    notebook_limit_banner_state: NotebookLimitBannerState {
        type: BannerState,
        default: BannerState::NotDismissed,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    workflow_limit_banner_state: WorkflowLimitBannerState {
        type: BannerState,
        default: BannerState::NotDismissed,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
]);
