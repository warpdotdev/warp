//! TUI-facing snapshot of the signed-in account and its organization.
//!
//! Mirrors [`crate::tui::TuiMcpManager`]: one app-side singleton model that
//! joins data scattered across auth/workspace singletons into a single plain
//! snapshot the headless `warp_tui` front-end can read without depending on
//! `warp_server_auth` or the workspace client directly. Both the `/status`
//! inline menu and the zero-state login line read it.
//!
//! The snapshot is computed live from [`AuthStateProvider`] and
//! [`UserWorkspaces`] on every read, so it is never stale; the subscriptions
//! only drive `Updated` events so observers re-render when auth or workspace
//! metadata changes.

use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Plain, dependency-free snapshot of the current user and organization as
/// seen by the TUI. Every field is `Option` so callers can degrade gracefully
/// (logged-out users, dev builds, workspaces not yet loaded).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuiUserInfoSnapshot {
    /// Whether a user is currently signed in.
    pub is_logged_in: bool,
    /// Display name (falls back to email locally on the caller side).
    pub username: Option<String>,
    /// The signed-in user's email, when available.
    pub email: Option<String>,
    /// The signed-in user's stable id, formatted as a string.
    pub user_id: Option<String>,
    /// The current workspace (organization) name, when one is loaded.
    pub org: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum TuiUserInfoManagerEvent {
    Updated,
}

/// Singleton model exposing [`TuiUserInfoSnapshot`] to the `warp_tui` front-end.
///
/// It subscribes to [`AuthManager`] and [`UserWorkspaces`] so it emits an event
/// whenever the underlying auth or workspace state changes; the snapshot
/// itself is recomputed live on read.
pub struct TuiUserInfoManager;

impl TuiUserInfoManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, _, ctx| {
            me.refresh(ctx);
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, _, ctx| {
            me.refresh(ctx);
        });
        Self
    }

    /// Test-only constructor that skips subscribing to singletons which may
    /// not be registered in every test fixture. Reads still resolve live.
    #[cfg(any(test, all(feature = "tui", feature = "test-util")))]
    pub fn new_for_test(_ctx: &mut ModelContext<Self>) -> Self {
        Self
    }

    /// Computes the current user/org snapshot live from the auth and workspace
    /// singletons. Returns an all-`None`/logged-out snapshot when the
    /// singletons are absent or no user is signed in.
    pub fn snapshot(&self, ctx: &AppContext) -> TuiUserInfoSnapshot {
        let auth = AuthStateProvider::as_ref(ctx).get();
        let org = UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .map(|workspace| workspace.name.clone());
        TuiUserInfoSnapshot {
            is_logged_in: super::has_validated_identity(auth),
            username: auth.username_for_display(),
            email: auth.user_email(),
            user_id: auth.user_id().map(|uid| uid.as_string()),
            org,
        }
    }

    fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.emit(TuiUserInfoManagerEvent::Updated);
    }
}

impl Entity for TuiUserInfoManager {
    type Event = TuiUserInfoManagerEvent;
}

impl SingletonEntity for TuiUserInfoManager {}
