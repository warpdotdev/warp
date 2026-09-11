use std::collections::HashMap;

use session_sharing_protocol::common::{AgentAttachment, ParticipantId};
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
    /// Received through session sharing while a native run was starting.
    SharedSessionInjection,
    /// Filed via the `/queue <prompt>` slash command.
    QueueSlashCommand,
    /// Filed via the auto-queue toggle in the warping indicator.
    AutoQueueToggle,
    /// Filed because auto-queue was in effect during an agent-requested long-running command.
    LrcAutoQueue,
    /// Filed while an agent-requested run_shell_command action's snapshot has not yet fired.
    /// Locked for manual push and auto-fire until the snapshot fires.
    PendingLrcAutoQueue,
    /// Filed as the follow-up prompt of a `/compact-and <prompt>` slash command, waiting for
    /// the summarize to finish.
    CompactAndSlashCommand,
    /// Filed as the follow-up prompt of a `/fork-and-compact <prompt>` slash command on the
    /// forked conversation, waiting for the fork's summarize to finish.
    ForkAndCompactSlashCommand,
}

/// Whether a queued row is a local prompt, an attributed shared-session prompt, or a command.
#[derive(Debug, Clone)]
enum QueuedQueryKind {
    /// An agent prompt, with any image/file attachments captured from the input when it was
    /// queued. The attachments fire with the prompt and are dropped when the row is removed.
    Prompt { attachments: Vec<PendingAttachment> },
    SharedSessionPrompt {
        participant_id: ParticipantId,
        attachments: Vec<AgentAttachment>,
    },
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
    pub(crate) fn new_shared_session_prompt(
        text: String,
        participant_id: ParticipantId,
        attachments: Vec<AgentAttachment>,
    ) -> Self {
        Self {
            id: QueuedQueryId::new(),
            text,
            origin: QueuedQueryOrigin::SharedSessionInjection,
            kind: QueuedQueryKind::SharedSessionPrompt {
                participant_id,
                attachments,
            },
        }
    }

    pub(crate) fn shared_session_prompt(&self) -> Option<(&ParticipantId, &[AgentAttachment])> {
        match &self.kind {
            QueuedQueryKind::SharedSessionPrompt {
                participant_id,
                attachments,
            } => Some((participant_id, attachments)),
            QueuedQueryKind::Prompt { .. } | QueuedQueryKind::Command => None,
        }
    }

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
            QueuedQueryKind::Command | QueuedQueryKind::SharedSessionPrompt { .. } => &[],
        }
    }

    /// Returns true if this row is locked from user mutation, reorder, and auto-fire.
    /// Locked rows cannot be edited, deleted, reordered, pushed manually, or auto-fired by
    /// the drain mechanism. The initial Cloud Mode row is locked permanently; PendingLrcAutoQueue
    /// rows are locked only until the action snapshot fires.
    pub fn is_locked(&self) -> bool {
        matches!(
            self.origin,
            QueuedQueryOrigin::InitialCloudMode | QueuedQueryOrigin::PendingLrcAutoQueue
        )
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

/// How queued prompts for a conversation are delivered to the agent. Selected per-conversation;
/// not user-facing yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum QueuedPromptDeliveryMode {
    /// The next queued row is sent only once the conversation fully finishes on its own
    /// (`FinishedReceivingOutput` with a genuine finish reason). Used for local queueing
    /// surfaces (`/queue`, the auto-queue toggle, LRC auto-queue).
    #[default]
    Queueing,
    /// A queued row is sent on the next request made for the conversation -- a natural
    /// continuation (e.g. a tool-result follow-up or an orchestration-event injection) if one
    /// occurs first, otherwise the conversation going fully idle -- rather than always waiting
    /// for the whole turn to finish. Each row is still sent as its own individual
    /// request/exchange, never combined with another queued row. Set automatically for
    /// conversations bound to an ambient/Oz-driven native run
    /// (`BlocklistAIController::bind_native_prompt_conversation`).
    Steering,
}

/// Per-conversation queue / edit / toggle state.
/// Lives inside [`QueuedQueryModel::queues`]; a missing key means empty queue, no edit in
/// progress, and no explicit auto-queue override (so the cached default from
/// [`AISettings::default_prompt_submission_mode`] is used).
#[derive(Default)]
struct ConversationQueueState {
    queue: Vec<QueuedQuery>,
    /// True from when this conversation is bound for native startup injections until its
    /// initial prompt is actually sent. Sharing can deliver startup follow-ups during this
    /// window; they are held in `queue` until setup finishes, at which point normal dispatch
    /// (steering or idle-drain, depending on `delivery_mode`) takes over.
    native_setup_pending: bool,
    /// How queued rows for this conversation are delivered. See [`QueuedPromptDeliveryMode`].
    delivery_mode: QueuedPromptDeliveryMode,
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
    DispatchStateChanged {
        conversation_id: AIConversationId,
    },
    Appended {
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
    },
    /// Emitted when PendingLrcAutoQueue rows are transitioned to LrcAutoQueue after
    /// the action snapshot fires.
    RowUnlocked {
        conversation_id: AIConversationId,
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
    pub(crate) fn begin_native_setup(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.queues
            .entry(conversation_id)
            .or_default()
            .native_setup_pending = true;
        ctx.emit(QueuedQueryEvent::DispatchStateChanged { conversation_id });
    }

    /// Clears the native setup barrier once the initial prompt has actually been sent. Does
    /// *not* dispatch any prompt that arrived in the meantime -- the caller
    /// (`BlocklistAIController::dispatch_queued_warp_agent_prompt`) does that immediately
    /// afterward, once it's safe to do so (see that method's doc comment for why the two can't
    /// be combined into one step here).
    pub(crate) fn finish_native_setup(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(state) = self.queues.get_mut(&conversation_id)
            && state.native_setup_pending
        {
            state.native_setup_pending = false;
            log::info!(
                "event=setup_released conversation_id={conversation_id} queue_len={}",
                state.queue.len(),
            );
            ctx.emit(QueuedQueryEvent::DispatchStateChanged { conversation_id });
        }
    }

    /// True while native startup follow-ups for `conversation_id` must be held rather than
    /// dispatched (auto-fire, "Send now", and Enter-to-send all consult this).
    pub(crate) fn is_dispatch_blocked(&self, conversation_id: AIConversationId) -> bool {
        self.queues
            .get(&conversation_id)
            .is_some_and(|state| state.native_setup_pending)
    }

    /// Sets the delivery mode for `conversation_id`'s queue. See [`QueuedPromptDeliveryMode`].
    pub(crate) fn set_delivery_mode(
        &mut self,
        conversation_id: AIConversationId,
        mode: QueuedPromptDeliveryMode,
    ) {
        self.queues
            .entry(conversation_id)
            .or_default()
            .delivery_mode = mode;
    }

    /// Returns the delivery mode for `conversation_id`'s queue, defaulting to `Queueing` when no
    /// mode has been explicitly set. See [`QueuedPromptDeliveryMode`].
    pub(crate) fn delivery_mode(
        &self,
        conversation_id: AIConversationId,
    ) -> QueuedPromptDeliveryMode {
        self.queues
            .get(&conversation_id)
            .map(|state| state.delivery_mode)
            .unwrap_or_default()
    }

    /// True when `conversation_id`'s queue is in `Steering` mode.
    pub(crate) fn is_steering(&self, conversation_id: AIConversationId) -> bool {
        self.delivery_mode(conversation_id) == QueuedPromptDeliveryMode::Steering
    }

    /// True when `conversation_id` still has native startup work outstanding: either setup
    /// hasn't finished yet, or one or more rows are still queued waiting to be dispatched (e.g.
    /// a dispatch was deferred because a CLI subagent was active). Used by the ambient driver to
    /// know whether to keep the run alive for pending injections.
    pub(crate) fn has_pending_native_injections(&self, conversation_id: AIConversationId) -> bool {
        self.queues
            .get(&conversation_id)
            .is_some_and(|state| state.native_setup_pending || !state.queue.is_empty())
    }

    /// Removes and returns every row queued for `conversation_id`, in FIFO order, emitting a
    /// `Removed` event for each. Used by
    /// `BlocklistAIController::unbind_native_prompt_conversation` to drop any prompts that never
    /// made it out when the run ends, regardless of whether they were queued locally or via a
    /// shared-session injection.
    pub(crate) fn clear_queue(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Vec<QueuedQuery> {
        let Some(state) = self.queues.get_mut(&conversation_id) else {
            return Vec::new();
        };
        let cleared = std::mem::take(&mut state.queue);
        state.editing = None;
        for row in &cleared {
            ctx.emit(QueuedQueryEvent::Removed {
                conversation_id,
                query_id: row.id,
            });
        }
        cleared
    }

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
            BlocklistAIHistoryEvent::UpdatedConversationStatus { .. } => {
                // Steering-mode conversations don't rely on this event to deliver queued rows:
                // they're dispatched one at a time as soon as a natural request boundary occurs
                // (see `BlocklistAIController::steer_head_prompt_for_request` and
                // `dispatch_queued_warp_agent_prompt`). `TerminalView`'s own turn-completion
                // drain (`drain_queued_prompts`) remains the fallback for both modes once a
                // turn genuinely finishes.
            }
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                self.drop_conversation(*conversation_id, ctx);
            }
            BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
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
        if let Some(state) = self.queues.remove(&conversation_id) {
            if !state.queue.is_empty() {
                log::warn!(
                    "event=queue_discarded conversation_id={conversation_id} reason=conversation_removed queue_len={}",
                    state.queue.len(),
                );
            }
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
        !self.is_dispatch_blocked(conversation_id)
            && self
                .queues
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
            .all_live_conversations_for_terminal_surface(terminal_view_id)
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

    /// Transitions all `PendingLrcAutoQueue` rows for `conversation_id` to `LrcAutoQueue`,
    /// unlocking them for auto-fire when the command completes. Emits `RowUnlocked` if any
    /// rows were changed.
    pub fn unlock_pending_lrc_rows(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        let mut unlocked = false;
        for row in state.queue.iter_mut() {
            if row.origin == QueuedQueryOrigin::PendingLrcAutoQueue {
                row.origin = QueuedQueryOrigin::LrcAutoQueue;
                unlocked = true;
            }
        }
        if unlocked {
            ctx.emit(QueuedQueryEvent::RowUnlocked { conversation_id });
        }
    }

    /// Removes all `PendingLrcAutoQueue` rows for `conversation_id` so stale locked
    /// rows do not linger.
    pub fn remove_pending_lrc_rows(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(state) = self.queues.get_mut(&conversation_id) else {
            return;
        };
        let mut removed_ids = Vec::new();
        state.queue.retain(|row| {
            if row.origin == QueuedQueryOrigin::PendingLrcAutoQueue {
                removed_ids.push(row.id);
                false
            } else {
                true
            }
        });
        for query_id in removed_ids {
            ctx.emit(QueuedQueryEvent::Removed {
                conversation_id,
                query_id,
            });
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
        log::info!(
            "event=queued_prompt_appended conversation_id={conversation_id} query_id={query_id:?} origin={:?} participant_id={:?} queue_len={} setup_pending={}",
            query.origin,
            query
                .shared_session_prompt()
                .map(|(participant_id, _)| participant_id),
            state.queue.len() + 1,
            state.native_setup_pending,
        );
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
        if self.is_dispatch_blocked(conversation_id) {
            return None;
        }
        let state = self.queues.get_mut(&conversation_id)?;
        if state.queue.first()?.is_locked()
            || state.queue.first()?.shared_session_prompt().is_some()
        {
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
        if let Some(state) = self.queues.get(&conversation_id)
            && !state.queue.is_empty()
            && (self.is_dispatch_blocked(conversation_id) || state.queue[0].is_locked())
        {
            log::info!(
                "event=queue_drain_blocked conversation_id={conversation_id} queue_len={} head_id={:?} setup_pending={} head_locked={}",
                state.queue.len(),
                state.queue[0].id,
                state.native_setup_pending,
                state.queue[0].is_locked(),
            );
        }
        if self.is_dispatch_blocked(conversation_id) {
            return None;
        }
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
        if state.queue[idx].shared_session_prompt().is_none() {
            log::info!(
                "event=queued_prompt_removed conversation_id={conversation_id} query_id={query_id:?} reason=dispatched_or_restored queue_len_after={}",
                state.queue.len() - 1,
            );
        }
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
        log::warn!(
            "event=restored_after_failed_send conversation_id={conversation_id} query_id={query_id:?} index={insert_index} queue_len_after={}",
            state.queue.len() + 1,
        );
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
        log::info!(
            "event=deleted conversation_id={conversation_id} query_id={query_id:?} queue_len_after={}",
            state.queue.len() - 1,
        );
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
            .any(|q| q.id == query_id && !q.is_locked() && q.shared_session_prompt().is_none())
        {
            return;
        }
        let prev_edit = state.editing.replace(query_id);
        if let Some(prev) = prev_edit
            && prev != query_id
        {
            ctx.emit(QueuedQueryEvent::EditCancelled {
                conversation_id,
                query_id: prev,
            });
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
