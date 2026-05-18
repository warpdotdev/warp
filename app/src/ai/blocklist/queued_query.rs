use uuid::Uuid;
use warpui::{Entity, ModelContext};

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
}

/// What the auto-fire drain should do with a popped row.
#[derive(Debug)]
pub enum AutofireAction {
    /// Submit this prompt as a normal queued user query.
    Submit { text: String },
    /// The popped row was in edit mode at the time of pop.
    /// The caller places `text` in the input box after deciding it is safe to pop the edited row.
    PopFromEditMode { text: String },
}

/// Queue of follow-up prompts for the active conversation in this terminal view, plus queue UI
/// and submission state.
///
/// The model is per-terminal-view and implicitly scoped to whichever conversation owns the agent
/// view; entries are wiped on agent-view exit and on `ClearedConversationsInTerminalView`.
pub struct QueuedQueryModel {
    queue: Vec<QueuedQuery>,
    /// The row currently in edit mode, if any.
    editing: Option<QueuedQueryId>,
    /// Whether the queue panel is currently collapsed (header visible, rows hidden).
    collapsed: bool,
    /// When true, submitting a prompt while the selected conversation is responding will queue it
    /// instead of sending it immediately.
    queue_next_prompt_enabled: bool,
}

/// Events emitted by `QueuedQueryModel` so views can re-render and panels can refocus.
#[derive(Debug, Clone)]
pub enum QueuedQueryEvent {
    Appended { query_id: QueuedQueryId },
    Removed { query_id: QueuedQueryId },
    Replaced { query_id: QueuedQueryId },
    Reordered,
    EditEntered { query_id: QueuedQueryId },
    EditCommitted { query_id: QueuedQueryId },
    EditCancelled { query_id: QueuedQueryId },
    CollapseToggled { collapsed: bool },
    Cleared,
    QueueNextPromptToggled,
}

impl Entity for QueuedQueryModel {
    type Event = QueuedQueryEvent;
}

impl QueuedQueryModel {
    pub fn new() -> Self {
        Self {
            queue: Vec::new(),
            editing: None,
            collapsed: false,
            queue_next_prompt_enabled: false,
        }
    }

    /// Returns the current queue.
    pub fn queue(&self) -> &[QueuedQuery] {
        &self.queue
    }

    /// Returns true if there is at least one queued prompt.
    pub fn has_queue(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Returns the row currently in edit mode, if any.
    pub fn editing_row(&self) -> Option<QueuedQueryId> {
        self.editing
    }

    /// Returns true when the first queued row is currently being edited.
    pub fn first_row_is_in_edit_mode(&self) -> bool {
        let Some(editing_row_id) = self.editing else {
            return false;
        };
        self.queue
            .first()
            .is_some_and(|query| query.id == editing_row_id)
    }

    /// Returns true if the queue panel is collapsed.
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    pub fn is_queue_next_prompt_enabled(&self) -> bool {
        self.queue_next_prompt_enabled
    }

    pub fn toggle_queue_next_prompt(&mut self, ctx: &mut ModelContext<Self>) {
        self.queue_next_prompt_enabled = !self.queue_next_prompt_enabled;
        ctx.emit(QueuedQueryEvent::QueueNextPromptToggled);
    }

    /// Appends `query` to the tail of the queue.
    pub fn append(&mut self, query: QueuedQuery, ctx: &mut ModelContext<Self>) -> QueuedQueryId {
        let id = query.id;
        self.queue.push(query);
        ctx.emit(QueuedQueryEvent::Appended { query_id: id });
        id
    }

    /// Pops the first row in the queue and returns it.
    /// Used by the auto-fire drain when there is no edit-mode special case to handle.
    pub fn pop_front(&mut self, ctx: &mut ModelContext<Self>) -> Option<QueuedQuery> {
        if self.queue.is_empty() {
            return None;
        }
        let popped = self.queue.remove(0);
        if self.editing == Some(popped.id) {
            self.editing = None;
        }
        self.clear_empty_queue_state();
        ctx.emit(QueuedQueryEvent::Removed {
            query_id: popped.id,
        });
        Some(popped)
    }

    /// Auto-fire drain entry point.
    /// Returns `None` for empty queues; otherwise pops the first row and returns whether the caller
    /// should submit it normally or treat it as a popped edit-mode row.
    ///
    /// `edit_text_override` lets the caller pass the live editor buffer text when the first
    /// row is in edit mode (the model only tracks committed row text).
    pub fn pop_for_autofire(
        &mut self,
        edit_text_override: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AutofireAction> {
        let first = self.queue.first()?;
        let first_in_edit_mode = self.editing == Some(first.id);
        let mut popped = self.queue.remove(0);
        if first_in_edit_mode {
            if let Some(text) = edit_text_override {
                popped.text = text;
            }
            self.editing = None;
        }
        let removed_id = popped.id;
        let text = popped.text;
        self.clear_empty_queue_state();
        ctx.emit(QueuedQueryEvent::Removed {
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
        query_id: QueuedQueryId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<QueuedQuery> {
        let idx = self.queue.iter().position(|q| q.id == query_id)?;
        let removed = self.queue.remove(idx);
        if self.editing == Some(query_id) {
            self.editing = None;
        }
        self.clear_empty_queue_state();
        ctx.emit(QueuedQueryEvent::Removed { query_id });
        Some(removed)
    }

    /// Replaces the text of a specific row by id, if present.
    /// No-op when `query_id` does not exist.
    pub fn replace_text_by_id(
        &mut self,
        query_id: QueuedQueryId,
        new_text: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(row) = self.queue.iter_mut().find(|q| q.id == query_id) else {
            return;
        };
        if row.text == new_text {
            return;
        }
        row.text = new_text;
        ctx.emit(QueuedQueryEvent::Replaced { query_id });
    }

    /// Moves the row identified by `source_id` to position `target_index` within the queue.
    /// `target_index` is interpreted as the index in the post-removal list.
    pub fn reorder(
        &mut self,
        source_id: QueuedQueryId,
        target_index: usize,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(source_idx) = self.queue.iter().position(|q| q.id == source_id) else {
            return;
        };
        let row = self.queue.remove(source_idx);
        let clamped = target_index.min(self.queue.len());
        self.queue.insert(clamped, row);
        ctx.emit(QueuedQueryEvent::Reordered);
    }

    /// Enters edit mode for `query_id`. If another row was being edited, that edit is implicitly
    /// committed (its current row text remains as-is).
    pub fn enter_edit_mode(&mut self, query_id: QueuedQueryId, ctx: &mut ModelContext<Self>) {
        let row_exists = self.queue.iter().any(|r| r.id == query_id);
        if !row_exists {
            return;
        }

        if let Some(prev) = self.editing.take() {
            if prev != query_id {
                ctx.emit(QueuedQueryEvent::EditCommitted { query_id: prev });
            }
        }

        self.editing = Some(query_id);
        ctx.emit(QueuedQueryEvent::EditEntered { query_id });
    }

    /// Commits the in-progress edit by replacing the row's text with `new_text` and clearing
    /// edit state. If `new_text` is empty, the edit is cancelled and the original row text stays.
    pub fn commit_edit(&mut self, new_text: String, ctx: &mut ModelContext<Self>) {
        let Some(query_id) = self.editing.take() else {
            return;
        };

        if new_text.is_empty() {
            ctx.emit(QueuedQueryEvent::EditCancelled { query_id });
            return;
        }

        // `replace_text_by_id` handles event emission and row lookup safety.
        self.replace_text_by_id(query_id, new_text, ctx);
        ctx.emit(QueuedQueryEvent::EditCommitted { query_id });
    }

    /// Cancels the in-progress edit without modifying the row's text.
    pub fn cancel_edit(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(query_id) = self.editing.take() else {
            return;
        };
        ctx.emit(QueuedQueryEvent::EditCancelled { query_id });
    }

    /// Sets the collapsed state of the queue panel.
    pub fn set_collapsed(&mut self, collapsed: bool, ctx: &mut ModelContext<Self>) {
        if self.collapsed != collapsed {
            self.collapsed = collapsed;
            ctx.emit(QueuedQueryEvent::CollapseToggled { collapsed });
        }
    }

    /// Removes all queue, edit, and collapse state.
    /// Used when the agent view is exited or all conversations in the terminal view are cleared.
    pub fn clear_all(&mut self, ctx: &mut ModelContext<Self>) {
        let had_state = !self.queue.is_empty() || self.editing.is_some() || self.collapsed;
        self.queue.clear();
        self.editing = None;
        self.collapsed = false;
        if had_state {
            ctx.emit(QueuedQueryEvent::Cleared);
        }
    }

    fn clear_empty_queue_state(&mut self) {
        if self.queue.is_empty() {
            self.collapsed = false;
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
