//! Executor for `AIAgentActionType::WaitForEvents`.
//!
//! Schedules a watchdog timer so that the wait completes with `Completed`
//! if no events arrive within the idle window.

use std::collections::HashMap;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{AIAgentActionResultType, AIAgentActionType, WaitForEventsResult};
use crate::ai::blocklist::BlocklistAIHistoryModel;

/// Fallback when `idle_timeout_seconds` is unset (0). Matches the worker
/// VM idle ceiling.
pub(crate) const DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS: i32 = 30 * 60;

/// Subtracted from the server-supplied timeout so the client closes the
/// wait before the worker idle-shutdown.
pub(crate) const CLIENT_WATCHDOG_SAFETY_MARGIN: Duration = Duration::from_secs(30);

/// Lower bound after the safety-margin subtraction; keeps tiny stamped
/// values from clamping to 0.
pub(crate) const HARD_FLOOR: Duration = Duration::from_secs(5);

/// Apply the safety margin and hard floor. Non-positive input falls back
/// to the default.
pub(crate) fn watchdog_timeout_for_stamped_seconds(stamped_seconds: i32) -> Duration {
    let seconds = if stamped_seconds <= 0 {
        DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS
    } else {
        stamped_seconds
    };
    let stamped = Duration::from_secs(seconds as u64);
    stamped
        .checked_sub(CLIENT_WATCHDOG_SAFETY_MARGIN)
        .filter(|d| *d >= HARD_FLOOR)
        .unwrap_or(HARD_FLOOR)
}

/// Currently-in-flight WaitForEvents action state.
struct PendingWait {
    tool_call_id: String,
    sender: async_channel::Sender<WaitForEventsResult>,
    /// Handle to the watchdog timer. Aborted on cancel so the future
    /// doesn't survive up to ~30 minutes past supersede.
    watchdog_handle: SpawnedFutureHandle,
}

pub struct WaitForEventsExecutor {
    terminal_view_id: EntityId,
    /// Bumped on each fresh execute(); stale watchdog closures no-op when
    /// they fire.
    conversation_generation: HashMap<AIConversationId, usize>,
    pending: HashMap<AIConversationId, PendingWait>,
}

impl WaitForEventsExecutor {
    pub fn new(terminal_view_id: EntityId, _ctx: &mut ModelContext<Self>) -> Self {
        Self {
            terminal_view_id,
            conversation_generation: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    pub(super) fn should_autoexecute(
        &self,
        _input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        // Synthesized from a server-emitted tool call; no confirmation.
        true
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let AIAgentActionType::WaitForEvents {
            tool_call_id,
            idle_timeout_seconds,
        } = &input.action.action
        else {
            return ActionExecution::InvalidAction;
        };

        let tool_call_id = tool_call_id.clone();
        let conversation_id = input.conversation_id;
        let timeout = watchdog_timeout_for_stamped_seconds(*idle_timeout_seconds);

        // Bump the counter so any prior watchdog closure observes a
        // stale generation.
        let generation = self
            .conversation_generation
            .entry(conversation_id)
            .or_insert(0);
        *generation += 1;
        let expected_generation = *generation;

        // Schedule the watchdog first so we can store its handle in the
        // pending entry. The closure no-ops on a stale generation.
        let watchdog_tool_call_id = tool_call_id.clone();
        let watchdog_handle = ctx.spawn(
            async move {
                warpui::r#async::Timer::after(timeout).await;
            },
            move |me, (), ctx| {
                me.fire_watchdog_if_current(
                    conversation_id,
                    &watchdog_tool_call_id,
                    expected_generation,
                    ctx,
                );
            },
        );

        // Replace any prior pending entry. Dropping the prior sender wakes
        // its receiver with Err and the prior watchdog is aborted so it
        // can't fire after the new wait starts.
        let (sender, receiver) = async_channel::bounded(1);
        if let Some(prev) = self.pending.insert(
            conversation_id,
            PendingWait {
                tool_call_id: tool_call_id.clone(),
                sender,
                watchdog_handle,
            },
        ) {
            prev.watchdog_handle.abort();
            drop(prev.sender);
        }

        // Flip the conversation into WaitingForEvents before returning so
        // downstream subscribers see the yield immediately.
        let terminal_view_id = self.terminal_view_id;
        BlocklistAIHistoryModel::handle(ctx).update(ctx, move |history_model, ctx| {
            history_model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::WaitingForEvents,
                ctx,
            );
        });

        ActionExecution::new_async(async move { receiver.recv().await }, move |result, _ctx| {
            let wait_result = match result {
                Ok(result) => result,
                // Sender dropped via external cancellation (e.g. resume
                // injection, user query). The action_model removed this
                // action from `async_executing_actions` before dropping
                // the sender, so the spawn callback that wraps this
                // closure will suppress the result. The value here is
                // unobservable in that path; pick `Completed` defensively.
                Err(_) => WaitForEventsResult::Completed,
            };
            AIAgentActionResultType::WaitForEvents(wait_result)
        })
    }

    /// Drop the in-flight wait so a later watchdog fire is a no-op.
    /// Caller (`BlocklistAIActionExecutor::cancel_running_async_action`)
    /// has already removed the action from `async_executing_actions`, so
    /// the spawn callback will silently discard the result that surfaces
    /// from the dropped sender.
    pub(crate) fn cancel_execution(&mut self, tool_call_id: &str) {
        let Some(conversation_id) = self
            .pending
            .iter()
            .find(|(_, pending)| pending.tool_call_id == tool_call_id)
            .map(|(id, _)| *id)
        else {
            return;
        };
        let Some(pending) = self.pending.remove(&conversation_id) else {
            return;
        };
        if let Some(gen) = self.conversation_generation.get_mut(&conversation_id) {
            *gen += 1;
        }
        pending.watchdog_handle.abort();
        drop(pending.sender);
    }

    fn fire_watchdog_if_current(
        &mut self,
        conversation_id: AIConversationId,
        tool_call_id: &str,
        expected_generation: usize,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .conversation_generation
            .get(&conversation_id)
            .copied()
            .unwrap_or(0)
            != expected_generation
        {
            return;
        }
        let Some(pending) = self.pending.get(&conversation_id) else {
            return;
        };
        if pending.tool_call_id != tool_call_id {
            return;
        }
        // Defensive: only fire if the conversation is still waiting. If
        // some other path transitioned the conversation out of
        // `WaitingForEvents` without going through `cancel_execution`,
        // sending `Completed` now would inject a stale tool-call result
        // into a conversation the server has long since moved past.
        let still_waiting = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|c| matches!(c.status(), ConversationStatus::WaitingForEvents));
        if !still_waiting {
            log::info!(
                "WaitForEventsExecutor: watchdog stale (conversation no longer waiting); \
                 dropping pending entry conversation_id={conversation_id:?} \
                 tool_call_id={tool_call_id}"
            );
            self.pending.remove(&conversation_id);
            return;
        }
        let Some(pending) = self.pending.remove(&conversation_id) else {
            return;
        };
        log::info!(
            "WaitForEventsExecutor: watchdog fired conversation_id={conversation_id:?} \
             tool_call_id={tool_call_id}"
        );
        let _ = pending.sender.try_send(WaitForEventsResult::Completed);
    }
}

impl Entity for WaitForEventsExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "wait_for_events_tests.rs"]
mod tests;
