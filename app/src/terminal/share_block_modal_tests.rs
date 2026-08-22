use std::sync::Arc;

use warpui::App;
use warpui::browser::escape_html_attribute;
use warpui::platform::WindowStyle;

use super::*;
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::ids::ServerId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::block::MockBlockClient;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::workspace::Workspace;

#[test]
fn escape_html_attribute_escapes_attribute_breakout_characters() {
    assert_eq!(
        escape_html_attribute("\" onload=\"alert(1)\" data-x='><script>alert(1)</script>&"),
        "&quot; onload=&quot;alert(1)&quot; data-x=&#39;&gt;&lt;script&gt;alert(1)&lt;/script&gt;&amp;"
    );
}

#[test]
fn escape_html_attribute_leaves_safe_text_unchanged() {
    assert_eq!(
        escape_html_attribute("embedded warp block"),
        "embedded warp block"
    );
}

fn team_for_test(uid: ServerId, name: &str, enterprise_redaction_enabled: bool) -> Team {
    let mut team = Team {
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
    };
    team.settings.secret_redaction.enabled.value = enterprise_redaction_enabled;
    team
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

fn init_share_block_modal_test_app(app: &mut App, workspaces: Vec<Workspace>) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(CloudModel::mock);
}

fn new_share_block_modal(ctx: &mut ViewContext<ShareBlockModal>) -> ShareBlockModal {
    ShareBlockModal::new(None, Arc::new(MockBlockClient::new()), ctx)
}

/// Two windows, each assigned to a different team in the *same* workspace (not swapped one
/// at a time), one enforcing enterprise secret redaction and one not. A test built around
/// `current_workspace()` -- reading a single ambient "current" workspace/team rather than the
/// asking window's own team -- cannot distinguish this scenario, since both windows would read
/// the same answer; only a genuinely window-scoped read can show them differing at the same
/// instant.
///
/// Then removes the enforcing team from the workspace, which reconciles the first window onto
/// the remaining (non-enforcing) team, and asserts only that window's state changes while the
/// second window's is undisturbed -- covering a live team reassignment for an already-open
/// modal, which is what let a stale cached read (rather than resolving fresh) leave redaction
/// optional after an already-open share modal's window moved into a team that requires it.
#[test]
fn test_share_block_modal_enterprise_redaction_is_scoped_per_window() {
    let team_enforcing = team_for_test(123.into(), "team-enforcing", true);
    let team_respecting = team_for_test(456.into(), "team-respecting", false);

    App::test((), |mut app| async move {
        init_share_block_modal_test_app(
            &mut app,
            vec![workspace_for_test(vec![
                team_enforcing.clone(),
                team_respecting.clone(),
            ])],
        );

        let (window_id_enforcing, view_enforcing) =
            app.add_window(WindowStyle::NotStealFocus, new_share_block_modal);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id_enforcing, team_enforcing.uid, ctx);
        });

        let (_window_id_respecting, view_respecting) =
            app.add_window(WindowStyle::NotStealFocus, new_share_block_modal);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(_window_id_respecting, team_respecting.uid, ctx);
        });

        app.read(|ctx| {
            assert!(
                view_enforcing
                    .as_ref(ctx)
                    .is_enterprise_secret_redaction_enabled(ctx),
                "the window on the enforcing team should require redaction"
            );
            assert!(
                !view_respecting
                    .as_ref(ctx)
                    .is_enterprise_secret_redaction_enabled(ctx),
                "the window on the respecting team should not require redaction, even though \
                 both windows share one workspace and one `current_workspace()`"
            );
        });

        // Remove the enforcing team from the workspace: the first window reconciles onto the
        // only remaining team (respecting), simulating an already-open modal's window losing
        // its team. The second window's team is untouched.
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces
                .update_workspaces(vec![workspace_for_test(vec![team_respecting.clone()])], ctx);
        });

        app.read(|ctx| {
            assert!(
                !view_enforcing
                    .as_ref(ctx)
                    .is_enterprise_secret_redaction_enabled(ctx),
                "after its team was removed and it reconciled onto the respecting team, this \
                 window's modal must stop requiring redaction -- a stale cached read would \
                 still show the old team's enforced-redaction state here"
            );
            assert!(
                !view_respecting
                    .as_ref(ctx)
                    .is_enterprise_secret_redaction_enabled(ctx),
                "the second window's team was never touched by the first window's reconciliation"
            );
        });
    })
}
