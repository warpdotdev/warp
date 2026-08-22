//! Gives the headless TUI's single window a team, and remembers the choice across sessions.
//!
//! The GUI registers every window with [`UserWorkspaces`] as it is created (`RootView::new`),
//! which is what lets a per-window team scope answer for it. The TUI has no `RootView`, so
//! until this model runs its window is absent from `UserWorkspaces`'s window table entirely
//! and any scope question about it resolves to nothing — silently, since an unregistered
//! window looks exactly like a teamless one to every caller that does not check.
//!
//! Registration cannot happen when the window is created: the TUI mounts before login, and the
//! user's teams are only known once a workspaces-metadata response lands. So this model waits
//! for [`UserWorkspacesEvent::TeamsChanged`] — emitted whenever a response is applied — and
//! registers then, preferring the team the last TUI session ended on.
//!
//! There is no resolution state machine. `UserWorkspaces`'s window table is the single source
//! of truth: [`UserWorkspaces::is_window_registered`] answers "has this window got a team
//! yet", and registration is a no-op once it has. `/team` reassigns afterwards and this model
//! must never undo that, which falls out of the same check rather than needing to be arranged.
//!
//! A stored team can go stale — the user leaves it, an admin removes them, or they log in as
//! somebody else — so it is only used while the user still belongs to it. Not for safety:
//! a stale uid cannot leak another team's settings, because `UserWorkspaces::team_from_uid`
//! searches only the current workspace and so resolves it to no team at all. It is for
//! promptness. `reconcile_window_team_assignments` would eventually correct a stale
//! assignment, but it runs *before* `TeamsChanged` is emitted, so a team registered from this
//! handler misses the sweep that would have fixed it and waits for the next poll — leaving the
//! session on no team for up to a full poll interval instead of on its default team.

use warp_core::user_preferences::GetUserPreferences as _;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity, WindowId};

use crate::server::ids::ServerId;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

/// Where the last-used team is stored. Local rather than cloud-synced: it records what this
/// machine's TUI was doing, mirroring how the GUI persists `WindowSnapshot::team_uid`
/// alongside the rest of a window's local state.
const LAST_TEAM_STORAGE_KEY: &str = "TuiLastTeamUid";

/// Singleton that gives the TUI window a team once the server names the user's teams, and
/// records explicit `/team` choices for the next session.
pub struct TuiTeamScope {
    window_id: WindowId,
}

impl TuiTeamScope {
    /// Registers the singleton and arranges for `window_id` to be given a team as soon as the
    /// user's teams are known.
    pub fn register(window_id: WindowId, ctx: &mut AppContext) -> ModelHandle<Self> {
        ctx.add_singleton_model(move |ctx| Self::new(window_id, ctx))
    }

    fn new(window_id: WindowId, ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
            if matches!(event, UserWorkspacesEvent::TeamsChanged) {
                me.register_window_if_unset(ctx);
            }
        });
        Self { window_id }
    }

    /// Switches the window onto `team_uid` and remembers it for the next session.
    pub fn switch_to_team(&self, team_uid: ServerId, ctx: &mut ModelContext<Self>) {
        let window_id = self.window_id;
        UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
            user_workspaces.switch_window_to_team(window_id, team_uid, ctx);
        });
        store_last_team_uid(team_uid, ctx);
    }

    fn register_window_if_unset(&mut self, ctx: &mut ModelContext<Self>) {
        let window_id = self.window_id;
        if UserWorkspaces::as_ref(ctx).is_window_registered(window_id) {
            return;
        }
        let restored_team_uid = restore_last_team_uid(ctx);
        let user_workspaces = UserWorkspaces::as_ref(ctx);
        let team_uid = restored_team_uid
            .filter(|team_uid| user_workspaces.team_from_uid(*team_uid).is_some())
            .or_else(|| user_workspaces.inherited_or_default_team_uid(None));
        UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, team_uid, ctx);
        });
    }
}

/// Reads the last-used team. Anything unreadable or unparseable degrades to `None` rather than
/// failing, matching how the GUI restores a persisted `WindowSnapshot::team_uid`.
pub(crate) fn restore_last_team_uid(ctx: &AppContext) -> Option<ServerId> {
    ctx.private_user_preferences()
        .read_value(LAST_TEAM_STORAGE_KEY)
        .ok()
        .flatten()
        .and_then(|stored| serde_json::from_str::<ServerId>(&stored).ok())
}

pub(crate) fn store_last_team_uid(team_uid: ServerId, ctx: &AppContext) {
    let Ok(serialized) = serde_json::to_string(&team_uid) else {
        return;
    };
    let _ = ctx
        .private_user_preferences()
        .write_value(LAST_TEAM_STORAGE_KEY, serialized);
}

impl Entity for TuiTeamScope {
    /// Registration is observed through `UserWorkspaces`'s own `WindowTeamChanged`, which is
    /// what scoped consumers already subscribe to, so this model has nothing of its own to
    /// announce.
    type Event = ();
}

impl SingletonEntity for TuiTeamScope {}

#[cfg(test)]
#[path = "team_scope_tests.rs"]
mod tests;
