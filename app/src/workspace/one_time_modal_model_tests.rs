use futures::FutureExt;
use warpui::{App, SingletonEntity};

use super::{
    free_ai_removal_modal_decision, AISettings, FeatureIntroId, FreeAiRemovalModalDecision,
    OneTimeModalModel, FEATURE_INTROS,
};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::workspaces::workspace::CustomerType;

#[test]
fn wait_until_auto_handoff_sleep_modal_closed_tracks_modal_state() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |_, ctx| {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                // Resolves immediately while the modal is closed.
                assert!(model
                    .wait_until_auto_handoff_sleep_modal_closed()
                    .now_or_never()
                    .is_some());

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
