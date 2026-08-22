use std::sync::Arc;

use warpui::{App, SingletonEntity, WindowId};

use super::get_secret_obfuscation_mode_for_window;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    EnforceableSetting, TeamSecretRedactionSettings, TeamSettings, Workspace,
};

fn team_with_redaction(uid: i64, enabled: bool) -> Team {
    Team {
        uid: uid.into(),
        name: format!("team-{uid}"),
        color: None,
        invite_link: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: TeamSettings {
            secret_redaction: TeamSecretRedactionSettings {
                enabled: EnforceableSetting {
                    value: enabled,
                    is_enforced_by_workspace: false,
                },
                regexes: Default::default(),
            },
            ..Default::default()
        },
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: TeamVisibility::Open,
    }
}

fn workspace_with_teams(teams: Vec<Team>) -> Workspace {
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

/// The enforcement acceptance criterion for this PR: two windows on different teams must get
/// independent, correct enterprise-redaction behavior concurrently, resolved from each window's
/// own current team rather than any ambient, process-wide value.
#[test]
fn test_get_secret_obfuscation_mode_for_window_follows_each_windows_own_team_concurrently() {
    let team_a = team_with_redaction(123, true);
    let team_b = team_with_redaction(456, false);
    let workspace = workspace_with_teams(vec![team_a.clone(), team_b.clone()]);

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(MockWorkspaceClient::new()),
                vec![workspace],
                ctx,
            )
        });

        let window_a = WindowId::new();
        let window_b = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_a, team_a.uid, ctx);
            user_workspaces.set_team_for_window(window_b, team_b.uid, ctx);
        });

        app.read(|ctx| {
            assert!(
                get_secret_obfuscation_mode_for_window(window_a, ctx).should_redact_secret(),
                "window A's own team requires enterprise redaction"
            );
            assert!(
                !get_secret_obfuscation_mode_for_window(window_b, ctx).should_redact_secret(),
                "window B's team does not require enterprise redaction, even though window \
                 A's does concurrently"
            );
        });
    })
}

/// A window with no team must not fall back to any other team's enterprise-redaction policy.
#[test]
fn test_get_secret_obfuscation_mode_for_window_defaults_safe_without_a_team() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(MockWorkspaceClient::new()),
                vec![],
                ctx,
            )
        });

        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, None, ctx);
        });

        app.read(|ctx| {
            assert!(
                !get_secret_obfuscation_mode_for_window(window_id, ctx).should_redact_secret(),
                "a window with no team must not be redacted based on some other team's policy"
            );
        });
    })
}
