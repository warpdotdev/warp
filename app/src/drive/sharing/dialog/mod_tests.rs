use chrono::Local;
use session_sharing_protocol::common::SessionId;
use warpui::{App, SingletonEntity, ViewHandle};

use super::{SharingDialog, SharingDialogMode};
use crate::drive::sharing::ShareableObject;
use crate::terminal::TerminalView;
use crate::terminal::shared_session::manager::Manager;
use crate::terminal::shared_session::{SharedSessionSource, SharedSessionStatus};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::{
    add_window_with_id_and_terminal, initialize_app_for_terminal_view,
};
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    EnforceableSetting, TeamLinkSharingSettings, TeamSettings, Workspace,
};

fn set_shared_session_status(
    terminal: &ViewHandle<TerminalView>,
    status: SharedSessionStatus,
    app: &mut App,
) {
    terminal.update(app, |view, _| {
        view.model.lock().set_shared_session_status(status);
    });
}

fn assert_session_link_state(
    terminal: &ViewHandle<TerminalView>,
    dialog: &ViewHandle<SharingDialog>,
    expected_session_id: Option<SessionId>,
    app: &App,
) {
    terminal.read(app, |view, ctx| {
        let shared_session_status = view.model.lock().shared_session_status().clone();
        let manager = Manager::as_ref(ctx);
        assert_eq!(
            manager.session_id_for_link(&view.id(), &shared_session_status),
            expected_session_id
        );
        assert_eq!(
            manager.has_session_link(&view.id(), &shared_session_status),
            expected_session_id.is_some()
        );
    });

    dialog.read(app, |dialog, ctx| {
        assert_eq!(
            dialog.has_shared_session_link(ctx),
            expected_session_id.is_some()
        );
    });
}

#[test]
fn session_qr_code_requires_status_eligible_matching_session_id() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(Manager::new);
        app.add_singleton_model(
            |_| crate::workspaces::user_profiles::UserProfiles::new(Vec::new()),
        );
        let terminal = add_window_with_terminal(&mut app, None);
        let first_session_id = SessionId::new();

        terminal.update(&mut app, |view, ctx| {
            let window_id = ctx.window_id();
            Manager::handle(ctx).update(ctx, |manager, ctx| {
                manager.started_share(terminal.downgrade(), first_session_id, window_id, ctx);
            });
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveSharer);
        });

        let first_target = ShareableObject::Session {
            handle: terminal.downgrade(),
            session_id: first_session_id,
            started_at: Local::now(),
        };
        let window_id = app.update(|ctx| ctx.window_ids().next().unwrap());
        let dialog = app.add_typed_action_view(window_id, |ctx| {
            SharingDialog::new(Some(first_target.clone()), ctx)
        });

        assert_session_link_state(&terminal, &dialog, Some(first_session_id), &app);
        dialog.update(&mut app, |dialog, ctx| dialog.show_qr_code(ctx));
        dialog.read(&app, |dialog, _| {
            assert_eq!(dialog.mode, SharingDialogMode::QrCode);
        });

        Manager::handle(&app).update(&mut app, |manager, ctx| {
            manager.stopped_share(terminal.id(), ctx);
        });

        for status in [
            SharedSessionStatus::NotShared,
            SharedSessionStatus::FinishedViewer,
        ] {
            set_shared_session_status(&terminal, status, &mut app);
            assert_session_link_state(&terminal, &dialog, Some(first_session_id), &app);
        }

        set_shared_session_status(&terminal, SharedSessionStatus::SharePending, &mut app);
        assert_session_link_state(&terminal, &dialog, None, &app);
        dialog.update(&mut app, |dialog, ctx| {
            dialog.refresh_shared_session_link(ctx);
            assert_eq!(dialog.mode, SharingDialogMode::Access);
        });

        for status in [
            SharedSessionStatus::ViewPending,
            SharedSessionStatus::ActiveViewer {
                role: Default::default(),
            },
            SharedSessionStatus::SharePendingPreBootstrap {
                source: SharedSessionSource::default(),
            },
            SharedSessionStatus::ActiveSharer,
        ] {
            set_shared_session_status(&terminal, status, &mut app);
            assert_session_link_state(&terminal, &dialog, None, &app);
            dialog.update(&mut app, |dialog, ctx| {
                dialog.mode = SharingDialogMode::Access;
                dialog.show_qr_code(ctx);
                assert_eq!(dialog.mode, SharingDialogMode::Access);
            });
        }

        let second_session_id = SessionId::new();
        terminal.update(&mut app, |view, ctx| {
            let window_id = ctx.window_id();
            Manager::handle(ctx).update(ctx, |manager, ctx| {
                manager.started_share(terminal.downgrade(), second_session_id, window_id, ctx);
            });
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveSharer);
        });

        terminal.read(&app, |view, ctx| {
            let shared_session_status = view.model.lock().shared_session_status().clone();
            assert_eq!(
                Manager::as_ref(ctx).session_id_for_link(&view.id(), &shared_session_status),
                Some(second_session_id)
            );
        });
        dialog.read(&app, |dialog, ctx| {
            assert!(!dialog.has_shared_session_link(ctx));
        });

        dialog.update(&mut app, |dialog, ctx| {
            dialog.set_target(
                Some(ShareableObject::Session {
                    handle: terminal.downgrade(),
                    session_id: second_session_id,
                    started_at: Local::now(),
                }),
                ctx,
            );
        });
        assert_session_link_state(&terminal, &dialog, Some(second_session_id), &app);
    });
}

/// A team whose admins either permit or forbid both link-sharing channels.
fn team_with_link_sharing(uid: i64, name: &str, permitted: bool) -> Team {
    let permission = EnforceableSetting {
        value: permitted,
        is_enforced_by_workspace: false,
    };
    Team {
        uid: uid.into(),
        name: name.to_string(),
        color: None,
        invite_link: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: TeamSettings {
            link_sharing: TeamLinkSharingSettings {
                anyone_with_link_sharing_enabled: permission.clone(),
                direct_link_sharing_enabled: permission,
            },
            ..Default::default()
        },
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: TeamVisibility::Open,
    }
}

/// Replaces the user's workspaces with one holding `teams` and selects it, the way a
/// workspaces-metadata refresh does. Windows already assigned to a team that `teams` no longer
/// contains reconcile onto the first remaining one.
fn install_workspace_with_teams(app: &mut App, teams: Vec<Team>) {
    let workspace = Workspace {
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
    };
    let workspace_uid = workspace.uid;

    let user_workspaces = UserWorkspaces::handle(&*app);
    user_workspaces.update(app, |user_workspaces, ctx| {
        user_workspaces.update_workspaces(vec![workspace], ctx);
        user_workspaces.set_current_workspace_uid(workspace_uid, ctx);
    });
}

#[test]
fn link_sharing_gates_resolve_each_windows_own_team() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (permitted_window, _permitted_terminal) =
            add_window_with_id_and_terminal(&mut app, None);
        let (forbidden_window, _forbidden_terminal) =
            add_window_with_id_and_terminal(&mut app, None);

        let permitted_team = team_with_link_sharing(123, "permits-sharing", true);
        let forbidden_team = team_with_link_sharing(456, "forbids-sharing", false);
        install_workspace_with_teams(
            &mut app,
            vec![permitted_team.clone(), forbidden_team.clone()],
        );

        let user_workspaces = UserWorkspaces::handle(&app);
        user_workspaces.update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(permitted_window, permitted_team.uid, ctx);
            user_workspaces.set_team_for_window(forbidden_window, forbidden_team.uid, ctx);
        });

        let permitted_dialog =
            app.add_typed_action_view(permitted_window, |ctx| SharingDialog::new(None, ctx));
        let forbidden_dialog =
            app.add_typed_action_view(forbidden_window, |ctx| SharingDialog::new(None, ctx));

        permitted_dialog.read(&app, |dialog, ctx| {
            assert!(dialog.can_anyone_with_link_share(ctx));
            assert!(dialog.can_direct_link_share(ctx));
        });
        forbidden_dialog.read(&app, |dialog, ctx| {
            assert!(
                !dialog.can_anyone_with_link_share(ctx),
                "a dialog in a window on a forbidding team must not inherit the other window's \
                 permission"
            );
            assert!(!dialog.can_direct_link_share(ctx));
        });
    });
}

/// The dialog re-reads these gates when it renders and again before it acts, in
/// `send_invitations` and in the `SetLinkPermissions` handler. Resolving them from the window
/// rather than caching them at open is what stops an already-open dialog from sharing under
/// the policy it opened with after its window has moved to a team that forbids sharing.
#[test]
fn link_sharing_gates_follow_a_window_onto_its_new_team() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (window_id, _terminal) = add_window_with_id_and_terminal(&mut app, None);

        let permitted_team = team_with_link_sharing(123, "permits-sharing", true);
        let forbidden_team = team_with_link_sharing(456, "forbids-sharing", false);
        install_workspace_with_teams(
            &mut app,
            vec![permitted_team.clone(), forbidden_team.clone()],
        );

        let user_workspaces = UserWorkspaces::handle(&app);
        user_workspaces.update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, permitted_team.uid, ctx);
        });

        let dialog = app.add_typed_action_view(window_id, |ctx| SharingDialog::new(None, ctx));
        dialog.read(&app, |dialog, ctx| {
            assert!(dialog.can_anyone_with_link_share(ctx));
            assert!(dialog.can_direct_link_share(ctx));
        });

        // The permitting team leaves the workspace, so the window reconciles onto the
        // forbidding one while the dialog is still open.
        install_workspace_with_teams(&mut app, vec![forbidden_team]);

        dialog.read(&app, |dialog, ctx| {
            assert!(
                !dialog.can_anyone_with_link_share(ctx),
                "an open dialog must lose link sharing when its window moves to a team that \
                 forbids it"
            );
            assert!(!dialog.can_direct_link_share(ctx));
        });
    });
}

#[test]
fn link_sharing_gates_deny_when_the_dialogs_window_has_no_team() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (window_id, _terminal) = add_window_with_id_and_terminal(&mut app, None);
        install_workspace_with_teams(&mut app, vec![team_with_link_sharing(123, "team", true)]);

        // Deliberately left unregistered: no window team can be resolved for this dialog.
        let dialog = app.add_typed_action_view(window_id, |ctx| SharingDialog::new(None, ctx));

        dialog.read(&app, |dialog, ctx| {
            assert!(
                !dialog.can_anyone_with_link_share(ctx),
                "a dialog whose window team cannot be resolved must not offer a sharing channel \
                 governed by an unknown policy"
            );
            assert!(!dialog.can_direct_link_share(ctx));
        });
    });
}
