//! Shared singleton holding cross-pane UI state for the orchestration pill
//! bar: the set of pinned child-agent conversations (persisted so pins
//! survive restarts) and a per-orchestrator horizontal scroll handle so
//! the pill row's scroll offset survives switching between sibling panes.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use warpui::elements::ClippedScrollStateHandle;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};

/// Toggle `id` in `pinned` — insert when absent, remove when present.
/// Returns the new membership state (`true` = now pinned).
/// Extracted so toggle semantics can be unit tested without a singleton.
pub(super) fn toggle_pin_in_set(
    pinned: &mut HashSet<AIConversationId>,
    id: AIConversationId,
) -> bool {
    if pinned.remove(&id) {
        false
    } else {
        pinned.insert(id);
        true
    }
}

/// Singleton owning cross-pane UI state for the orchestration pill bar.
pub struct OrchestrationPillBarModel {
    /// In-memory mirror of which child conversations are pinned. The
    /// persisted source of truth lives on each conversation; this set
    /// exists for fast lookups and cross-pane event notification.
    pinned: HashSet<AIConversationId>,
    /// One scroll handle per orchestrator conversation id. Every pill
    /// bar rendering the same orchestration tree clones the same handle
    /// so panning in one pane is reflected in sibling panes. `RefCell`
    /// so handles can be lazily created from `&AppContext` paths.
    horizontal_scroll_states: RefCell<HashMap<AIConversationId, ClippedScrollStateHandle>>,
}

impl OrchestrationPillBarModel {
    /// Construct the singleton seeded with the set of conversation ids that
    /// were already pinned in persisted storage at startup.
    pub fn new(initial_pinned: HashSet<AIConversationId>, ctx: &mut ModelContext<Self>) -> Self {
        // Prune deleted conversations globally so per-pane code never
        // has to (and can't accidentally clobber sibling panes' pins).
        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_handle, |this, event, ctx| match event {
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                if this.pinned.remove(conversation_id) {
                    ctx.emit(OrchestrationPillBarEvent::PinSetChanged);
                }
                // Drop the matching scroll handle so deleted orchestrators
                // don't leak entries. No event needed — nothing subscribes
                // to scroll-state changes.
                this.horizontal_scroll_states
                    .borrow_mut()
                    .remove(conversation_id);
            }
            _ => {}
        });

        Self {
            pinned: initial_pinned,
            horizontal_scroll_states: RefCell::new(HashMap::new()),
        }
    }

    /// Returns `true` if the given conversation id is currently pinned.
    pub fn is_pinned(&self, conversation_id: &AIConversationId) -> bool {
        self.pinned.contains(conversation_id)
    }

    /// Returns the shared horizontal scroll handle for the given
    /// orchestration tree, lazily creating one on first access. Every
    /// pill bar in that tree clones the same handle, so panning in one
    /// pane stays in sync with sibling panes.
    pub fn horizontal_scroll_state_for(
        &self,
        orchestrator_id: AIConversationId,
    ) -> ClippedScrollStateHandle {
        self.horizontal_scroll_states
            .borrow_mut()
            .entry(orchestrator_id)
            .or_default()
            .clone()
    }

    /// Toggle whether `conversation_id` is pinned, persist the new state to
    /// the underlying conversation, and notify subscribers.
    pub fn toggle_pin(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>) {
        let now_pinned = toggle_pin_in_set(&mut self.pinned, conversation_id);
        // Push the change down to the conversation so the next session
        // restore reflects it. Doing this even if the conversation isn't
        // yet loaded is fine — the in-memory set is the active source
        // until that conversation rehydrates.
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.set_conversation_pinned(conversation_id, now_pinned, ctx);
        });
        ctx.emit(OrchestrationPillBarEvent::PinSetChanged);
    }

    /// Clears the in-memory pinned set and the scroll handle cache.
    /// Invoked on logout so the next user does not inherit the previous
    /// account's UI state; persisted pins are wiped by the sqlite reset
    /// that runs alongside logout.
    pub fn reset(&mut self) {
        self.pinned.clear();
        self.horizontal_scroll_states.borrow_mut().clear();
    }
}

impl Entity for OrchestrationPillBarModel {
    type Event = OrchestrationPillBarEvent;
}

impl SingletonEntity for OrchestrationPillBarModel {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrchestrationPillBarEvent {
    /// The pinned set changed (toggle or history-driven prune).
    PinSetChanged,
}

#[cfg(test)]
#[path = "orchestration_pill_bar_model_tests.rs"]
mod tests;
