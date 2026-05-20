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
use std::collections::HashMap;
use std::sync::Arc;
use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

/// Listens for conversation status changes and CLI agent session status
/// changes, then reports the corresponding task state to the server via
/// `update_agent_task`. This centralises task status reporting so that
/// individual call-sites (driver, controller, etc.) no longer need to
/// call `update_agent_task` for state transitions.
///
/// For Oz harness conversations, status is derived from
/// `BlocklistAIHistoryEvent::UpdatedConversationStatus` and the `task_id`
/// is read from the `AIConversation`.
///
/// For third-party harnesses (e.g. Claude Code), status is derived from
/// `CLIAgentSessionsModelEvent::StatusChanged`. Because these sessions
/// do not create conversations in the history model, the driver must
/// register a `terminal_view_id → task_id` mapping via
/// `register_cli_session`.
///
/// Registered unconditionally — handles task status reporting for all
/// conversations, not just v2 orchestrated ones.
pub struct TaskStatusSyncModel {
    ai_client: Arc<dyn AIClient>,
    /// Maps terminal view IDs to task IDs for third-party harness sessions
    /// that don't have conversations in `BlocklistAIHistoryModel`.
    cli_session_task_ids: HashMap<EntityId, AmbientAgentTaskId>,
}

pub enum TaskStatusSyncModelEvent {}

impl TaskStatusSyncModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

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
        }
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
        // Report IN_PROGRESS immediately
        // by CLIAgentSessionsModel::register_listener is never emitted as a
        // StatusChanged event, so we must report it at registration time.
        self.fire_update(task_id, AgentTaskState::InProgress, None, ctx);
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
                self.cli_session_task_ids.remove(terminal_view_id);
            }
            _ => {}
        }
    }

    fn on_conversation_status_updated(
        &self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let (task_id, task_state, status_message) = {
            let Some(conversation) =
                BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
            else {
                return;
            };
            // Viewers of shared sessions must not report status — they don't
            // own the task. Currently also protected by the absence of task_id,
            // but this guard makes the intent explicit.
            if conversation.is_viewing_shared_session() {
                return;
            }
            // Skip remote child placeholder conversations — the remote worker's
            // own client handles status reporting. Reporting here would
            // prematurely move remote tasks from QUEUED to IN_PROGRESS before
            // the worker can claim them. Local children are NOT skipped because
            // they execute in this client and have no separate reporter.
            if conversation.is_remote_child() {
                return;
            }
            let Some(task_id) = conversation.task_id() else {
                return;
            };
            let (state, msg) = map_conversation_status(conversation);
            (task_id, state, msg)
        };

        self.fire_update(task_id, task_state, status_message, ctx);
    }

    fn on_cli_session_status_changed(
        &self,
        terminal_view_id: EntityId,
        status: &CLIAgentSessionStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(&task_id) = self.cli_session_task_ids.get(&terminal_view_id) else {
            return;
        };

        let (task_state, status_message) = map_cli_session_status(status);
        self.fire_update(task_id, task_state, status_message, ctx);
    }

    /// Sends an `update_agent_task` request to the server (fire-and-forget).
    fn fire_update(
        &self,
        task_id: AmbientAgentTaskId,
        task_state: AgentTaskState,
        status_message: Option<TaskStatusUpdate>,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = self.ai_client.clone();
        ctx.spawn(
            async move {
                if let Err(err) = ai_client
                    .update_agent_task(task_id, Some(task_state), None, None, status_message)
                    .await
                {
                    log::warn!(
                        "TaskStatusSyncModel: failed to update task {task_id} to {task_state:?}: {err:#}"
                    );
                }
            },
            |_, _, _| {},
        );
    }
}

impl Entity for TaskStatusSyncModel {
    type Event = TaskStatusSyncModelEvent;
}

impl SingletonEntity for TaskStatusSyncModel {}

/// Maps conversation state to an `AgentTaskState` and optional status message.
/// For errors, extracts the specific error from the last exchange when available.
fn map_conversation_status(
    conversation: &AIConversation,
) -> (AgentTaskState, Option<TaskStatusUpdate>) {
    match conversation.status() {
        ConversationStatus::InProgress => (AgentTaskState::InProgress, None),
        ConversationStatus::Success => (AgentTaskState::Succeeded, None),
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
            match renderable_error {
                Some(error) => classify_renderable_error(error),
                None => (
                    AgentTaskState::Error,
                    Some(TaskStatusUpdate::message("Agent encountered an error")),
                ),
            }
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
        RenderableAIError::Other { error_message, .. } => (
            AgentTaskState::Error,
            Some(TaskStatusUpdate::with_error_code(
                error_message,
                PlatformErrorCode::InternalError,
            )),
        ),
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
#[path = "task_status_sync_model_tests.rs"]
mod tests;
