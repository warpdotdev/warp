use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};
use warp_core::features::FeatureFlag;

use super::DriveSortOrder;

pub const HAS_AUTO_OPENED_WELCOME_FOLDER: &str = "HasAutoOpenedWelcomeFolder";

define_settings_group!(WarpDriveSettings, settings: [
    sorting_choice: WarpDriveSortingChoice {
        type: DriveSortOrder,
        default: DriveSortOrder::ByObjectType,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "warp_drive.sorting_choice",
        description: "The sort order for items in Warp Drive.",
    },
    sharing_onboarding_block_shown: WarpDriveSharingOnboardingBlockShown {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    // Controls whether Warp Drive appears in the tools panel, command palette, and command search.
    enable_warp_drive: EnableWarpDrive {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "warp_drive.enabled",
        description: "Whether Warp Drive is enabled.",
    },
]);

impl WarpDriveSettings {
    /// Returns whether Warp Drive should be considered enabled.
    /// Returns `false` when the user is anonymous or fully logged out,
    /// regardless of the user setting.
    pub fn is_warp_drive_enabled(app: &warpui::AppContext) -> bool {
        use warpui::SingletonEntity as _;
        let is_anonymous_or_logged_out = FeatureFlag::SkipFirebaseAnonymousUser.is_enabled()
            && crate::auth::AuthStateProvider::as_ref(app)
                .get()
                .is_anonymous_or_logged_out();
        *Self::as_ref(app).enable_warp_drive && !is_anonymous_or_logged_out
    }
}
