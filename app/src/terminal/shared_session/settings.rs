use std::time::Duration;

use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud};

define_settings_group!(SharedSessionSettings, settings: [
    onboarding_block_shown: SessionSharingOnboardingBlockShown {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    inactivity_period_before_ending_session: InactivityPeriodBeforeEndingSession {
        type: Duration,
        // After a total of 30 min of inactivity, we will end the session
        default: Duration::from_secs(1800),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    inactivity_period_before_warning: InactivityPeriodBeforeWarning {
        type: Duration,
        // After a total of 25 min of inactivity, we will show a warning modal
        default: Duration::from_secs(1500),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    inactivity_period_before_revoking_roles: InactivityPeriodBeforeRevokingRoles {
        type: Duration,
        // After a total of 10 min of inactivity, we will revoke all executor roles
        default: Duration::from_secs(600),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    // Killswitch: when false, the sharer ignores viewer terminal size reports.
    viewer_driven_sizing_enabled: ViewerDrivenSizingEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
]);

impl SharedSessionSettings {
    /// Returns time between showing the inactivity warning modal and ending the session.
    pub fn inactivity_period_between_warning_and_ending_session(&self) -> Duration {
        *self.inactivity_period_before_ending_session.value()
            - *self.inactivity_period_before_warning.value()
    }

    /// Returns time between revoking roles and showing the inactivity warning modal.
    pub fn inactivity_period_between_revoking_roles_and_warning(&self) -> Duration {
        *self.inactivity_period_before_warning.value()
            - *self.inactivity_period_before_revoking_roles.value()
    }
}
