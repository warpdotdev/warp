use std::collections::HashMap;

use uuid::Uuid;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};

/// A globally unique identifier for a single queued prompt row.
/// Used by the queue panel to address rows across reorder, edit, and delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueuedQueryId(Uuid);

impl QueuedQueryId {
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Where a queued prompt came from.
/// The origin is informational for telemetry; FIFO ordering and firing semantics are uniform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedQueryOrigin {
    /// Filed while the initial Cloud Mode prompt waits to be handed off.
    InitialCloudMode,
    /// Filed via the `/queue <prompt>` slash command.
    QueueSlashCommand,
    /// Filed via the auto-queue toggle in the warping indicator.
    AutoQueueToggle,
}

/// A single queued prompt.
#[derive(Debug, Clone)]
pub struct QueuedQuery {
    id: QueuedQueryId,
    text: String,
    origin: QueuedQueryOrigin,
}

impl QueuedQuery {
    pub fn new(text: String, origin: QueuedQueryOrigin) -> Self {
        Self {
            id: QueuedQueryId::new(),
            text,
            origin,
        }
    }

    pub fn id(&self) -> QueuedQueryId {
        self.id
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn origin(&self) -> QueuedQueryOrigin {
        self.origin
    }

    /// Returns true if this row is locked from user mutation, reorder, and auto-fire.
    /// Currently only the locked initial Cloud Mode row is non-mutable; lifecycle code
    /// removes it explicitly via [`QueuedQueryModel::remove_initial_cloud_mode_row`].
    pub fn is_locked(&self) -> bool {
        matches!(self.origin, QueuedQueryOrigin::InitialCloudMode)
    }
}

/// What the auto-fire drain should do with a popped row.
#[derive(Debug)]
pub enum AutofireAction {
    /// Submit this prompt as a normal queued user query.
    Submit { text: String },
    /// The popped row was in edit mode at the time of pop.
    /// The caller places `text` (the row's last committed text) in the input box.
    PopFromEditMode { text: String },
}

/// Per-conversation queue / edit / toggle state.
/// Lives inside [`QueuedQueryModel::queues`]; a missing key means empty queue, no edit in
/// progress, and toggle off.
#[derive(Default)]
struct ConversationQueueState {
    queue: Vec<QueuedQuery>,
    editing: Option<QueuedQueryId>,
    queue_next_prompt_enabled: bool,
}

/// App-wide singleton owning the queued prompts and auto-queue toggle for every conversation,
/// indexed by [`AIConversationId`]. Queues outlive the agent-view session that originated them;
/// cleanup is driven by [`BlocklistAIHistoryModel`] lifecycle events that this model subscribes
/// to in [`QueuedQueryModel::new`].
pub struct QueuedQueryModel {
    queues: HashMap<AIConversationId, ConversationQueueState>,
}

/// Events emitted by [`QueuedQueryModel`]. Every variant carries the `conversation_id` it applies
/// to so subscribers can filter to the conversation they care about.
#[derive(Debug, Clone)]
pub enum QueuedQueryEvent {
    Appended {
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
    },
    Removed {
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
    },
    Reordered {
        conversation_id: AIConversationId,
    },
    EditEntered {
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
    },
    EditCommitted {
        conversation_id: AIConversationId,
        #[allow(dead_code)]
        query_id: QueuedQueryId,
    },
    EditCancelled {
        conversation_id: AIConversationId,
        #[allow(dead_code)]
        query_id: QueuedQueryId,
    },
    Cleared {
        conversation_id: AIConversationId,
    },
    QueueNextPromptToggled {
        conversation_id: AIConversationId,
    },
}

impl Entity for QueuedQueryModel {
    type Event = QueuedQueryEvent;
}

impl SingletonEntity for QueuedQueryModel {}

impl QueuedQueryModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Drop queue/toggle state for any conversation that is removed, deleted, or cleared
        // from its owning terminal view. Agent-view exit is intentionally NOT subscribed to:
        // conversations (cloud agents in particular) outlive their visible session.
        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_handle, |this, event, ctx| {
            this.handle_history_event(event, ctx);
        });

        Self {
            queues: HashMap::new(),
        }
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                self.drop_conversation(*conversation_id, ctx);
            }
            BlocklistAIHistoryEvent::ClearedConversationsInTerminalView {
                cleared_conversation_ids,
                ..
            } => {
                for conversation_id in cleared_conversation_ids.clone() {
                    self.drop_conversation(conversation_id, ctx);
                }
            }
            _ => {}
        }
    }

    fn drop_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.queues.remove(&conversation_id).is_some() {
            ctx.emit(QueuedQueryEvent::Cleared { conversation_id });
        }
    }

    /// Returns the queue for `conversation_id`. Returns an empty slice when no entry exists.
    pub fn queue(&self, conversation_id: AIConversationId) -> &[QueuedQuery] {
        self.queues
            .get(&conversation_id)
            .map(|state| state.queue.as_slice())
            .unwrap_or(&[])
    }

    /// Returns true when `conversation_id` has at least one queued prompt.
    pub fn has_queue(&self, conversation_id: AIConversationId) -> bool {
        self.queues
            .get(&conversation_id)
            .is_some_and(|state| !state.queue.is_empty())
    }

    /// Returns the row currently in edit mode for `conversation_id`, if any.
    pub fn editing_row(&self, conversation_id: AIConversationId) -> Option<QueuedQueryId> {
        self.queues
            .get(&conversation_id)
            .and_then(|state| state.editing)
    }

    /// Returns true when the head row of `conversation_id`'s queue is currently being edited.
    pub fn first_row_is_in_edit_mode(&self, conversation_id: AIConversationId) -> bool {
        let Some(state) = self.queues.get(&conversation_id) else {
            return false;
        };
        let Some(editing_id) = state.editing else {
            return false;
        };
        state.queue.first().is_some_and(|q| q.id == editing_id)
    }

    /// Returns the per-conversation auto-queue toggle state. Defaults to false for conversations
    /// that have never been touched.
    pub fn is_queue_next_prompt_enabled(&self, conversation_id: AIConversationId) -> bool {
        self.queues
            .get(&conversation_id)
            .is_some_and(|state| state.queue_next_prompt_enabled)
    }

    /// Toggles the per-conversation auto-queue state.
    pub fn toggle_queue_next_prompt(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let state = self.queues.entry(conversation_id).or_default();
        state.queue_next_prompt_enabled = !state.queue_next_prompt_enabled;
        ctx.emit(QueuedQueryEvent::QueueNextPromptToggled { conversation_id });
    }

    /// Appends `query` to the tail of `conversation_id`'s queue.
    pub fn append(
        &mut self,
        conversation_id: AIConversationId,
        query: QueuedQuery,
        ctx: &mut ModelContext<Self>,
    ) -> QueuedQueryId {
        let query_id = query.id;
        let state = self.queues.entry(conversation_id).or_default();
        state.queue.push(query);
        ctx.emit(QueuedQueryEvent::Appended {
            conversation_id,
            query_id,
        });
        query_id
    }

    /// Pops the first row in `conversation_id`'s queue and returns it.
    /// Used by the non-clean drain path (Error / Cancelled) to restore a single popped
    /// prompt to the input editor. No-ops when the head is locked
    /// ([`QueuedQuery::is_locked`]) so a status-transition arriving before the lifecycle
    /// cleanup events cannot clobber the locked initial Cloud Mode row.
    pub fn pop_front(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<QueuedQuery> {
        let state = self.queues.get_mut(&conversation_id)?;
        if state.queue.first()?.is_locked() {
            return None;
        }
        let popped = state.queue.remove(0);
        if state.editing == Some(popped.id) {
            state.editing = None;
        }
        ctx.emit(QueuedQueryEvent::Removed {
            conversation_id,
            query_id: popped.id,
        });
        Some(popped)
    }

    /// Auto-fire drain entry point for `conversation_id`.
    /// Returns `None` for empty queues or when the head is locked
    /// ([`QueuedQuery::is_locked`]); otherwise pops the first row and returns whether
    /// the caller should submit it normally or treat it as a popped edit-mode row.
    pub fn pop_for_autofire(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AutofireAction> {
        let state = self.queues.get_mut(&conversation_id)?;
        let first = state.queue.first()?;
        if first.is_locked() {
            return None;
        }
        let first_in_edit_mode = state.editing == Some(first.id);
        let popped = state.queue.remove(0);
        if first_in_edit_mode {
            state.editing = None;
        }
        ctx.emit(QueuedQueryEvent::Removed {
            conversation_id,
            query_id: popped.id,
        });

        Some(if first_in_edit_mode {
            AutofireAction::PopFromEditMode { text: popped.text }
        } else {
            AutofireAction::Submit { text: popped.text }
        })
    }

    /// Removes a specific row by id within `conversation_id`'s queue, if present. Returns the
    /// removed row. No-ops when the target row is locked ([`QueuedQuery::is_locked`]); the
    /// locked initial Cloud Mode row is only removable via
    /// [`Self::remove_initial_cloud_mode_row`].
    pub fn remove_by_id(
        &mut self,
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<QueuedQuery> {
        let state = self.queues.get_mut(&conversation_id)?;
        let idx = state.queue.iter().position(|q| q.id == query_id)?;
        if state.queue[idx].is_locked() {
            return None;
        }
        let removed = state.queue.remove(idx);
        if state.editing == Some(query_id) {
            state.editing = None;
        }
        ctx.emit(QueuedQueryEvent::Removed {
            conversation_id,
            query_id,
        });
        Some(removed)
    }

    /// Removes the locked initial Cloud Mode row from `conversation_id`'s queue, if it is still
    /// at the queue head.
    pub fn remove_initial_cloud_mode_row(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<QueuedQuery> {
        let state = self.queues.get_mut(&conversation_id)?;
        if !state
            .queue
            .first()
            .is_some_and(|row| row.origin == QueuedQueryOrigin::InitialCloudMode)
        {
            return None;
        }
        let removed = state.queue.remove(0);
        if state.editing == Some(removed.id) {
            state.editing = None;
        }
        ctx.emit(QueuedQueryEvent::Removed {
            conversation_id,
            query_id: removed.id,
        });
        Some(removed)
    }

    /// Moves the row identified by `source_id` to position `target_index` within
    /// `conversation_id`'s queue. `target_index` is interpreted as the index in the post-removal
    /// list and is clamped to the queue length. No-ops when the source row is locked
    /// ([`QueuedQuery::is_locked`]) or when the move would displace a locked row off the head of
    /// the queue.
    pub fn reorder(
        &mut self,
        conversation_id: AIConversationId,
        source_id: QueuedQueryId,
        target_index: usize,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        let Some(source_idx) = state.queue.iter().position(|q| q.id == source_id) else {
            return;
        };
        let head_is_locked = state.queue.first().is_some_and(|row| row.is_locked());
        if state.queue[source_idx].is_locked() || (target_index == 0 && head_is_locked) {
            return;
        }
        let row = state.queue.remove(source_idx);
        let clamped = target_index.min(state.queue.len());
        state.queue.insert(clamped, row);
        ctx.emit(QueuedQueryEvent::Reordered { conversation_id });
    }

    /// Enters edit mode for `query_id` in `conversation_id`'s queue. If another row was being
    /// edited, that edit is cancelled (its text is unchanged, per the spec). No-ops when the
    /// target row is locked ([`QueuedQuery::is_locked`]).
    pub fn enter_edit_mode(
        &mut self,
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        if !state
            .queue
            .iter()
            .any(|q| q.id == query_id && !q.is_locked())
        {
            return;
        }
        let prev_edit = state.editing.replace(query_id);
        if let Some(prev) = prev_edit {
            if prev != query_id {
                ctx.emit(QueuedQueryEvent::EditCancelled {
                    conversation_id,
                    query_id: prev,
                });
            }
        }
        ctx.emit(QueuedQueryEvent::EditEntered {
            conversation_id,
            query_id,
        });
    }

    /// Commits the in-progress edit in `conversation_id` by replacing the row's text with
    /// `new_text` and clearing edit state. An empty `new_text` cancels the edit and leaves the
    /// original row text untouched.
    pub fn commit_edit(
        &mut self,
        conversation_id: AIConversationId,
        new_text: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        let Some(query_id) = state.editing.take() else {
            return;
        };
        if new_text.is_empty() {
            ctx.emit(QueuedQueryEvent::EditCancelled {
                conversation_id,
                query_id,
            });
            return;
        }
        if let Some(row) = state.queue.iter_mut().find(|q| q.id == query_id) {
            row.text = new_text;
        }
        ctx.emit(QueuedQueryEvent::EditCommitted {
            conversation_id,
            query_id,
        });
    }

    /// Cancels the in-progress edit in `conversation_id` without modifying the row's text.
    pub fn cancel_edit(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>) {
        let Some(state) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        let Some(query_id) = state.editing.take() else {
            return;
        };
        ctx.emit(QueuedQueryEvent::EditCancelled {
            conversation_id,
            query_id,
        });
    }
}

#[cfg(test)]
#[path = "queued_query_tests.rs"]
mod tests;
