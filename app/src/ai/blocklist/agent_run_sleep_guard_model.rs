use std::collections::HashMap;
use std::time::Duration;

use instant::Instant;
use prevent_sleep::Guard;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{Entity, ModelContext, SingletonEntity};

use super::history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;

/// Maximum time an inactive conversation may retain its wake assertion.
pub const AGENT_RUN_SLEEP_GUARD_CAP: Duration = Duration::from_secs(20 * 60);
const EXPIRY_CHECK_INTERVAL: Duration = Duration::from_secs(60);
const GUARD_REASON: &str = "Agent Mode run in-progress";

struct GuardState {
    _guard: Guard,
    deadline: Instant,
}

/// Owns one system-sleep assertion per active Agent Mode conversation.
///
/// The deadline is refreshed by each turn and locally drained action result. The
/// assertion itself is deliberately not dropped during refreshes, so the OS wake
/// request remains continuous across the inter-turn local-command window.
pub struct AgentRunSleepGuardModel {
    guards: HashMap<AIConversationId, GuardState>,
    expiry_timer: Option<SpawnedFutureHandle>,
    #[cfg(test)]
    now_for_test: Option<Instant>,
}

impl Entity for AgentRunSleepGuardModel {
    type Event = ();
}

impl SingletonEntity for AgentRunSleepGuardModel {}

impl AgentRunSleepGuardModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let history = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history, |me, _, event, ctx| {
            me.handle_history_event(event, ctx);
        });
        Self {
            guards: HashMap::new(),
            expiry_timer: None,
            #[cfg(test)]
            now_for_test: None,
        }
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id,
                new_status,
                ..
            } => {
                if is_active_status(new_status) {
                    self.refresh_or_acquire(*conversation_id, ctx);
                } else {
                    self.release(*conversation_id);
                }
            }
            BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                cleared_conversation_ids,
                ..
            } => {
                for id in cleared_conversation_ids {
                    self.release(*id);
                }
            }
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => self.release(*conversation_id),
            _ => {}
        }
        self.expire_guards_at(self.now(), ctx);
    }

    /// Refreshes the cap for a genuinely active conversation. If a previous
    /// cap expiry released its guard, the next activity re-acquires it.
    pub fn refresh(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>) {
        if BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| is_active_status(conversation.status()))
        {
            self.refresh_or_acquire(conversation_id, ctx);
        }
    }

    fn refresh_or_acquire(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let deadline = self.now() + AGENT_RUN_SLEEP_GUARD_CAP;
        if let Some(state) = self.guards.get_mut(&conversation_id) {
            state.deadline = deadline;
        } else {
            self.guards.insert(
                conversation_id,
                GuardState {
                    _guard: prevent_sleep::prevent_sleep(GUARD_REASON),
                    deadline,
                },
            );
        }
        self.schedule_expiry_check(ctx);
    }

    fn release(&mut self, conversation_id: AIConversationId) {
        self.guards.remove(&conversation_id);
    }

    fn schedule_expiry_check(&mut self, ctx: &mut ModelContext<Self>) {
        if self.expiry_timer.is_some() {
            return;
        }
        self.expiry_timer = Some(ctx.spawn(
            async {
                Timer::after(EXPIRY_CHECK_INTERVAL).await;
            },
            |me, _, ctx| {
                me.expiry_timer = None;
                me.expire_guards_at(me.now(), ctx);
            },
        ));
    }

    fn now(&self) -> Instant {
        #[cfg(test)]
        if let Some(now) = self.now_for_test {
            return now;
        }
        Instant::now()
    }

    fn expire_guards_at(&mut self, now: Instant, ctx: &mut ModelContext<Self>) {
        let expired = self
            .guards
            .iter()
            .filter_map(|(id, state)| (state.deadline <= now).then_some(*id))
            .collect::<Vec<_>>();
        for id in expired {
            self.guards.remove(&id);
            log::warn!(
                "Agent Mode sleep guard cap expired for conversation {id:?}; releasing wake assertion"
            );
            send_telemetry_from_ctx!(
                TelemetryEvent::AgentRunSleepGuardCapExpired {
                    conversation_id: id,
                },
                ctx
            );
        }
        if !self.guards.is_empty() {
            self.schedule_expiry_check(ctx);
        }
    }

    #[cfg(test)]
    pub(crate) fn held_guard_count(&self) -> usize {
        self.guards.len()
    }

    #[cfg(test)]
    pub(crate) fn expire_for_test(&mut self, now: Instant, ctx: &mut ModelContext<Self>) {
        self.expire_guards_at(now, ctx);
    }

    #[cfg(test)]
    pub(crate) fn set_now_for_test(&mut self, now: Instant) {
        self.now_for_test = Some(now);
    }
}

#[cfg(test)]
#[path = "agent_run_sleep_guard_model_tests.rs"]
mod tests;

fn is_active_status(status: &ConversationStatus) -> bool {
    matches!(
        status,
        ConversationStatus::InProgress | ConversationStatus::TransientError
    )
}

impl Drop for AgentRunSleepGuardModel {
    fn drop(&mut self) {
        if let Some(handle) = self.expiry_timer.take() {
            handle.abort();
        }
        self.guards.clear();
    }
}
