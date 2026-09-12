use settings::{PublicPreferences, Setting, SettingsManager};
use warp_core::features::FeatureFlag;
use warpui::{App, AppContext, SingletonEntity};
use warpui_extras::user_preferences;

use super::*;

fn init_test_app(ctx: &mut AppContext) {
    ctx.add_singleton_model(move |_| {
        PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(move |_| -> settings::PrivatePreferences {
        settings::PrivatePreferences::new(
            Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
        )
    });
    ctx.add_singleton_model(|_| SettingsManager::default());
}

fn write_public(ctx: &AppContext, key: &str, duration: Duration) {
    // Any of the three (now-public) settings routes to the same PublicPreferences backend;
    // `preferences_for_setting` is the public API for reaching it from outside the
    // `settings` crate.
    InactivityPeriodBeforeRevokingRoles::preferences_for_setting(ctx)
        .write_value(key, serde_json::to_string(&duration).unwrap())
        .unwrap();
}

// ---------------------------------------------------------------------------
// Ordering enforcement (review findings: file-originated and cloud-originated triples)
// ---------------------------------------------------------------------------

#[test]
fn register_corrects_out_of_order_values_from_storage() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        // Simulate a hand-edited settings file with revoke > warn.
        app.update(|ctx| {
            write_public(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(1000),
            );
            write_public(
                ctx,
                InactivityPeriodBeforeWarning::storage_key(),
                Duration::from_secs(500),
            );
        });

        app.update(|ctx| {
            SharedSessionSettings::register(ctx);
            SharedSessionSettings::enforce_inactivity_ordering(ctx);
        });

        app.read(|ctx| {
            let settings = SharedSessionSettings::as_ref(ctx);
            let revoke = *settings.inactivity_period_before_revoking_roles.value();
            let warn = *settings.inactivity_period_before_warning.value();
            let end = *settings.inactivity_period_before_ending_session.value();
            assert!(
                revoke <= warn && warn <= end,
                "ordering must hold after loading an inconsistent file: \
                 revoke={revoke:?} warn={warn:?} end={end:?}"
            );
            assert_eq!(warn, revoke, "warn should be pulled up to revoke's value");
        });
    });
}

#[test]
fn cloud_sync_update_producing_bad_ordering_gets_corrected() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        app.update(|ctx| {
            SharedSessionSettings::register(ctx);
            SharedSessionSettings::enforce_inactivity_ordering(ctx);
        });

        // A cloud-synced update sets `end` below the current `warn` (defaults: revoke=600s,
        // warn=1500s, end=1800s).
        app.update(|ctx| {
            SharedSessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .inactivity_period_before_ending_session
                    .set_value_from_cloud_sync(Duration::from_secs(100), ctx)
                    .unwrap();
            });
        });

        app.read(|ctx| {
            let settings = SharedSessionSettings::as_ref(ctx);
            let revoke = *settings.inactivity_period_before_revoking_roles.value();
            let warn = *settings.inactivity_period_before_warning.value();
            let end = *settings.inactivity_period_before_ending_session.value();
            assert!(
                revoke <= warn && warn <= end,
                "ordering must hold after a bad cloud sync update: \
                 revoke={revoke:?} warn={warn:?} end={end:?}"
            );
            assert_eq!(
                end, warn,
                "end should be pulled back up to warn's value rather than left below it"
            );
        });
    });
}

#[test]
fn ordering_enforcement_leaves_zeros_alone() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        // A TOML file (or cloud update) disabling revoke and warning, with end enabled --
        // an internally consistent all-but-end-disabled configuration that must not be
        // "corrected" into something else just because zero is numerically the smallest
        // value.
        app.update(|ctx| {
            write_public(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::ZERO,
            );
            write_public(
                ctx,
                InactivityPeriodBeforeWarning::storage_key(),
                Duration::ZERO,
            );
            write_public(
                ctx,
                InactivityPeriodBeforeEndingSession::storage_key(),
                SECS_30,
            );
        });

        app.update(|ctx| {
            SharedSessionSettings::register(ctx);
            SharedSessionSettings::enforce_inactivity_ordering(ctx);
        });

        app.read(|ctx| {
            let settings = SharedSessionSettings::as_ref(ctx);
            assert_eq!(
                *settings.inactivity_period_before_revoking_roles.value(),
                Duration::ZERO
            );
            assert_eq!(
                *settings.inactivity_period_before_warning.value(),
                Duration::ZERO
            );
            assert_eq!(
                *settings.inactivity_period_before_ending_session.value(),
                SECS_30,
                "end must be left untouched -- a disabled revoke/warning is not a bound on it"
            );
        });
    });
}

#[test]
fn zero_middle_phase_does_not_let_its_two_enabled_neighbors_skip_comparison() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        // revoke=10m, warning disabled, end=5m: the disabled middle phase must not let
        // revoke and end skip being compared against each other.
        app.update(|ctx| {
            write_public(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(600),
            );
            write_public(
                ctx,
                InactivityPeriodBeforeWarning::storage_key(),
                Duration::ZERO,
            );
            write_public(
                ctx,
                InactivityPeriodBeforeEndingSession::storage_key(),
                Duration::from_secs(300),
            );
        });

        app.update(|ctx| {
            SharedSessionSettings::register(ctx);
            SharedSessionSettings::enforce_inactivity_ordering(ctx);
        });

        app.read(|ctx| {
            let settings = SharedSessionSettings::as_ref(ctx);
            let revoke = *settings.inactivity_period_before_revoking_roles.value();
            let warn = *settings.inactivity_period_before_warning.value();
            let end = *settings.inactivity_period_before_ending_session.value();
            assert_eq!(
                warn,
                Duration::ZERO,
                "the disabled warning phase must stay disabled"
            );
            assert!(
                revoke <= end,
                "end must be pulled up to at least revoke's value even with the disabled \
                 warning phase in between: revoke={revoke:?} end={end:?}"
            );
            assert_eq!(end, revoke, "end should be pulled up to revoke's value");
        });
    });
}

// ---------------------------------------------------------------------------
// Zero-disables-a-phase matrix (APP-5313 follow-up)
// ---------------------------------------------------------------------------

/// Builds a `SharedSessionSettings` group directly with the given (revoke, warn, end)
/// durations, bypassing storage/registration entirely so the ladder-gating matrix can be
/// tested as pure logic.
fn settings_with(revoke: Duration, warn: Duration, end: Duration) -> SharedSessionSettings {
    SharedSessionSettings {
        onboarding_block_shown: SessionSharingOnboardingBlockShown::new(None),
        inactivity_period_before_ending_session: InactivityPeriodBeforeEndingSession::new(Some(
            end,
        )),
        inactivity_period_before_warning: InactivityPeriodBeforeWarning::new(Some(warn)),
        inactivity_period_before_revoking_roles: InactivityPeriodBeforeRevokingRoles::new(Some(
            revoke,
        )),
        viewer_driven_sizing_enabled: ViewerDrivenSizingEnabled::new(None),
    }
}

const SECS_10: Duration = Duration::from_secs(10);
const SECS_25: Duration = Duration::from_secs(25);
const SECS_30: Duration = Duration::from_secs(30);

#[test]
fn all_zero_arms_nothing() {
    let settings = settings_with(Duration::ZERO, Duration::ZERO, Duration::ZERO);
    assert_eq!(settings.next_inactivity_phase(), None);
}

#[test]
fn full_ladder_unaffected_when_nothing_is_zero() {
    let settings = settings_with(SECS_10, SECS_25, SECS_30);
    assert_eq!(
        settings.next_inactivity_phase(),
        Some((InactivityPhase::RevokeEditorRoles, SECS_10))
    );
    assert_eq!(
        settings.next_phase_after_revoke(),
        Some((InactivityPhase::ShowWarning, SECS_25 - SECS_10))
    );
}

#[test]
fn revoke_disabled_jumps_straight_to_warning() {
    let settings = settings_with(Duration::ZERO, SECS_25, SECS_30);
    assert_eq!(
        settings.next_inactivity_phase(),
        Some((InactivityPhase::ShowWarning, SECS_25)),
        "with revoke off, the first armed phase should be the full warning duration, not an \
         offset from a skipped revoke"
    );
}

#[test]
fn revoke_and_warning_disabled_jumps_straight_to_end() {
    let settings = settings_with(Duration::ZERO, Duration::ZERO, SECS_30);
    assert_eq!(
        settings.next_inactivity_phase(),
        Some((InactivityPhase::EndSession, SECS_30))
    );
}

#[test]
fn end_disabled_folds_the_warning_phase_off_too() {
    // Warn has a non-zero value of its own, but end=0 means there's nothing to warn about.
    let settings = settings_with(SECS_10, SECS_25, Duration::ZERO);
    assert!(!settings.is_warning_phase_enabled());
    assert_eq!(
        settings.next_phase_after_revoke(),
        None,
        "warning is disabled (end=0) and end is disabled, so nothing should arm after revoke"
    );
}

#[test]
fn revoke_only_enabled_stays_read_only_indefinitely() {
    // revoke on, warning and end both off: after revoking, nothing further should arm.
    let settings = settings_with(SECS_10, Duration::ZERO, Duration::ZERO);
    assert_eq!(
        settings.next_inactivity_phase(),
        Some((InactivityPhase::RevokeEditorRoles, SECS_10))
    );
    assert_eq!(settings.next_phase_after_revoke(), None);
}

#[test]
fn revoke_enabled_with_only_end_enabled_skips_the_warning() {
    let settings = settings_with(SECS_10, Duration::ZERO, SECS_30);
    assert_eq!(
        settings.next_phase_after_revoke(),
        Some((InactivityPhase::EndSession, SECS_30 - SECS_10)),
        "warning is disabled by its own zero value, so end should arm directly after revoke"
    );
}

#[test]
fn derived_intervals_never_panic_on_out_of_order_values() {
    // Directly construct an inconsistent group (bypassing ordering enforcement, which
    // operates on a registered model, not a plain value) to prove the derived-interval
    // helpers are defensive regardless of how a bad ordering arises.
    let settings = SharedSessionSettings {
        onboarding_block_shown: SessionSharingOnboardingBlockShown::new(None),
        inactivity_period_before_ending_session: InactivityPeriodBeforeEndingSession::new(Some(
            Duration::from_secs(10),
        )),
        inactivity_period_before_warning: InactivityPeriodBeforeWarning::new(Some(
            Duration::from_secs(500),
        )),
        inactivity_period_before_revoking_roles: InactivityPeriodBeforeRevokingRoles::new(Some(
            Duration::from_secs(600),
        )),
        viewer_driven_sizing_enabled: ViewerDrivenSizingEnabled::new(None),
    };

    assert_eq!(
        settings.inactivity_period_between_warning_and_ending_session(),
        Duration::ZERO
    );
    assert_eq!(
        settings.inactivity_period_between_revoking_roles_and_warning(),
        Duration::ZERO
    );
}
