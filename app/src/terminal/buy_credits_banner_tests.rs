use settings::PrivatePreferences;
use warpui::platform::WindowStyle;
use warpui::{App, TypedActionView, ViewHandle, WindowId};

use super::*;
use crate::ai::request_usage_model::AIRequestUsageModel;
use crate::auth::user_uid::{TEST_USER_EMAIL, TEST_USER_UID};
use crate::auth::{AuthStateProvider, UserUid};
use crate::pricing::PricingInfoModel;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::{MockWorkspaceClient, PurchaseAddonCreditsOutcome};
use crate::workspaces::team::{MembershipRole, Team, TeamMember, TeamVisibility};
use crate::workspaces::user_workspaces::WorkspacesMetadataResponse;
use crate::workspaces::workspace::Workspace;

fn team_for_test(uid: i64, name: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_string(),
        color: None,
        invite_link: None,
        // The logged-in test user (`AuthStateProvider::new_for_test`) is an owner of every
        // test team, so both banners' admin-permission checks resolve the same way.
        members: vec![TeamMember {
            uid: UserUid::new(TEST_USER_UID),
            email: TEST_USER_EMAIL.to_string(),
            role: MembershipRole::Owner,
        }],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: TeamVisibility::Open,
    }
}

fn workspace_for_test(teams: Vec<Team>) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams,
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

fn metadata_response(workspace: Workspace) -> WorkspacesMetadataResponse {
    WorkspacesMetadataResponse {
        workspaces: vec![workspace],
        joinable_teams: vec![],
        experiments: None,
        feature_model_choices: None,
        ai_credit_availability: None,
        user_purchase_policy: None,
    }
}

fn initialize_app(app: &mut App, workspace: Workspace, workspace_client: MockWorkspaceClient) {
    app.add_singleton_model(|_| warp_core::ui::appearance::Appearance::mock());
    app.add_singleton_model(crate::settings::PrivacySettings::mock);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| PricingInfoModel::new());
    app.add_singleton_model(
        crate::server::telemetry::context_provider::AppTelemetryContextProvider::new_context_provider,
    );
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    if app.models_of_type::<PrivatePreferences>().is_empty() {
        app.update(crate::settings::init_and_register_user_preferences);
    }
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            std::sync::Arc::new(MockTeamClient::new()),
            std::sync::Arc::new(workspace_client),
            vec![workspace],
            ctx,
        )
    });
}

fn create_banner_window(app: &mut App) -> (WindowId, ViewHandle<BuyCreditsBanner>) {
    app.add_window(WindowStyle::NotStealFocus, BuyCreditsBanner::new)
}

/// Waits until `condition` holds, waking on any invalidation of `window_ids` rather than
/// sleeping a fixed duration. A purchase's mock call resolving is not the same moment as the
/// banner's own state finishing its update (the model-level continuation that resolves the
/// mock call still has to hand off to the view-level continuation that applies the result), so
/// this waits on the actual state transition instead of the call itself.
async fn wait_for_window_state(
    app: &App,
    window_ids: &[WindowId],
    mut condition: impl FnMut(&App) -> bool,
) {
    let (notify_tx, notify_rx) = async_channel::unbounded::<()>();
    for window_id in window_ids {
        let notify_tx = notify_tx.clone();
        app.on_window_invalidated(*window_id, move |_, _| {
            let _ = notify_tx.try_send(());
        });
    }
    for _ in 0..10_000 {
        if condition(app) {
            return;
        }
        notify_rx
            .recv()
            .await
            .expect("expected a window invalidation while waiting for the condition");
    }
    panic!("condition did not become true after many window invalidations");
}

/// Purchase on team A, then switch the purchasing window to team B before the purchase
/// resolves, with a second window already sitting on team B throughout. The completion
/// path must persist auto-reload for team A — the team the purchase actually targeted —
/// and the uninvolved team-B banner must not react to team A's purchase at all.
#[test]
fn purchase_completion_targets_the_originally_purchased_team_not_the_windows_current_team() {
    let _banner_toggle_flag = FeatureFlag::BuildPlanAutoReloadBannerToggle.override_enabled(true);

    let team_a = team_for_test(100, "Team A");
    let team_b = team_for_test(200, "Team B");
    let team_a_uid = team_a.uid;
    let team_b_uid = team_b.uid;
    let workspace = workspace_for_test(vec![team_a.clone(), team_b.clone()]);

    let mut workspace_client = MockWorkspaceClient::new();
    {
        let team_a = team_a.clone();
        let team_b = team_b.clone();
        workspace_client
            .expect_purchase_addon_credits()
            .withf(move |team_uid, credits| *team_uid == Some(team_a_uid) && *credits == 500)
            .times(1)
            .returning(move |_, _| {
                Ok(PurchaseAddonCreditsOutcome::Completed(Box::new(
                    metadata_response(workspace_for_test(vec![team_a.clone(), team_b.clone()])),
                )))
            });
    }
    // The regression this guards: before the fix, the completion path re-derived the
    // team from the window (by then reassigned to team B) instead of the team the
    // purchase actually targeted. This expectation only matches team A; a call with
    // team B's UID (the bug) or a second, unexpected call (a stray reaction from team
    // B's own banner) both fail mock verification.
    //
    // The `returning` closure also signals `settings_update_called_rx` the moment the mock
    // is invoked, so the test can wait deterministically for this specific call instead of
    // sleeping a fixed duration or draining the executor's task queue.
    let (settings_update_called_tx, settings_update_called_rx) = async_channel::unbounded::<()>();
    {
        let team_a = team_a.clone();
        let team_b = team_b.clone();
        workspace_client
            .expect_update_addon_credits_settings()
            .withf(move |team_uid, auto_reload_enabled, _, _| {
                *team_uid == team_a_uid && *auto_reload_enabled == Some(true)
            })
            .times(1)
            .returning(move |_, _, _, _| {
                let _ = settings_update_called_tx.try_send(());
                Ok(metadata_response(workspace_for_test(vec![
                    team_a.clone(),
                    team_b.clone(),
                ])))
            });
    }

    App::test((), |mut app| async move {
        initialize_app(&mut app, workspace, workspace_client);

        let (window_a, view_a) = create_banner_window(&mut app);
        let (window_b, view_b) = create_banner_window(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_a, team_a_uid, ctx);
            user_workspaces.set_team_for_window(window_b, team_b_uid, ctx);
        });

        // Both banners opt into the banner-toggle auto-reload experiment, so a banner
        // that incorrectly reacted to another window's purchase would still be able to
        // call `update_addon_credits_settings` and get caught by the mock above.
        view_a.update(&mut app, |view, ctx| {
            view.addon_credits_options = vec![AddonCreditsOption {
                credits: 500,
                price_usd_cents: 500,
            }];
            view.handle_action(&Action::ToggleAutoReload, ctx);
        });
        view_b.update(&mut app, |view, ctx| {
            view.handle_action(&Action::ToggleAutoReload, ctx);
        });

        // Click "buy" on team A's banner.
        view_a.update(&mut app, |view, ctx| {
            view.handle_action(
                &Action::PurchaseAddonCredits {
                    team_uid: Some(team_a_uid),
                },
                ctx,
            );
        });

        // Window A switches to team B before the purchase resolves: team A leaves the
        // workspace, so window A's assignment reconciles to the only remaining team.
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![workspace_for_test(vec![team_b])], ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_a),
                Some(team_b_uid),
                "window A should have reconciled onto team B before the purchase resolved"
            );
        });

        // Wait for the settings-update mock to actually be called, rather than sleeping a
        // fixed duration or assuming a particular number of executor turns. This resolves
        // once the purchase call, its completion continuation, and the auto-reload settings
        // update it triggers have all run.
        settings_update_called_rx
            .recv()
            .await
            .expect("update_addon_credits_settings should have been called");

        // Mock verification on drop asserts `update_addon_credits_settings` was called
        // exactly once, for team A. `MockWorkspaceClient`'s `Drop` impl panics on
        // unmet or violated expectations, which nextest surfaces as a test failure.
    })
}

/// Three windows each purchase credits for their own team before any of the purchases
/// resolve, and the three purchases resolve to three different outcomes (completed,
/// budget-rejected, checkout-required). A single per-banner "is a purchase pending" flag
/// (rather than a value correlated to the specific call) would let one banner's resolution
/// consume another's pending marker and leave the true result stranded when its own event
/// finally arrived; each banner's outcome must instead depend only on its own purchase.
#[test]
fn concurrent_purchases_across_windows_resolve_independently() {
    let team_a = team_for_test(100, "Team A"); // resolves Completed
    let team_b = team_for_test(200, "Team B"); // resolves Rejected (budget exceeded)
    let team_c = team_for_test(300, "Team C"); // resolves CheckoutRequired
    let team_a_uid = team_a.uid;
    let team_b_uid = team_b.uid;
    let team_c_uid = team_c.uid;
    let workspace = workspace_for_test(vec![team_a.clone(), team_b.clone(), team_c.clone()]);

    // Each `returning` closure signals this channel with the team it was called for, so the
    // test can wait deterministically for all three concurrent calls to resolve instead of
    // sleeping a fixed duration or assuming a particular number of executor turns.
    let (call_done_tx, call_done_rx) = async_channel::unbounded::<ServerId>();

    let mut workspace_client = MockWorkspaceClient::new();
    {
        let team_a = team_a.clone();
        let team_b = team_b.clone();
        let team_c = team_c.clone();
        let call_done_tx = call_done_tx.clone();
        workspace_client
            .expect_purchase_addon_credits()
            .withf(move |team_uid, _| *team_uid == Some(team_a_uid))
            .times(1)
            .returning(move |_, _| {
                let _ = call_done_tx.try_send(team_a_uid);
                Ok(PurchaseAddonCreditsOutcome::Completed(Box::new(
                    metadata_response(workspace_for_test(vec![
                        team_a.clone(),
                        team_b.clone(),
                        team_c.clone(),
                    ])),
                )))
            });
    }
    {
        let call_done_tx = call_done_tx.clone();
        workspace_client
            .expect_purchase_addon_credits()
            .withf(move |team_uid, _| *team_uid == Some(team_b_uid))
            .times(1)
            .returning(move |_, _| {
                let _ = call_done_tx.try_send(team_b_uid);
                Err(warp_graphql::error::BudgetExceededError {
                    message: "Monthly spend limit reached".to_string(),
                }
                .into())
            });
    }
    {
        let call_done_tx = call_done_tx.clone();
        workspace_client
            .expect_purchase_addon_credits()
            .withf(move |team_uid, _| *team_uid == Some(team_c_uid))
            .times(1)
            .returning(move |_, _| {
                let _ = call_done_tx.try_send(team_c_uid);
                Ok(PurchaseAddonCreditsOutcome::CheckoutRequired {
                    checkout_url: "https://example.com/checkout".to_string(),
                })
            });
    }

    App::test((), |mut app| async move {
        initialize_app(&mut app, workspace, workspace_client);

        let (window_a, view_a) = create_banner_window(&mut app);
        let (window_b, view_b) = create_banner_window(&mut app);
        let (window_c, view_c) = create_banner_window(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_a, team_a_uid, ctx);
            user_workspaces.set_team_for_window(window_b, team_b_uid, ctx);
            user_workspaces.set_team_for_window(window_c, team_c_uid, ctx);
        });

        // Click "buy" on all three banners before any purchase resolves.
        for (view, team_uid) in [
            (&view_a, team_a_uid),
            (&view_b, team_b_uid),
            (&view_c, team_c_uid),
        ] {
            view.update(&mut app, |view, ctx| {
                view.addon_credits_options = vec![AddonCreditsOption {
                    credits: 500,
                    price_usd_cents: 500,
                }];
                view.handle_action(
                    &Action::PurchaseAddonCredits {
                        team_uid: Some(team_uid),
                    },
                    ctx,
                );
            });
        }

        let mut resolved = std::collections::HashSet::new();
        while resolved.len() < 3 {
            let team_uid = call_done_rx
                .recv()
                .await
                .expect("all three purchase calls should resolve");
            resolved.insert(team_uid);
        }

        // The mock calls resolving is not the same moment as each banner's own state
        // finishing its update; wait for that separately.
        wait_for_window_state(&app, &[window_a, window_b, window_c], |app| {
            app.read(|ctx| {
                !view_a.as_ref(ctx).purchase_addon_credits_loading
                    && !view_b.as_ref(ctx).purchase_addon_credits_loading
                    && !view_c.as_ref(ctx).purchase_addon_credits_loading
            })
        })
        .await;

        app.read(|ctx| {
            view_a.read(ctx, |view, _| {
                assert!(!view.purchase_addon_credits_loading);
                assert!(
                    !view.should_display_banner,
                    "team A's purchase completed outright; it must not show team B's budget-exceeded outcome"
                );
                assert!(
                    !view.checkout_pending,
                    "team A's purchase completed synchronously; it must not show team C's checkout-pending outcome"
                );
            });
            view_b.read(ctx, |view, _| {
                assert!(!view.purchase_addon_credits_loading);
                assert!(
                    view.should_display_banner,
                    "team B's purchase was budget-rejected; it must show its own monthly-limit outcome"
                );
                assert!(!view.checkout_pending);
            });
            view_c.read(ctx, |view, _| {
                assert!(!view.purchase_addon_credits_loading);
                assert!(!view.should_display_banner);
                assert!(
                    view.checkout_pending,
                    "team C's purchase required checkout; it must show its own checkout-pending outcome"
                );
                assert_eq!(view.checkout_pending_team_uid, Some(Some(team_c_uid)));
            });
        });

        // Mock verification on drop asserts each team's purchase call happened exactly
        // once, with none of the three matchers seeing a call meant for another team.
    })
}
