//! `session.start`. It fires once per conversation, just before the conversation's first request
//! goes out, and that request waits for it: a bounded barrier. A hook's `env` becomes the
//! hook-session environment every later hook in the conversation receives. A conversation that
//! already has exchanges (one restored from disk, or resumed) is not a new session and never
//! fires it.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ai::hooks::{DispatchOutcome, HookBoundary};
use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::{BoxFuture, Shared};
use warp_core::execution_mode::AppExecutionMode;
use warpui::r#async::Timer;
use warpui::{AppContext, EntityId, ModelHandle, SingletonEntity};

use super::envelope::{EnvelopeSite, envelope_for_session_start};
use super::{HookRuntime, longest_matching_timeout, owns_turn_lifecycle};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::terminal::model::session::active_session::ActiveSession;

/// The variables a conversation's `session.start` hooks set.
pub(crate) type SessionEnv = Vec<(String, String)>;

/// What every waiter on an in-flight `session.start` receives when it finishes.
type SessionDone = Shared<BoxFuture<'static, SessionEnv>>;

enum SessionStartState {
    /// The hooks are running. Dropping the sender wakes every waiter with no environment.
    InFlight {
        done: SessionDone,
        sender: oneshot::Sender<SessionEnv>,
    },
    /// The hooks ran, or were never going to run for this conversation.
    Done { env: SessionEnv },
}

/// Per-conversation `session.start` state.
#[derive(Default)]
pub(crate) struct SessionStarts {
    states: HashMap<AIConversationId, SessionStartState>,
}

impl SessionStarts {
    pub(super) fn env_for(&self, conversation_id: AIConversationId) -> SessionEnv {
        match self.states.get(&conversation_id) {
            Some(SessionStartState::Done { env }) => env.clone(),
            Some(SessionStartState::InFlight { .. }) | None => Vec::new(),
        }
    }

    pub(super) fn forget(&mut self, conversation_id: AIConversationId) {
        self.states.remove(&conversation_id);
    }

    /// Records the hooks' answer. A conversation forgotten while its hooks ran is left forgotten.
    fn finish(&mut self, conversation_id: AIConversationId, env: SessionEnv) {
        let Some(SessionStartState::InFlight { sender, .. }) = self.states.remove(&conversation_id)
        else {
            return;
        };
        let _ = sender.send(env.clone());
        self.states
            .insert(conversation_id, SessionStartState::Done { env });
    }

    #[cfg(test)]
    pub(crate) fn is_in_flight(&self, conversation_id: AIConversationId) -> bool {
        matches!(
            self.states.get(&conversation_id),
            Some(SessionStartState::InFlight { .. })
        )
    }
}

/// Starts `conversation_id`'s `session.start` hooks if they have not run yet and returns what the
/// conversation's first request must wait for: the hook-session environment, available once the
/// hooks finish or the longest matching timeout passes, whichever comes first. `None` means there
/// is nothing to wait for: hooks are off, no session hook applies, the hooks already ran, or the
/// conversation is not new.
///
/// The active hook set is pinned here first, so the first request of a run sees the project
/// hooks its posture admits.
pub(crate) fn barrier(
    conversation_id: AIConversationId,
    terminal_surface_id: EntityId,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) -> Option<BoxFuture<'static, SessionEnv>> {
    if !HookRuntime::is_installed(ctx) {
        return None;
    }
    let runtime = HookRuntime::handle(ctx);
    runtime.update(ctx, |runtime, ctx| {
        runtime.pin_for_run(Some(terminal_surface_id), ctx);
    });
    if !HookRuntime::is_active(ctx) {
        return None;
    }
    match runtime
        .as_ref(ctx)
        .session_starts
        .states
        .get(&conversation_id)
    {
        Some(SessionStartState::Done { .. }) => return None,
        Some(SessionStartState::InFlight { done, .. }) => {
            // A second request before the first finished waits on the same hooks.
            log::debug!("session.start for {conversation_id} is already running");
            return Some(wait_for(done.clone(), POLL_FOR_IN_FLIGHT));
        }
        None => {}
    }

    let mark_done = |ctx: &mut AppContext| {
        runtime.update(ctx, |runtime, _| {
            runtime
                .session_starts
                .states
                .insert(conversation_id, SessionStartState::Done { env: Vec::new() });
        });
    };
    if !is_new_conversation(conversation_id, ctx) {
        mark_done(ctx);
        return None;
    }
    let snapshot = runtime.as_ref(ctx).snapshot(ctx);
    if !snapshot.has_bindings_for(HookBoundary::SessionStart) {
        mark_done(ctx);
        return None;
    }
    let is_background_agent = ctx.has_singleton_model::<AppExecutionMode>()
        && AppExecutionMode::as_ref(ctx).is_autonomous();
    let site = EnvelopeSite::new(conversation_id, active_session);
    let envelope = envelope_for_session_start(is_background_agent, &site, ctx);
    let Some(timeout) = longest_matching_timeout(&snapshot, &envelope) else {
        mark_done(ctx);
        return None;
    };

    let (sender, receiver) = oneshot::channel();
    let done: SessionDone = receiver.map(|env| env.unwrap_or_default()).boxed().shared();
    let wait = wait_for(done.clone(), timeout);
    runtime.update(ctx, |runtime, ctx| {
        runtime.session_starts.states.insert(
            conversation_id,
            SessionStartState::InFlight { done, sender },
        );
        let dispatch = runtime.dispatcher().dispatch(
            envelope,
            snapshot,
            runtime.dispatch_policy(),
            Arc::new(AtomicBool::new(false)),
        );
        ctx.spawn(dispatch, move |runtime, outcome: DispatchOutcome, _| {
            let env = if outcome.cancelled {
                Vec::new()
            } else {
                outcome.effects.session_env
            };
            log::info!(
                "session.start finished for {conversation_id}: {} variable(s) set",
                env.len()
            );
            runtime.session_starts.finish(conversation_id, env);
        });
    });
    Some(wait)
}

/// How long a request that arrives while the hooks are already running waits for them. The first
/// request holds the real deadline; a later one only needs the answer if it comes quickly.
const POLL_FOR_IN_FLIGHT: Duration = Duration::from_secs(5);

/// The hooks' answer, or no environment once `timeout` has passed without one.
fn wait_for(done: SessionDone, timeout: Duration) -> BoxFuture<'static, SessionEnv> {
    async move {
        futures::select_biased! {
            env = done.fuse() => env,
            _ = Timer::after(timeout).fuse() => {
                log::warn!(
                    "session.start hooks did not finish within {timeout:?}; the first request \
                     goes out without their environment"
                );
                Vec::new()
            }
        }
    }
    .boxed()
}

/// Whether `conversation_id` is a conversation this client drives that has not sent a request
/// yet. One restored with exchanges, or resumed, is not a new session.
fn is_new_conversation(conversation_id: AIConversationId, ctx: &AppContext) -> bool {
    owns_turn_lifecycle(conversation_id, ctx)
        && BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| conversation.exchange_count() == 0)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
