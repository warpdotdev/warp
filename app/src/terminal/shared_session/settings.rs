use std::time::Duration;

use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud};
use warp_errors::report_if_error;
use warpui::{AppContext, ModelHandle, SingletonEntity};

define_settings_group!(SharedSessionSettings, settings: [
    onboarding_block_shown: SessionSharingOnboardingBlockShown {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    inactivity_period_before_ending_session: InactivityPeriodBeforeEndingSession {
        type: Duration,
        // After a total of 30 min of inactivity, we will end the session
        default: Duration::from_secs(1800),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "session_sharing.inactivity.end_session_after_secs",
        description: "How long a shared session can be inactive before it is automatically ended, in seconds.",
    },
    inactivity_period_before_warning: InactivityPeriodBeforeWarning {
        type: Duration,
        // After a total of 25 min of inactivity, we will show a warning modal
        default: Duration::from_secs(1500),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "session_sharing.inactivity.warning_after_secs",
        description: "How long to wait before warning that a shared session will end due to inactivity, in seconds",
    },
    inactivity_period_before_revoking_roles: InactivityPeriodBeforeRevokingRoles {
        type: Duration,
        // After a total of 10 min of inactivity, we will revoke all executor roles
        default: Duration::from_secs(600),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "session_sharing.inactivity.revoke_edit_access_after_secs",
        description: "Idle period before shared sessions are made read-only",
    },
    // Killswitch: when false, the sharer ignores viewer terminal size reports.
    viewer_driven_sizing_enabled: ViewerDrivenSizingEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
]);

/// A phase of the sharer inactivity ladder, in the order it can fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InactivityPhase {
    RevokeEditorRoles,
    ShowWarning,
    EndSession,
}

/// A frozen copy of the three inactivity durations, held for one sharer idle period so
/// every phase transition within it is judged against the same durations, rather than a
/// mix of old and new ones if the settings change while a timer for that period is already
/// armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InactivityLadderSnapshot {
    pub revoke: Duration,
    pub warn: Duration,
    pub end: Duration,
}

impl InactivityLadderSnapshot {
    pub fn capture(settings: &SharedSessionSettings) -> Self {
        Self {
            revoke: *settings.inactivity_period_before_revoking_roles.value(),
            warn: *settings.inactivity_period_before_warning.value(),
            end: *settings.inactivity_period_before_ending_session.value(),
        }
    }

    /// Whether the warning phase is enabled: it needs both a non-zero warning duration of
    /// its own, and a non-zero end duration -- a countdown to an end that will never come
    /// would be misleading, so disabling the end phase disables the warning too.
    fn is_warning_phase_enabled(&self) -> bool {
        !self.warn.is_zero() && !self.end.is_zero()
    }

    /// Determines which phase of the inactivity ladder should be armed next, and how long
    /// from *now* (the point activity was last observed) it should fire after, skipping any
    /// disabled (zero-duration) phase. Returns `None` when every phase is disabled, meaning
    /// no idle timeout should be armed at all.
    pub fn next_inactivity_phase(&self) -> Option<(InactivityPhase, Duration)> {
        if !self.revoke.is_zero() {
            return Some((InactivityPhase::RevokeEditorRoles, self.revoke));
        }
        self.next_phase_after_revoke()
    }

    /// Determines which phase should be armed after the revoke phase has already happened
    /// (or was itself disabled), and how long from *that point* it should fire after.
    /// Returns `None` when both the warning and end phases are disabled, meaning the ladder
    /// should stop advancing (the session stays shared, permanently read-only if roles were
    /// revoked, until the sharer changes these settings or ends it explicitly).
    pub fn next_phase_after_revoke(&self) -> Option<(InactivityPhase, Duration)> {
        if self.is_warning_phase_enabled() {
            return Some((
                InactivityPhase::ShowWarning,
                self.warn.saturating_sub(self.revoke),
            ));
        }
        if !self.end.is_zero() {
            return Some((
                InactivityPhase::EndSession,
                self.end.saturating_sub(self.revoke),
            ));
        }
        None
    }

    /// Returns time between showing the inactivity warning modal and ending the session.
    ///
    /// Uses `saturating_sub` as defense-in-depth: these three durations are cumulative
    /// time-since-last-activity (each setting's default doc comment says "after a total of
    /// N min"), which requires `revoke <= warn <= end` for a meaningful ladder --
    /// `SharedSessionSettings::enforce_inactivity_ordering` maintains that invariant on
    /// every non-UI update path, and the settings UI's own clamping (see
    /// `app/src/settings_view/features_page.rs`) maintains it for UI edits, but
    /// `saturating_sub` still guards against a panic if either is ever bypassed. Callers
    /// must only reach this once they've confirmed (via [`Self::next_inactivity_phase`] or
    /// [`Self::next_phase_after_revoke`]) that both the warning and end phases are enabled
    /// -- a zero `end` disables the warning phase entirely, so this is never a meaningful
    /// duration to compute in that case.
    pub fn period_between_warning_and_ending_session(&self) -> Duration {
        self.end.saturating_sub(self.warn)
    }
}

impl SharedSessionSettings {
    /// See [`InactivityLadderSnapshot::period_between_warning_and_ending_session`].
    pub fn inactivity_period_between_warning_and_ending_session(&self) -> Duration {
        InactivityLadderSnapshot::capture(self).period_between_warning_and_ending_session()
    }

    /// Returns time between revoking roles and showing the inactivity warning modal.
    ///
    /// See [`Self::inactivity_period_between_warning_and_ending_session`] for why this
    /// uses `saturating_sub`.
    pub fn inactivity_period_between_revoking_roles_and_warning(&self) -> Duration {
        self.inactivity_period_before_warning
            .value()
            .saturating_sub(*self.inactivity_period_before_revoking_roles.value())
    }

    /// See [`InactivityLadderSnapshot::is_warning_phase_enabled`].
    pub fn is_warning_phase_enabled(&self) -> bool {
        InactivityLadderSnapshot::capture(self).is_warning_phase_enabled()
    }

    /// See [`InactivityLadderSnapshot::next_inactivity_phase`].
    pub fn next_inactivity_phase(&self) -> Option<(InactivityPhase, Duration)> {
        InactivityLadderSnapshot::capture(self).next_inactivity_phase()
    }

    /// See [`InactivityLadderSnapshot::next_phase_after_revoke`].
    pub fn next_phase_after_revoke(&self) -> Option<(InactivityPhase, Duration)> {
        InactivityLadderSnapshot::capture(self).next_phase_after_revoke()
    }

    /// Keeps each enabled phase at or above every earlier enabled phase, without
    /// re-enabling zero-disabled phases.
    fn advance_ladder_floor(phase: Duration, floor: &mut Duration) -> Duration {
        if phase.is_zero() {
            return phase;
        }
        let corrected = phase.max(*floor);
        *floor = corrected;
        corrected
    }

    /// Corrects the inactivity durations in place if the enabled ones among them violate
    /// the required `revoke <= warn <= end` ordering, clamping an out-of-order value up to
    /// the highest enabled value before it rather than rejecting the update outright.
    fn correct_inactivity_ordering(handle: &ModelHandle<Self>, ctx: &mut AppContext) {
        let (revoke, warn, end) = handle.read(ctx, |settings, _| {
            (
                *settings.inactivity_period_before_revoking_roles.value(),
                *settings.inactivity_period_before_warning.value(),
                *settings.inactivity_period_before_ending_session.value(),
            )
        });

        let mut floor = Duration::ZERO;
        Self::advance_ladder_floor(revoke, &mut floor);
        let corrected_warn = Self::advance_ladder_floor(warn, &mut floor);
        let corrected_end = Self::advance_ladder_floor(end, &mut floor);

        handle.clone().update(ctx, |settings, ctx| {
            if corrected_warn != warn {
                report_if_error!(
                    settings
                        .inactivity_period_before_warning
                        .set_value(corrected_warn, ctx)
                );
            }
            if corrected_end != end {
                report_if_error!(
                    settings
                        .inactivity_period_before_ending_session
                        .set_value(corrected_end, ctx)
                );
            }
        });
    }

    /// Keeps the inactivity durations in a valid `revoke <= warn <= end` order among
    /// whichever are enabled (zero means disabled and exempt), no matter how they change
    /// outside of the settings UI, which clamps its own edits separately.
    pub fn enforce_inactivity_ordering(ctx: &mut AppContext) {
        let handle = Self::handle(ctx);
        Self::correct_inactivity_ordering(&handle, ctx);

        ctx.subscribe_to_model(&handle, |settings_handle, event, ctx| {
            if matches!(
                event,
                SharedSessionSettingsChangedEvent::InactivityPeriodBeforeRevokingRoles { .. }
                    | SharedSessionSettingsChangedEvent::InactivityPeriodBeforeWarning { .. }
                    | SharedSessionSettingsChangedEvent::InactivityPeriodBeforeEndingSession { .. }
            ) {
                Self::correct_inactivity_ordering(&settings_handle, ctx);
            }
        });
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;
