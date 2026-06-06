#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

#[cfg(not(target_family = "wasm"))]
use anyhow::anyhow;
use futures::future::BoxFuture;
#[cfg(not(target_family = "wasm"))]
use futures::future::Either;
use futures::FutureExt;
use warp_core::send_telemetry_from_ctx;
#[cfg(not(target_family = "wasm"))]
use warpui::r#async::Timer;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionResultType, AIAgentActionType, SendMessageToAgentResult,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::ai::blocklist::telemetry::{
    BlocklistOrchestrationTelemetryEvent, TeamAgentCommunicationFailedEvent,
    TeamAgentCommunicationFailureReason, TeamAgentCommunicationKind,
    TeamAgentCommunicationTransport, TeamAgentOrchestrationVersion,
};
use crate::server::server_api::ai::{SendAgentMessageRequest, SendAgentMessageResponse};
use crate::server::server_api::ServerApiProvider;

#[cfg(not(target_family = "wasm"))]
const SEND_AGENT_MESSAGE_TIMEOUT: Duration = Duration::from_secs(15);

pub struct SendMessageToAgentExecutor {
    ambient_agent_task_id: Option<AmbientAgentTaskId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendMessageTaskResolution {
    ConversationTask,
    AmbientTaskFallback,
    NoTaskContext,
}

fn sender_run_id_and_task_id_for_send(
    conversation_id: AIConversationId,
    ambient_agent_task_id: Option<AmbientAgentTaskId>,
    ctx: &AppContext,
) -> (
    String,
    Option<AmbientAgentTaskId>,
    SendMessageTaskResolution,
) {
    let conversation = BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id);
    let conversation_task_id = conversation.and_then(|conversation| conversation.task_id());
    let (task_id, task_resolution) = match (conversation_task_id, ambient_agent_task_id) {
        (Some(task_id), _) => (Some(task_id), SendMessageTaskResolution::ConversationTask),
        (None, Some(task_id)) => (
            Some(task_id),
            SendMessageTaskResolution::AmbientTaskFallback,
        ),
        (None, None) => (None, SendMessageTaskResolution::NoTaskContext),
    };
    let sender_run_id = conversation
        .and_then(|conversation| conversation.run_id())
        .or_else(|| task_id.map(|task_id| task_id.to_string()))
        .unwrap_or_default();
    (sender_run_id, task_id, task_resolution)
}

#[cfg(not(target_family = "wasm"))]
async fn send_agent_message_with_timeout(
    server_api: std::sync::Arc<crate::server::server_api::ServerApi>,
    ai_client: std::sync::Arc<dyn crate::server::server_api::ai::AIClient>,
    task_id: Option<AmbientAgentTaskId>,
    request: SendAgentMessageRequest,
) -> anyhow::Result<SendAgentMessageResponse, anyhow::Error> {
    let task_id_for_timeout = task_id.map(|task_id| task_id.to_string());
    let send_message = async move {
        match task_id {
            Some(task_id) => {
                server_api
                    .send_agent_message_for_task(&task_id, request)
                    .await
            }
            None => ai_client.send_agent_message(request).await,
        }
    };
    let timeout = Timer::after(SEND_AGENT_MESSAGE_TIMEOUT);
    futures::pin_mut!(send_message);
    futures::pin_mut!(timeout);

    match futures::future::select(send_message, timeout).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(anyhow!(
            "Timed out sending orchestration message{}",
            task_id_for_timeout
                .map(|task_id| format!(" for task {task_id}"))
                .unwrap_or_default()
        )),
    }
}

#[cfg(target_family = "wasm")]
async fn send_agent_message_with_timeout(
    server_api: std::sync::Arc<crate::server::server_api::ServerApi>,
    ai_client: std::sync::Arc<dyn crate::server::server_api::ai::AIClient>,
    task_id: Option<AmbientAgentTaskId>,
    request: SendAgentMessageRequest,
) -> anyhow::Result<SendAgentMessageResponse, anyhow::Error> {
    match task_id {
        Some(task_id) => {
            server_api
                .send_agent_message_for_task(&task_id, request)
                .await
        }
        None => ai_client.send_agent_message(request).await,
    }
}

impl SendMessageToAgentExecutor {
    pub fn new() -> Self {
        Self {
            ambient_agent_task_id: None,
        }
    }

    pub fn set_ambient_agent_task_id(&mut self, id: Option<AmbientAgentTaskId>) {
        self.ambient_agent_task_id = id;
    }

    pub(super) fn should_autoexecute(
        &self,
        _input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        true
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> AnyActionExecution {
        let AIAgentAction {
            action:
                AIAgentActionType::SendMessageToAgent {
                    addresses,
                    subject,
                    message,
                },
            ..
        } = input.action
        else {
            return ActionExecution::<()>::InvalidAction.into();
        };

        let conversation_id = input.conversation_id;
        let addresses = addresses.clone();
        let subject = subject.clone();
        let message_body = message.clone();

        let (sender_run_id, task_id, task_resolution) =
            sender_run_id_and_task_id_for_send(conversation_id, self.ambient_agent_task_id, ctx);
        let log_addresses = addresses.clone();
        let log_subject = subject.clone();
        let log_sender_run_id = sender_run_id.clone();
        let log_task_id = task_id.map(|task_id| task_id.to_string());
        let log_body_len = message_body.chars().count();
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        log::info!(
            "Sending orchestration message: conversation_id={conversation_id:?} resolution={task_resolution:?} sender_run_id={log_sender_run_id:?} task_id={log_task_id:?} target_agent_ids={log_addresses:?} subject={log_subject:?} body_len={log_body_len}"
        );
        let request = SendAgentMessageRequest {
            to: addresses,
            subject,
            body: message_body,
            sender_run_id,
        };
        ActionExecution::new_async(
            async move {
                send_agent_message_with_timeout(server_api, ai_client, task_id, request).await
            },
            move |result, ctx| match result {
                Ok(response) => {
                    let message_id = response.message_ids.into_iter().next().unwrap_or_default();
                    log::info!(
                        "Sent orchestration message: conversation_id={conversation_id:?} resolution={task_resolution:?} sender_run_id={log_sender_run_id:?} task_id={log_task_id:?} target_agent_ids={log_addresses:?} subject={log_subject:?} body_len={log_body_len} message_id={message_id:?}"
                    );
                    AIAgentActionResultType::SendMessageToAgent(
                        SendMessageToAgentResult::Success { message_id },
                    )
                }
                Err(err) => {
                    let error_message = err.to_string();
                    send_telemetry_from_ctx!(
                        BlocklistOrchestrationTelemetryEvent::TeamAgentCommunicationFailed(
                            TeamAgentCommunicationFailedEvent {
                                communication_kind: TeamAgentCommunicationKind::Message,
                                transport: TeamAgentCommunicationTransport::ServerApi,
                                orchestration_version: TeamAgentOrchestrationVersion::V2,
                                failure_reason: TeamAgentCommunicationFailureReason::RequestFailed,
                                source_conversation_id: conversation_id,
                                source_run_id: (!log_sender_run_id.is_empty())
                                    .then(|| log_sender_run_id.clone()),
                                target_count: Some(log_addresses.len()),
                                lifecycle_event_type: None,
                                error_message: Some(error_message.clone()),
                            }
                        ),
                        ctx
                    );
                    log::warn!(
                        "Failed to send child-agent message via server API: conversation_id={conversation_id:?} resolution={task_resolution:?} sender_run_id={log_sender_run_id:?} task_id={log_task_id:?} target_agent_ids={log_addresses:?} subject={log_subject:?} body_len={log_body_len} error={err:#}"
                    );
                    AIAgentActionResultType::SendMessageToAgent(
                        SendMessageToAgentResult::Error(error_message),
                    )
                }
            },
        )
        .into()
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Default for SendMessageToAgentExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for SendMessageToAgentExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "send_message_tests.rs"]
mod tests;
