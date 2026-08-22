//! Resolves the headless TUI's single window onto a team and registers it.
//!
//! The GUI registers every window with [`UserWorkspaces`] as it is created
//! (`RootView::new`), which is what lets a per-window team scope answer for it. The TUI has
//! no `RootView`, so until this model runs its window is absent from
//! `UserWorkspaces`'s window table entirely and any scope question about it resolves to
//! nothing — silently, since an unregistered window looks exactly like a teamless one to
//! every caller that does not check.
//!
//! Resolution cannot happen when the window is created: the TUI mounts before login, and the
//! user's teams are only known once a workspaces-metadata response lands. So this model waits
//! for [`UserWorkspacesEvent::TeamsChanged`] — emitted whenever a response is applied — and
//! resolves against that server-authoritative list. It deliberately does not resolve from the
//! locally cached workspace list, which can name a team the user has since left.
//!
//! The result is settled once and never revisited. The TUI has no team switcher, so a session
//! stays on the team it started on; a caller that needs the window's team reads it back from
//! `UserWorkspaces` rather than caching anything derived from it here.

use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity, WindowId};

use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::{
    TeamResolutionError, UserWorkspaces, UserWorkspacesEvent,
};

/// Events emitted by [`TuiTeamScope`].
#[derive(Debug, Clone)]
pub enum TuiTeamScopeEvent {
    /// The window was registered with the resolved team, which may be `None` when the user
    /// genuinely belongs to no team. Scope questions about the window answer from here on.
    Resolved { team_uid: Option<ServerId> },
    /// No single team could be resolved. The session must not continue: see
    /// [`TeamResolutionError`] for why refusing beats guessing.
    Failed(TeamResolutionError),
}

enum ResolutionState {
    /// Waiting for a workspaces-metadata response to name the user's teams.
    Pending,
    /// Resolution produced an answer, successful or not. It is never revisited.
    Settled,
}

/// Singleton that owns the TUI window's one-time team resolution.
pub struct TuiTeamScope {
    /// The team named on the command line, if any.
    requested_team: Option<String>,
    window_id: WindowId,
    state: ResolutionState,
}

impl TuiTeamScope {
    /// Registers the singleton and begins resolving `requested_team` for `window_id`.
    ///
    /// Returns the handle so the caller can subscribe to [`TuiTeamScopeEvent`]; a
    /// `Failed` event is the caller's cue to stop the session.
    pub fn register(
        requested_team: Option<String>,
        window_id: WindowId,
        ctx: &mut AppContext,
    ) -> ModelHandle<Self> {
        ctx.add_singleton_model(move |ctx| Self::new(requested_team, window_id, ctx))
    }

    fn new(
        requested_team: Option<String>,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
            if matches!(event, UserWorkspacesEvent::TeamsChanged) {
                me.resolve(ctx);
            }
        });
        Self {
            requested_team,
            window_id,
            state: ResolutionState::Pending,
        }
    }

    fn resolve(&mut self, ctx: &mut ModelContext<Self>) {
        if matches!(self.state, ResolutionState::Settled) {
            return;
        }
        let resolved =
            UserWorkspaces::as_ref(ctx).resolve_requested_team(self.requested_team.as_deref());
        self.state = ResolutionState::Settled;

        match resolved {
            Ok(team_uid) => {
                let window_id = self.window_id;
                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    user_workspaces.register_window(window_id, team_uid, ctx);
                });
                ctx.emit(TuiTeamScopeEvent::Resolved { team_uid });
            }
            Err(error) => ctx.emit(TuiTeamScopeEvent::Failed(error)),
        }
    }
}

impl Entity for TuiTeamScope {
    type Event = TuiTeamScopeEvent;
}

impl SingletonEntity for TuiTeamScope {}

#[cfg(test)]
#[path = "team_scope_tests.rs"]
mod tests;
