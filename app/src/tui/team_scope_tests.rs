use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{
    AddSingletonModel, App, AppContext, Element, Entity, SingletonEntity, TypedActionView, View,
    WeakViewHandle, WindowId,
};

use super::{TuiTeamScope, TuiTeamScopeEvent};
use crate::server::ids::ServerId;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::PrivacySettings;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::{TeamResolutionError, TeamScope, UserWorkspaces};
use crate::workspaces::workspace::Workspace;

type ObservedEvents = Rc<RefCell<Vec<TuiTeamScopeEvent>>>;

/// Stands in for the TUI's root view: `team_context` needs a view to locate a window from.
#[derive(Default)]
struct TeamScopeTestView;

impl Entity for TeamScopeTestView {
    type Event = ();
}

impl View for TeamScopeTestView {
    fn ui_name() -> &'static str {
        "TeamScopeTestView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for TeamScopeTestView {
    type Action = ();
}

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

fn create_window(app: &mut App) -> (WindowId, WeakViewHandle<TeamScopeTestView>) {
    let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| TeamScopeTestView);
    (window_id, view.downgrade())
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

/// The window's scope as a settings getter would see it. `team_uid_for_window` returns `None`
/// both for a window that was never registered and for one registered with no team, so it
/// cannot witness registration; `team_context` distinguishes them. `None` here means the
/// window is absent from `UserWorkspaces` entirely, `Some(None)` means registered and
/// teamless.
fn resolved_scope(app: &App, view: &WeakViewHandle<TeamScopeTestView>) -> Option<Option<ServerId>> {
    app.read(|ctx| {
        UserWorkspaces::as_ref(ctx)
            .team_context(view, ctx)
            .map(|context| context.team_uid())
    })
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
        let (window_id, view) = create_window(&mut app);
        let events = register_scope(&mut app, None, window_id);

        assert!(
            events.borrow().is_empty(),
            "the cached workspace list can name a team the user has since left, so nothing \
             should resolve before a response lands"
        );
        assert_eq!(
            resolved_scope(&app, &view),
            None,
            "the window should still be unregistered"
        );

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Resolved { team_uid: Some(uid) }] if *uid == only_team.uid
        ));
        assert_eq!(
            resolved_scope(&app, &view),
            Some(Some(only_team.uid)),
            "the window must be registered so per-window scope can answer for it"
        );
    })
}

#[test]
fn resolves_teamless_when_the_user_is_on_no_team() {
    let workspace = workspace(vec![]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let (window_id, view) = create_window(&mut app);
        let events = register_scope(&mut app, None, window_id);

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Resolved { team_uid: None }]
        ));
        assert_eq!(
            resolved_scope(&app, &view),
            Some(None),
            "a teamless session must still register, so scope reads resolve to a teamless \
             answer rather than to an absent window"
        );
    })
}

#[test]
fn refuses_to_resolve_when_the_user_is_on_more_than_one_team() {
    let workspace = workspace(vec![team(123, "Platform"), team(456, "Security")]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let (window_id, view) = create_window(&mut app);
        let events = register_scope(&mut app, None, window_id);

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Failed(
                TeamResolutionError::NoTeamSelected { .. }
            )]
        ));
        assert_eq!(
            resolved_scope(&app, &view),
            None,
            "a refused resolution must leave the window unregistered rather than land it on \
             an arbitrary team"
        );
    })
}

#[test]
fn a_requested_team_settles_an_otherwise_ambiguous_session() {
    let platform = team(123, "Platform");
    let security = team(456, "Security");
    let workspace = workspace(vec![platform, security.clone()]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let (window_id, view) = create_window(&mut app);
        let events = register_scope(&mut app, Some("Security"), window_id);

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Resolved { team_uid: Some(uid) }] if *uid == security.uid
        ));
        assert_eq!(
            resolved_scope(&app, &view),
            Some(Some(security.uid)),
            "the requested team wins over the first team in the workspace"
        );
    })
}

/// `WARP_TEAM=` and `--team "$TEAM"` with an unset variable both reach the flag as `Some("")`,
/// so the whole path has to treat blank as "not requested" rather than refuse to start.
#[test]
fn a_blank_requested_team_starts_the_session() {
    let only_team = team(123, "Platform");
    let workspace = workspace(vec![only_team.clone()]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let (window_id, view) = create_window(&mut app);
        let events = register_scope(&mut app, Some(""), window_id);

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Resolved { team_uid: Some(uid) }] if *uid == only_team.uid
        ));
        assert_eq!(resolved_scope(&app, &view), Some(Some(only_team.uid)));
    })
}

#[test]
fn refuses_to_resolve_a_team_the_user_is_not_on() {
    let workspace = workspace(vec![team(123, "Platform")]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let (window_id, view) = create_window(&mut app);
        let events = register_scope(&mut app, Some("Growth"), window_id);

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert!(matches!(
            events.borrow().as_slice(),
            [TuiTeamScopeEvent::Failed(
                TeamResolutionError::UnknownTeam { .. }
            )]
        ));
        assert_eq!(
            resolved_scope(&app, &view),
            None,
            "an unknown team must not silently fall back to the workspace's only team"
        );
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
        let (window_id, view) = create_window(&mut app);
        let events = register_scope(&mut app, None, window_id);

        deliver_workspaces_response(&mut app, vec![single_team_workspace]);
        deliver_workspaces_response(&mut app, vec![two_team_workspace]);

        assert_eq!(
            events.borrow().len(),
            1,
            "gaining a second team mid-session must not retroactively refuse the session"
        );
        assert_eq!(resolved_scope(&app, &view), Some(Some(platform.uid)));
    })
}
