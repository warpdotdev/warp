//! `prompt.submit`, and the start of a turn. Every prompt the controller sends starts a turn: a
//! turn id is allocated here and every later envelope in the conversation carries it. A prompt a
//! person typed, or a stop hook asked for, also runs the `prompt.submit` hooks, which can stop it
//! from being sent. The conversation's first request additionally waits for `session.start`.
//! When no hook applies the controller sends at once, on the same call stack as today.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ai::hooks::{
    DispatchOutcome, DispatchPolicy, HookBoundary, HookDispatcher, HookEnvelope, HookMessenger,
    HookNotice, HookNoticeKind, HookVerdict, MergedEffects, PromptProvenance, RegistrySnapshot,
};
use futures::FutureExt;
use futures::future::BoxFuture;
use warpui::{AppContext, EntityId, ModelHandle, SingletonEntity};

use super::envelope::{EnvelopeSite, envelope_for_prompt_submit};
use super::session::{self, SessionEnv};
use super::{HookRuntime, longest_matching_timeout, owns_turn_lifecycle};
use crate::ai::agent::conversation::AIConversationId;
use crate::terminal::model::session::active_session::ActiveSession;

/// Reason a halted prompt is reported with when the hook gave none.
pub(crate) const DEFAULT_HALT_REASON: &str = "An agent hook stopped this prompt from being sent.";

/// Reason a prompt is halted with when a hook asked for a confirmation nobody can give.
pub(crate) const ASK_UNAVAILABLE_HALT_REASON: &str = "An agent hook asked for confirmation before sending this prompt, which is not available for \
     prompts, so the prompt was not sent.";

/// A prompt someone wrote, as opposed to an input Warp built (a tool result, a resume, the server
/// side prompt of a run).
pub(crate) struct SubmittedPrompt {
    pub text: String,
    pub provenance: PromptProvenance,
}

/// What the controller must do before sending a prepared query.
pub(crate) enum TurnPreparation {
    /// Nothing waits: send now.
    Ready,
    /// Hooks run first; the decision says whether to send.
    Pending(BoxFuture<'static, PromptDecision>),
}

/// The outcome of a prompt's hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PromptDecision {
    /// Send the prompt.
    Proceed(PromptTicket),
    /// A hook stopped the prompt. The person was told why.
    Halted(PromptTicket),
    /// The prompt was given up on while its hooks ran: a newer prompt replaced it, or the
    /// conversation was cancelled.
    Cancelled,
}

/// Names one prompt whose hooks are running, so a decision can be matched to the prompt it was
/// made for.
#[derive(Debug, Clone)]
pub(crate) struct PromptTicket(Arc<AtomicBool>);

impl PartialEq for PromptTicket {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for PromptTicket {}

/// Prompts whose `prompt.submit` hooks are running, one per conversation. A newer prompt for the
/// same conversation replaces the older one, whose hooks are told to stop.
#[derive(Default)]
pub(crate) struct PendingPrompts {
    cancels: HashMap<AIConversationId, Arc<AtomicBool>>,
}

impl PendingPrompts {
    fn begin(&mut self, conversation_id: AIConversationId) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(replaced) = self.cancels.insert(conversation_id, cancel.clone()) {
            replaced.store(true, Ordering::Release);
        }
        cancel
    }

    /// Stops the pending prompt's hooks, if there is one.
    pub(super) fn forget(&mut self, conversation_id: AIConversationId) {
        if let Some(cancel) = self.cancels.remove(&conversation_id) {
            cancel.store(true, Ordering::Release);
        }
    }

    /// Drops the entry `ticket` names, if it is still the pending one.
    fn settle(&mut self, conversation_id: AIConversationId, ticket: &PromptTicket) {
        if self
            .cancels
            .get(&conversation_id)
            .is_some_and(|cancel| Arc::ptr_eq(cancel, &ticket.0))
        {
            self.cancels.remove(&conversation_id);
        }
    }

    fn is_pending(&self, conversation_id: AIConversationId) -> bool {
        self.cancels.contains_key(&conversation_id)
    }
}

/// Everything the `prompt.submit` dispatch needs once the session barrier has passed.
struct PromptDispatch {
    envelope: HookEnvelope,
    dispatcher: Arc<HookDispatcher>,
    snapshot: Arc<RegistrySnapshot>,
    policy: DispatchPolicy,
    messenger: Arc<dyn HookMessenger>,
    cancel: Arc<AtomicBool>,
}

/// Starts the turn `prompt` (or an input Warp built, when `None`) begins in `conversation_id`,
/// and says what the controller must wait for before sending. With hooks off this is one atomic
/// load and returns [`TurnPreparation::Ready`].
pub(crate) fn prepare_turn(
    prompt: Option<SubmittedPrompt>,
    conversation_id: AIConversationId,
    terminal_surface_id: EntityId,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) -> TurnPreparation {
    if !HookRuntime::is_installed(ctx) {
        return TurnPreparation::Ready;
    }
    // The barrier pins the run first, so this is the earliest point the active set is known.
    let barrier = session::barrier(conversation_id, terminal_surface_id, active_session, ctx);
    if !HookRuntime::is_active(ctx) {
        return TurnPreparation::Ready;
    }
    let runtime = HookRuntime::handle(ctx);
    let starts_chain = prompt
        .as_ref()
        .is_some_and(|prompt| prompt.provenance == PromptProvenance::User);
    runtime.update(ctx, |runtime, _| {
        runtime.start_turn(conversation_id);
        runtime.turn_stops.note_new_turn(conversation_id);
        if starts_chain {
            runtime.turn_stops.start_chain(conversation_id);
        }
    });

    let dispatch = prompt
        .filter(|_| owns_turn_lifecycle(conversation_id, ctx))
        .and_then(|prompt| prompt_dispatch(prompt, conversation_id, active_session, ctx));
    match (barrier, dispatch) {
        (None, None) => TurnPreparation::Ready,
        (barrier, None) => TurnPreparation::Pending(
            async move {
                if let Some(barrier) = barrier {
                    barrier.await;
                }
                PromptDecision::Proceed(PromptTicket(Arc::new(AtomicBool::new(false))))
            }
            .boxed(),
        ),
        (barrier, Some(dispatch)) => TurnPreparation::Pending(
            async move {
                let env = match barrier {
                    Some(barrier) => Some(barrier.await),
                    None => None,
                };
                run_prompt_dispatch(dispatch, env, conversation_id).await
            }
            .boxed(),
        ),
    }
}

/// Prepares the `prompt.submit` dispatch for `prompt`, or `None` when no binding would run.
fn prompt_dispatch(
    prompt: SubmittedPrompt,
    conversation_id: AIConversationId,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) -> Option<PromptDispatch> {
    let runtime = HookRuntime::handle(ctx);
    let snapshot = runtime.as_ref(ctx).snapshot(ctx);
    if !snapshot.has_bindings_for(HookBoundary::PromptSubmit) {
        return None;
    }
    let site = EnvelopeSite::new(conversation_id, active_session);
    let envelope =
        envelope_for_prompt_submit(prompt.text, Vec::new(), prompt.provenance, &site, ctx);
    longest_matching_timeout(&snapshot, &envelope)?;
    let (dispatcher, policy, messenger, cancel) = runtime.update(ctx, |runtime, _| {
        (
            runtime.dispatcher(),
            runtime.dispatch_policy(),
            runtime.messenger(),
            runtime.pending_prompts.begin(conversation_id),
        )
    });
    Some(PromptDispatch {
        envelope,
        dispatcher,
        snapshot,
        policy,
        messenger,
        cancel,
    })
}

/// Runs the prompt's hooks, once the session environment (if any was waited for) is known.
async fn run_prompt_dispatch(
    dispatch: PromptDispatch,
    session_env: Option<SessionEnv>,
    conversation_id: AIConversationId,
) -> PromptDecision {
    let PromptDispatch {
        mut envelope,
        dispatcher,
        snapshot,
        policy,
        messenger,
        cancel,
    } = dispatch;
    if let Some(env) = session_env {
        envelope.session_env = env;
    }
    let outcome: DispatchOutcome = dispatcher
        .dispatch(envelope, snapshot, policy, cancel.clone())
        .await;
    if outcome.cancelled || cancel.load(Ordering::Acquire) {
        return PromptDecision::Cancelled;
    }
    let ticket = PromptTicket(cancel);
    match halt_reason(&outcome.effects) {
        Some(reason) => {
            messenger.notify(HookNotice {
                conversation_id,
                action_id: None,
                kind: HookNoticeKind::Denied,
                text: reason,
            });
            PromptDecision::Halted(ticket)
        }
        None => PromptDecision::Proceed(ticket),
    }
}

/// Why the prompt must not be sent, if the hooks said so. A `deny` on this boundary means the
/// same as `continue: false`. An `ask` cannot be answered for a prompt, so it halts too rather
/// than being dropped silently. `Defer` never reaches here: the dispatcher refuses it on a
/// headless surface and reports it elsewhere.
fn halt_reason(effects: &MergedEffects) -> Option<String> {
    let first_reason = || {
        effects
            .reasons
            .first()
            .map(|reason| reason.text.clone())
            .unwrap_or_else(|| DEFAULT_HALT_REASON.to_owned())
    };
    match effects.verdict {
        HookVerdict::Deny => Some(first_reason()),
        HookVerdict::Ask => Some(
            effects
                .reasons
                .first()
                .map(|reason| format!("{ASK_UNAVAILABLE_HALT_REASON} ({})", reason.text))
                .unwrap_or_else(|| ASK_UNAVAILABLE_HALT_REASON.to_owned()),
        ),
        HookVerdict::NoOpinion | HookVerdict::Allow | HookVerdict::Defer => {
            effects.halt_turn.then(first_reason)
        }
    }
}

/// Settles the bookkeeping for a decision and says whether the prompt goes out. A prompt that
/// does not go out may leave a turn whose finalization was waiting on it; the controller is
/// told so it can finish that turn.
pub(crate) fn finish_turn_preparation(
    conversation_id: AIConversationId,
    decision: &PromptDecision,
    ctx: &mut AppContext,
) -> PromptOutcome {
    if !HookRuntime::is_installed(ctx) {
        return match decision {
            PromptDecision::Proceed(_) => PromptOutcome::Send,
            PromptDecision::Halted(_) | PromptDecision::Cancelled => PromptOutcome::DoNotSend {
                finalize_turn: false,
            },
        };
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, _| match decision {
        PromptDecision::Proceed(ticket) => {
            runtime.pending_prompts.settle(conversation_id, ticket);
            runtime.turn_stops.turn_started(conversation_id);
            PromptOutcome::Send
        }
        PromptDecision::Halted(ticket) => {
            runtime.pending_prompts.settle(conversation_id, ticket);
            PromptOutcome::DoNotSend {
                finalize_turn: runtime.turn_stops.turn_did_not_start(conversation_id),
            }
        }
        PromptDecision::Cancelled => PromptOutcome::DoNotSend {
            finalize_turn: runtime.turn_stops.turn_did_not_start(conversation_id),
        },
    })
}

/// What the controller does with a prepared query once its hooks have decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptOutcome {
    Send,
    DoNotSend {
        /// A turn was waiting for this prompt to take over the conversation; it did not, so
        /// the turn must be finished.
        finalize_turn: bool,
    },
}

/// Whether a prompt's hooks are running for `conversation_id`.
pub(crate) fn is_pending(conversation_id: AIConversationId, ctx: &AppContext) -> bool {
    HookRuntime::is_installed(ctx)
        && HookRuntime::as_ref(ctx)
            .pending_prompts
            .is_pending(conversation_id)
}

/// Stops the hooks of a prompt still waiting to be sent in `conversation_id`.
pub(crate) fn cancel_pending(conversation_id: AIConversationId, ctx: &mut AppContext) {
    if !HookRuntime::is_installed(ctx) {
        return;
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime.pending_prompts.forget(conversation_id);
    });
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
