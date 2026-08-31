//! Adapter between the local Codex app-server protocol and Warp's embedded
//! multi-agent response stream.
//!
//! This is intentionally a provider at the Agent API boundary: selecting the
//! synthetic `Codex (ChatGPT)` model in the Cmd+Return Agent routes the request
//! here instead of launching a terminal CLI session or calling Warp's hosted
//! inference endpoint.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail};
use codex_app_server::{
    ApprovalPolicy, Client, ClientOptions, Notification, SandboxMode, ServerRequest,
    ServerRequestResponse, ThreadOptions, TurnEvent,
};
use futures_util::StreamExt as _;
use serde_json::json;
use warp_multi_agent_api as api;
use warp_multi_agent_api::client_action as api_client_action;
use warp_multi_agent_api::response_event as api_response_event;
use warp_multi_agent_api::response_event::stream_finished as stream_finished_event;

use super::{ConvertToAPITypeError, RequestParams, ResponseStream, ServerConversationToken};
use crate::ai::agent::AIAgentInput;
use crate::server::server_api::AIApiError;

const CONVERSATION_TOKEN_PREFIX: &str = "codex-app-server:";
pub(super) fn is_codex_conversation_token(token: &ServerConversationToken) -> bool {
    token.as_str().starts_with(CONVERSATION_TOKEN_PREFIX)
}

fn thread_id_from_conversation_token(token: &ServerConversationToken) -> Option<&str> {
    token
        .as_str()
        .strip_prefix(CONVERSATION_TOKEN_PREFIX)
        .filter(|thread_id| !thread_id.is_empty())
}

fn conversation_token_for_thread(thread_id: &str) -> ServerConversationToken {
    ServerConversationToken::new(format!("{CONVERSATION_TOKEN_PREFIX}{thread_id}"))
}

pub(super) async fn generate_codex_app_server_output(
    params: RequestParams,
    cancellation_rx: futures::channel::oneshot::Receiver<()>,
) -> Result<ResponseStream, ConvertToAPITypeError> {
    let stream = async_stream::stream! {
        if params.session_context.is_remote() {
            yield Err(api_error(anyhow!(
                "Codex (ChatGPT) currently supports local Warp sessions only; switch to a local tab or select another Agent model"
            )));
            return;
        }

        // Passive prompt-suggestion requests are implementation details of
        // Warp's hosted harness and have no direct Codex app-server analogue.
        // Complete them as a no-op rather than creating a persisted Codex
        // thread that the user never sees.
        if !params.input.is_empty()
            && params.input.iter().all(|input| {
                matches!(input, AIAgentInput::TriggerPassiveSuggestion { .. })
            })
        {
            let request_id = uuid::Uuid::new_v4().to_string();
            let conversation_id = params
                .conversation_token
                .as_ref()
                .filter(|token| is_codex_conversation_token(token))
                .map(|token| token.as_str().to_owned())
                .unwrap_or_else(|| {
                    format!("{CONVERSATION_TOKEN_PREFIX}passive-{}", uuid::Uuid::new_v4())
                });
            yield Ok(stream_init_event(conversation_id, request_id));
            yield Ok(stream_finished_event());
            return;
        }

        let prompt = match prompt_for_request(&params) {
            Ok(prompt) => prompt,
            Err(error) => {
                yield Err(api_error(error));
                return;
            }
        };
        let cwd = params
            .session_context
            .current_working_directory()
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut client = match Client::spawn(ClientOptions::default()).await {
            Ok(client) => client,
            Err(error) => {
                yield Err(api_error(error.context(
                    "Could not start the local Codex app-server. Install the Codex CLI or set WARP_CODEX_PATH"
                )));
                return;
            }
        };
        let account = match client.account(false).await {
            Ok(status) => status,
            Err(error) => {
                yield Err(api_error(error.context("Could not read the Codex account")));
                return;
            }
        };
        if account.account.is_none() {
                yield Err(api_error(anyhow!(
                    "Codex is not signed in. Open Warp Settings → AI → Warp Agent → Custom Inference and connect ChatGPT"
                )));
                return;
        }

        let mut options = ThreadOptions::new(cwd);
        options.thread_source = "warp-embedded-agent".to_owned();
        configure_permissions(&params, &mut options);

        let existing_thread_id = params
            .conversation_token
            .as_ref()
            .and_then(thread_id_from_conversation_token)
            .map(str::to_owned);
        let thread_id = match existing_thread_id {
            Some(thread_id) => match client.resume_thread(&thread_id, &options).await {
                Ok(thread_id) => thread_id,
                Err(error) => {
                    yield Err(api_error(error));
                    return;
                }
            },
            None => match client.start_thread(&options).await {
                Ok(thread_id) => thread_id,
                Err(error) => {
                    yield Err(api_error(error));
                    return;
                }
            },
        };

        let mut pre_turn_notifications = Vec::new();
        let mut server_request_handler = safe_embedded_server_request;
        let turn_id = match client
            .start_turn(
                &thread_id,
                &prompt,
                &mut |notification| pre_turn_notifications.push(notification.clone()),
                &mut server_request_handler,
            )
            .await
        {
            Ok(turn_id) => turn_id,
            Err(error) => {
                yield Err(api_error(error));
                return;
            }
        };

        // A brand-new Warp conversation starts with an optimistic local root
        // task. Give it a stable provider-backed id so the normal CreateTask
        // action upgrades that optimistic task exactly as the hosted API does.
        // Existing/forked conversations keep their current root task id so
        // switching providers does not duplicate the visible transcript.
        let needs_task_creation = params.conversation_token.is_none()
            && params.forked_from_conversation_token.is_none();
        let root_task_id = if needs_task_creation {
            uuid::Uuid::new_v4().to_string()
        } else {
            root_task_id(&params.tasks).unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        };
        let message_id = uuid::Uuid::new_v4().to_string();
        let conversation_token = conversation_token_for_thread(&thread_id);

        yield Ok(stream_init_event(
            conversation_token.as_str().to_owned(),
            turn_id.clone(),
        ));
        if needs_task_creation || params.tasks.is_empty() {
            yield Ok(client_action_event(api_client_action::Action::CreateTask(
                api_client_action::CreateTask {
                    task: Some(api::Task {
                        id: root_task_id.clone(),
                        description: String::new(),
                        dependencies: None,
                        messages: vec![],
                        summary: String::new(),
                        server_data: String::new(),
                    }),
                },
            )));
        }
        yield Ok(client_action_event(api_client_action::Action::AddMessagesToTask(
            api_client_action::AddMessagesToTask {
                task_id: root_task_id.clone(),
                messages: vec![agent_output_message(
                    &root_task_id,
                    &message_id,
                    &turn_id,
                    String::new(),
                )],
            },
        )));

        let mut accumulated_text = String::new();
        let mut terminal_error: Option<String> = None;

        for notification in pre_turn_notifications {
            if let Some(event) = translate_notification(
                &notification,
                &root_task_id,
                &message_id,
                &turn_id,
                &mut accumulated_text,
                &mut terminal_error,
            ) {
                yield Ok(event);
            }
        }

        loop {
            match client
                .next_turn_event(&thread_id, &turn_id, &mut server_request_handler)
                .await
            {
                Ok(TurnEvent::Notification(notification)) => {
                    if let Some(event) = translate_notification(
                        &notification,
                        &root_task_id,
                        &message_id,
                        &turn_id,
                        &mut accumulated_text,
                        &mut terminal_error,
                    ) {
                        yield Ok(event);
                    }
                }
                Ok(TurnEvent::Completed(result)) => {
                    if let Some(error) = result.error.or(terminal_error) {
                        yield Err(api_error(anyhow!("Codex turn failed: {error}")));
                        return;
                    }
                    if result.status != "completed" {
                        yield Err(api_error(anyhow!(
                            "Codex turn ended with status {}",
                            result.status
                        )));
                        return;
                    }
                    yield Ok(stream_finished_event());
                    return;
                }
                Err(error) => {
                    yield Err(api_error(error));
                    return;
                }
            }
        }
    };

    Ok(Box::pin(stream.take_until(cancellation_rx)))
}

fn configure_permissions(params: &RequestParams, options: &mut ThreadOptions) {
    // Warp's embedded tool-approval UI and Codex's bidirectional approval
    // callbacks are distinct protocols. Run Codex non-interactively inside a
    // workspace sandbox for normal embedded use. Only Warp's explicit
    // unsupervised + unsandboxed execution mode maps to full access.
    options.approval_policy = ApprovalPolicy::Never;
    options.sandbox = if matches!(params.autonomy_level, api::AutonomyLevel::Unsupervised)
        && matches!(params.isolation_level, api::IsolationLevel::None)
    {
        SandboxMode::DangerFullAccess
    } else {
        SandboxMode::WorkspaceWrite
    };
}

fn prompt_for_request(params: &RequestParams) -> anyhow::Result<String> {
    let mut parts = Vec::new();

    // When a user switches an existing Warp-hosted conversation to Codex, the
    // Codex thread has no prior rollout. Carry the visible transcript into the
    // first Codex prompt. Subsequent Codex turns resume their own persisted
    // thread and do not duplicate the transcript.
    let switching_from_another_provider = params
        .conversation_token
        .as_ref()
        .is_some_and(|token| !is_codex_conversation_token(token));
    if switching_from_another_provider {
        let transcript = visible_task_transcript(&params.tasks);
        if !transcript.is_empty() {
            parts.push(format!(
                "Existing Warp Agent conversation context:\n\n{transcript}"
            ));
        }
    }

    for input in &params.input {
        match input {
            AIAgentInput::UserQuery { query, .. }
            | AIAgentInput::AutoCodeDiffQuery { query, .. }
            | AIAgentInput::CreateNewProject { query, .. } => parts.push(query.clone()),
            AIAgentInput::CloneRepository { .. }
            | AIAgentInput::InitProjectRules { .. }
            | AIAgentInput::CreateEnvironment { .. }
            | AIAgentInput::PassiveSuggestionResult { .. } => {
                if let Some(query) = input.display_query() {
                    parts.push(query);
                }
            }
            AIAgentInput::CodeReview {
                review_comments, ..
            } => parts.push(crate::terminal::cli_agent::build_review_prompt(
                review_comments,
            )),
            AIAgentInput::SummarizeConversation { prompt, .. } => parts.push(
                prompt
                    .clone()
                    .unwrap_or_else(|| "Summarize this conversation.".to_owned()),
            ),
            AIAgentInput::InvokeSkill {
                skill, user_query, ..
            } => {
                let query = user_query
                    .as_ref()
                    .map(|query| query.query.as_str())
                    .filter(|query| !query.is_empty())
                    .unwrap_or("Follow the skill instructions.");
                parts.push(format!(
                    "Use the following skill named `{}`:\n\n{}\n\nUser request:\n{}",
                    skill.name, skill.content, query
                ));
            }
            AIAgentInput::ResumeConversation { .. } => {
                parts.push("Continue the current task.".to_owned());
            }
            AIAgentInput::ActionResult { .. }
            | AIAgentInput::MessagesReceivedFromAgents { .. }
            | AIAgentInput::EventsFromAgents { .. }
            | AIAgentInput::OrchestrationConfigUpdate { .. } => {
                parts.push(input.to_string());
            }
            AIAgentInput::StartFromAmbientRunPrompt { .. } => {
                bail!("Codex (ChatGPT) cannot resolve a cloud ambient-run prompt locally")
            }
            AIAgentInput::TriggerPassiveSuggestion { .. } => {}
        }
    }

    let prompt = parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    if prompt.is_empty() {
        bail!("The Codex provider received no text input to send")
    }
    Ok(prompt)
}

fn visible_task_transcript(tasks: &[api::Task]) -> String {
    let mut lines = Vec::new();
    for task in tasks {
        for message in &task.messages {
            match message.message.as_ref() {
                Some(api::message::Message::UserQuery(query)) if !query.query.is_empty() => {
                    lines.push(format!("USER: {}", query.query));
                }
                Some(api::message::Message::AgentOutput(output)) if !output.text.is_empty() => {
                    lines.push(format!("ASSISTANT: {}", output.text));
                }
                _ => {}
            }
        }
    }
    lines.join("\n\n")
}

fn root_task_id(tasks: &[api::Task]) -> Option<String> {
    tasks
        .iter()
        .find(|task| {
            task.dependencies
                .as_ref()
                .is_none_or(|dependencies| dependencies.parent_task_id.is_empty())
        })
        .or_else(|| tasks.first())
        .map(|task| task.id.clone())
        .filter(|id| !id.is_empty())
}

fn translate_notification(
    notification: &Notification,
    task_id: &str,
    message_id: &str,
    request_id: &str,
    accumulated_text: &mut String,
    terminal_error: &mut Option<String>,
) -> Option<api::ResponseEvent> {
    if let Some(delta) = notification.agent_message_delta() {
        accumulated_text.push_str(delta);
        return Some(append_agent_output_event(
            task_id,
            message_id,
            request_id,
            delta.to_owned(),
        ));
    }

    if let Some(completed) = notification.completed_agent_message() {
        if accumulated_text.is_empty() {
            accumulated_text.push_str(completed);
            return Some(append_agent_output_event(
                task_id,
                message_id,
                request_id,
                completed.to_owned(),
            ));
        }
        if let Some(suffix) = completed.strip_prefix(accumulated_text.as_str())
            && !suffix.is_empty()
        {
            accumulated_text.push_str(suffix);
            return Some(append_agent_output_event(
                task_id,
                message_id,
                request_id,
                suffix.to_owned(),
            ));
        }
        if completed != accumulated_text {
            *accumulated_text = completed.to_owned();
            return Some(update_agent_output_event(
                task_id,
                message_id,
                request_id,
                completed.to_owned(),
            ));
        }
    }

    if let Some(warning) = notification.warning_message() {
        log::warn!("Codex app-server warning: {warning}");
    }
    if let Some(error) = notification.error_message() {
        *terminal_error = Some(error.to_owned());
    }
    None
}

fn safe_embedded_server_request(request: &ServerRequest) -> ServerRequestResponse {
    // In normal operation ApprovalPolicy::Never means command/file approvals
    // are not requested. If a newer server still asks, fail closed rather than
    // silently broadening access beyond the configured sandbox.
    match request.method.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            ServerRequestResponse::result(json!({ "decision": "decline" }))
        }
        "execCommandApproval" | "applyPatchApproval" => ServerRequestResponse::result(json!({
            "decision": { "denied": { "rejection": "Warp embedded Codex provider denied an unexpected approval request" } },
        })),
        "item/permissions/requestApproval" => ServerRequestResponse::result(json!({
            "permissions": { "fileSystem": null, "network": null },
            "scope": "turn",
        })),
        "item/tool/requestUserInput" => ServerRequestResponse::result(json!({ "answers": {} })),
        "mcpServer/elicitation/request" => {
            ServerRequestResponse::result(json!({ "action": "decline" }))
        }
        "item/tool/call" => ServerRequestResponse::result(json!({
            "success": false,
            "contentItems": [{
                "type": "inputText",
                "text": "Warp has no matching dynamic tool implementation",
            }],
        })),
        method => ServerRequestResponse::method_not_found(method),
    }
}

fn stream_init_event(conversation_id: String, request_id: String) -> api::ResponseEvent {
    api::ResponseEvent {
        r#type: Some(api_response_event::Type::Init(
            api_response_event::StreamInit {
                conversation_id,
                request_id,
                run_id: String::new(),
            },
        )),
    }
}

fn client_action_event(action: api_client_action::Action) -> api::ResponseEvent {
    api::ResponseEvent {
        r#type: Some(api_response_event::Type::ClientActions(
            api_response_event::ClientActions {
                actions: vec![api::ClientAction {
                    action: Some(action),
                }],
            },
        )),
    }
}

fn agent_output_message(
    task_id: &str,
    message_id: &str,
    request_id: &str,
    text: String,
) -> api::Message {
    api::Message {
        id: message_id.to_owned(),
        task_id: task_id.to_owned(),
        request_id: request_id.to_owned(),
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput { text },
        )),
        ..Default::default()
    }
}

fn append_agent_output_event(
    task_id: &str,
    message_id: &str,
    request_id: &str,
    delta: String,
) -> api::ResponseEvent {
    client_action_event(api_client_action::Action::AppendToMessageContent(
        api_client_action::AppendToMessageContent {
            task_id: task_id.to_owned(),
            message: Some(agent_output_message(task_id, message_id, request_id, delta)),
            mask: Some(prost_types::FieldMask {
                paths: vec!["agent_output.text".to_owned()],
            }),
        },
    ))
}

fn update_agent_output_event(
    task_id: &str,
    message_id: &str,
    request_id: &str,
    text: String,
) -> api::ResponseEvent {
    client_action_event(api_client_action::Action::UpdateTaskMessage(
        api_client_action::UpdateTaskMessage {
            task_id: task_id.to_owned(),
            message: Some(agent_output_message(task_id, message_id, request_id, text)),
            mask: Some(prost_types::FieldMask {
                paths: vec!["agent_output.text".to_owned()],
            }),
        },
    ))
}

fn stream_finished_event() -> api::ResponseEvent {
    api::ResponseEvent {
        r#type: Some(api_response_event::Type::Finished(
            api_response_event::StreamFinished {
                reason: Some(stream_finished_event::Reason::Done(
                    stream_finished_event::Done {},
                )),
                conversation_usage_metadata: None,
                token_usage: vec![],
                should_refresh_model_config: false,
                #[allow(deprecated)]
                request_cost: None,
                request_charges: None,
            },
        )),
    }
}

fn api_error(error: anyhow::Error) -> Arc<AIApiError> {
    Arc::new(AIApiError::Other(error))
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
