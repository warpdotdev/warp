use settings::{PrivatePreferences, PublicPreferences, Setting, SettingsManager};
use warp_core::features::FeatureFlag;
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::{App, AppContext, SingletonEntity};
use warpui_extras::user_preferences;

use super::*;

fn init_test_app(ctx: &mut AppContext) {
    ctx.add_singleton_model(move |_| {
        PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(move |_| -> PrivatePreferences {
        PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
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

fn write_private(ctx: &AppContext, key: &str, duration: Duration) {
    ctx.private_user_preferences()
        .write_value(key, serde_json::to_string(&duration).unwrap())
        .unwrap();
}

// ---------------------------------------------------------------------------
// Legacy private -> public migration (review finding 1)
// ---------------------------------------------------------------------------

#[test]
fn legacy_private_value_survives_migration_even_when_settings_file_marker_already_set() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        // Simulate a pre-existing user for whom the general native->TOML migration already
        // ran and recorded its completion marker, before these three settings became public.
        app.update(|ctx| {
            ctx.private_user_preferences()
                .write_value("SettingsFileMigrationComplete", "true".to_owned())
                .unwrap();
            write_private(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(900),
            );
        });

        app.update(|ctx| {
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                *SharedSessionSettings::as_ref(ctx)
                    .inactivity_period_before_revoking_roles
                    .value(),
                Duration::from_secs(900),
                "legacy private-store value should survive the flip to a public setting"
            );
        });
    });
}

#[test]
fn migration_does_not_overwrite_already_set_public_value() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        app.update(|ctx| {
            // The user already has an explicit value in the new public location...
            write_public(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(120),
            );
            // ...while a stale legacy private-store value also happens to exist.
            write_private(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(999),
            );
        });

        app.update(|ctx| {
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                *SharedSessionSettings::as_ref(ctx)
                    .inactivity_period_before_revoking_roles
                    .value(),
                Duration::from_secs(120),
                "migration must not clobber a value already explicitly set in the public location"
            );
        });
    });
}

#[test]
fn migration_is_one_time_via_its_own_marker() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        app.update(|ctx| {
            write_private(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::from_secs(900),
            );
        });

        // Register once, then run the migration explicitly (simulating a launch with a
        // pre-existing private-store value).
        app.update(|ctx| {
            SharedSessionSettings::register(ctx);
        });
        app.update(migrate_legacy_private_inactivity_settings);

        app.read(|ctx| {
            assert_eq!(
                ctx.private_user_preferences()
                    .read_value(LEGACY_INACTIVITY_SETTINGS_MIGRATED_KEY)
                    .unwrap()
                    .as_deref(),
                Some("true"),
                "migration should record its own completion marker"
            );
            assert_eq!(
                *SharedSessionSettings::as_ref(ctx)
                    .inactivity_period_before_revoking_roles
                    .value(),
                Duration::from_secs(900)
            );
        });

        // The user then explicitly clears the migrated value (e.g. removing it from their
        // settings file / resetting to default).
        app.update(|ctx| {
            SharedSessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .inactivity_period_before_revoking_roles
                    .clear_value(ctx)
                    .unwrap();
            });
        });

        // Running the migration again (simulating a second launch) must be a no-op: its own
        // marker is already set, so it must not re-copy the stale legacy value and clobber
        // the user's explicit reset.
        app.update(migrate_legacy_private_inactivity_settings);

        app.read(|ctx| {
            assert_eq!(
                *SharedSessionSettings::as_ref(ctx)
                    .inactivity_period_before_revoking_roles
                    .value(),
                InactivityPeriodBeforeRevokingRoles::default_value(),
                "migration must not re-run once its own marker is set"
            );
        });
    });
}

#[test]
fn migration_does_not_treat_a_legacy_private_zero_as_the_new_off_sentinel() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        app.update(init_test_app);

        // Before this zero-disables-a-phase increment, these settings were private with no
        // UI to request "no timeout at all" -- a legacy zero meant an immediate timer, not
        // a deliberate request to disable it. Simulate a user who has exactly that stale
        // value on disk.
        app.update(|ctx| {
            write_private(
                ctx,
                InactivityPeriodBeforeRevokingRoles::storage_key(),
                Duration::ZERO,
            );
        });

        app.update(|ctx| {
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                *SharedSessionSettings::as_ref(ctx)
                    .inactivity_period_before_revoking_roles
                    .value(),
                InactivityPeriodBeforeRevokingRoles::default_value(),
                "a legacy private-store zero must not be carried over as the new Off \
                 sentinel -- the user never asked to disable this phase, so the non-zero \
                 default should apply instead"
            );
        });
    });
}

// ---------------------------------------------------------------------------
// Ordering enforcement at the authoritative boundary (review finding 2)
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
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
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
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
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
fn zero_is_exempt_from_the_ordering_comparison() {
    // A disabled (zero) phase in either position never violates ordering.
    assert!(SharedSessionSettings::ladder_phase_order_ok(
        Duration::ZERO,
        SECS_10
    ));
    assert!(SharedSessionSettings::ladder_phase_order_ok(
        SECS_10,
        Duration::ZERO
    ));
    assert!(SharedSessionSettings::ladder_phase_order_ok(
        Duration::ZERO,
        Duration::ZERO
    ));
    // Ordinary non-zero comparisons are unaffected.
    assert!(SharedSessionSettings::ladder_phase_order_ok(
        SECS_10, SECS_25
    ));
    assert!(!SharedSessionSettings::ladder_phase_order_ok(
        SECS_25, SECS_10
    ));
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
            SharedSessionSettings::register_and_enforce_inactivity_ordering(ctx);
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
fn derived_intervals_never_panic_on_out_of_order_values() {
    // Directly construct an inconsistent group (bypassing the ordering enforcement entirely)
    // to prove the derived-interval helpers are defensive regardless of how a bad ordering
    // arises, not just against the paths this change actively guards.
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
