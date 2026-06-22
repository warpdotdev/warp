use std::collections::HashMap;

use uuid::Uuid;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel, PendingAttachment};
use crate::features::FeatureFlag;
use crate::settings::{
    AISettings, AISettingsChangedEvent, LongRunningCommandSubmissionMode, PromptSubmissionMode,
};
use crate::terminal::model::block::Block;

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
    /// Filed because auto-queue was in effect during an agent-requested long-running command.
    LrcAutoQueue,
    /// Filed as the follow-up prompt of a `/compact-and <prompt>` slash command, waiting for
    /// the summarize to finish.
    CompactAndSlashCommand,
    /// Filed as the follow-up prompt of a `/fork-and-compact <prompt>` slash command on the
    /// forked conversation, waiting for the fork's summarize to finish.
    ForkAndCompactSlashCommand,
}

/// Whether a queued row is an agent prompt or a shell command. Attachments live inside the
/// `Prompt` variant so a `Command` structurally cannot carry any.
#[derive(Debug, Clone)]
enum QueuedQueryKind {
    /// An agent prompt, with any image/file attachments captured from the input when it was
    /// queued. The attachments fire with the prompt and are dropped when the row is removed.
    Prompt { attachments: Vec<PendingAttachment> },
    /// A shell command run in the terminal (or via the shared session for cloud panes).
    Command,
}

/// A single queued row: an agent prompt or a shell command.
#[derive(Debug, Clone)]
pub struct QueuedQuery {
    id: QueuedQueryId,
    text: String,
    origin: QueuedQueryOrigin,
    kind: QueuedQueryKind,
}

impl QueuedQuery {
    pub fn new(text: String, origin: QueuedQueryOrigin) -> Self {
        Self::new_with_attachments(text, origin, Vec::new())
    }

    pub fn new_with_attachments(
        text: String,
        origin: QueuedQueryOrigin,
        attachments: Vec<PendingAttachment>,
    ) -> Self {
        Self {
            id: QueuedQueryId::new(),
            text,
            origin,
            kind: QueuedQueryKind::Prompt { attachments },
        }
    }

    /// Builds a queued shell command. Commands never carry attachments.
    pub fn new_command(text: String, origin: QueuedQueryOrigin) -> Self {
        Self {
            id: QueuedQueryId::new(),
            text,
            origin,
            kind: QueuedQueryKind::Command,
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

    /// Returns true if this row is a shell command rather than an agent prompt.
    pub fn is_command(&self) -> bool {
        matches!(self.kind, QueuedQueryKind::Command)
    }

    pub fn attachments(&self) -> &[PendingAttachment] {
        match &self.kind {
            QueuedQueryKind::Prompt { attachments } => attachments,
            QueuedQueryKind::Command => &[],
        }
    }

    /// Returns true if this row is locked from user mutation, reorder, and auto-fire.
    /// Currently only the locked initial Cloud Mode row is non-mutable; lifecycle code
    /// removes it explicitly via [`QueuedQueryModel::remove_initial_cloud_mode_row`].
    pub fn is_locked(&self) -> bool {
        matches!(self.origin, QueuedQueryOrigin::InitialCloudMode)
    }
}

/// What the auto-fire drain should do with the head row. Produced by
/// [`QueuedQueryModel::peek_autofire`] *without* removing the row; the caller removes it via
/// [`QueuedQueryModel::remove_fired_row`] once the prompt has been dispatched or restored.
#[derive(Debug)]
pub enum AutofireAction {
    /// Submit this prompt as a normal queued user query. The row stays in the queue so the send
    /// path can read its attachments by `query_id`; the caller removes it afterward.
    Submit {
        query_id: QueuedQueryId,
        text: String,
    },
    /// The head row was in edit mode. The caller restores `text` (the row's last committed text)
    /// and `attachments` to the input box, then removes the row. `is_command` distinguishes a
    /// shell command (no attachments; restored in shell mode) from an agent prompt, so the
    /// restored row keeps its kind instead of being re-submitted as the wrong type.
    PopFromEditMode {
        query_id: QueuedQueryId,
        text: String,
        attachments: Vec<PendingAttachment>,
        is_command: bool,
    },
    /// Execute this row as a shell command (its kind is `Command`). The caller runs the command,
    /// removes the row, and waits for the command to finish before draining the next row.
    ExecuteCommand {
        query_id: QueuedQueryId,
        command: String,
    },
}

/// Per-conversation queue / edit / toggle state.
/// Lives inside [`QueuedQueryModel::queues`]; a missing key means empty queue, no edit in
/// progress, and no explicit auto-queue override (so the cached default from
/// [`AISettings::default_prompt_submission_mode`] is used).
#[derive(Default)]
struct ConversationQueueState {
    queue: Vec<QueuedQuery>,
    editing: Option<QueuedQueryId>,
    /// Explicit per-conversation override. `None` defers to the model's cached
    /// `default_mode`; `Some` means the user has toggled this conversation
    /// at least once.
    queue_next_prompt_override: Option<bool>,
    /// True while a drained shell command from this queue is running. Set when the command is
    /// dispatched and cleared when it finishes; keeps the queue accepting new rows while the
    /// agent is idle and gates the next drain until the command completes.
    command_in_flight: bool,
    /// Manual queue toggle made during an agent-requested long-running command. Cleared when
    /// the command ends; never touches `queue_next_prompt_override`.
    queue_next_lrc_prompt_override: Option<bool>,
}

/// App-wide singleton owning the queued prompts and auto-queue toggle for every conversation,
/// indexed by [`AIConversationId`]. Queues outlive the agent-view session that originated them;
/// cleanup is driven by [`BlocklistAIHistoryModel`] lifecycle events that this model subscribes
/// to in [`QueuedQueryModel::new`].
pub struct QueuedQueryModel {
    queues: HashMap<AIConversationId, ConversationQueueState>,
    /// Cached value of the `AISettings::default_prompt_submission_mode` setting,
    /// refreshed by an `AISettingsChangedEvent::DefaultPromptSubmissionMode`
    /// subscription. Used as the fallback when a conversation has no explicit
    /// per-conversation override. Caching keeps the warping-indicator render
    /// path doing only a hashmap lookup plus a comparison.
    default_mode: PromptSubmissionMode,
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
    /// The `AISettings::default_prompt_submission_mode` setting changed, so the
    /// effective value of `is_queue_next_prompt_enabled` may have changed for
    /// every conversation without an explicit override. Subscribers that
    /// display the toggle state should re-render.
    DefaultModeChanged,
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
        ctx.subscribe_to_model(&history_handle, |this, _, event, ctx| {
            this.handle_history_event(event, ctx);
        });

        // Cache the default submission mode and refresh whenever the AI setting
        // changes. The render path consults the cache instead of dereferencing
        // the setting on every call. The LRC submission-mode setting is read by
        // callers directly, but its changes also re-emit `DefaultModeChanged` so
        // the chip and ghost text re-render with the new effective state.
        let default_mode = AISettings::as_ref(ctx).default_prompt_submission_mode;
        let ai_settings_handle = AISettings::handle(ctx);
        ctx.subscribe_to_model(&ai_settings_handle, |this, _, event, ctx| match event {
            AISettingsChangedEvent::PromptSubmissionMode { .. } => {
                this.default_mode = AISettings::as_ref(ctx).default_prompt_submission_mode;
                ctx.emit(QueuedQueryEvent::DefaultModeChanged);
            }
            AISettingsChangedEvent::LongRunningCommandSubmissionMode { .. } => {
                ctx.emit(QueuedQueryEvent::DefaultModeChanged);
            }
            _ => {}
        });

        Self {
            queues: HashMap::new(),
            default_mode,
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

    /// Returns true when a queued row would auto-fire for `conversation_id` the next time the
    /// conversation finishes successfully. Mirrors [`Self::peek_autofire`]'s gating: false for an
    /// empty queue or a locked head row (which never auto-fires).
    pub fn has_autofireable_prompt(&self, conversation_id: AIConversationId) -> bool {
        self.queues
            .get(&conversation_id)
            .and_then(|state| state.queue.first())
            .is_some_and(|first| !first.is_locked())
    }

    /// Marks that a dispatched queued command is running for `conversation_id`. While set, the
    /// queue keeps accepting new rows (the agent is idle) and the next drain waits for the
    /// command to finish.
    pub fn arm_command_in_flight(&mut self, conversation_id: AIConversationId) {
        self.queues
            .entry(conversation_id)
            .or_default()
            .command_in_flight = true;
    }

    /// Clears the in-flight-command marker for `conversation_id`.
    pub fn clear_command_in_flight(&mut self, conversation_id: AIConversationId) {
        if let Some(state) = self.queues.get_mut(&conversation_id) {
            state.command_in_flight = false;
        }
    }

    /// Returns true while a dispatched queued command is running for `conversation_id`.
    pub fn has_command_in_flight(&self, conversation_id: AIConversationId) -> bool {
        self.queues
            .get(&conversation_id)
            .is_some_and(|state| state.command_in_flight)
    }

    /// Returns the conversation owned by `terminal_view_id` that currently has a queued command in
    /// flight, if any.
    pub fn command_in_flight_for_terminal_view(
        &self,
        terminal_view_id: EntityId,
        history_model: &BlocklistAIHistoryModel,
    ) -> Option<AIConversationId> {
        history_model
            .all_live_conversations_for_terminal_view(terminal_view_id)
            .find_map(|conversation| {
                self.has_command_in_flight(conversation.id())
                    .then_some(conversation.id())
            })
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

    /// Returns the effective auto-queue state for `conversation_id`, given the terminal's
    /// `active_block`.
    pub fn is_queue_next_prompt_enabled(
        &self,
        conversation_id: AIConversationId,
        active_block: &Block,
        app: &AppContext,
    ) -> bool {
        if is_lrc_auto_queue_active(active_block, conversation_id, app) {
            // While an agent controls the active agent-requested command, the command-scoped
            // toggle governs queueing.
            self.is_queue_next_prompt_enabled_during_lrc(conversation_id)
        } else {
            // Otherwise the per-conversation toggle governs queueing.
            self.is_queue_next_prompt_toggle_enabled(conversation_id)
        }
    }

    /// Auto-queue state while an eligible agent-requested long-running command is active: on unless
    /// toggled off for the duration of the command.
    fn is_queue_next_prompt_enabled_during_lrc(&self, conversation_id: AIConversationId) -> bool {
        self.queues
            .get(&conversation_id)
            .and_then(|state| state.queue_next_lrc_prompt_override)
            .unwrap_or(true)
    }

    /// Per-conversation auto-queue toggle state, ignoring any long-running-command override:
    /// the explicit toggle when set, otherwise on when the default submission mode is `Queue`.
    pub(crate) fn is_queue_next_prompt_toggle_enabled(
        &self,
        conversation_id: AIConversationId,
    ) -> bool {
        self.queues
            .get(&conversation_id)
            .and_then(|state| state.queue_next_prompt_override)
            .unwrap_or(self.default_mode == PromptSubmissionMode::Queue)
    }

    /// Toggles the per-conversation auto-queue state. Computes the effective
    /// current value (which may come from the cached default) before writing
    /// its inverse as an explicit override, so toggling from the setting-driven
    /// default flips correctly.
    pub fn toggle_queue_next_prompt(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let current = self.is_queue_next_prompt_toggle_enabled(conversation_id);
        let state = self.queues.entry(conversation_id).or_default();
        state.queue_next_prompt_override = Some(!current);
        ctx.emit(QueuedQueryEvent::QueueNextPromptToggled { conversation_id });
    }

    /// Toggles the auto-queue state for the duration of the eligible long-running command.
    pub fn toggle_queue_next_prompt_during_lrc(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let current = self.is_queue_next_prompt_enabled_during_lrc(conversation_id);
        let state = self.queues.entry(conversation_id).or_default();
        state.queue_next_lrc_prompt_override = Some(!current);
        ctx.emit(QueuedQueryEvent::QueueNextPromptToggled { conversation_id });
    }

    /// Clears the LRC-scoped auto-queue override when the long-running command ends, so the
    /// conversation reverts to its pre-command queue state.
    pub fn clear_queue_next_lrc_prompt_override(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        if state.queue_next_lrc_prompt_override.take().is_some() {
            ctx.emit(QueuedQueryEvent::QueueNextPromptToggled { conversation_id });
        }
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

    /// Auto-fire drain entry point for `conversation_id`. Returns the action for the head row
    /// *without* removing it (so the send path can read its attachments by id), or `None` for an
    /// empty queue or a locked head ([`QueuedQuery::is_locked`]). The caller removes the row via
    /// [`Self::remove_fired_row`] once it has been dispatched or restored to the input.
    pub fn peek_autofire(&self, conversation_id: AIConversationId) -> Option<AutofireAction> {
        let state = self.queues.get(&conversation_id)?;
        let first = state.queue.first()?;
        if first.is_locked() {
            return None;
        }
        let first_in_edit_mode = state.editing == Some(first.id);
        Some(if first_in_edit_mode {
            AutofireAction::PopFromEditMode {
                query_id: first.id,
                text: first.text.clone(),
                attachments: first.attachments().to_vec(),
                is_command: first.is_command(),
            }
        } else if first.is_command() {
            AutofireAction::ExecuteCommand {
                query_id: first.id,
                command: first.text.clone(),
            }
        } else {
            AutofireAction::Submit {
                query_id: first.id,
                text: first.text.clone(),
            }
        })
    }

    /// Removes the row `query_id` from `conversation_id`'s queue after it has been fired. In the
    /// edit-mode auto-fire path, the caller first restores the row's committed text and
    /// attachments to the input, then calls this to drop the row and clear edit state.
    pub fn remove_fired_row(
        &mut self,
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        let Some(idx) = state.queue.iter().position(|q| q.id == query_id) else {
            return;
        };
        state.queue.remove(idx);
        if state.editing == Some(query_id) {
            state.editing = None;
        }
        ctx.emit(QueuedQueryEvent::Removed {
            conversation_id,
            query_id,
        });
    }

    /// Restores a fired row when submission fails after the row was removed.
    pub(crate) fn restore_fired_row(
        &mut self,
        conversation_id: AIConversationId,
        insert_index: usize,
        query: QueuedQuery,
        ctx: &mut ModelContext<Self>,
    ) {
        let state = self.queues.entry(conversation_id).or_default();
        let query_id = query.id;
        if state.queue.iter().any(|queued| queued.id == query_id) {
            return;
        }
        let insert_index = insert_index.min(state.queue.len());
        state.queue.insert(insert_index, query);
        ctx.emit(QueuedQueryEvent::Appended {
            conversation_id,
            query_id,
        });
    }

    /// Returns the attachments captured on the queued row `query_id` within `conversation_id`'s
    /// queue, or an empty slice if no such row exists. Used by the send path to attach a fired
    /// queued prompt's images/files without removing the row first.
    pub fn attachments_for(
        &self,
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
    ) -> &[PendingAttachment] {
        self.queues
            .get(&conversation_id)
            .and_then(|state| state.queue.iter().find(|q| q.id == query_id))
            .map(QueuedQuery::attachments)
            .unwrap_or(&[])
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

/// Returns true when queue mode is auto-enabled for `conversation_id`: an agent controls
/// `active_block`'s agent-requested long-running command, and the user's settings opt into
/// queueing prompts for the duration of such commands.
pub(crate) fn is_lrc_auto_queue_active(
    active_block: &Block,
    conversation_id: AIConversationId,
    app: &AppContext,
) -> bool {
    let ai_settings = AISettings::as_ref(app);
    FeatureFlag::QueueSlashCommand.is_enabled()
        && ai_settings.default_prompt_submission_mode == PromptSubmissionMode::Interrupt
        && ai_settings.long_running_command_submission_mode
            == LongRunningCommandSubmissionMode::QueueUntilCommandCompletes
        && active_block.is_agent_in_control()
        && active_block.is_agent_requested_command()
        && active_block.ai_conversation_id() == Some(conversation_id)
}

#[cfg(test)]
#[path = "queued_query_tests.rs"]
mod tests;
