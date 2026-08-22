use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use warpui::{AddSingletonModel, App, SingletonEntity, WindowId};

use super::{TuiTeamScope, TuiTeamScopeEvent};
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::PrivacySettings;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::{TeamResolutionError, UserWorkspaces};
use crate::workspaces::workspace::Workspace;

type ObservedEvents = Rc<RefCell<Vec<TuiTeamScopeEvent>>>;

fn team(uid: i64, name: &str) -> Team {
    Team::from_local_cache(uid.into(), name.to_owned(), None, None, None)
}

fn workspace(teams: Vec<Team>) -> Workspace {
    Workspace::from_local_cache(
        "workspace_uid123456789".to_owned().into(),
        "test".to_owned(),
        Some(teams),
    )
}

/// Registers the singletons `TuiTeamScope` reads, seeded with `workspaces` so the current
/// workspace is selected the way `TeamUpdateManager` would have selected it.
fn register_singletons(app: &mut App, workspaces: Vec<Workspace>) {
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
}

fn register_scope(
    app: &mut App,
    requested_team: Option<&str>,
    window_id: WindowId,
) -> ObservedEvents {
    let events: ObservedEvents = Default::default();
    let events_for_subscription = events.clone();
    let requested_team = requested_team.map(str::to_owned);
    app.update(|ctx| {
        let scope = TuiTeamScope::register(requested_team, window_id, ctx);
        ctx.subscribe_to_model(&scope, move |_, event, _| {
            events_for_subscription.borrow_mut().push(event.clone());
        });
    });
    events
}

/// Stands in for a workspaces-metadata response landing, which is the only thing that tells
/// the TUI which teams the user is actually on.
fn deliver_workspaces_response(app: &mut App, workspaces: Vec<Workspace>) {
    let user_workspaces = UserWorkspaces::handle(&*app);
    user_workspaces.update(app, |user_workspaces, ctx| {
        user_workspaces.update_workspaces(workspaces, ctx);
    });
}

#[test]
fn resolves_the_sole_team_only_once_the_server_names_it() {
    let only_team = team(123, "Platform");
    let workspace = workspace(vec![only_team.clone()]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let window_id = WindowId::new();
        let events = register_scope(&mut app, None, window_id);

        assert!(
            events.borrow().is_empty(),
            "the cached workspace list can name a team the user has since left, so nothing \
             should resolve before a response lands"
        );
        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                None
            );
        });

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Resolved { team_uid: Some(uid) }] if *uid == only_team.uid
        ));
        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                Some(only_team.uid),
                "the window must be registered so per-window scope can answer for it"
            );
        });
    })
}

#[test]
fn resolves_teamless_when_the_user_is_on_no_team() {
    let workspace = workspace(vec![]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let window_id = WindowId::new();
        let events = register_scope(&mut app, None, window_id);

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Resolved { team_uid: None }]
        ));
    })
}

#[test]
fn refuses_to_resolve_when_the_user_is_on_more_than_one_team() {
    let workspace = workspace(vec![team(123, "Platform"), team(456, "Security")]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let window_id = WindowId::new();
        let events = register_scope(&mut app, None, window_id);

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Failed(
                TeamResolutionError::NoTeamSelected { .. }
            )]
        ));
        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                None,
                "a refused resolution must leave the window unregistered rather than land it \
                 on an arbitrary team"
            );
        });
    })
}

#[test]
fn a_requested_team_settles_an_otherwise_ambiguous_session() {
    let platform = team(123, "Platform");
    let security = team(456, "Security");
    let workspace = workspace(vec![platform, security.clone()]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let window_id = WindowId::new();
        let events = register_scope(&mut app, Some("Security"), window_id);

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Resolved { team_uid: Some(uid) }] if *uid == security.uid
        ));
        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                Some(security.uid),
                "the requested team wins over the first team in the workspace"
            );
        });
    })
}

#[test]
fn refuses_to_resolve_a_team_the_user_is_not_on() {
    let workspace = workspace(vec![team(123, "Platform")]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let window_id = WindowId::new();
        let events = register_scope(&mut app, Some("Growth"), window_id);

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Failed(
                TeamResolutionError::UnknownTeam { .. }
            )]
        ));
        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                None,
                "an unknown team must not silently fall back to the workspace's only team"
            );
        });
    })
}

/// The TUI has no team switcher, so the session stays on the team it started on even as
/// later polls arrive.
#[test]
fn later_responses_do_not_re_resolve_the_session() {
    let platform = team(123, "Platform");
    let security = team(456, "Security");
    let single_team_workspace = workspace(vec![platform.clone()]);
    let two_team_workspace = workspace(vec![platform.clone(), security]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![single_team_workspace.clone()]);
        let window_id = WindowId::new();
        let events = register_scope(&mut app, None, window_id);

        deliver_workspaces_response(&mut app, vec![single_team_workspace]);
        deliver_workspaces_response(&mut app, vec![two_team_workspace]);

        assert_eq!(
            events.borrow().len(),
            1,
            "gaining a second team mid-session must not retroactively refuse the session"
        );
        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                Some(platform.uid)
            );
        });
    })
}
