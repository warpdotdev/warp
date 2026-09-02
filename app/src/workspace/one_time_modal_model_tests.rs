use std::sync::Arc;

use futures::FutureExt;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity};

use super::{
    AISettings, AuthManager, AuthManagerEvent, AuthStateProvider, FACTORIES_LAUNCH_SEEN_KEY,
    FEATURE_INTROS, FeatureIntroId, FreeAiRemovalModalDecision, OneTimeModalModel,
    ServerApiProvider, free_ai_removal_modal_decision,
};
use crate::server::experiments::ServerExperiments;
use crate::server::server_api::auth::MockAuthClient;
use crate::test_util::assert_eventually;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::CustomerType;

/// Waits until the Factories launch modal's in-flight impression claim (see
/// `OneTimeModalModel::pending_factories_launch_claim`) has resolved, instead of
/// sleeping for a fixed duration and hoping the background claim task and its
/// callback have both run by then.
async fn wait_for_factories_launch_claim_to_resolve(app: &mut App) {
    assert_eventually!(
        app.read(|ctx| !OneTimeModalModel::as_ref(ctx).pending_factories_launch_claim),
        "expected the Factories launch modal claim to resolve"
    );
}

#[test]
fn wait_until_auto_handoff_sleep_modal_closed_tracks_modal_state() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                // Resolves immediately while the modal is closed.
                assert!(
                    model
                        .wait_until_auto_handoff_sleep_modal_closed()
                        .now_or_never()
                        .is_some()
                );

                // The auto-resume path creates its wait future before the
                // modal opens (e.g. while offline during sleep); it must
                // still observe the modal that opens later.
                let pending_probe = model.wait_until_auto_handoff_sleep_modal_closed();
                let resolving_waiter = model.wait_until_auto_handoff_sleep_modal_closed();

                model.set_auto_handoff_sleep_modal_open(true, ctx);

                // Pending while the modal is open, because the future reads
                // live modal state at poll time.
                assert!(pending_probe.now_or_never().is_none());

                model.mark_auto_handoff_sleep_modal_dismissed(ctx);

                // An existing waiter resolves once the modal closes.
                assert!(resolving_waiter.now_or_never().is_some());
            });
        });
    });
}

#[test]
fn test_free_ai_removal_modal_decision_matrix() {
    struct Case {
        name: &'static str,
        customer_type: Option<CustomerType>,
        is_warp_ai_enabled: bool,
        has_byok_or_byoe: bool,
        completed_new_onboarding: bool,
        has_zero_base_credits: bool,
        workspaces_fetched: bool,
        expected: FreeAiRemovalModalDecision,
    }

    let cases = [
        Case {
            name: "free user with AI enabled and no base credits sees the modal",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::Show,
        },
        Case {
            name: "free user who still receives base credits defers (ICP)",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: false,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::Defer,
        },
        Case {
            name: "free user with AI disabled is marked seen silently",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: false,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "free user with a BYO key or endpoint is marked seen silently",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: true,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "free user who completed the new onboarding is marked seen silently",
            customer_type: Some(CustomerType::Free),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: true,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "paid (Build) user is marked seen silently",
            customer_type: Some(CustomerType::Build),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "paid (BuildMax) user is marked seen silently",
            customer_type: Some(CustomerType::BuildMax),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "enterprise user is marked seen silently",
            customer_type: Some(CustomerType::Enterprise),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "legacy paid (Prosumer) user is marked seen silently",
            customer_type: Some(CustomerType::Prosumer),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
        Case {
            name: "unknown customer type defers until billing data resolves",
            customer_type: Some(CustomerType::Unknown),
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::Defer,
        },
        Case {
            name: "missing workspace defers before the first server fetch",
            customer_type: None,
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: false,
            expected: FreeAiRemovalModalDecision::Defer,
        },
        Case {
            name: "missing workspace after a server fetch with no base credits is a solo free user",
            customer_type: None,
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::Show,
        },
        Case {
            name: "solo user who still receives base credits defers (ICP)",
            customer_type: None,
            is_warp_ai_enabled: true,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: false,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::Defer,
        },
        Case {
            name: "missing workspace with AI disabled is marked seen silently",
            customer_type: None,
            is_warp_ai_enabled: false,
            has_byok_or_byoe: false,
            completed_new_onboarding: false,
            has_zero_base_credits: true,
            workspaces_fetched: true,
            expected: FreeAiRemovalModalDecision::MarkSeenSilently,
        },
    ];

    for case in cases {
        assert_eq!(
            free_ai_removal_modal_decision(
                case.customer_type,
                case.is_warp_ai_enabled,
                case.has_byok_or_byoe,
                case.completed_new_onboarding,
                case.has_zero_base_credits,
                case.workspaces_fetched,
            ),
            case.expected,
            "case failed: {}",
            case.name,
        );
    }
}

#[test]
fn feature_intro_triggers_for_unseen_feature() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let key = FeatureIntroId::CustomModelRouter.as_key();
            let window_id = ctx.window_id();
            let active_window = ctx.windows().active_window();

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));
                // Simulate the startup race where the modal queue runs before
                // on_active_window_changed has assigned a target window.
                model.target_window_id = None;

                let shown = model.check_and_trigger_feature_intro_modal(ctx);

                // The feature is marked seen up front, whether or not it is shown on
                // the current channel.
                assert!(AISettings::as_ref(ctx).is_feature_intro_seen(key));
                if shown {
                    assert_eq!(
                        model.active_feature_intro,
                        Some(FeatureIntroId::CustomModelRouter)
                    );
                    // Prefer binding to the focused window immediately. If the
                    // window manager has not yet reported an active window, the
                    // intro stays pending until `update_target_window_id`.
                    if active_window.is_some() {
                        assert_eq!(model.target_window_id, Some(window_id));
                        assert_eq!(
                            model.active_feature_intro(),
                            Some(FeatureIntroId::CustomModelRouter)
                        );
                    } else {
                        assert_eq!(model.target_window_id, None);
                        assert_eq!(model.active_feature_intro(), None);
                    }
                }

                // It is shown at most once: a second check is a no-op.
                assert!(!model.check_and_trigger_feature_intro_modal(ctx));
            });
        });
    });
}

#[test]
fn feature_intro_becomes_visible_when_target_window_is_assigned() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                // Intro selected before any window is active (no active window
                // available to bind yet).
                model.target_window_id = None;
                model.active_feature_intro = Some(FeatureIntroId::CustomModelRouter);
                assert_eq!(model.active_feature_intro(), None);

                model.update_target_window_id(window_id, ctx);

                assert_eq!(model.target_window_id, Some(window_id));
                assert_eq!(
                    model.active_feature_intro(),
                    Some(FeatureIntroId::CustomModelRouter)
                );
            });
        });
    });
}

#[test]
fn agent_cli_launch_modal_shows_at_most_once() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::AgentCliLaunchModal.override_enabled(true);

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);

                let shown = model.check_and_trigger_agent_cli_launch_modal(ctx);

                // The seen marker is written up front, whether or not the modal
                // is shown on the current channel.
                assert!(*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);
                assert_eq!(model.is_agent_cli_launch_modal_open, shown);

                // A second check is a no-op, so the modal is never shown twice.
                assert!(!model.check_and_trigger_agent_cli_launch_modal(ctx));

                model.mark_agent_cli_launch_modal_dismissed(ctx);
                assert!(!model.is_agent_cli_launch_modal_open);
                assert!(!model.check_and_trigger_agent_cli_launch_modal(ctx));
            });
        });
    });
}

#[test]
fn agent_cli_launch_modal_pre_dismissed_for_new_users_on_auth_complete() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            // Building the model installs the AuthComplete subscription under test.
            let _model = OneTimeModalModel::handle(ctx);

            // A user who hasn't completed onboarding is a fresh signup.
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false)
            );
            assert!(!*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);

            AuthManager::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(AuthManagerEvent::AuthComplete);
            });
        });

        // Without this pre-dismissal a new signup would be shown the modal on
        // their second startup, right after onboarding.
        app.read(|ctx| {
            assert!(*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);
        });
    });
}

#[test]
fn agent_cli_launch_modal_skipped_when_flag_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::AgentCliLaunchModal.override_enabled(false);

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!model.check_and_trigger_agent_cli_launch_modal(ctx));
                // The seen marker stays untouched so the modal can still be
                // shown once the flag is turned on.
                assert!(!*AISettings::as_ref(ctx).did_check_to_trigger_agent_cli_launch_modal);
            });
        });
    });
}

#[test]
fn feature_intro_ineligible_entry_is_skipped_without_being_marked_seen() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            // CustomModelRouter's own eligibility requires AI to be enabled;
            // disabling it makes the sole `FEATURE_INTROS` entry ineligible.
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings.is_any_ai_enabled.set_value(false, ctx);
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!model.check_and_trigger_feature_intro_modal(ctx));
                assert!(
                    !AISettings::as_ref(ctx)
                        .is_feature_intro_seen(FeatureIntroId::CustomModelRouter.as_key()),
                    "an ineligible intro must not be consumed, so it can still show once eligible"
                );
            });
        });
    });
}

#[test]
fn feature_intro_skipped_when_all_seen() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                // Mirror the new-user pre-dismissal: mark every registered intro seen.
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    for intro in FEATURE_INTROS {
                        settings.mark_feature_intro_seen(intro.id.as_key(), ctx);
                    }
                });
                for intro in FEATURE_INTROS {
                    assert!(AISettings::as_ref(ctx).is_feature_intro_seen(intro.id.as_key()));
                }

                assert!(!model.check_and_trigger_feature_intro_modal(ctx));
                assert_eq!(model.active_feature_intro, None);
            });
        });
    });
}

#[test]
fn maybe_check_and_trigger_feature_intro_modal_respects_guards() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let key = FeatureIntroId::CustomModelRouter.as_key();
            let window_id = ctx.window_id();

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                // `is_any_modal_open` also requires a target window; bind one so
                // opening a modal below is actually observed as "open".
                model.update_target_window_id(window_id, ctx);

                // Before the initial modal-check pass has completed, a recheck
                // must not act on the (possibly stale) seen markers.
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));

                model.has_completed_initial_modal_checks = true;

                // While a higher-priority modal is open, a recheck is deferred.
                model.set_oz_launch_modal_open(true, ctx);
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));

                // Once it closes and both guards are satisfied, the recheck runs.
                model.set_oz_launch_modal_open(false, ctx);
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(AISettings::as_ref(ctx).is_feature_intro_seen(key));
            });
        });
    });
}

#[test]
fn higher_priority_modal_dismissal_rechecks_feature_intro() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let key = FeatureIntroId::CustomModelRouter.as_key();
            let window_id = ctx.window_id();

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                model.has_completed_initial_modal_checks = true;
                model.set_oz_launch_modal_open(true, ctx);
                assert!(model.is_any_modal_open());
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));

                // Dismissing a modal that sits ahead of feature intros in
                // `check_and_trigger_all_modals` must give them a chance to show,
                // rather than waiting for the next full app-level check.
                model.mark_oz_launch_modal_dismissed(ctx);

                assert!(!model.is_oz_launch_modal_open);
                assert!(AISettings::as_ref(ctx).is_feature_intro_seen(key));
            });
        });
    });
}

#[test]
fn factories_launch_modal_requires_validated_cta_url() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);

            // No server-configured CTA URL yet: the eligibility check must fail
            // closed rather than fall back to the generic Contact Sales link.
            assert!(!UserWorkspaces::as_ref(ctx).has_validated_factories_launch_modal_cta_url());
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!model.check_and_trigger_factories_launch_modal(ctx));
            });

            // A weak byte-inequality check would accept any of these; a real
            // safety gate in front of `ctx.open_url` must not.
            let malformed_or_fallback = [
                "",
                "   ",
                "not a url",
                "http://cal.com/warp",
                "javascript:alert(1)",
                "/relative/path",
                "https://www.warp.dev/contact-sales",
                "https://www.warp.dev/contact-sales/",
                "  https://www.warp.dev/contact-sales  ",
            ];
            for value in malformed_or_fallback {
                UserWorkspaces::handle(ctx).update(ctx, |workspaces, _ctx| {
                    workspaces.set_factories_launch_modal_cta_url(Some(value.to_string()));
                });
                assert!(
                    !UserWorkspaces::as_ref(ctx).has_validated_factories_launch_modal_cta_url(),
                    "expected {value:?} to be rejected"
                );
                OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                    assert!(
                        !model.check_and_trigger_factories_launch_modal(ctx),
                        "expected {value:?} to be ineligible"
                    );
                });
            }
        });
    });
}

/// Configures a validated CTA URL so the Factories launch modal is eligible.
/// Callers must separately hold `FeatureFlag::FactoriesLaunchModal.override_enabled(true)`
/// for the duration of the test.
fn prepare_factories_launch_eligible(ctx: &mut warpui::AppContext) {
    UserWorkspaces::handle(ctx).update(ctx, |workspaces, _ctx| {
        workspaces.set_factories_launch_modal_cta_url(Some("https://cal.com/warp".to_string()));
    });
}

#[test]
fn custom_model_router_requires_ai_enabled_but_factories_launch_does_not() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);
            prepare_factories_launch_eligible(ctx);
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings.is_any_ai_enabled.set_value(false, ctx);
            });

            let custom_model_router = FEATURE_INTROS
                .iter()
                .find(|intro| intro.id == FeatureIntroId::CustomModelRouter)
                .unwrap();

            // The generic AI-enabled gate now lives only on CustomModelRouter's
            // own eligibility. The Factories launch modal's dedicated trigger
            // function doesn't reference AI settings at all, so it stays
            // eligible regardless.
            assert!(!(custom_model_router.eligible)(ctx));

            let mut auth_client = MockAuthClient::new();
            auth_client
                .expect_claim_feature_intro_impression()
                .times(1)
                .return_once(|_| Ok(true));
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(model.check_and_trigger_factories_launch_modal(ctx));
            });
        });
    });
}

#[test]
fn factories_launch_modal_requires_winning_the_server_claim() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);

        terminal.update(&mut app, |_, ctx| {
            prepare_factories_launch_eligible(ctx);

            // Simulate another device having already claimed the impression.
            let mut auth_client = MockAuthClient::new();
            auth_client
                .expect_claim_feature_intro_impression()
                .withf(|intro_key| intro_key == FACTORIES_LAUNCH_SEEN_KEY)
                .times(1)
                .return_once(|_| Ok(false));
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                let handled = model.check_and_trigger_factories_launch_modal(ctx);

                // The seen marker must NOT be written until the claim resolves:
                // writing it eagerly (and then treating a lost/failed claim as
                // "shown") would burn the user's only impression before we even
                // know whether another device actually won it.
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY));
                assert!(handled);
                // The modal must not open until the claim resolves.
                assert!(!model.is_factories_launch_modal_open());
            });
        });

        // Let the spawned claim future resolve.
        wait_for_factories_launch_claim_to_resolve(&mut app).await;

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                assert!(
                    !model.is_factories_launch_modal_open(),
                    "a lost claim must not show the modal on this device"
                );
                assert!(
                    AISettings::as_ref(ctx).is_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY),
                    "a genuinely lost claim (another device won) is now known, so it's safe \
                     to mark seen"
                );
                assert!(
                    !model.pending_factories_launch_claim,
                    "the in-flight guard must clear once the claim resolves"
                );
            });
        });
    });
}

#[test]
fn factories_launch_modal_network_error_does_not_burn_the_impression() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);

        terminal.update(&mut app, |_, ctx| {
            prepare_factories_launch_eligible(ctx);

            // Simulate an offline device / transient server error.
            let mut auth_client = MockAuthClient::new();
            auth_client
                .expect_claim_feature_intro_impression()
                .withf(|intro_key| intro_key == FACTORIES_LAUNCH_SEEN_KEY)
                .times(1)
                .return_once(|_| Err(anyhow::anyhow!("network unreachable")));
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(model.check_and_trigger_factories_launch_modal(ctx));
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY));
            });
        });

        wait_for_factories_launch_claim_to_resolve(&mut app).await;

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                assert!(
                    !model.is_factories_launch_modal_open(),
                    "a failed claim must not show the modal on this device"
                );
                // The critical assertion: a request error must NOT burn the
                // user's only (globally-synced) impression. Being offline or
                // hitting a transient failure must leave the intro eligible
                // to retry, not permanently suppress it on every device.
                assert!(
                    !AISettings::as_ref(ctx).is_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY),
                    "a request error must leave the intro unseen so it can retry"
                );
                assert!(!model.pending_factories_launch_claim);

                // A later recheck (e.g. once connectivity resumes) must retry
                // the claim rather than treating the intro as already handled.
                let mut retry_client = MockAuthClient::new();
                retry_client
                    .expect_claim_feature_intro_impression()
                    .withf(|intro_key| intro_key == FACTORIES_LAUNCH_SEEN_KEY)
                    .times(1)
                    .return_once(|_| Ok(true));
                ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                    provider.set_auth_client_for_test(Arc::new(retry_client));
                });
                assert!(model.check_and_trigger_factories_launch_modal(ctx));
            });
        });

        wait_for_factories_launch_claim_to_resolve(&mut app).await;

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                assert!(
                    model.is_factories_launch_modal_open(),
                    "the retry should succeed once connectivity is restored"
                );
                assert!(AISettings::as_ref(ctx).is_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY));
            });
        });
    });
}

/// Users who already saw the old, non-blocking `FeatureIntroModal` popover
/// version of this announcement have `"factories_launch"` persisted in
/// `seen_feature_intro_ids` (the exact key this modal's dedicated code path
/// still reads via `FACTORIES_LAUNCH_SEEN_KEY`). This writes the literal
/// string rather than referencing that constant, so the test still catches
/// a future accidental rename of the constant silently breaking the
/// continuity guarantee it exists to protect: nobody who saw the popover
/// should ever see the modal too.
#[test]
fn factories_launch_modal_suppressed_by_legacy_popover_seen_marker() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);

        terminal.update(&mut app, |_, ctx| {
            prepare_factories_launch_eligible(ctx);
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.mark_feature_intro_seen("factories_launch", ctx);
            });

            // A MockAuthClient with no configured expectations panics if called,
            // so this also proves no claim is even attempted.
            let auth_client = MockAuthClient::new();
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(
                    !model.check_and_trigger_factories_launch_modal(ctx),
                    "a user who already saw the popover must not be re-announced via the modal"
                );
                assert!(!model.is_factories_launch_modal_open());
                assert!(!model.pending_factories_launch_claim);
            });
        });
    });
}

/// Regression test: while the Factories launch modal's impression claim is
/// in flight, its slot must be reserved so a competing recheck (e.g. from an
/// `AIRequestUsageModel` or `ExperimentsUpdated` event) cannot open a
/// different modal underneath a winning claim. See
/// `OneTimeModalModel::pending_factories_launch_claim`.
#[test]
fn factories_launch_claim_in_flight_reserves_the_modal_slot() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);
        let key = FeatureIntroId::CustomModelRouter.as_key();

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            prepare_factories_launch_eligible(ctx);

            let mut auth_client = MockAuthClient::new();
            auth_client
                .expect_claim_feature_intro_impression()
                .withf(|intro_key| intro_key == FACTORIES_LAUNCH_SEEN_KEY)
                .times(1)
                .return_once(|_| Ok(true));
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                model.has_completed_initial_modal_checks = true;

                assert!(model.check_and_trigger_factories_launch_modal(ctx));
                assert!(
                    model.pending_factories_launch_claim,
                    "the claim should still be in flight"
                );
                assert!(
                    model.is_any_modal_open(),
                    "an in-flight claim must reserve the modal slot"
                );

                // A competing recheck firing while the claim is in flight must not
                // open a different modal into the reserved slot.
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(
                    !AISettings::as_ref(ctx).is_feature_intro_seen(key),
                    "a competing feature intro must not be consumed while the Factories \
                     claim is pending"
                );
                assert!(model.active_feature_intro().is_none());
            });
        });

        wait_for_factories_launch_claim_to_resolve(&mut app).await;

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                assert!(
                    model.is_factories_launch_modal_open(),
                    "the winning claim should open the modal now that it's no longer pending"
                );

                // The reserved slot is now occupied by the actual modal, so a
                // competing recheck still must not sneak a different modal in.
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));

                // Once Factories is dismissed, the slot frees up and the intro
                // that was deferred throughout can finally show.
                model.mark_factories_launch_modal_dismissed(ctx);
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(AISettings::as_ref(ctx).is_feature_intro_seen(key));
            });
        });
    });
}

/// Regression test: a stalled claim must not permanently suppress the entire
/// one-time-modal queue. Uses a claim future that never resolves (bypassing
/// `MockAuthClient`, whose mocked `async fn` always resolves synchronously and so
/// cannot simulate a hang) together with a very short timeout, so the test proves
/// the timeout path itself — not just that a real 15-second wait eventually clears
/// the flag.
#[test]
fn factories_launch_claim_timeout_releases_the_reservation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let key = FeatureIntroId::CustomModelRouter.as_key();

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                model.has_completed_initial_modal_checks = true;

                model.claim_and_show_factories_launch_modal_with_claim(
                    futures::future::pending,
                    std::time::Duration::from_millis(20),
                    ctx,
                );
                assert!(model.pending_factories_launch_claim);
                assert!(
                    model.is_any_modal_open(),
                    "the in-flight claim must reserve the modal slot"
                );

                // A competing recheck while the claim is stalled must still be
                // suppressed, exactly as it would be for a claim in flight for any
                // other reason.
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));
            });
        });

        // Let the 20ms timeout elapse and the abortable future's callback run.
        wait_for_factories_launch_claim_to_resolve(&mut app).await;

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                assert!(
                    !model.pending_factories_launch_claim,
                    "a timed-out claim must release the reservation"
                );
                assert!(
                    !model.is_factories_launch_modal_open(),
                    "a timed-out claim must not show the modal on this device"
                );
                assert!(
                    !model.is_any_modal_open(),
                    "the slot must not remain reserved once the claim has timed out"
                );
                // The seen marker must not be burned by a request that never
                // actually resolved either way.
                assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY));

                // With the reservation released, a modal that was deferred while the
                // claim was pending can now run.
                model.maybe_check_and_trigger_feature_intro_modal(ctx);
                assert!(
                    AISettings::as_ref(ctx).is_feature_intro_seen(key),
                    "another eligible modal must be able to run once the reservation clears"
                );
            });
        });
    });
}

#[test]
fn factories_launch_check_reruns_after_feature_intro_dismissal() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            prepare_factories_launch_eligible(ctx);
            // Isolate this test from the free-AI-removal check that also runs
            // in `resume_modal_checks_after_feature_intro`, ahead of Factories.
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings
                    .did_check_to_trigger_free_ai_removal_modal
                    .set_value(true, ctx);
            });

            let mut auth_client = MockAuthClient::new();
            auth_client
                .expect_claim_feature_intro_impression()
                .withf(|intro_key| intro_key == FACTORIES_LAUNCH_SEEN_KEY)
                .times(1)
                .return_once(|_| Ok(true));
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                model.force_open_feature_intro(FeatureIntroId::CustomModelRouter, ctx);
                assert!(model.active_feature_intro().is_some());

                // Dismissing the feature intro must resume the queue into the
                // Factories check that sits right after it in
                // `check_and_trigger_all_modals`, not stop at feature intro.
                model.mark_feature_intro_dismissed(ctx);
                assert!(model.active_feature_intro().is_none());
                assert!(
                    !model.is_factories_launch_modal_open(),
                    "still awaiting the claim"
                );
            });
        });

        wait_for_factories_launch_claim_to_resolve(&mut app).await;

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                assert!(
                    model.is_factories_launch_modal_open(),
                    "the Factories check must have re-run once the feature intro was dismissed"
                );
            });
        });
    });
}

#[test]
fn factories_launch_claim_that_resolves_while_feature_intro_is_open_is_held_pending() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let _flag = FeatureFlag::FactoriesLaunchModal.override_enabled(true);

        terminal.update(&mut app, |_, ctx| {
            let window_id = ctx.window_id();
            prepare_factories_launch_eligible(ctx);

            let mut auth_client = MockAuthClient::new();
            auth_client
                .expect_claim_feature_intro_impression()
                .withf(|intro_key| intro_key == FACTORIES_LAUNCH_SEEN_KEY)
                .times(1)
                .return_once(|_| Ok(true));
            ServerApiProvider::handle(ctx).update(ctx, |provider, _ctx| {
                provider.set_auth_client_for_test(Arc::new(auth_client));
            });

            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.update_target_window_id(window_id, ctx);
                assert!(model.check_and_trigger_factories_launch_modal(ctx));

                // Simulate a feature intro popover opening during the claim's
                // round trip (e.g. from a fresh `ExperimentsUpdated` event).
                // Feature intros are intentionally excluded from
                // `is_any_modal_open`, which is exactly the gap this guards.
                model.force_open_feature_intro(FeatureIntroId::CustomModelRouter, ctx);
                assert!(model.active_feature_intro().is_some());
            });
        });

        // Let the spawned claim future resolve while the feature intro is open.
        wait_for_factories_launch_claim_to_resolve(&mut app).await;

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(
                    !model.is_factories_launch_modal_open(),
                    "a win that resolves while the feature intro is open must not stack on top \
                     of it"
                );
                assert!(
                    model.factories_launch_pending_display,
                    "the win must be held for display once the popover closes"
                );
                assert!(AISettings::as_ref(ctx).is_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY));

                // Dismissing the feature intro must release the held win.
                model.mark_feature_intro_dismissed(ctx);
                assert!(
                    model.is_factories_launch_modal_open(),
                    "dismissing the popover should reveal the win that was held back"
                );
                assert!(!model.factories_launch_pending_display);
            });
        });
    });
}

#[test]
fn feature_intro_recheck_on_experiments_updated() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let key = FeatureIntroId::CustomModelRouter.as_key();

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, _ctx| {
                model.has_completed_initial_modal_checks = true;
            });
        });
        app.read(|ctx| {
            assert!(!AISettings::as_ref(ctx).is_feature_intro_seen(key));
        });

        // A fresh experiments fetch (e.g. after the initial modal-check
        // pass already ran) must re-trigger the feature-intro check.
        terminal.update(&mut app, |_, ctx| {
            ServerExperiments::handle(ctx).update(ctx, |experiments, ctx| {
                experiments.apply_latest_state(vec![], ctx);
            });
        });

        app.read(|ctx| {
            assert!(AISettings::as_ref(ctx).is_feature_intro_seen(key));
        });
    });
}
