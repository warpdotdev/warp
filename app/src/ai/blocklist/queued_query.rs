use std::collections::{HashMap, HashSet};

use uuid::Uuid;
use warpui::{Entity, ModelContext};

use crate::ai::agent::conversation::AIConversationId;

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
/// The origin is informational (telemetry, render rules, debug); the FIFO ordering and firing
/// semantics are uniform across origins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedQueryOrigin {
    /// Filed via the `/queue <prompt>` slash command.
    QueueSlashCommand,
    /// Filed via the auto-queue toggle in the warping indicator.
    AutoQueueToggle,
    /// Filed via `/compact-and <prompt>`.
    /// The summarization itself is dispatched immediately at file time; the prompt fires after.
    CompactAnd,
    /// Filed via `/fork-and-compact <prompt>`.
    /// The fork + summarization is dispatched immediately at file time; the prompt fires after.
    ForkAndCompact,
    /// The initial prompt for a non-Oz Cloud Mode run waiting for its harness CLI to start.
    /// Owned by the harness lifecycle; the queue panel renders it without edit / delete /
    /// reorder affordances and the auto-fire path skips it.
    InitialCloudMode,
}

impl QueuedQueryOrigin {
    /// Returns true for rows that should be displayed but never auto-fired by the queue
    /// drain or mutated by the user (the harness owns their lifecycle).
    pub fn is_user_managed(self) -> bool {
        !matches!(self, QueuedQueryOrigin::InitialCloudMode)
    }
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

    pub fn into_text(self) -> String {
        self.text
    }
}

/// What the auto-fire drain should do with a popped row.
#[derive(Debug)]
pub enum AutofireAction {
    /// Submit this prompt as a normal queued user query.
    Submit { text: String },
    /// The popped row was in edit mode at the time of pop.
    /// The caller should place `text` in the input box only if the input is empty;
    /// otherwise discard it.
    PopFromEditMode { text: String },
}

/// Per-conversation queue of follow-up prompts plus queue UI and submission state.
pub struct QueuedQueryModel {
    queues: HashMap<AIConversationId, Vec<QueuedQuery>>,
    /// At most one row across all conversations may be in edit mode.
    editing: Option<EditingRow>,
    /// Conversations whose queue panel is currently collapsed (header visible, rows hidden).
    collapsed: HashSet<AIConversationId>,
    /// When true, submitting a prompt while the selected conversation is responding will queue it
    /// instead of sending it immediately.
    queue_next_prompt_enabled: bool,
}

#[derive(Debug, Clone)]
struct EditingRow {
    conversation_id: AIConversationId,
    query_id: QueuedQueryId,
}

/// Events emitted by `QueuedQueryModel` so views can re-render and panels can refocus.
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
    Replaced {
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
        query_id: QueuedQueryId,
    },
    EditCancelled {
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
    },
    CollapseToggled {
        conversation_id: AIConversationId,
        collapsed: bool,
    },
    Cleared {
        conversation_id: AIConversationId,
    },
    QueueNextPromptToggled,
}

impl Entity for QueuedQueryModel {
    type Event = QueuedQueryEvent;
}

impl QueuedQueryModel {
    pub fn new() -> Self {
        Self {
            queues: HashMap::new(),
            editing: None,
            collapsed: HashSet::new(),
            queue_next_prompt_enabled: false,
        }
    }

    /// Returns the queue for `conversation_id`, or an empty slice if the conversation has no
    /// queue.
    pub fn queue_for(&self, conversation_id: AIConversationId) -> &[QueuedQuery] {
        self.queues
            .get(&conversation_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the text of the first row in the conversation's queue, if any.
    pub fn first_text(&self, conversation_id: AIConversationId) -> Option<&str> {
        self.queues
            .get(&conversation_id)?
            .first()
            .map(|q| q.text.as_str())
    }

    /// Returns true if `conversation_id` has at least one queued prompt.
    pub fn has_queue(&self, conversation_id: AIConversationId) -> bool {
        self.queues
            .get(&conversation_id)
            .is_some_and(|q| !q.is_empty())
    }

    /// Returns the row currently in edit mode for `conversation_id`, if any.
    pub fn editing_row(&self, conversation_id: AIConversationId) -> Option<QueuedQueryId> {
        self.editing
            .as_ref()
            .filter(|e| e.conversation_id == conversation_id)
            .map(|e| e.query_id)
    }

    /// Returns true if the queue panel for `conversation_id` is collapsed.
    pub fn is_collapsed(&self, conversation_id: AIConversationId) -> bool {
        self.collapsed.contains(&conversation_id)
    }

    pub fn is_queue_next_prompt_enabled(&self) -> bool {
        self.queue_next_prompt_enabled
    }

    pub fn toggle_queue_next_prompt(&mut self, ctx: &mut ModelContext<Self>) {
        self.queue_next_prompt_enabled = !self.queue_next_prompt_enabled;
        ctx.emit(QueuedQueryEvent::QueueNextPromptToggled);
    }

    /// Appends `query` to the tail of the queue for `conversation_id`.
    pub fn append(
        &mut self,
        conversation_id: AIConversationId,
        query: QueuedQuery,
        ctx: &mut ModelContext<Self>,
    ) -> QueuedQueryId {
        let id = query.id;
        self.queues.entry(conversation_id).or_default().push(query);
        ctx.emit(QueuedQueryEvent::Appended {
            conversation_id,
            query_id: id,
        });
        id
    }

    /// Pops the first row in the conversation's queue and returns it.
    /// Used by the auto-fire drain when there is no edit-mode special case to handle.
    pub fn pop_front(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<QueuedQuery> {
        let popped = {
            let queue = self.queues.get_mut(&conversation_id)?;
            if queue.is_empty() {
                return None;
            }
            queue.remove(0)
        };
        // Clear edit mode if the popped row was the one being edited.
        if self
            .editing
            .as_ref()
            .is_some_and(|e| e.conversation_id == conversation_id && e.query_id == popped.id)
        {
            self.editing = None;
        }
        self.clear_empty_queue_state(conversation_id);
        ctx.emit(QueuedQueryEvent::Removed {
            conversation_id,
            query_id: popped.id,
        });
        Some(popped)
    }
    /// Pops the first row only when the row is user-managed.
    pub fn pop_front_user_managed(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<QueuedQuery> {
        let first = self.queues.get(&conversation_id)?.first()?;
        if !first.origin.is_user_managed() {
            return None;
        }
        self.pop_front(conversation_id, ctx)
    }

    /// Auto-fire drain entry point.
    /// Returns `None` for empty queues and for `InitialCloudMode` rows at the head; otherwise
    /// pops the first row and returns whether the caller should submit it normally or treat
    /// it as a popped edit-mode row.
    ///
    /// `edit_text_override` lets the caller pass the live editor buffer text when the first
    /// row is in edit mode (the model only tracks committed row text).
    pub fn pop_for_autofire(
        &mut self,
        conversation_id: AIConversationId,
        edit_text_override: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AutofireAction> {
        let (mut popped, first_in_edit_mode) = {
            let queue = self.queues.get_mut(&conversation_id)?;
            let first = queue.first()?;
            if !first.origin.is_user_managed() {
                return None;
            }

            let first_in_edit_mode = self
                .editing
                .as_ref()
                .is_some_and(|e| e.conversation_id == conversation_id && e.query_id == first.id);

            (queue.remove(0), first_in_edit_mode)
        };
        if first_in_edit_mode {
            if let Some(text) = edit_text_override {
                popped.text = text;
            }
            self.editing = None;
        }
        let removed_id = popped.id;
        let text = popped.text;
        self.clear_empty_queue_state(conversation_id);
        ctx.emit(QueuedQueryEvent::Removed {
            conversation_id,
            query_id: removed_id,
        });

        Some(if first_in_edit_mode {
            AutofireAction::PopFromEditMode { text }
        } else {
            AutofireAction::Submit { text }
        })
    }

    /// Removes a specific row by id, if present. Returns the removed row.
    pub fn remove_by_id(
        &mut self,
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<QueuedQuery> {
        let removed = {
            let queue = self.queues.get_mut(&conversation_id)?;
            let idx = queue.iter().position(|q| q.id == query_id)?;
            queue.remove(idx)
        };
        if self
            .editing
            .as_ref()
            .is_some_and(|e| e.conversation_id == conversation_id && e.query_id == query_id)
        {
            self.editing = None;
        }
        self.clear_empty_queue_state(conversation_id);
        ctx.emit(QueuedQueryEvent::Removed {
            conversation_id,
            query_id,
        });
        Some(removed)
    }

    /// Replaces the text of a specific row by id, if present.
    /// No-op when `query_id` does not exist or the row's origin is not user-managed.
    pub fn replace_text_by_id(
        &mut self,
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
        new_text: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(queue) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        let Some(row) = queue.iter_mut().find(|q| q.id == query_id) else {
            return;
        };
        if !row.origin.is_user_managed() {
            return;
        }
        if row.text == new_text {
            return;
        }
        row.text = new_text;
        ctx.emit(QueuedQueryEvent::Replaced {
            conversation_id,
            query_id,
        });
    }

    /// Moves the row identified by `source_id` to position `target_index` within its queue.
    /// `target_index` is interpreted as the index in the post-removal list.
    /// No-op when the row is not user-managed.
    pub fn reorder(
        &mut self,
        conversation_id: AIConversationId,
        source_id: QueuedQueryId,
        target_index: usize,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(queue) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        let Some(source_idx) = queue.iter().position(|q| q.id == source_id) else {
            return;
        };
        if !queue[source_idx].origin.is_user_managed() {
            return;
        }
        let row = queue.remove(source_idx);
        let first_user_managed_index = queue
            .iter()
            .take_while(|row| !row.origin.is_user_managed())
            .count();
        let clamped = target_index.max(first_user_managed_index).min(queue.len());
        queue.insert(clamped, row);
        ctx.emit(QueuedQueryEvent::Reordered { conversation_id });
    }

    /// Enters edit mode for `query_id`. If another row was being edited, that edit is implicitly
    /// committed (its current row text remains as-is). Cloud Mode rows are non-editable and this
    /// call is a no-op for them.
    pub fn enter_edit_mode(
        &mut self,
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
        ctx: &mut ModelContext<Self>,
    ) {
        let row_origin = self
            .queues
            .get(&conversation_id)
            .and_then(|q| q.iter().find(|r| r.id == query_id))
            .map(|r| r.origin);
        let Some(origin) = row_origin else { return };
        if !origin.is_user_managed() {
            return;
        }

        if let Some(prev) = self.editing.take() {
            if prev.conversation_id != conversation_id || prev.query_id != query_id {
                ctx.emit(QueuedQueryEvent::EditCommitted {
                    conversation_id: prev.conversation_id,
                    query_id: prev.query_id,
                });
            }
        }

        self.editing = Some(EditingRow {
            conversation_id,
            query_id,
        });
        ctx.emit(QueuedQueryEvent::EditEntered {
            conversation_id,
            query_id,
        });
    }

    /// Commits the in-progress edit by replacing the row's text with `new_text` and clearing
    /// edit state. If `new_text` is empty, the edit is cancelled and the original row text stays.
    pub fn commit_edit(&mut self, new_text: String, ctx: &mut ModelContext<Self>) {
        let Some(EditingRow {
            conversation_id,
            query_id,
        }) = self.editing.take()
        else {
            return;
        };

        if new_text.is_empty() {
            ctx.emit(QueuedQueryEvent::EditCancelled {
                conversation_id,
                query_id,
            });
            return;
        }

        // `replace_text_by_id` handles event emission and row lookup safety.
        self.replace_text_by_id(conversation_id, query_id, new_text, ctx);
        ctx.emit(QueuedQueryEvent::EditCommitted {
            conversation_id,
            query_id,
        });
    }

    /// Cancels the in-progress edit without modifying the row's text.
    pub fn cancel_edit(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(EditingRow {
            conversation_id,
            query_id,
        }) = self.editing.take()
        else {
            return;
        };
        ctx.emit(QueuedQueryEvent::EditCancelled {
            conversation_id,
            query_id,
        });
    }

    /// Sets the collapsed state of the queue panel for `conversation_id`.
    pub fn set_collapsed(
        &mut self,
        conversation_id: AIConversationId,
        collapsed: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let changed = if collapsed {
            self.collapsed.insert(conversation_id)
        } else {
            self.collapsed.remove(&conversation_id)
        };
        if changed {
            ctx.emit(QueuedQueryEvent::CollapseToggled {
                conversation_id,
                collapsed,
            });
        }
    }

    /// Removes the queue (and any associated edit / collapse state) for `conversation_id`.
    pub fn clear_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let removed_queue = self.queues.remove(&conversation_id);
        let removed_collapsed = self.collapsed.remove(&conversation_id);
        let cleared_editing = self
            .editing
            .as_ref()
            .is_some_and(|e| e.conversation_id == conversation_id);
        if cleared_editing {
            self.editing = None;
        }
        if removed_queue.is_some() || removed_collapsed || cleared_editing {
            ctx.emit(QueuedQueryEvent::Cleared { conversation_id });
        }
    }

    /// Removes all queues, edit state, and collapse state.
    /// Used when the agent view is exited or all conversations in the terminal view are cleared.
    pub fn clear_all(&mut self, ctx: &mut ModelContext<Self>) {
        let conversation_ids: Vec<AIConversationId> = self.queues.keys().copied().collect();
        for conversation_id in conversation_ids {
            self.clear_for_conversation(conversation_id, ctx);
        }
        // `clear_for_conversation` already handles editing for known queues; clear any stragglers.
        self.editing = None;
        self.collapsed.clear();
    }

    fn clear_empty_queue_state(&mut self, conversation_id: AIConversationId) {
        if self
            .queues
            .get(&conversation_id)
            .is_some_and(|queue| queue.is_empty())
        {
            self.queues.remove(&conversation_id);
            self.collapsed.remove(&conversation_id);
        }
    }
}

impl Default for QueuedQueryModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "queued_query_tests.rs"]
mod tests;
