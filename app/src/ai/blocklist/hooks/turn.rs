//! `turn.stop`. When a turn would be marked successful, its finalization is held until the stop
//! hooks finish, so a run that exits on success cannot exit between the hooks and the follow-up
//! they may ask for. A follow-up is queued as an ordinary prompt, behind anything a person already
//! queued, with provenance that says which binding asked for it. Each binding may ask for at most
//! its `loop_limit` follow-ups in one chain (the user's prompt and everything queued from it),
//! and a chain never grows past the host cap.
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
//!
//! A turn that was cancelled or failed also fires the boundary, with the matching status, but
//! only to be observed: its status is already set, so nothing is held, and a follow-up the hooks
//! ask for is refused. [`hold_ended_turn`] records the stop when the status changes,
//! [`start_ended_turn_stops`] starts it once the stream that ended the turn is cleaned up (so
//! the stream's own `response.complete` goes first), and [`finish_ended_turn_stop`] reports what
//! the hooks asked for in vain. An Oz run that is about to exit on that status waits for the
//! stop through [`ended_turn_stops_settled`], so a hook that records the end of every turn sees
//! the last one. Each turn fires at most one stop: a turn whose completed stop ran, and which was
//! then cancelled or failed while the hooks were still running, does not fire a second one.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ai::hooks::{
    ContinuationRequest, DiagnosticSeverity, DispatchOutcome, FollowUpChainId, HOOK_FOLLOW_UP_CAP,
    HookBindingId, HookBoundary, HookDiagnostic, HookEnvelope, HookMessenger, HookVerdict,
    MergedEffects, PromptProvenance, TurnStopStatus,
};
use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::Shared;
use warpui::r#async::{BoxFuture, Timer};
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::envelope::{EnvelopeSite, envelope_for_turn_stop};
use super::{HookRuntime, longest_matching_timeout, owns_turn_lifecycle};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, QueuedQuery, QueuedQueryModel, QueuedQueryOrigin,
};
use crate::terminal::model::session::active_session::ActiveSession;

/// How much longer than its hooks' timeout a run exit waits for an ended turn's stop, which
/// covers the time between the status change and the dispatch's start.
#[cfg_attr(
    target_family = "wasm",
    allow(dead_code, reason = "run exit waiting is unavailable in WASM builds")
)]
const ENDED_TURN_STOP_EXIT_GRACE: Duration = Duration::from_secs(5);

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

/// The `turn.stop` of a turn that was cancelled or failed. Nothing is decided from its answer.
struct EndedTurnStop {
    cancel: Arc<AtomicBool>,
    /// Present until the dispatch starts.
    pending: Option<PendingTurnStop>,
    /// The dispatch's bound, kept for [`ended_turn_stops_settled`] once `pending` is taken.
    #[cfg_attr(
        target_family = "wasm",
        allow(dead_code, reason = "run exit waiting is unavailable in WASM builds")
    )]
    timeout: Duration,
    /// Settles [`Self::done`] when dropped, which happens once the dispatch is over or the stop
    /// is forgotten before it started.
    finished: Option<oneshot::Sender<()>>,
    #[cfg_attr(
        target_family = "wasm",
        allow(dead_code, reason = "run exit waiting is unavailable in WASM builds")
    )]
    done: Shared<BoxFuture<'static, ()>>,
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
    /// The stops of turns that were cancelled or failed, waiting to start or running. A new turn
    /// can end while an earlier one's hooks still run, so there can be several.
    ended: HashMap<AIConversationId, Vec<EndedTurnStop>>,
    /// How many exchanges each conversation had when its last stop started. A turn is told from
    /// the next by that count, so a turn whose stop already ran fires no second one when its
    /// status changes again.
    stopped_at: HashMap<AIConversationId, usize>,
    chains: HashMap<AIConversationId, FollowUpChain>,
    /// Conversations whose stop hooks finished while a prompt's own hooks were still running.
    /// That prompt owns the status if it goes out; if it does not, the turn is still unfinished.
    deferred: HashSet<AIConversationId>,
    /// Conversations whose stop hooks asked for a follow-up and whose turn the controller has not
    /// finished yet. Their status still says in progress, but nothing of the turn is left to
    /// interrupt.
    awaiting_follow_up: HashSet<AIConversationId>,
}

impl TurnStops {
    pub(super) fn forget(&mut self, conversation_id: AIConversationId) {
        self.cancel(conversation_id);
        for stop in self.ended.remove(&conversation_id).unwrap_or_default() {
            stop.cancel.store(true, Ordering::Release);
        }
        self.stopped_at.remove(&conversation_id);
        self.chains.remove(&conversation_id);
        self.deferred.remove(&conversation_id);
        self.awaiting_follow_up.remove(&conversation_id);
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

    /// How many stops of ended turns are waiting to start or running for `conversation_id`.
    #[cfg(test)]
    pub(crate) fn ended_stop_count(&self, conversation_id: AIConversationId) -> usize {
        self.ended.get(&conversation_id).map_or(0, Vec::len)
    }

    #[cfg(test)]
    pub(crate) fn chain_total(&self, conversation_id: AIConversationId) -> u32 {
        self.chains
            .get(&conversation_id)
            .map_or(0, |chain| chain.total)
    }
}

/// Records a hold on `conversation_id`'s success when a `turn.stop` binding matches, and returns
/// whether the turn is now held.
pub(crate) fn hold_turn(
    conversation_id: AIConversationId,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) -> bool {
    let Some(pending) = pending_stop_for(
        conversation_id,
        TurnStopStatus::Completed,
        active_session,
        ctx,
    ) else {
        return false;
    };
    let exchange_count = exchange_count(conversation_id, ctx);
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime.turn_stops.cancel(conversation_id);
        runtime.turn_stops.held.insert(
            conversation_id,
            HeldTurn {
                cancel: Arc::new(AtomicBool::new(false)),
                pending: Some(pending),
                exchange_count,
            },
        );
    });
    true
}

/// The `turn.stop` envelope for `conversation_id` ending with `status`, and how long its hooks
/// may take, when a binding would run for it.
fn pending_stop_for(
    conversation_id: AIConversationId,
    status: TurnStopStatus,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &AppContext,
) -> Option<PendingTurnStop> {
    if !HookRuntime::is_active(ctx) || !owns_turn_lifecycle(conversation_id, ctx) {
        return None;
    }
    let runtime = HookRuntime::as_ref(ctx);
    let snapshot = runtime.snapshot(ctx);
    if !snapshot.has_bindings_for(HookBoundary::TurnStop) {
        return None;
    }
    let continuation_count = runtime
        .turn_stops
        .chains
        .get(&conversation_id)
        .map_or(0, |chain| chain.total);
    let site = EnvelopeSite::new(conversation_id, active_session);
    let envelope = envelope_for_turn_stop(status, continuation_count, &site, ctx);
    let timeout = longest_matching_timeout(&snapshot, &envelope)?;
    Some(PendingTurnStop { envelope, timeout })
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
        let held = runtime.turn_stops.held.get_mut(&conversation_id)?;
        let pending = held.pending.take()?;
        let ticket = held.cancel.clone();
        let exchange_count = held.exchange_count;
        runtime
            .turn_stops
            .stopped_at
            .insert(conversation_id, exchange_count);
        let dispatch = dispatch_within_timeout(runtime, conversation_id, pending, &ticket, ctx);
        let future: BoxFuture<'static, TurnStopOutcome> = Box::pin(async move {
            let outcome = dispatch.await;
            TurnStopOutcome { ticket, outcome }
        });
        Some(future)
    })
}

/// Runs `pending`'s hooks, for at most its timeout. On timeout the hooks are told to stop and
/// the result is `None`.
fn dispatch_within_timeout(
    runtime: &HookRuntime,
    conversation_id: AIConversationId,
    PendingTurnStop { envelope, timeout }: PendingTurnStop,
    ticket: &Arc<AtomicBool>,
    ctx: &AppContext,
) -> BoxFuture<'static, Option<DispatchOutcome>> {
    let dispatch = runtime.dispatcher().dispatch(
        envelope,
        runtime.snapshot(ctx),
        runtime.is_headless(ctx),
        ticket.clone(),
    );
    let ticket = ticket.clone();
    Box::pin(async move {
        futures::select_biased! {
            outcome = dispatch.fuse() => Some(outcome),
            _ = Timer::after(timeout).fuse() => {
                log::warn!(
                    "turn.stop hooks for {conversation_id} did not finish within {timeout:?}; \
                     the turn is finished without them"
                );
                ticket.store(true, Ordering::Release);
                None
            }
        }
    })
}

/// The `turn.stop` status a conversation status change reports, when the change ends a turn
/// without completing it: a turn that was under way, in whatever form, and is now cancelled or
/// failed.
fn ended_turn_status(
    prev_status: &ConversationStatus,
    new_status: &ConversationStatus,
) -> Option<TurnStopStatus> {
    if prev_status.is_done() {
        return None;
    }
    match new_status {
        ConversationStatus::Cancelled => Some(TurnStopStatus::Cancelled),
        ConversationStatus::Error => Some(TurnStopStatus::Error),
        ConversationStatus::InProgress
        | ConversationStatus::Success
        | ConversationStatus::TransientError
        | ConversationStatus::Blocked { .. }
        | ConversationStatus::WaitingForEvents => None,
    }
}

/// Records the `turn.stop` of a turn that a status change from `prev_status` to `new_status`
/// ended without completing, when a binding matches, and returns whether one is now waiting to
/// start. A turn whose stop is already recorded fires no second one, whether its completed stop
/// ran and it was then cancelled or failed under those hooks, or the same status change is
/// reported again.
pub(crate) fn hold_ended_turn(
    conversation_id: AIConversationId,
    prev_status: &ConversationStatus,
    new_status: &ConversationStatus,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) -> bool {
    let Some(status) = ended_turn_status(prev_status, new_status) else {
        return false;
    };
    if !HookRuntime::is_installed(ctx) {
        return false;
    }
    let exchange_count = exchange_count(conversation_id, ctx);
    if HookRuntime::as_ref(ctx)
        .turn_stops
        .stopped_at
        .get(&conversation_id)
        == Some(&exchange_count)
    {
        log::debug!(
            "turn.stop for {conversation_id} ending as {status:?} is not fired: the turn's stop \
             was already recorded"
        );
        return false;
    }
    let Some(pending) = pending_stop_for(conversation_id, status, active_session, ctx) else {
        return false;
    };
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime
            .turn_stops
            .stopped_at
            .insert(conversation_id, exchange_count);
        let (finished, done) = oneshot::channel();
        let done: BoxFuture<'static, ()> = Box::pin(async move {
            let _ = done.await;
        });
        let timeout = pending.timeout;
        runtime
            .turn_stops
            .ended
            .entry(conversation_id)
            .or_default()
            .push(EndedTurnStop {
                cancel: Arc::new(AtomicBool::new(false)),
                pending: Some(pending),
                timeout,
                finished: Some(finished),
                done: done.shared(),
            });
        true
    })
}

/// What an ended turn's stop hooks produced, tied to the stop they ran for.
pub(crate) struct EndedTurnStopOutcome {
    ticket: Arc<AtomicBool>,
    /// `None` when the hooks did not finish within their timeout.
    outcome: Option<DispatchOutcome>,
    /// Settles the stop's `done` when dropped, after [`finish_ended_turn_stop`] has let go of
    /// the stop.
    _finished: Option<oneshot::Sender<()>>,
}

/// Starts the `turn.stop` dispatches of `conversation_id`'s ended turns that are waiting to
/// start, each bounded by the longest matching binding timeout.
pub(crate) fn start_ended_turn_stops(
    conversation_id: AIConversationId,
    ctx: &mut AppContext,
) -> Vec<BoxFuture<'static, EndedTurnStopOutcome>> {
    if !HookRuntime::is_installed(ctx) {
        return Vec::new();
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, ctx| {
        let Some(stops) = runtime.turn_stops.ended.get_mut(&conversation_id) else {
            return Vec::new();
        };
        let mut started = Vec::new();
        for stop in stops.iter_mut() {
            let Some(pending) = stop.pending.take() else {
                continue;
            };
            started.push((pending, stop.cancel.clone(), stop.finished.take()));
        }
        started
            .into_iter()
            .map(|(pending, ticket, finished)| {
                let dispatch =
                    dispatch_within_timeout(runtime, conversation_id, pending, &ticket, ctx);
                let future: BoxFuture<'static, EndedTurnStopOutcome> = Box::pin(async move {
                    let outcome = dispatch.await;
                    EndedTurnStopOutcome {
                        ticket,
                        outcome,
                        _finished: finished,
                    }
                });
                future
            })
            .collect()
    })
}

/// Lets go of an ended turn's stop once its hooks have finished, reporting what they asked for
/// that a stop after a turn that did not complete cannot do.
pub(crate) fn finish_ended_turn_stop(
    conversation_id: AIConversationId,
    EndedTurnStopOutcome {
        ticket, outcome, ..
    }: EndedTurnStopOutcome,
    ctx: &mut AppContext,
) {
    if !HookRuntime::is_installed(ctx) {
        return;
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        if let Some(stops) = runtime.turn_stops.ended.get_mut(&conversation_id) {
            stops.retain(|stop| !Arc::ptr_eq(&stop.cancel, &ticket));
            if stops.is_empty() {
                runtime.turn_stops.ended.remove(&conversation_id);
            }
        }
    });
    let Some(outcome) = outcome.filter(|outcome| !outcome.cancelled) else {
        return;
    };
    let messenger = HookRuntime::as_ref(ctx).messenger();
    report_effects_a_stop_cannot_have(conversation_id, &outcome.effects, &*messenger);
    if let Some(binding) = &outcome.effects.continuation_binding {
        log::info!(
            "turn.stop hook {binding} asked for a follow-up after a turn that did not complete, \
             which means nothing"
        );
        messenger.diagnostic(HookDiagnostic {
            source: binding.source.clone(),
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "{binding} asked for a follow-up after a turn that was cancelled or failed; a \
                 follow-up is sent only after a turn that completed, so it was ignored"
            ),
        });
    }
}

/// A future that settles once every stop of an ended turn of `conversation_id` that is waiting
/// to start or running has finished, or `None` when there is none. The wait is bounded by the
/// stops' timeouts plus [`ENDED_TURN_STOP_EXIT_GRACE`], so a run exit that waits on it cannot
/// hang on a stop that never starts.
#[cfg_attr(
    target_family = "wasm",
    allow(dead_code, reason = "Oz runs are unavailable in WASM builds")
)]
pub(crate) fn ended_turn_stops_settled(
    conversation_id: AIConversationId,
    ctx: &AppContext,
) -> Option<BoxFuture<'static, ()>> {
    if !HookRuntime::is_installed(ctx) {
        return None;
    }
    let stops = HookRuntime::as_ref(ctx)
        .turn_stops
        .ended
        .get(&conversation_id)?;
    let bound = stops.iter().map(|stop| stop.timeout).max()? + ENDED_TURN_STOP_EXIT_GRACE;
    let done = stops
        .iter()
        .map(|stop| stop.done.clone())
        .collect::<Vec<_>>();
    Some(Box::pin(async move {
        futures::select_biased! {
            _ = futures::future::join_all(done).fuse() => {}
            _ = Timer::after(bound).fuse() => {
                log::warn!(
                    "the turn.stop hooks of {conversation_id}'s last turn did not finish within \
                     {bound:?}; the run goes on without waiting for them"
                );
            }
        }
    }))
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
    /// conversation is idle, call [`end_follow_up_wait`], and mark the turn successful if
    /// nothing is left running.
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
        runtime.update(ctx, |runtime, _| {
            runtime
                .turn_stops
                .awaiting_follow_up
                .insert(conversation_id);
        });
        TurnStopResolution::FollowUpQueued
    } else {
        TurnStopResolution::Finalize
    }
}

/// Whether `conversation_id`'s turn is over in all but status: its stop hooks asked for a
/// follow-up, and the queue's head has yet to be sent and the turn marked successful.
pub(crate) fn is_awaiting_follow_up(conversation_id: AIConversationId, ctx: &AppContext) -> bool {
    HookRuntime::is_installed(ctx)
        && HookRuntime::as_ref(ctx)
            .turn_stops
            .awaiting_follow_up
            .contains(&conversation_id)
}

/// Ends the wait [`resolve_turn_stop`] began when it queued a follow-up, whether or not the
/// queue's head could be sent.
pub(crate) fn end_follow_up_wait(conversation_id: AIConversationId, ctx: &mut AppContext) {
    if !HookRuntime::is_installed(ctx) {
        return;
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime
            .turn_stops
            .awaiting_follow_up
            .remove(&conversation_id);
    });
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

/// Reports a verdict or a `halt_turn` from a stop hook, which no stop can act on.
fn report_effects_a_stop_cannot_have(
    conversation_id: AIConversationId,
    effects: &MergedEffects,
    messenger: &dyn HookMessenger,
) {
    if !effects.halt_turn && effects.verdict == HookVerdict::NoOpinion {
        return;
    }
    let what = if effects.halt_turn {
        "asked to stop the turn"
    } else {
        "gave a permission verdict"
    };
    log::info!("a turn.stop hook for {conversation_id} {what}, which means nothing on stop");
    if let Some(binding) = &effects.verdict_binding {
        messenger.diagnostic(HookDiagnostic {
            source: binding.source.clone(),
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "{binding} {what}; a stop hook can only ask for a follow-up, so it was ignored"
            ),
        });
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
    report_effects_a_stop_cannot_have(conversation_id, &effects, &*messenger);
    let MergedEffects {
        continuation,
        continuation_binding,
        ..
    } = effects;
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
