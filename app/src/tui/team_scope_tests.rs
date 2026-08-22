use std::sync::Arc;

use settings::PrivatePreferences;
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{
    AddSingletonModel, App, AppContext, Element, Entity, SingletonEntity, TypedActionView, View,
    WeakViewHandle, WindowId,
};
use warpui_extras::user_preferences;

use super::{TuiTeamScope, restore_last_team_uid, store_last_team_uid};
use crate::server::ids::ServerId;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::PrivacySettings;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::{TeamScope, UserWorkspaces};
use crate::workspaces::workspace::Workspace;

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

fn register_singletons(app: &mut App, workspaces: Vec<Workspace>) {
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_| {
        PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
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

/// The window's team as a settings getter would see it. `team_uid_for_window` returns `None`
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
/// the TUI which teams the user is on.
fn deliver_workspaces_response(app: &mut App, workspaces: Vec<Workspace>) {
    let user_workspaces = UserWorkspaces::handle(&*app);
    user_workspaces.update(app, |user_workspaces, ctx| {
        user_workspaces.update_workspaces(workspaces, ctx);
    });
}

#[test]
fn registers_the_default_team_only_once_the_server_names_it() {
    let platform = team(123, "Platform");
    let workspace = workspace(vec![platform.clone(), team(456, "Security")]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let (window_id, view) = create_window(&mut app);
        app.update(|ctx| TuiTeamScope::register(window_id, ctx));

        assert_eq!(
            resolved_scope(&app, &view),
            None,
            "nothing should be registered before a metadata response lands"
        );

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert_eq!(
            resolved_scope(&app, &view),
            Some(Some(platform.uid)),
            "with no stored team the window takes the same default a new GUI window would"
        );
    })
}

#[test]
fn registers_teamless_when_the_user_is_on_no_team() {
    let workspace = workspace(vec![]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let (window_id, view) = create_window(&mut app);
        app.update(|ctx| TuiTeamScope::register(window_id, ctx));

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert_eq!(
            resolved_scope(&app, &view),
            Some(None),
            "a teamless session must still register, so scope reads resolve to a teamless \
             answer rather than to an absent window"
        );
    })
}

#[test]
fn a_stored_team_is_preferred_over_the_default() {
    let platform = team(123, "Platform");
    let security = team(456, "Security");
    let workspace = workspace(vec![platform, security.clone()]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        app.update(|ctx| store_last_team_uid(security.uid, ctx));
        let (window_id, view) = create_window(&mut app);
        app.update(|ctx| TuiTeamScope::register(window_id, ctx));

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert_eq!(
            resolved_scope(&app, &view),
            Some(Some(security.uid)),
            "the team the last session ended on wins over the workspace's first team"
        );
    })
}

/// A stored team survives leaving it, being removed from it, and signing in as somebody else,
/// so it is checked against the user's current teams before use.
///
/// Reconcile does not cover this. It runs *before* `TeamsChanged` is emitted, so a team
/// registered from that handler lands after the sweep that would have corrected it and waits
/// for the next poll — leaving the session on no team meanwhile. This test failed before the
/// check existed, which is how that ordering was found.
#[test]
fn a_stale_stored_team_falls_back_to_the_default() {
    let platform = team(123, "Platform");
    let workspace = workspace(vec![platform.clone()]);
    let departed_team = team(999, "Departed");

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        app.update(|ctx| store_last_team_uid(departed_team.uid, ctx));
        let (window_id, view) = create_window(&mut app);
        app.update(|ctx| TuiTeamScope::register(window_id, ctx));

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert_eq!(
            resolved_scope(&app, &view),
            Some(Some(platform.uid)),
            "a team the user is no longer in must not survive the first metadata response"
        );
    })
}

#[test]
fn switching_teams_moves_the_window_and_is_remembered() {
    let platform = team(123, "Platform");
    let security = team(456, "Security");
    let workspace = workspace(vec![platform, security.clone()]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        let (window_id, view) = create_window(&mut app);
        let scope = app.update(|ctx| TuiTeamScope::register(window_id, ctx));
        deliver_workspaces_response(&mut app, vec![workspace.clone()]);

        scope.update(&mut app, |scope, ctx| {
            scope.switch_to_team(security.uid, ctx);
        });

        assert_eq!(resolved_scope(&app, &view), Some(Some(security.uid)));
        app.read(|ctx| {
            assert_eq!(
                restore_last_team_uid(ctx),
                Some(security.uid),
                "the next session should start on the team just chosen"
            );
        });

        // A later poll must not drag the window back onto the default.
        deliver_workspaces_response(&mut app, vec![workspace]);
        assert_eq!(resolved_scope(&app, &view), Some(Some(security.uid)));
    })
}

#[test]
fn an_unreadable_stored_team_degrades_to_the_default() {
    let platform = team(123, "Platform");
    let workspace = workspace(vec![platform.clone()]);

    App::test((), |mut app| async move {
        register_singletons(&mut app, vec![workspace.clone()]);
        app.update(|ctx| {
            // A value written by a future version, or corrupted on disk.
            let _ = ctx
                .private_user_preferences()
                .write_value(super::LAST_TEAM_STORAGE_KEY, "not-a-team-uid".to_owned());
        });
        let (window_id, view) = create_window(&mut app);
        app.update(|ctx| TuiTeamScope::register(window_id, ctx));

        app.read(|ctx| assert_eq!(restore_last_team_uid(ctx), None));

        deliver_workspaces_response(&mut app, vec![workspace]);

        assert_eq!(resolved_scope(&app, &view), Some(Some(platform.uid)));
    })
}
