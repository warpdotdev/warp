use std::path::PathBuf;

use warpui::App;

use super::*;
use crate::ai::agent::AIAgentActionId;
use crate::test_util::terminal::{
    add_window_with_id_and_terminal, initialize_app_for_terminal_view,
};
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{AdminEnablementSetting, EnforceableSetting, Workspace};

fn team_with_codebase_context(uid: i64, setting: AdminEnablementSetting) -> Team {
    let mut team = Team {
        uid: uid.into(),
        name: format!("team-{uid}"),
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
    team.settings.codebase_context = EnforceableSetting {
        value: setting,
        is_enforced_by_workspace: false,
    };
    team
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

/// Regression test for the retrieval boundary bypass: `send_request` used to authorize only via
/// `should_use_codebase_indexing` (a process-wide "any known team" gate), so a window on a
/// denied team could still retrieve local/remote indexed content as long as some other window's
/// team allowed it. Uses real windows and real `TerminalView`s (not opaque `WindowId`s) so the
/// same `window_id_for_terminal_surface` scan the production code runs is exercised here.
#[test]
fn send_request_denies_a_window_whose_team_disables_codebase_context_but_not_an_allowed_one() {
    let denied_team = team_with_codebase_context(101, AdminEnablementSetting::Disable);
    let allowed_team = team_with_codebase_context(102, AdminEnablementSetting::Enable);
    let denied_team_uid = denied_team.uid;
    let allowed_team_uid = allowed_team.uid;
    let workspace = workspace_with_teams(vec![denied_team, allowed_team]);
    let workspace_uid = workspace.uid;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![workspace], ctx);
            user_workspaces.set_current_workspace_uid(workspace_uid, ctx);
        });

        let (denied_window, denied_view) = add_window_with_id_and_terminal(&mut app, None);
        let (allowed_window, allowed_view) = add_window_with_id_and_terminal(&mut app, None);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(denied_window, denied_team_uid, ctx);
            user_workspaces.set_team_for_window(allowed_window, allowed_team_uid, ctx);
        });

        let denied_terminal_view_id = denied_view.read(&app, |view, _| view.view_id());
        let allowed_terminal_view_id = allowed_view.read(&app, |view, _| view.view_id());

        let denied_controller =
            app.add_model(|ctx| GetRelevantFilesController::new(denied_terminal_view_id, ctx));
        let allowed_controller =
            app.add_model(|ctx| GetRelevantFilesController::new(allowed_terminal_view_id, ctx));

        let denied_result = denied_controller.update(&mut app, |controller, ctx| {
            controller.send_request(
                GetRelevantFilesRequestTarget::Local {
                    directory: PathBuf::from("/tmp"),
                },
                "query".to_string(),
                None,
                AIAgentActionId::from("denied-action".to_string()),
                ctx,
            )
        });
        assert!(
            matches!(
                denied_result,
                Err(GetRelevantFilesError::NotAuthorizedForTeam)
            ),
            "a window on a team that disables codebase context must be denied at the retrieval \
             boundary, got {denied_result:?}"
        );

        let allowed_result = allowed_controller.update(&mut app, |controller, ctx| {
            controller.send_request(
                GetRelevantFilesRequestTarget::Local {
                    directory: PathBuf::from("/tmp"),
                },
                "query".to_string(),
                None,
                AIAgentActionId::from("allowed-action".to_string()),
                ctx,
            )
        });
        assert!(
            !matches!(
                allowed_result,
                Err(GetRelevantFilesError::NotAuthorizedForTeam)
            ),
            "a window on an allowed team must not be blocked by team policy, got {allowed_result:?}"
        );
    })
}
