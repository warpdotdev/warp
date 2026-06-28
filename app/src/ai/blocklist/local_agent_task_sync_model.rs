mod update_queue;

use std::collections::HashMap;
use std::sync::Arc;

use session_sharing_protocol::common::SessionId;
use update_queue::LocalTaskUpdateQueue;
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
    /// Serializes and coalesces model-owned updates independently per task.
    update_queue: LocalTaskUpdateQueue,
}

pub enum LocalAgentTaskSyncModelEvent {}
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
        ctx.subscribe_to_model(&history_model, |me, _, event, ctx| {
            me.handle_history_event(event, ctx);
        });

        let cli_sessions_model = CLIAgentSessionsModel::handle(ctx);
        ctx.subscribe_to_model(&cli_sessions_model, |me, _, event, ctx| {
            me.handle_cli_session_event(event, ctx);
        });

        Self {
            ai_client,
            cli_session_task_ids: HashMap::new(),
            update_queue: LocalTaskUpdateQueue::default(),
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
        self.enqueue_update(
            task_id,
            LocalTaskUpdate {
                task_state: Some(AgentTaskState::InProgress),
                ..LocalTaskUpdate::default()
            },
            ctx,
        );
    }

    fn remove_queued_update_state_for_run_id(&mut self, run_id: Option<&str>) {
        let Some(task_id) = run_id.and_then(|run_id| run_id.parse::<AmbientAgentTaskId>().ok())
        else {
            return;
        };
        self.update_queue.remove_task(&task_id);
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
                self.remove_queued_update_state_for_run_id(run_id.as_deref());
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
                    self.update_queue.remove_task(&task_id);
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

        self.enqueue_update(task_id, update, ctx);
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

        self.enqueue_update(task_id, update, ctx);
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
        self.enqueue_update(
            task_id,
            LocalTaskUpdate {
                task_state: Some(task_state),
                status_message,
                ..LocalTaskUpdate::default()
            },
            ctx,
        );
    }

    /// Enqueues a model-owned update without blocking the event producer.
    fn enqueue_update(
        &mut self,
        task_id: AmbientAgentTaskId,
        update: LocalTaskUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(update) = self.update_queue.enqueue(task_id, update) {
            self.send_update(task_id, update, ctx);
        }
    }

    /// Sends the active update for a task and drains its next queued update
    /// after completion.
    fn send_update(
        &mut self,
        task_id: AmbientAgentTaskId,
        update: LocalTaskUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
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
            move |me, result, ctx| {
                if let Some(update) = me.update_queue.record_result(task_id, result.is_ok()) {
                    me.send_update(task_id, update, ctx);
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
