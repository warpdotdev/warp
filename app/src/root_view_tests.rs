use onboarding::{OfferVariant, SelectedSettings, UICustomizationSettings};
use warp_core::features::FeatureFlag;
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::{App, SingletonEntity};

use super::{
    AccountFirstCompletion, HAS_COMPLETED_ONBOARDING_KEY, RootView, has_completed_local_onboarding,
    offer_variant_for_account_class, refresh_pending_onboarding_choices,
    requires_post_onboarding_login,
};
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::server::server_api::ServerApiProvider;
use crate::workspaces::workspace::FtueAccountClass;

fn initialize_app(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

#[test]
fn account_first_class_uses_paid_status_then_fresh_request_limit() {
    assert_eq!(
        RootView::account_first_class(true, Some(0)),
        FtueAccountClass::Paid
    );
    assert_eq!(
        RootView::account_first_class(true, Some(300)),
        FtueAccountClass::Paid
    );
    assert_eq!(
        RootView::account_first_class(true, None),
        FtueAccountClass::Paid
    );
    assert_eq!(
        RootView::account_first_class(false, Some(300)),
        FtueAccountClass::FreeIcp
    );
    assert_eq!(
        RootView::account_first_class(false, Some(0)),
        FtueAccountClass::FreeStandard
    );
    assert_eq!(
        RootView::account_first_class(false, None),
        FtueAccountClass::FreeStandard
    );
}

fn set_local_onboarding_completed(app: &mut App, completed: bool) {
    app.update(|ctx| {
        ctx.private_user_preferences()
            .write_value(
                HAS_COMPLETED_ONBOARDING_KEY,
                serde_json::to_string(&completed).unwrap(),
            )
            .unwrap();
    });
}

#[test]
fn account_first_requires_login_even_without_ai_or_drive_settings() {
    let _account_first = FeatureFlag::AccountFirstOnboarding.override_enabled(true);

    assert!(requires_post_onboarding_login(false, false, false));
    assert!(!requires_post_onboarding_login(true, false, false));
}

#[test]
fn fallback_flow_only_requires_login_for_account_backed_settings() {
    let _account_first = FeatureFlag::AccountFirstOnboarding.override_enabled(false);
    let _settings_modes = FeatureFlag::OpenWarpNewSettingsModes.override_enabled(true);

    assert!(!requires_post_onboarding_login(false, false, false));
    assert!(requires_post_onboarding_login(false, true, false));
    assert!(requires_post_onboarding_login(false, false, true));
}

#[test]
fn account_first_classes_route_to_paid_or_the_expected_offer() {
    assert_eq!(
        offer_variant_for_account_class(FtueAccountClass::Paid),
        None
    );
    assert_eq!(
        offer_variant_for_account_class(FtueAccountClass::FreeIcp),
        Some(OfferVariant::HeadStart)
    );
    assert_eq!(
        offer_variant_for_account_class(FtueAccountClass::FreeStandard),
        Some(OfferVariant::ChooseHowToStart)
    );
}

#[test]
fn account_first_completion_metadata_matches_terminal_outcomes() {
    let cases = [
        (
            AccountFirstCompletion::AccountSkipped,
            "account_skipped",
            None,
            false,
        ),
        (
            AccountFirstCompletion::PaidTeam,
            "paid_team",
            Some(FtueAccountClass::Paid),
            true,
        ),
        (
            AccountFirstCompletion::FreeIcpSetupLater,
            "free_icp_setup_later",
            Some(FtueAccountClass::FreeIcp),
            true,
        ),
        (
            AccountFirstCompletion::FreeStandardSetupLater,
            "free_standard_setup_later",
            Some(FtueAccountClass::FreeStandard),
            true,
        ),
        (
            AccountFirstCompletion::UpgradeCompleted,
            "upgrade_completed",
            Some(FtueAccountClass::Paid),
            true,
        ),
    ];

    for (completion, completion_type, account_class, starts_agent_tutorial) in cases {
        assert_eq!(completion.completion_type(), completion_type);
        assert_eq!(completion.account_class(), account_class);
        assert_eq!(completion.starts_agent_tutorial(), starts_agent_tutorial);
    }
}

#[test]
fn refreshing_pending_onboarding_choices_replaces_stale_settings() {
    let settings = |use_vertical_tabs| SelectedSettings::Terminal {
        ui_customization: Some(UICustomizationSettings {
            use_vertical_tabs,
            show_conversation_history: false,
            show_project_explorer: true,
            show_global_search: false,
            show_warp_drive: false,
            show_code_review_button: true,
        }),
        cli_agent_toolbar_enabled: true,
        show_agent_notifications: false,
    };

    let mut pending_settings = Some(settings(false));
    let mut pending_tutorial = None;
    let latest_settings = settings(true);

    refresh_pending_onboarding_choices(
        &latest_settings,
        &mut pending_settings,
        &mut pending_tutorial,
    );

    let Some(SelectedSettings::Terminal {
        ui_customization: Some(ui),
        ..
    }) = pending_settings
    else {
        panic!("latest terminal settings should replace the pending snapshot");
    };
    assert!(ui.use_vertical_tabs);
    assert!(pending_tutorial.is_some());
}

/// Regression test for the bug fixed by introducing
/// `RootView::sync_local_onboarding_to_server`: when a user completed onboarding
/// pre-login and later authenticated via a non-login-slide entrypoint (i.e. while
/// already in `Terminal` state), the server-side `is_onboarded` flag was never
/// flipped. The helper runs unconditionally on `AuthComplete` and must flip the
/// flag when all preconditions hold.
#[test]
fn test_sync_flips_server_is_onboarded_when_local_onboarding_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Seed the "has_completed_local_onboarding" preference and make the user
        // appear not yet onboarded on the server. The default test user is
        // non-anonymous, so the guards in the helper won't short-circuit.
        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            assert!(has_completed_local_onboarding(ctx));
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true),
                "sync should have invoked AuthManager::set_user_onboarded"
            );
        });
    });
}

/// If the user hasn't completed local onboarding, the helper must leave the
/// server-side flag untouched — onboarding hasn't actually happened yet.
#[test]
fn test_sync_noop_when_local_onboarding_not_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Do not set HAS_COMPLETED_ONBOARDING_KEY; it defaults to false.
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false),
                "sync should not have changed is_onboarded when local onboarding is incomplete"
            );
        });
    });
}

/// The server-side flag should also be left untouched when it is already set,
/// even if local onboarding is complete — avoids redundant server calls.
#[test]
fn test_sync_noop_when_already_onboarded_on_server() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            // User::test() defaults to is_onboarded = true; assert that and
            // leave it in place.
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });
    });
}
