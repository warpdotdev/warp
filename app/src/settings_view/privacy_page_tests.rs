use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warpui::App;
use warpui::platform::WindowStyle;

use super::*;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::ids::ServerId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::sync_queue::SyncQueue;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

fn team_for_test(uid: ServerId, name: &str) -> Team {
    Team {
        uid,
        name: name.to_string(),
        color: None,
        invite_link: None,
        members: vec![],
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
        billing_metadata: teams
            .first()
            .map(|t| t.billing_metadata.clone())
            .unwrap_or_default(),
        teams,
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

fn init_privacy_page_test_app(app: &mut App, workspaces: Vec<Workspace>) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
}

/// Builds an `AppAnalyticsWidget` bound to the same view as `view_handle`. Its
/// `should_show_zdr_badge` reads team policy exactly as the widget owned by a real
/// `PrivacyPageView` does, so it lets a test observe that page's team-derived state
/// without reaching into the page's private widget list.
fn zdr_badge_probe(view_handle: WeakViewHandle<PrivacyPageView>) -> AppAnalyticsWidget {
    AppAnalyticsWidget {
        view_handle,
        switch_state: Default::default(),
        docs_link_mouse_state: Default::default(),
        zdr_badge_mouse_state: Default::default(),
    }
}

/// Instantiates two real `PrivacyPageView`s in two real windows (not synthetic views) within
/// the *same* workspace, one on team A (which disables UGC collection) and one on team B
/// (which respects the user's setting), open at the same time. A test that only ever has one
/// team present at a time -- swapping the workspace's team list in place, one window, one
/// `current_workspace()` -- cannot distinguish correct window-scoping from the old
/// `current_workspace()`-based reads: with a single team present, both would give the same
/// answer. Two windows on two teams *simultaneously present in one workspace* is the case
/// only a genuinely per-window read can get right, since `current_workspace()` has no way to
/// answer differently for the two windows.
///
/// Then removes team A from the workspace, which reconciles window A's assignment onto team B
/// (the only remaining team) while leaving window B untouched, and asserts only window A's
/// widget state changes. This exercises `PrivacyPageView::new`'s full construction (including
/// the `UserWorkspaces` observation added to fix the page never redrawing after a window team
/// change), unlike a synthetic test view that never runs that constructor.
///
/// This does not independently assert that a redraw is *scheduled* after a team change:
/// `warpui_core`'s invalidation-draining and frame-simulation APIs
/// (`take_all_invalidations_for_window`, `simulate_render_frame`) are `pub(crate)`/`#[cfg(test)]`
/// to that crate and not reachable from `warp`'s tests, so nothing here can distinguish "a
/// redraw was scheduled" from "a redraw was already scheduled" once a window has rendered once.
#[test]
fn test_privacy_page_view_reflects_window_team_change() {
    let mut team_a = team_for_test(123.into(), "team-a");
    team_a.settings.ugc_collection.value = UgcCollectionEnablementSetting::Disable;
    let team_b = team_for_test(456.into(), "team-b");

    App::test((), |mut app| async move {
        init_privacy_page_test_app(
            &mut app,
            vec![workspace_for_test(vec![team_a.clone(), team_b.clone()])],
        );

        let (window_a, view_a) = app.add_window(WindowStyle::NotStealFocus, PrivacyPageView::new);
        let (window_b, view_b) = app.add_window(WindowStyle::NotStealFocus, PrivacyPageView::new);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_a, team_a.uid, ctx);
            user_workspaces.set_team_for_window(window_b, team_b.uid, ctx);
        });

        let weak_view_a = view_a.downgrade();
        let weak_view_b = view_b.downgrade();
        app.read(|ctx| {
            assert!(
                zdr_badge_probe(weak_view_a.clone()).should_show_zdr_badge(ctx),
                "window A's team disables UGC collection, so its ZDR badge should show"
            );
            assert!(
                !zdr_badge_probe(weak_view_b.clone()).should_show_zdr_badge(ctx),
                "window B's team respects the user's UGC setting, so its ZDR badge should not \
                 show, even though both windows share one workspace and one `current_workspace()`"
            );
        });

        // Remove team A from the workspace: window A reconciles onto the only remaining team
        // (team B). Window B's team is untouched.
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![workspace_for_test(vec![team_b.clone()])], ctx);
        });
        app.read(|ctx| {
            assert!(
                !zdr_badge_probe(weak_view_a.clone()).should_show_zdr_badge(ctx),
                "after reconciling onto team B, which respects the user's UGC setting, \
                 window A's ZDR badge should no longer show"
            );
            assert!(
                !zdr_badge_probe(weak_view_b.clone()).should_show_zdr_badge(ctx),
                "window B's team was never touched by window A's reconciliation"
            );
        });

        // Reconcile every window onto no team by removing every team.
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![], ctx);
        });
        app.read(|ctx| {
            assert!(
                !zdr_badge_probe(weak_view_a.clone()).should_show_zdr_badge(ctx),
                "a window with no team should not show the ZDR badge"
            );
            assert!(
                !zdr_badge_probe(weak_view_b.clone()).should_show_zdr_badge(ctx),
                "a window with no team should not show the ZDR badge"
            );
        });
    })
}
