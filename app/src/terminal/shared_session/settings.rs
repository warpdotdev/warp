use std::time::Duration;

use settings::macros::define_settings_group;
use settings::manager::SettingsManager;
use settings::{RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud};
use warp_core::user_preferences::GetUserPreferences as _;
use warp_errors::{report_error, report_if_error};
use warpui::{AppContext, ModelHandle, SingletonEntity};

use crate::features::FeatureFlag;

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
        description: "How long a shared session can be inactive before you're warned it's about to end, in seconds.",
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
        description: "How long a shared session can be inactive before edit access is automatically revoked from everyone you're sharing with, in seconds.",
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

impl SharedSessionSettings {
    /// Returns time between showing the inactivity warning modal and ending the session.
    ///
    /// Uses `saturating_sub` as defense-in-depth: `register_and_enforce_inactivity_ordering`
    /// keeps these durations in `revoke <= warn <= end` order at every point they become
    /// authoritative (initial load, cloud sync, disk hot-reload), but a plain `Duration`
    /// subtraction still panics on underflow if that invariant is ever violated by some
    /// path this doesn't cover. Callers must only reach this once they've confirmed (via
    /// [`Self::next_inactivity_phase`] or [`Self::next_phase_after_revoke`]) that both the
    /// warning and end phases are enabled -- a zero `end` disables the warning phase
    /// entirely, so this is never a meaningful duration to compute in that case.
    pub fn inactivity_period_between_warning_and_ending_session(&self) -> Duration {
        self.inactivity_period_before_ending_session
            .value()
            .saturating_sub(*self.inactivity_period_before_warning.value())
    }

    /// Returns time between revoking roles and showing the inactivity warning modal.
    ///
    /// See [`Self::inactivity_period_between_warning_and_ending_session`] for why this
    /// uses `saturating_sub` and must only be called once the warning phase is confirmed
    /// enabled.
    pub fn inactivity_period_between_revoking_roles_and_warning(&self) -> Duration {
        self.inactivity_period_before_warning
            .value()
            .saturating_sub(*self.inactivity_period_before_revoking_roles.value())
    }

    /// Whether the warning phase is enabled: it needs both a non-zero warning duration of
    /// its own, and a non-zero end duration -- a countdown to an end that will never come
    /// would be misleading, so disabling the end phase disables the warning too.
    pub fn is_warning_phase_enabled(&self) -> bool {
        !self.inactivity_period_before_warning.value().is_zero()
            && !self
                .inactivity_period_before_ending_session
                .value()
                .is_zero()
    }

    /// Determines which phase of the inactivity ladder should be armed next, and how long
    /// from *now* (the point activity was last observed) it should fire after, skipping any
    /// disabled (zero-duration) phase. Returns `None` when every phase is disabled, meaning
    /// no idle timeout should be armed at all.
    pub fn next_inactivity_phase(&self) -> Option<(InactivityPhase, Duration)> {
        let revoke = *self.inactivity_period_before_revoking_roles.value();
        if !revoke.is_zero() {
            return Some((InactivityPhase::RevokeEditorRoles, revoke));
        }
        self.next_phase_after_revoke()
    }

    /// Determines which phase should be armed after the revoke phase has already happened
    /// (or was itself disabled), and how long from *that point* it should fire after.
    /// Returns `None` when both the warning and end phases are disabled, meaning the ladder
    /// should stop advancing (the session stays shared, permanently read-only if roles were
    /// revoked, until the sharer changes these settings or ends it explicitly).
    pub fn next_phase_after_revoke(&self) -> Option<(InactivityPhase, Duration)> {
        let revoke = *self.inactivity_period_before_revoking_roles.value();
        let end = *self.inactivity_period_before_ending_session.value();
        if self.is_warning_phase_enabled() {
            let warn = *self.inactivity_period_before_warning.value();
            return Some((InactivityPhase::ShowWarning, warn.saturating_sub(revoke)));
        }
        if !end.is_zero() {
            return Some((InactivityPhase::EndSession, end.saturating_sub(revoke)));
        }
        None
    }

    /// Registers this settings group, migrates any legacy private-store values for the
    /// inactivity durations (see [`migrate_legacy_private_inactivity_settings`]), and keeps
    /// those durations in a valid `revoke <= warn <= end` order no matter how they change:
    /// at startup (including a hand-edited settings file), via cloud sync, and via disk
    /// hot-reload.
    ///
    /// This ordering is required by the sharer inactivity ladder in
    /// `app/src/terminal/view/shared_session/view_impl.rs`, which derives the time between
    /// phases via `Duration` subtraction and would otherwise be handed an inconsistent
    /// triple whenever these settings are loaded or synced out of order (a plain settings
    /// UI edit is already clamped in `app/src/settings_view/features_page.rs`, but that
    /// clamp doesn't cover these other paths).
    pub fn register_and_enforce_inactivity_ordering(ctx: &mut AppContext) -> ModelHandle<Self> {
        let handle = Self::register(ctx);

        // Runs after `register()` so the SettingsManager already has the update functions
        // for these storage keys (`update_setting_with_storage_key` requires it).
        migrate_legacy_private_inactivity_settings(ctx);
        Self::enforce_inactivity_ordering(&handle, ctx);

        ctx.subscribe_to_model(&handle, |settings_handle, event, ctx| {
            if matches!(
                event,
                SharedSessionSettingsChangedEvent::InactivityPeriodBeforeRevokingRoles { .. }
                    | SharedSessionSettingsChangedEvent::InactivityPeriodBeforeWarning { .. }
                    | SharedSessionSettingsChangedEvent::InactivityPeriodBeforeEndingSession { .. }
            ) {
                Self::enforce_inactivity_ordering(&settings_handle, ctx);
            }
        });

        handle
    }

    /// Whether `earlier` is allowed to occur at or before `later` in the inactivity ladder.
    ///
    /// A zero duration means that phase is disabled, not "immediately" -- it isn't a point
    /// on the same numeric axis as an enabled phase, so it's exempt from the comparison in
    /// either position rather than treated as the smallest legal duration.
    fn ladder_phase_order_ok(earlier: Duration, later: Duration) -> bool {
        earlier.is_zero() || later.is_zero() || earlier <= later
    }

    /// Corrects the inactivity durations in place if they violate the required
    /// `revoke <= warn <= end` ordering, clamping an out-of-order value up to its earlier
    /// neighbor rather than rejecting the update outright.
    fn enforce_inactivity_ordering(handle: &ModelHandle<Self>, ctx: &mut AppContext) {
        let (revoke, warn, end) = handle.read(ctx, |settings, _| {
            (
                *settings.inactivity_period_before_revoking_roles.value(),
                *settings.inactivity_period_before_warning.value(),
                *settings.inactivity_period_before_ending_session.value(),
            )
        });

        let corrected_warn = if Self::ladder_phase_order_ok(revoke, warn) {
            warn
        } else {
            revoke
        };
        let corrected_end = if Self::ladder_phase_order_ok(corrected_warn, end) {
            end
        } else {
            corrected_warn
        };

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
}

/// Key written to the private (platform-native) store once the legacy private values for
/// the inactivity durations below have been migrated into their new public location.
///
/// These three settings used to be `private: true` (APP-5313); flipping them to public
/// means `new_from_storage` only reads `PublicPreferences`, so without this one-time copy,
/// an existing user's customized values would silently revert to the defaults. This marker
/// is independent of `SETTINGS_FILE_MIGRATION_COMPLETE_KEY` in `app/src/settings/init.rs`,
/// which is already set for existing `SettingsFile` users and would otherwise never revisit
/// these newly-public keys.
const LEGACY_INACTIVITY_SETTINGS_MIGRATED_KEY: &str =
    "SharedSessionInactivitySettingsMigratedFromPrivateStore";

/// One-time migration: copies each inactivity duration's legacy private-store value into
/// its new public (TOML) location, but only when the public location doesn't already have
/// a value, so it never clobbers a value the user has already set through the new UI or
/// settings file.
fn migrate_legacy_private_inactivity_settings(ctx: &mut AppContext) {
    // When the settings file feature is off, public settings fall back to the same private
    // store as before, so there's nothing to migrate.
    if !FeatureFlag::SettingsFile.is_enabled() {
        return;
    }

    let already_migrated = ctx
        .private_user_preferences()
        .read_value(LEGACY_INACTIVITY_SETTINGS_MIGRATED_KEY)
        .unwrap_or_default()
        .as_deref()
        == Some("true");
    if already_migrated {
        return;
    }

    let keys = [
        InactivityPeriodBeforeRevokingRoles::storage_key(),
        InactivityPeriodBeforeWarning::storage_key(),
        InactivityPeriodBeforeEndingSession::storage_key(),
    ];

    let values_to_migrate: Vec<(&'static str, String)> = keys
        .into_iter()
        .filter(|key| {
            matches!(
                SettingsManager::as_ref(ctx).read_local_setting_value(key, ctx),
                Ok(None)
            )
        })
        .filter_map(|key| {
            let value = ctx
                .private_user_preferences()
                .read_value(key)
                .unwrap_or_default()?;
            Some((key, value))
        })
        // Before this change, these settings were private with no UI to request "no
        // timeout at all", so a legacy zero meant an immediate timer, not "disabled" --
        // it was never a real user request for an uncapped session. Carrying it over
        // as-is would silently and permanently disable that phase for a user who never
        // asked for that. Leave the public key absent instead, so the non-zero default
        // applies; a zero already present in the *public* location is unaffected by this
        // filter (it's excluded above because the public key already has a value), since
        // that one is a real, current choice made under the new semantics.
        .filter(|(_, value)| {
            serde_json::from_str::<Duration>(value)
                .map(|duration| !duration.is_zero())
                .unwrap_or(true)
        })
        .collect();

    SettingsManager::handle(ctx).update(ctx, |manager, ctx| {
        for (key, value) in values_to_migrate {
            if let Err(err) = manager.update_setting_with_storage_key(key, value, false, ctx) {
                report_error!(
                    err.context(format!("Failed to migrate legacy inactivity setting {key}"))
                );
            }
        }
    });

    report_if_error!(
        ctx.private_user_preferences()
            .write_value(LEGACY_INACTIVITY_SETTINGS_MIGRATED_KEY, "true".to_owned())
            .map_err(|err| anyhow::anyhow!(err))
    );
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;
