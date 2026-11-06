use std::time::SystemTime;

use ai::api_keys::{ApiKeyManager, ChatGPTConnection, ChatGPTConnectionStatus};
use futures::FutureExt;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity, WindowId};

use super::{
    AISettings, AuthManager, AuthManagerEvent, AuthStateProvider, CloudPreferencesSyncer,
    FEATURE_INTROS, FeatureIntroId, FreeAiRemovalModalDecision, OneTimeModalModel,
    free_ai_removal_modal_decision, hoa_onboarding,
};
use crate::test_util::terminal::{
    add_window_with_id_and_terminal, add_window_with_terminal, initialize_app_for_terminal_view,
};
use crate::workspaces::workspace::CustomerType;

/// Registers the cloud preferences syncer on top of the standard terminal test setup, and
/// returns the window the ChatGPT plan modal would target.
fn initialize_app_for_chatgpt_plan_modal(app: &mut App) -> WindowId {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|ctx| {
        CloudPreferencesSyncer::new(false, std::path::PathBuf::new(), true, ctx)
    });
    add_window_with_id_and_terminal(app, None).0
}

fn set_chatgpt_connected(token_sharing_active: bool, app: &mut App) {
    ApiKeyManager::handle(app).update(app, |manager, ctx| {
        manager.set_chatgpt_connection_status(
            ChatGPTConnectionStatus::Connected(ChatGPTConnection {
                email: None,
                connected_at: SystemTime::now(),
                token_sharing_active,
            }),
            ctx,
        );
    });
}

fn mark_cloud_preferences_loaded(app: &mut App) {
    CloudPreferencesSyncer::handle(app).update(app, |syncer, _| {
        syncer.mark_initial_load_completed_for_test();
    });
}

#[test]
fn chatgpt_plan_modal_shows_once_for_an_active_subscription() {
    App::test((), |mut app| async move {
        let window_id = initialize_app_for_chatgpt_plan_modal(&mut app);
        let _flag = FeatureFlag::ChatGPTSubscription.override_enabled(true);
        set_chatgpt_connected(true, &mut app);
        mark_cloud_preferences_loaded(&mut app);

        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            assert!(!*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);

            let shown = model.check_and_trigger_chatgpt_plan_modal(window_id, ctx);

            // The seen marker is written up front, whether or not the modal is shown on
            // the current channel.
            assert!(*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
            assert_eq!(model.is_chatgpt_plan_modal_open, shown);
            if shown {
                assert_eq!(model.target_window_id, Some(window_id));
                assert!(model.is_chatgpt_plan_modal_open());
            }

            // A second check is a no-op, so the modal is never shown twice.
            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));

            model.mark_chatgpt_plan_modal_dismissed(ctx);
            assert!(!model.is_chatgpt_plan_modal_open);
            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));
        });
    });
}

#[test]
fn chatgpt_plan_modal_requires_an_active_subscription() {
    App::test((), |mut app| async move {
        let window_id = initialize_app_for_chatgpt_plan_modal(&mut app);
        let _flag = FeatureFlag::ChatGPTSubscription.override_enabled(true);
        mark_cloud_preferences_loaded(&mut app);

        // Nothing connected yet.
        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));
            assert!(!*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
        });

        // Linked, but the server holds no delegated credentials, so plan sharing is off.
        set_chatgpt_connected(false, &mut app);
        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));
            assert!(!*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
        });
    });
}

#[test]
fn chatgpt_plan_modal_waits_for_the_initial_preferences_load() {
    App::test((), |mut app| async move {
        let window_id = initialize_app_for_chatgpt_plan_modal(&mut app);
        let _flag = FeatureFlag::ChatGPTSubscription.override_enabled(true);
        set_chatgpt_connected(true, &mut app);

        // The synced seen marker can't be trusted until the initial preferences load
        // lands, so the check defers without consuming the marker.
        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));
            assert!(!*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
        });

        mark_cloud_preferences_loaded(&mut app);
        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            let shown = model.check_and_trigger_chatgpt_plan_modal(window_id, ctx);
            assert!(*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
            assert_eq!(model.is_chatgpt_plan_modal_open, shown);
        });
    });
}

#[test]
fn chatgpt_plan_modal_trusts_preferences_already_loaded_for_the_same_user() {
    App::test((), |mut app| async move {
        let window_id = initialize_app_for_chatgpt_plan_modal(&mut app);
        let _flag = FeatureFlag::ChatGPTSubscription.override_enabled(true);

        // The Sign in with ChatGPT handoff re-fetches the user, which resets the syncer's
        // loaded state, but this session already loaded the same user's preferences.
        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            model.record_cloud_preferences_loaded(ctx);
            assert!(model.cloud_preferences_loaded_for.is_some());
        });
        set_chatgpt_connected(true, &mut app);

        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            assert!(!CloudPreferencesSyncer::as_ref(ctx).has_completed_initial_load());
            let shown = model.check_and_trigger_chatgpt_plan_modal(window_id, ctx);
            assert!(*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
            assert_eq!(model.is_chatgpt_plan_modal_open, shown);
        });
    });
}

#[test]
fn chatgpt_plan_modal_waits_for_a_fresh_preferences_load_after_logout() {
    App::test((), |mut app| async move {
        let window_id = initialize_app_for_chatgpt_plan_modal(&mut app);
        let _flag = FeatureFlag::ChatGPTSubscription.override_enabled(true);

        // Logout wipes the local seen marker, so a re-login of the same user must not
        // trust the earlier load and re-show the modal before cloud values arrive.
        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            model.record_cloud_preferences_loaded(ctx);
            model.on_log_out();
            assert!(model.cloud_preferences_loaded_for.is_none());
        });
        set_chatgpt_connected(true, &mut app);

        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            assert!(!CloudPreferencesSyncer::as_ref(ctx).has_completed_initial_load());
            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));
            assert!(!*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
        });
    });
}

#[test]
fn chatgpt_plan_modal_respects_the_synced_seen_marker() {
    App::test((), |mut app| async move {
        let window_id = initialize_app_for_chatgpt_plan_modal(&mut app);
        let _flag = FeatureFlag::ChatGPTSubscription.override_enabled(true);
        set_chatgpt_connected(true, &mut app);
        mark_cloud_preferences_loaded(&mut app);

        // Another device already showed the modal for this connection.
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            assert!(
                settings
                    .did_show_chatgpt_plan_modal
                    .set_value(true, ctx)
                    .is_ok()
            );
        });

        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));
            assert!(!model.is_chatgpt_plan_modal_open);
        });
    });
}

#[test]
fn chatgpt_plan_modal_defers_while_another_one_time_modal_is_open() {
    App::test((), |mut app| async move {
        let window_id = initialize_app_for_chatgpt_plan_modal(&mut app);
        let _flag = FeatureFlag::ChatGPTSubscription.override_enabled(true);
        set_chatgpt_connected(true, &mut app);
        mark_cloud_preferences_loaded(&mut app);

        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            model.target_window_id = Some(window_id);
            model.set_auto_handoff_sleep_modal_open(true, ctx);

            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));
            // Deferred rather than consumed: the marker stays unset so a later re-check
            // can still show it.
            assert!(!*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);

            model.mark_auto_handoff_sleep_modal_dismissed(ctx);
            let shown = model.check_and_trigger_chatgpt_plan_modal(window_id, ctx);
            assert!(*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
            assert_eq!(model.is_chatgpt_plan_modal_open, shown);
        });
    });
}

#[test]
fn chatgpt_plan_modal_defers_while_the_onboarding_tutorial_is_active() {
    App::test((), |mut app| async move {
        let window_id = initialize_app_for_chatgpt_plan_modal(&mut app);
        let _flag = FeatureFlag::ChatGPTSubscription.override_enabled(true);
        set_chatgpt_connected(true, &mut app);
        mark_cloud_preferences_loaded(&mut app);

        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            model.set_onboarding_tutorial_active(true, ctx);

            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));
            assert!(!*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);

            model.set_onboarding_tutorial_active(false, ctx);
            let shown = model.check_and_trigger_chatgpt_plan_modal(window_id, ctx);
            assert!(*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
            assert_eq!(model.is_chatgpt_plan_modal_open, shown);
        });
    });
}

#[test]
fn chatgpt_plan_modal_skipped_when_flag_disabled() {
    App::test((), |mut app| async move {
        let window_id = initialize_app_for_chatgpt_plan_modal(&mut app);
        let _flag = FeatureFlag::ChatGPTSubscription.override_enabled(false);
        set_chatgpt_connected(true, &mut app);
        mark_cloud_preferences_loaded(&mut app);

        OneTimeModalModel::handle(&app).update(&mut app, |model, ctx| {
            assert!(!model.check_and_trigger_chatgpt_plan_modal(window_id, ctx));
            model.on_workspace_shown(window_id, ctx);
            assert!(!model.is_chatgpt_plan_modal_open);
            // The marker stays untouched so the modal can still be shown once the flag
            // is turned on.
            assert!(!*AISettings::as_ref(ctx).did_show_chatgpt_plan_modal);
        });
    });
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
fn hoa_onboarding_pre_dismissed_for_new_users_on_auth_complete() {
    App::test((), |mut app| async move {
        let _hoa_onboarding_flow = FeatureFlag::HOAOnboardingFlow.override_enabled(true);
        let _vertical_tabs = FeatureFlag::VerticalTabs.override_enabled(true);
        let _hoa_notifications = FeatureFlag::HOANotifications.override_enabled(true);
        let _tab_configs = FeatureFlag::TabConfigs.override_enabled(true);
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|ctx| {
            CloudPreferencesSyncer::new(false, std::path::PathBuf::new(), true, ctx)
        });
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _model = OneTimeModalModel::handle(ctx);

            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            assert!(!hoa_onboarding::has_completed_hoa_onboarding(ctx));

            AuthManager::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(AuthManagerEvent::AuthComplete);
            });
        });
        terminal.update(&mut app, |_, ctx| {
            assert!(hoa_onboarding::has_completed_hoa_onboarding(ctx));
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                assert!(!model.check_and_trigger_hoa_onboarding(ctx));
            });
        });
    });
}

#[test]
fn hoa_onboarding_not_pre_dismissed_for_existing_users_on_auth_complete() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|ctx| {
            CloudPreferencesSyncer::new(false, std::path::PathBuf::new(), true, ctx)
        });
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            let _model = OneTimeModalModel::handle(ctx);

            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(true);
            assert!(!hoa_onboarding::has_completed_hoa_onboarding(ctx));

            AuthManager::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(AuthManagerEvent::AuthComplete);
            });
        });

        app.read(|ctx| {
            assert!(!hoa_onboarding::has_completed_hoa_onboarding(ctx));
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
