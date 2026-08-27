//! Tracks whether the signed-in viewer has Factory access, fetched once per authenticated
//! session so cloud-run links can route to Platform for enrolled viewers while the Factory
//! waitlist exists. See `specs/APP-5583/PRODUCT.md` and `specs/APP-5583/TECH.md`.

use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::{AuthStateProvider, UserUid};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::factory::FactoryAccessResponse;

/// The viewer's Factory access, as last resolved for the current authenticated session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FactoryAccess {
    /// Not yet resolved, or the resolution attempt timed out, failed, or returned a malformed
    /// response. Cloud-run links stay on Oz for the rest of the session; the check is not
    /// retried.
    #[default]
    Unknown,
    Allowed,
    Denied,
}

/// Emitted when the startup access check produces a result. Cloud-run links resolve again at
/// click time regardless, so subscribers only need this to repaint anything that displays the
/// access state itself.
pub enum FactoryAccessModelEvent {
    Resolved,
}

/// Application-scoped singleton holding the eager, once-per-session Factory access check.
///
/// One request fires after the first `AuthManagerEvent::AuthComplete` (or immediately at
/// construction if a persisted session is already logged in) and its result is held for the
/// rest of that authenticated session: no refresh timer, no retry, no foreground re-fetch.
/// `reset` is called from `auth::log_out` so the next authenticated session starts a fresh
/// check.
pub struct FactoryAccessModel {
    access: FactoryAccess,
    requested: bool,
    /// The in-flight probe, if any. Aborted on a session change as defence in depth (saves the
    /// wasted request), but this alone cannot guarantee correctness: see [`Self::generation`].
    probe: Option<SpawnedFutureHandle>,
    /// Bumped every time a new session's probe starts (see [`Self::begin_session`]), i.e. on
    /// [`Self::reset`] and whenever [`Self::request_if_needed`] observes a different
    /// authenticated user than [`Self::session_user`]. A probe's completion is applied only if
    /// this still matches the generation captured when that probe started.
    ///
    /// This is the actual correctness guarantee, not [`Self::probe`]'s abort: `ctx.spawn`'s
    /// `Abortable` wraps only the background future, so once its result has resolved and been
    /// placed on the completion channel, `abort()` no longer has any effect. Without this
    /// generation check, a completion already in flight when the session changes would still
    /// land and overwrite the new session's access.
    generation: u64,
    /// The authenticated user the current session's probe/result pertains to, captured
    /// alongside `generation` at [`Self::begin_session`] and compared at completion time so a
    /// stale response is discarded even in the (extremely unlikely) case that `generation` were
    /// to wrap.
    session_user: Option<UserUid>,
}

impl FactoryAccessModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, event, ctx| {
            if matches!(event, AuthManagerEvent::AuthComplete) {
                me.request_if_needed(ctx);
            }
        });

        let mut me = Self {
            access: FactoryAccess::Unknown,
            requested: false,
            probe: None,
            generation: 0,
            session_user: None,
        };
        if AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            me.request_if_needed(ctx);
        }
        me
    }

    /// Test-only constructor that sets a specific access value directly, bypassing the network
    /// probe. Lets dependent-crate tests (e.g. the TUI's) simulate the probe having already
    /// resolved, or resolving later via [`Self::set_access_for_test`], without a mocked client.
    #[cfg(any(test, feature = "test-util"))]
    pub fn new_for_test(access: FactoryAccess) -> Self {
        Self {
            access,
            requested: true,
            probe: None,
            generation: 0,
            session_user: None,
        }
    }

    /// Test-only setter that simulates the probe resolving (or re-resolving) to a new access
    /// value after construction, e.g. to verify that cloud-run links re-resolve at click time
    /// rather than caching whatever destination was current when a run was spawned.
    #[cfg(any(test, feature = "test-util"))]
    pub fn set_access_for_test(&mut self, access: FactoryAccess) {
        self.access = access;
    }

    pub fn access(&self) -> FactoryAccess {
        self.access
    }

    /// Starts a probe if this authenticated user hasn't already had one started for it. Called
    /// on construction (a persisted session already logged in) and on every `AuthComplete`,
    /// including a token refresh for the same user (a no-op here) and an `AuthComplete` for a
    /// *different* user that never went through `auth::log_out` (e.g. a remote-server daemon
    /// handoff) — the latter must still start a fresh session rather than keep the outgoing
    /// user's entitlement.
    fn request_if_needed(&mut self, ctx: &mut ModelContext<Self>) {
        let user = AuthStateProvider::as_ref(ctx).get().user_id();
        if self.requested && user == self.session_user {
            return;
        }
        self.begin_session(user, ctx);
    }

    /// Starts a fresh probe for `user`, aborting (defence in depth) and superseding any probe
    /// still in flight for a prior session.
    fn begin_session(&mut self, user: Option<UserUid>, ctx: &mut ModelContext<Self>) {
        if let Some(probe) = self.probe.take() {
            probe.abort();
        }
        self.generation += 1;
        let generation = self.generation;
        self.session_user = user;
        self.requested = true;
        self.access = FactoryAccess::Unknown;

        let client = ServerApiProvider::as_ref(ctx).get_factory_client();
        self.probe = Some(ctx.spawn(
            async move { client.get_factory_access().await },
            move |me, result, ctx| me.complete_probe(generation, user, result, ctx),
        ));
    }

    /// Applies a probe's result, unless a newer session (a logout, or a different authenticated
    /// user) has since superseded the one it was captured for — see [`Self::generation`].
    fn complete_probe(
        &mut self,
        generation: u64,
        user: Option<UserUid>,
        result: anyhow::Result<FactoryAccessResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        if generation != self.generation || user != self.session_user {
            return;
        }
        self.probe = None;
        self.access = match result {
            Ok(response) if response.allowed => FactoryAccess::Allowed,
            Ok(_) => FactoryAccess::Denied,
            Err(error) => {
                log::info!(
                    "Failed to resolve Factory access; cloud-run links stay on Oz \
                     for this session: {error:#}"
                );
                FactoryAccess::Unknown
            }
        };
        ctx.emit(FactoryAccessModelEvent::Resolved);
    }

    /// Resets to `Unknown` on logout so the next authenticated session starts a fresh check.
    /// Bumps the generation (see [`Self::generation`]) so a response for the ending session can
    /// never land afterward and apply to the next session's access, and aborts a still in-flight
    /// probe as defence in depth.
    pub fn reset(&mut self) {
        if let Some(probe) = self.probe.take() {
            probe.abort();
        }
        self.generation += 1;
        self.session_user = None;
        self.access = FactoryAccess::Unknown;
        self.requested = false;
    }
}

impl Entity for FactoryAccessModel {
    type Event = FactoryAccessModelEvent;
}

impl SingletonEntity for FactoryAccessModel {}

#[cfg(test)]
#[path = "factory_access_tests.rs"]
mod tests;
