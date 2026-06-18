use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use session_sharing_protocol::common::SessionId;
use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use super::history_model::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, ConversationStatusUpdate,
};
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::{AIAgentOutputStatus, FinishedAIAgentOutput, RenderableAIError};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{AIClient, TaskStatusUpdate};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::cli_agent_sessions::{
    CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};

/// Syncs locally-owned conversation state to the server `ai_tasks` row via
/// `AIClient::update_agent_task`. This includes task state, status message,
/// server conversation token (`conversation_id`), and shared session ID.
///
/// For Oz harness conversations, the model listens to
/// `BlocklistAIHistoryEvent::UpdatedConversationStatus` (state transitions)
/// and `BlocklistAIHistoryEvent::ConversationServerTokenAssigned` (so the
/// server conversation token is persisted as soon as the streamed `Init`
/// event arrives). It also handles
/// `BlocklistAIHistoryEvent::LocalSharedSessionEstablished` to link
/// shared session IDs to the task row.
///
/// For third-party harnesses (e.g. Claude Code), status is derived from
/// `CLIAgentSessionsModelEvent::StatusChanged`. Because these sessions do
/// not create conversations in the history model, the driver must register
/// a `terminal_view_id → task_id` mapping via `register_cli_session`.
pub struct LocalAgentTaskSyncModel {
    ai_client: Arc<dyn AIClient>,
    /// Maps terminal view IDs to task IDs for third-party harness sessions
    /// that don't have conversations in `BlocklistAIHistoryModel`.
    cli_session_task_ids: HashMap<EntityId, AmbientAgentTaskId>,
    /// Tracks model-owned `InProgress` delivery so redundant state transitions
    /// can be omitted without suppressing other fields in the same update.
    in_progress_delivery_states: HashMap<AmbientAgentTaskId, InProgressDeliveryState>,
    next_in_progress_delivery_generation: u64,
    /// Tracks the last successfully delivered server conversation token for
    /// each task. Unlike task state, the token is durable across executions.
    server_conversation_token_delivery_states:
        HashMap<AmbientAgentTaskId, ServerConversationTokenDeliveryState>,
    next_server_conversation_token_delivery_generation: u64,
    /// Disables token dedupe for tasks where different tokens overlapped in
    /// flight. Without per-task FIFO ordering, failing open is safest.
    server_conversation_token_dedupe_disabled: HashSet<AmbientAgentTaskId>,
}

pub enum LocalAgentTaskSyncModelEvent {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InProgressDeliveryState {
    InFlight(u64),
    Delivered,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ServerConversationTokenDeliveryState {
    InFlight {
        generation: u64,
        token: String,
        previously_delivered: Option<String>,
    },
    Delivered(String),
}
/// Aggregated update to send via `AIClient::update_agent_task`. Field names
/// match the server input shape so it is unambiguous which value flows to
/// which server field.
///
/// `server_conversation_token` is the server-assigned conversation token
/// (see `ServerConversationToken`), passed to the server in the
/// `conversation_id` field of `UpdateAgentTaskInput`. It is intentionally
/// distinct from the client-local `AIConversationId`, which never crosses
/// this boundary.
#[derive(Default)]
struct LocalTaskUpdate {
    task_state: Option<AgentTaskState>,
    session_id: Option<SessionId>,
    server_conversation_token: Option<String>,
    status_message: Option<TaskStatusUpdate>,
}
impl LocalTaskUpdate {
    fn is_empty(&self) -> bool {
        self.task_state.is_none()
            && self.session_id.is_none()
            && self.server_conversation_token.is_none()
            && self.status_message.is_none()
    }
}

impl LocalAgentTaskSyncModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        Self::new_with_ai_client(ai_client, ctx)
    }

    fn new_with_ai_client(ai_client: Arc<dyn AIClient>, ctx: &mut ModelContext<Self>) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |me, event, ctx| {
            me.handle_history_event(event, ctx);
        });

        let cli_sessions_model = CLIAgentSessionsModel::handle(ctx);
        ctx.subscribe_to_model(&cli_sessions_model, |me, event, ctx| {
            me.handle_cli_session_event(event, ctx);
        });

        Self {
            ai_client,
            cli_session_task_ids: HashMap::new(),
            in_progress_delivery_states: HashMap::new(),
            next_in_progress_delivery_generation: 0,
            server_conversation_token_delivery_states: HashMap::new(),
            next_server_conversation_token_delivery_generation: 0,
            server_conversation_token_dedupe_disabled: HashSet::new(),
        }
    }

    /// Test-only constructor that lets tests inject a mock `AIClient`.
    #[cfg(test)]
    pub(super) fn new_with_ai_client_for_test(
        ai_client: Arc<dyn AIClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self::new_with_ai_client(ai_client, ctx)
    }

    /// Registers a terminal view as a tracked CLI agent session so that
    /// status changes from `CLIAgentSessionsModel` are reported to the
    /// server. Called by `AgentDriver` when setting up a third-party
    /// harness run.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn register_cli_session(
        &mut self,
        terminal_view_id: EntityId,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.cli_session_task_ids.insert(terminal_view_id, task_id);
        // Report IN_PROGRESS immediately because the initial
        // `register_listener` call on `CLIAgentSessionsModel` never emits a
        // `StatusChanged` event, so we must report it at registration time.
        self.fire_update(
            task_id,
            LocalTaskUpdate {
                task_state: Some(AgentTaskState::InProgress),
                ..LocalTaskUpdate::default()
            },
            ctx,
        );
    }
    fn remove_delivery_state(&mut self, task_id: &AmbientAgentTaskId) {
        self.in_progress_delivery_states.remove(task_id);
        self.server_conversation_token_delivery_states
            .remove(task_id);
        self.server_conversation_token_dedupe_disabled
            .remove(task_id);
    }

    fn remove_delivery_state_for_run_id(&mut self, run_id: Option<&str>) {
        let Some(task_id) = run_id.and_then(|run_id| run_id.parse::<AmbientAgentTaskId>().ok())
        else {
            return;
        };
        self.remove_delivery_state(&task_id);
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id,
                update,
                ..
            } => {
                if matches!(update, ConversationStatusUpdate::Changed { .. }) {
                    self.on_conversation_status_updated(*conversation_id, ctx);
                }
            }
            // When the server token (and thus task_id) is first assigned to a
            // conversation, report its current status. This handles the race
            // where ConversationStatus::InProgress fires before task_id is
            // available — we catch up here once the task_id arrives.
            BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id, ..
            } => {
                self.on_conversation_status_updated(*conversation_id, ctx);
            }
            BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                conversation_id,
                session_id,
            } => {
                self.on_local_shared_session_established(*conversation_id, *session_id, ctx);
            }
            BlocklistAIHistoryEvent::RemoveConversation { run_id, .. }
            | BlocklistAIHistoryEvent::DeletedConversation { run_id, .. } => {
                self.remove_delivery_state_for_run_id(run_id.as_deref());
            }
            _ => {}
        }
    }

    fn handle_cli_session_event(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                status,
                ..
            } => {
                self.on_cli_session_status_changed(*terminal_view_id, status, ctx);
            }
            CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            } => {
                if let Some(task_id) = self.cli_session_task_ids.remove(terminal_view_id) {
                    self.remove_delivery_state(&task_id);
                }
            }
            _ => {}
        }
    }

    fn on_conversation_status_updated(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some((task_id, update)) =
            with_local_conversation(conversation_id, ctx, |conversation| {
                let (task_state, status_message) = map_conversation_status(conversation);
                LocalTaskUpdate {
                    task_state: Some(task_state),
                    server_conversation_token: conversation
                        .server_conversation_token()
                        .map(|token| token.as_str().to_string()),
                    status_message,
                    ..LocalTaskUpdate::default()
                }
            })
        else {
            return;
        };

        self.fire_update(task_id, update, ctx);
    }

    fn on_local_shared_session_established(
        &mut self,
        conversation_id: AIConversationId,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some((task_id, update)) =
            with_local_conversation(conversation_id, ctx, |_| LocalTaskUpdate {
                session_id: Some(session_id),
                ..LocalTaskUpdate::default()
            })
        else {
            return;
        };

        self.fire_update(task_id, update, ctx);
    }

    fn on_cli_session_status_changed(
        &mut self,
        terminal_view_id: EntityId,
        status: &CLIAgentSessionStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(&task_id) = self.cli_session_task_ids.get(&terminal_view_id) else {
            return;
        };

        let (task_state, status_message) = map_cli_session_status(status);
        self.fire_update(
            task_id,
            LocalTaskUpdate {
                task_state: Some(task_state),
                status_message,
                ..LocalTaskUpdate::default()
            },
            ctx,
        );
    }

    /// Sends an `update_agent_task` request to the server (fire-and-forget).
    fn fire_update(
        &mut self,
        task_id: AmbientAgentTaskId,
        mut update: LocalTaskUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut server_conversation_token_delivery_generation = None;
        if let Some(token) = update.server_conversation_token.clone() {
            if !self
                .server_conversation_token_dedupe_disabled
                .contains(&task_id)
            {
                let delivery_state = self
                    .server_conversation_token_delivery_states
                    .get(&task_id)
                    .cloned();
                match delivery_state {
                    Some(ServerConversationTokenDeliveryState::Delivered(delivered_token))
                        if delivered_token == token =>
                    {
                        update.server_conversation_token = None;
                    }
                    Some(ServerConversationTokenDeliveryState::InFlight {
                        token: in_flight_token,
                        ..
                    }) if in_flight_token != token => {
                        // Different tokens cannot be safely ordered by the current
                        // fire-and-forget transport, so stop deduping this task.
                        self.server_conversation_token_delivery_states
                            .remove(&task_id);
                        self.server_conversation_token_dedupe_disabled
                            .insert(task_id);
                    }
                    delivery_state => {
                        let previously_delivered = match delivery_state {
                            Some(ServerConversationTokenDeliveryState::Delivered(token)) => {
                                Some(token)
                            }
                            Some(ServerConversationTokenDeliveryState::InFlight {
                                previously_delivered,
                                ..
                            }) => previously_delivered,
                            None => None,
                        };
                        self.next_server_conversation_token_delivery_generation = self
                            .next_server_conversation_token_delivery_generation
                            .wrapping_add(1);
                        let generation = self.next_server_conversation_token_delivery_generation;
                        self.server_conversation_token_delivery_states.insert(
                            task_id,
                            ServerConversationTokenDeliveryState::InFlight {
                                generation,
                                token,
                                previously_delivered,
                            },
                        );
                        server_conversation_token_delivery_generation = Some(generation);
                    }
                }
            }
        }
        let mut in_progress_delivery_generation = None;
        match update.task_state {
            // Status messages are only applied by the server together with a
            // task state, so an update carrying one must retain its state.
            Some(AgentTaskState::InProgress) if update.status_message.is_none() => {
                match self.in_progress_delivery_states.get(&task_id).copied() {
                    Some(InProgressDeliveryState::Delivered) => {
                        update.task_state = None;
                    }
                    Some(InProgressDeliveryState::InFlight(_))
                        if update.session_id.is_none()
                            && update.server_conversation_token.is_none() =>
                    {
                        return;
                    }
                    Some(InProgressDeliveryState::InFlight(_)) | None => {
                        self.next_in_progress_delivery_generation =
                            self.next_in_progress_delivery_generation.wrapping_add(1);
                        let generation = self.next_in_progress_delivery_generation;
                        self.in_progress_delivery_states
                            .insert(task_id, InProgressDeliveryState::InFlight(generation));
                        in_progress_delivery_generation = Some(generation);
                    }
                }
            }
            Some(_) => {
                self.in_progress_delivery_states.remove(&task_id);
            }
            None => {}
        }

        if update.is_empty() {
            return;
        }
        let ai_client = self.ai_client.clone();
        let LocalTaskUpdate {
            task_state,
            session_id,
            server_conversation_token,
            status_message,
        } = update;
        ctx.spawn(
            async move {
                let result = ai_client
                    .update_agent_task(
                        task_id,
                        task_state,
                        session_id,
                        server_conversation_token.clone(),
                        status_message,
                    )
                    .await;
                if let Err(err) = &result {
                    log::warn!(
                        "LocalAgentTaskSyncModel: failed to update task {task_id} \
                         (state={task_state:?}, session_id={session_id:?}, \
                         server_conversation_token={server_conversation_token:?}): {err:#}"
                    );
                }
                result
            },
            move |me, result, _| {
                if let Some(generation) = in_progress_delivery_generation {
                    if me.in_progress_delivery_states.get(&task_id)
                        == Some(&InProgressDeliveryState::InFlight(generation))
                    {
                        if result.is_ok() {
                            me.in_progress_delivery_states
                                .insert(task_id, InProgressDeliveryState::Delivered);
                        } else {
                            me.in_progress_delivery_states.remove(&task_id);
                        }
                    }
                }

                if let Some(generation) = server_conversation_token_delivery_generation {
                    if me
                        .server_conversation_token_dedupe_disabled
                        .contains(&task_id)
                    {
                        return;
                    }
                    let Some(ServerConversationTokenDeliveryState::InFlight {
                        generation: current_generation,
                        token,
                        previously_delivered,
                    }) = me
                        .server_conversation_token_delivery_states
                        .get(&task_id)
                        .cloned()
                    else {
                        return;
                    };
                    if current_generation != generation {
                        return;
                    }

                    if result.is_ok() {
                        me.server_conversation_token_delivery_states.insert(
                            task_id,
                            ServerConversationTokenDeliveryState::Delivered(token),
                        );
                    } else if let Some(previously_delivered) = previously_delivered {
                        me.server_conversation_token_delivery_states.insert(
                            task_id,
                            ServerConversationTokenDeliveryState::Delivered(previously_delivered),
                        );
                    } else {
                        me.server_conversation_token_delivery_states
                            .remove(&task_id);
                    }
                }
            },
        );
    }
}

impl Entity for LocalAgentTaskSyncModel {
    type Event = LocalAgentTaskSyncModelEvent;
}

impl SingletonEntity for LocalAgentTaskSyncModel {}

/// Resolves a conversation ID to a `(task_id, value)` pair when the
/// conversation is owned by this client. Returns `None` for viewer
/// conversations, remote-child placeholders, conversations without a
/// `task_id`, and unknown conversation IDs.
fn with_local_conversation<T>(
    conversation_id: AIConversationId,
    ctx: &ModelContext<LocalAgentTaskSyncModel>,
    make_value: impl FnOnce(&AIConversation) -> T,
) -> Option<(AmbientAgentTaskId, T)> {
    let history = BlocklistAIHistoryModel::as_ref(ctx);
    let conversation = history.conversation(&conversation_id)?;
    // Viewers of shared sessions must not report status — they don't
    // own the task. Currently also protected by the absence of task_id,
    // but this guard makes the intent explicit.
    if conversation.is_viewing_shared_session() {
        return None;
    }
    // Skip remote child placeholder conversations — the remote worker's
    // own client handles status reporting. Reporting here would
    // prematurely move remote tasks from QUEUED to IN_PROGRESS before
    // the worker can claim them. Local children are NOT skipped because
    // they execute in this client and have no separate reporter.
    if conversation.is_remote_child() {
        return None;
    }
    let task_id = conversation.task_id()?;
    Some((task_id, make_value(conversation)))
}

/// Maps conversation state to an `AgentTaskState` and optional status message.
/// For errors, extracts the specific error from the last exchange when available.
fn map_conversation_status(
    conversation: &AIConversation,
) -> (AgentTaskState, Option<TaskStatusUpdate>) {
    match conversation.status() {
        ConversationStatus::InProgress => (AgentTaskState::InProgress, None),
        // Report WaitingForEvents as IN_PROGRESS so the server task state
        // matches the local view.
        ConversationStatus::WaitingForEvents => (AgentTaskState::InProgress, None),
        ConversationStatus::Success => (AgentTaskState::Succeeded, None),
        // Recovery pending: stay IN_PROGRESS, no message — `update_agent_task`
        // can't clear it later, so a "reconnecting" note would linger after resume.
        ConversationStatus::TransientError => (AgentTaskState::InProgress, None),
        ConversationStatus::Error => {
            // Extract the specific RenderableAIError from the last exchange to
            // classify ERROR vs FAILED and provide a PlatformErrorCode.
            let renderable_error = conversation
                .root_task_exchanges()
                .last()
                .and_then(|exchange| {
                    if let AIAgentOutputStatus::Finished {
                        finished_output: FinishedAIAgentOutput::Error { error, .. },
                    } = &exchange.output_status
                    {
                        Some(error)
                    } else {
                        None
                    }
                });
            task_update_for_conversation_error(renderable_error)
        }
        ConversationStatus::Cancelled => (
            AgentTaskState::Cancelled,
            Some(TaskStatusUpdate::message("Cancelled by user")),
        ),
        ConversationStatus::Blocked { blocked_action } => (
            AgentTaskState::Blocked,
            Some(TaskStatusUpdate::message(format!(
                "The agent got stuck waiting for user confirmation on the action: {blocked_action}"
            ))),
        ),
    }
}

/// Maps a conversation-level error to a terminal task update. In-flight recoveries
/// surface as `TransientError`, so an `Error` status is always terminal here — the
/// `will_attempt_resume` rendering hint is deliberately ignored.
fn task_update_for_conversation_error(
    error: Option<&RenderableAIError>,
) -> (AgentTaskState, Option<TaskStatusUpdate>) {
    match error {
        Some(error) => classify_renderable_error(error),
        None => (
            AgentTaskState::Error,
            Some(TaskStatusUpdate::message("Agent encountered an error")),
        ),
    }
}

/// Classifies a `RenderableAIError` into an `AgentTaskState` (ERROR vs FAILED)
/// and a `TaskStatusUpdate` with a `PlatformErrorCode` where applicable.
pub(crate) fn classify_renderable_error(
    error: &RenderableAIError,
) -> (AgentTaskState, Option<TaskStatusUpdate>) {
    match error {
        RenderableAIError::QuotaLimit {
            user_display_message,
        } => (
            AgentTaskState::Failed,
            Some(TaskStatusUpdate::with_error_code(
                user_display_message.as_deref().unwrap_or(
                    "Your team has run out of credits. Purchase more credits to continue.",
                ),
                PlatformErrorCode::InsufficientCredits,
            )),
        ),
        RenderableAIError::ServerOverloaded => (
            AgentTaskState::Error,
            Some(TaskStatusUpdate::with_error_code(
                "Warp is temporarily overloaded. Please try again shortly.",
                PlatformErrorCode::ResourceUnavailable,
            )),
        ),
        RenderableAIError::InternalWarpError => (
            AgentTaskState::Error,
            Some(TaskStatusUpdate::with_error_code(
                "An internal error occurred during the conversation. Please try again.",
                PlatformErrorCode::InternalError,
            )),
        ),
        RenderableAIError::ContextWindowExceeded(msg) => (
            AgentTaskState::Failed,
            Some(TaskStatusUpdate::with_error_code(
                format!("Context window exceeded: {msg}"),
                PlatformErrorCode::InternalError,
            )),
        ),
        RenderableAIError::InvalidApiKey { provider, .. } => (
            AgentTaskState::Failed,
            Some(TaskStatusUpdate::with_error_code(
                format!("Invalid API key for {provider}. Update your API key in settings."),
                PlatformErrorCode::AuthenticationRequired,
            )),
        ),
        RenderableAIError::AwsBedrockCredentialsExpiredOrInvalid { model_name } => (
            AgentTaskState::Failed,
            Some(TaskStatusUpdate::with_error_code(
                format!("AWS Bedrock credentials expired or invalid for {model_name}."),
                PlatformErrorCode::AuthenticationRequired,
            )),
        ),
        RenderableAIError::TransientNetworkError { .. } => (
            AgentTaskState::Error,
            Some(TaskStatusUpdate::with_error_code(
                error.to_string(),
                PlatformErrorCode::InternalError,
            )),
        ),
        RenderableAIError::Other {
            error_message,
            is_user_error,
            ..
        } => {
            if *is_user_error {
                (
                    AgentTaskState::Failed,
                    Some(TaskStatusUpdate::with_error_code(
                        error_message,
                        PlatformErrorCode::InvalidRequest,
                    )),
                )
            } else {
                (
                    AgentTaskState::Error,
                    Some(TaskStatusUpdate::with_error_code(
                        error_message,
                        PlatformErrorCode::InternalError,
                    )),
                )
            }
        }
    }
}

/// Maps a `CLIAgentSessionStatus` to an `AgentTaskState` and optional status message.
fn map_cli_session_status(
    status: &CLIAgentSessionStatus,
) -> (AgentTaskState, Option<TaskStatusUpdate>) {
    match status {
        CLIAgentSessionStatus::InProgress => (AgentTaskState::InProgress, None),
        CLIAgentSessionStatus::Success => (AgentTaskState::Succeeded, None),
        CLIAgentSessionStatus::Blocked { message } => (
            AgentTaskState::Blocked,
            message.as_ref().map(TaskStatusUpdate::message),
        ),
    }
}

#[cfg(test)]
#[path = "local_agent_task_sync_model_tests.rs"]
mod tests;
