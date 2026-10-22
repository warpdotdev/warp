//! `turn.stop`. When a turn would be marked successful, its finalization is held until the stop
//! hooks finish, so a run that exits on success cannot exit between the hooks and the follow-up
//! they may ask for. A follow-up is queued as an ordinary prompt, behind anything a person already
//! queued, with provenance that says which binding asked for it. Each binding may ask for at most
//! its `loop_limit` follow-ups in one chain (the user's prompt and everything queued from it),
//! and a chain never grows past the host cap. Only a turn that completed fires the boundary.
//!
//! Lifecycle of a completed turn:
//! 1. [`hold_turn`] (from `controller::hold_or_finalize_turn`, when the stream or the last action
//!    finishes) builds the `turn.stop` envelope and holds `ConversationStatus::Success` if a
//!    binding matches; otherwise the controller marks `Success` right away.
//! 2. [`start_turn_stop`] (from `controller::start_held_turn_stop`, once the stream is cleaned
//!    up so a follow-up could start a new one) spawns the dispatch under the longest matching
//!    timeout.
//! 3. [`resolve_turn_stop`] (when the dispatch settles) queues the follow-up, if any, and says
//!    whether the controller should mark `Success` (`Finalize`), send the queue's head and then
//!    mark `Success` (`FollowUpQueued`), or do nothing (`Dropped`).
//!
//! A newer turn can start while the hooks run. Whether it did is read from live state when the
//! hooks answer (a prompt still waiting, a request sent since the hold, a stream or actions in
//! flight), never from a flag set earlier, so a turn that came and went in the meantime leaves
//! nothing behind.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ai::hooks::{
    ContinuationRequest, DiagnosticSeverity, DispatchOutcome, FollowUpChainId, HOOK_FOLLOW_UP_CAP,
    HookBindingId, HookBoundary, HookDiagnostic, HookEnvelope, HookVerdict, MergedEffects,
    PromptProvenance, TurnStopStatus,
};
use futures::FutureExt;
use futures::future::BoxFuture;
use warpui::r#async::Timer;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::envelope::{EnvelopeSite, envelope_for_turn_stop};
use super::{HookRuntime, longest_matching_timeout, owns_turn_lifecycle};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, QueuedQuery, QueuedQueryModel, QueuedQueryOrigin,
};
use crate::terminal::model::session::active_session::ActiveSession;

/// A turn whose success is waiting on its stop hooks.
struct HeldTurn {
    cancel: Arc<AtomicBool>,
    /// Present until the dispatch starts.
    pending: Option<PendingTurnStop>,
    /// How many exchanges the conversation had when the hold was taken. One more means a request
    /// went out since, so another turn owns the conversation.
    exchange_count: usize,
}

/// A `turn.stop` dispatch that has not started yet.
struct PendingTurnStop {
    envelope: HookEnvelope,
    timeout: Duration,
}

/// How many follow-ups each stop binding has asked for since the user's last prompt.
struct FollowUpChain {
    id: FollowUpChainId,
    per_binding: HashMap<HookBindingId, u32>,
    total: u32,
}

impl FollowUpChain {
    fn new() -> Self {
        Self {
            id: FollowUpChainId::new(),
            per_binding: HashMap::new(),
            total: 0,
        }
    }
}

/// Per-conversation `turn.stop` state.
#[derive(Default)]
pub(crate) struct TurnStops {
    held: HashMap<AIConversationId, HeldTurn>,
    chains: HashMap<AIConversationId, FollowUpChain>,
    /// Conversations whose stop hooks finished while a prompt's own hooks were still running.
    /// That prompt owns the status if it goes out; if it does not, the turn is still unfinished.
    deferred: HashSet<AIConversationId>,
}

impl TurnStops {
    pub(super) fn forget(&mut self, conversation_id: AIConversationId) {
        self.cancel(conversation_id);
        self.chains.remove(&conversation_id);
        self.deferred.remove(&conversation_id);
    }

    /// Drops the held turn and tells its hooks to stop.
    fn cancel(&mut self, conversation_id: AIConversationId) {
        if let Some(held) = self.held.remove(&conversation_id) {
            held.cancel.store(true, Ordering::Release);
        }
    }

    /// A person's prompt starts a new chain; a hook's follow-up stays in the current one.
    pub(super) fn start_chain(&mut self, conversation_id: AIConversationId) {
        self.chains.insert(conversation_id, FollowUpChain::new());
    }

    fn chain_mut(&mut self, conversation_id: AIConversationId) -> &mut FollowUpChain {
        self.chains
            .entry(conversation_id)
            .or_insert_with(FollowUpChain::new)
    }

    /// Takes the note that an earlier turn's finalization was left to the prompt now settling.
    pub(super) fn take_deferred(&mut self, conversation_id: AIConversationId) -> bool {
        self.deferred.remove(&conversation_id)
    }

    #[cfg(test)]
    pub(crate) fn is_held(&self, conversation_id: AIConversationId) -> bool {
        self.held.contains_key(&conversation_id)
    }

    #[cfg(test)]
    pub(crate) fn chain_total(&self, conversation_id: AIConversationId) -> u32 {
        self.chains
            .get(&conversation_id)
            .map_or(0, |chain| chain.total)
    }
}

/// Records a hold on `conversation_id`'s success when a `turn.stop` binding matches, and returns
/// whether the turn is now held. Only a turn that completed is a stop event; a cancelled or
/// failed turn is not.
pub(crate) fn hold_turn(
    conversation_id: AIConversationId,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) -> bool {
    if !HookRuntime::is_active(ctx) || !owns_turn_lifecycle(conversation_id, ctx) {
        return false;
    }
    let runtime = HookRuntime::handle(ctx);
    let snapshot = runtime.as_ref(ctx).snapshot(ctx);
    if !snapshot.has_bindings_for(HookBoundary::TurnStop) {
        return false;
    }
    let continuation_count = runtime
        .as_ref(ctx)
        .turn_stops
        .chains
        .get(&conversation_id)
        .map_or(0, |chain| chain.total);
    let site = EnvelopeSite::new(conversation_id, active_session);
    let envelope =
        envelope_for_turn_stop(TurnStopStatus::Completed, continuation_count, &site, ctx);
    let Some(timeout) = longest_matching_timeout(&snapshot, &envelope) else {
        return false;
    };
    let exchange_count = exchange_count(conversation_id, ctx);
    runtime.update(ctx, |runtime, _| {
        runtime.turn_stops.cancel(conversation_id);
        runtime.turn_stops.held.insert(
            conversation_id,
            HeldTurn {
                cancel: Arc::new(AtomicBool::new(false)),
                pending: Some(PendingTurnStop { envelope, timeout }),
                exchange_count,
            },
        );
    });
    true
}

fn exchange_count(conversation_id: AIConversationId, ctx: &AppContext) -> usize {
    BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&conversation_id)
        .map_or(0, |conversation| conversation.exchange_count())
}

/// What a held turn's stop hooks produced, tied to the hold they ran for.
pub(crate) struct TurnStopOutcome {
    ticket: Arc<AtomicBool>,
    /// `None` when the hooks did not finish within their timeout.
    outcome: Option<DispatchOutcome>,
}

/// Starts the `turn.stop` dispatch of a held turn, if one is waiting to start, bounded by the
/// longest matching binding timeout.
pub(crate) fn start_turn_stop(
    conversation_id: AIConversationId,
    ctx: &mut AppContext,
) -> Option<BoxFuture<'static, TurnStopOutcome>> {
    if !HookRuntime::is_installed(ctx) {
        return None;
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, ctx| {
        let snapshot = runtime.snapshot(ctx);
        let policy = runtime.dispatch_policy();
        let held = runtime.turn_stops.held.get_mut(&conversation_id)?;
        let PendingTurnStop { envelope, timeout } = held.pending.take()?;
        let ticket = held.cancel.clone();
        let dispatch = runtime
            .dispatcher()
            .dispatch(envelope, snapshot, policy, ticket.clone());
        Some(
            async move {
                let outcome = futures::select_biased! {
                    outcome = dispatch.fuse() => Some(outcome),
                    _ = Timer::after(timeout).fuse() => {
                        log::warn!(
                            "turn.stop hooks for {conversation_id} did not finish within \
                             {timeout:?}; the turn is finished without them"
                        );
                        ticket.store(true, Ordering::Release);
                        None
                    }
                };
                TurnStopOutcome { ticket, outcome }
            }
            .boxed(),
        )
    })
}

/// The next step once a held turn's `turn.stop` hooks have finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnStopResolution {
    /// Nothing: the hold was cancelled, another turn owns the conversation, or a prompt whose
    /// own hooks are still running will own it if it goes out and finish this turn if it does
    /// not.
    Dropped,
    /// Mark the turn successful.
    Finalize,
    /// A follow-up prompt was queued for the conversation. Send the queue's head if the
    /// conversation is idle, and mark the turn successful if nothing is left running.
    FollowUpQueued,
}

/// Applies the stop hooks' answer for `conversation_id`. An answer for a hold that was cancelled
/// or replaced in the meantime is dropped. A follow-up is queued whatever else is true; the
/// resolution says who finishes the turn.
pub(crate) fn resolve_turn_stop(
    conversation_id: AIConversationId,
    TurnStopOutcome { ticket, outcome }: TurnStopOutcome,
    ctx: &mut AppContext,
) -> TurnStopResolution {
    if !HookRuntime::is_installed(ctx) {
        return TurnStopResolution::Dropped;
    }
    let runtime = HookRuntime::handle(ctx);
    let Some(held) = runtime.update(ctx, |runtime, _| {
        let is_current = runtime
            .turn_stops
            .held
            .get(&conversation_id)
            .is_some_and(|held| Arc::ptr_eq(&held.cancel, &ticket));
        is_current
            .then(|| runtime.turn_stops.held.remove(&conversation_id))
            .flatten()
    }) else {
        return TurnStopResolution::Dropped;
    };
    let effects = match outcome {
        Some(outcome) if !outcome.cancelled => Some(outcome.effects),
        Some(_) | None => None,
    };
    let follow_up = effects.and_then(|effects| follow_up_for(conversation_id, effects, ctx));
    let queued = match follow_up {
        Some(follow_up) => {
            QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                queue.append(conversation_id, follow_up, ctx);
            });
            true
        }
        None => false,
    };
    // A request sent since the hold (a person's prompt, a resume, an injection) started a turn
    // that owns the conversation now, whether or not anything of it is still in flight.
    if exchange_count(conversation_id, ctx) != held.exchange_count {
        return TurnStopResolution::Dropped;
    }
    let prompt_pending = runtime.update(ctx, |runtime, _| {
        let pending = runtime.pending_prompts.is_pending(conversation_id);
        if pending {
            runtime.turn_stops.deferred.insert(conversation_id);
        }
        pending
    });
    if prompt_pending {
        return TurnStopResolution::Dropped;
    }
    if queued {
        TurnStopResolution::FollowUpQueued
    } else {
        TurnStopResolution::Finalize
    }
}

/// Logs when a stop hook's follow-up is still at the head of `conversation_id`'s queue after
/// the turn it belongs to was finished, so nothing sends it until the conversation's next drain.
pub(crate) fn warn_if_follow_up_still_queued(conversation_id: AIConversationId, ctx: &AppContext) {
    if QueuedQueryModel::as_ref(ctx)
        .queue(conversation_id)
        .first()
        .is_some_and(|row| row.origin() == QueuedQueryOrigin::HookFollowUp)
    {
        log::warn!(
            "a stop hook's follow-up for {conversation_id} could not be sent now and stays queued \
             until the conversation's next drain"
        );
    }
}

/// The follow-up prompt the stop hooks asked for, if the chain's limits allow one more. Effects
/// a stop hook cannot have are reported and ignored.
fn follow_up_for(
    conversation_id: AIConversationId,
    effects: MergedEffects,
    ctx: &mut AppContext,
) -> Option<QueuedQuery> {
    let messenger = HookRuntime::as_ref(ctx).messenger();
    let MergedEffects {
        verdict,
        verdict_binding,
        halt_turn,
        continuation,
        continuation_binding,
        ..
    } = effects;
    if halt_turn || verdict != HookVerdict::NoOpinion {
        let what = if halt_turn {
            "asked to stop the turn"
        } else {
            "gave a permission verdict"
        };
        log::info!("a turn.stop hook for {conversation_id} {what}, which means nothing on stop");
        if let Some(binding) = verdict_binding {
            messenger.diagnostic(HookDiagnostic {
                source: binding.source.clone(),
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "{binding} {what}; a stop hook can only ask for a follow-up, so it was ignored"
                ),
            });
        }
    }
    let (request, binding) = continuation.zip(continuation_binding)?;
    let allowed = HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        let chain = runtime.turn_stops.chain_mut(conversation_id);
        let refused = refused_by(&request, &binding, chain);
        if refused.is_none() {
            *chain.per_binding.entry(binding.clone()).or_default() += 1;
            chain.total += 1;
        }
        refused.map_or(Ok(chain.id), Err)
    });
    match allowed {
        Ok(chain_id) => {
            log::info!(
                "turn.stop hook {binding} queued a follow-up for {conversation_id} (chain \
                 {chain_id:?})"
            );
            Some(QueuedQuery::new_hook_follow_up(
                request.text,
                PromptProvenance::HookFollowUp { binding, chain_id },
            ))
        }
        Err(refusal) => {
            log::info!("turn.stop hook {binding} asked for a follow-up but {refusal}");
            messenger.diagnostic(HookDiagnostic {
                source: binding.source.clone(),
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "{binding} asked for a follow-up but {refusal}; the turn ends here"
                ),
            });
            None
        }
    }
}

/// Why one more follow-up from `binding` is not allowed in `chain`, if it is not.
fn refused_by(
    request: &ContinuationRequest,
    binding: &HookBindingId,
    chain: &FollowUpChain,
) -> Option<String> {
    if chain.total >= HOOK_FOLLOW_UP_CAP {
        return Some(format!(
            "this turn already had {} follow-ups, the most Warp allows ({HOOK_FOLLOW_UP_CAP})",
            chain.total
        ));
    }
    let own_limit = request
        .loop_limit
        .map_or(HOOK_FOLLOW_UP_CAP, |limit| limit.min(HOOK_FOLLOW_UP_CAP));
    let used = chain.per_binding.get(binding).copied().unwrap_or(0);
    (used >= own_limit).then(|| {
        format!("it already asked for {used}, its loop_limit of {own_limit} for this turn")
    })
}

/// Drops the hold on `conversation_id`'s turn, or the note that a waiting prompt owes it its
/// finalization, because something else is deciding the conversation's status. The hooks are
/// told to stop and their answer is ignored. Returns whether there was anything to drop.
pub(crate) fn cancel_held_turn(conversation_id: AIConversationId, ctx: &mut AppContext) -> bool {
    if !HookRuntime::is_installed(ctx) {
        return false;
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        let was_held = runtime.turn_stops.held.contains_key(&conversation_id);
        runtime.turn_stops.cancel(conversation_id);
        let was_deferred = runtime.turn_stops.deferred.remove(&conversation_id);
        was_held || was_deferred
    })
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
