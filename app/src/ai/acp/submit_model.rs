use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use acpx::RuntimeContext;
use agent_client_protocol as acp;
use anyhow::{anyhow, Context, Result};
use async_process::Command;
use async_channel;
use futures::StreamExt;
use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, oneshot};
use warp_cli::agent::Harness;
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use super::{connection::Connection, path_search, registry, session_store::LocalAcpSessionStore, tool_calls};
use crate::ai::agent::local_acp_tool_call::LocalAcpToolCallMessage;
use crate::ai::agent::conversation::{AIConversationId, LocalAcpStreamChunk};
use crate::ai::agent::RenderableAIError;
use crate::ai::blocklist::{BlocklistAIHistoryModel, ResponseStreamId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocalAcpSubmitModelEvent {
    Submitted,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(crate) struct LocalAcpSubmitRequest {
    pub(crate) prompt: String,
    pub(crate) harness: Harness,
    pub(crate) model_id: Option<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) conversation_id: AIConversationId,
    pub(crate) stream_id: ResponseStreamId,
    pub(crate) terminal_view_id: EntityId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalAcpSubmitResult {
    pub(crate) session_id: String,
}

enum LocalAcpStreamEvent {
    Text(String),
    Thought(String),
    ToolCall(LocalAcpToolCallMessage),
}

pub(crate) struct LocalAcpSubmitModel {
    active_submission: Option<LocalAcpSubmitRequest>,
    workers: HashMap<LocalAcpWorkerKey, mpsc::UnboundedSender<LocalAcpWorkerRequest>>,
}

impl LocalAcpSubmitModel {
    pub(crate) fn new(_: &mut ModelContext<Self>) -> Self {
        Self {
            active_submission: None,
            workers: HashMap::new(),
        }
    }

    pub(crate) fn submit(&mut self, request: LocalAcpSubmitRequest, ctx: &mut ModelContext<Self>) {
        self.active_submission = Some(request.clone());
        let completion_request = request.clone();
        let stream_stream_id = completion_request.stream_id.clone();
        let stream_conversation_id = completion_request.conversation_id;
        let stream_terminal_view_id = completion_request.terminal_view_id;
        let (stream_tx, stream_rx) = async_channel::unbounded();
        let result_rx = self.enqueue_request(request, stream_tx);

        ctx.spawn_stream_local(
            stream_rx,
            move |_, event, ctx| {
                let chunk = match event {
                    LocalAcpStreamEvent::Text(chunk) => LocalAcpStreamChunk::Text(chunk),
                    LocalAcpStreamEvent::Thought(chunk) => LocalAcpStreamChunk::Thought(chunk),
                    LocalAcpStreamEvent::ToolCall(tool_call) => {
                        LocalAcpStreamChunk::ToolCall(tool_call)
                    }
                };
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.append_local_acp_stream_chunk(
                        &stream_stream_id,
                        stream_conversation_id,
                        stream_terminal_view_id,
                        chunk,
                        ctx,
                    );
                });
            },
            |_, _| {},
        );

        ctx.spawn(
            async move {
                result_rx
                    .await
                    .context("local ACP worker stopped before returning a response")?
            },
            move |me, result, ctx| {
                me.active_submission = None;
                match result {
                    Ok(result) => {
                        log::info!(
                            "Local ACP session {} completed",
                            result.session_id,
                        );
                        LocalAcpSessionStore::handle(ctx).update(ctx, |store, _ctx| {
                            store.set_session_id(
                                completion_request.harness,
                                result.session_id.clone(),
                            );
                        });
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                            history_model.mark_response_stream_completed_successfully(
                                &completion_request.stream_id,
                                completion_request.conversation_id,
                                completion_request.terminal_view_id,
                                ctx,
                            );
                        });
                        ctx.emit(LocalAcpSubmitModelEvent::Submitted);
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        log::error!("Local ACP submission failed: {message}");
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                            history_model.mark_response_stream_completed_with_error(
                                RenderableAIError::Other {
                                    error_message: message.clone(),
                                    will_attempt_resume: false,
                                    waiting_for_network: false,
                                },
                                &completion_request.stream_id,
                                completion_request.conversation_id,
                                completion_request.terminal_view_id,
                                ctx,
                            );
                        });
                        ctx.emit(LocalAcpSubmitModelEvent::Failed(message));
                    }
                }
            },
        );
    }

    fn enqueue_request(
        &mut self,
        request: LocalAcpSubmitRequest,
        stream_tx: async_channel::Sender<LocalAcpStreamEvent>,
    ) -> oneshot::Receiver<Result<LocalAcpSubmitResult>> {
        let key = LocalAcpWorkerKey::from_request(&request);
        let (result_tx, result_rx) = oneshot::channel();
        let worker_request = LocalAcpWorkerRequest {
            request,
            stream_tx,
            result_tx,
        };

        let sender = self
            .workers
            .entry(key.clone())
            .or_insert_with(|| spawn_local_acp_worker())
            .clone();

        if let Err(error) = sender.send(worker_request) {
            self.workers.remove(&key);
            let sender = self
                .workers
                .entry(key)
                .or_insert_with(|| spawn_local_acp_worker())
                .clone();
            let _ = sender.send(error.0);
        }

        result_rx
    }
}

impl Entity for LocalAcpSubmitModel {
    type Event = LocalAcpSubmitModelEvent;
}

impl SingletonEntity for LocalAcpSubmitModel {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LocalAcpWorkerKey {
    harness: Harness,
    cwd: PathBuf,
}

impl LocalAcpWorkerKey {
    fn from_request(request: &LocalAcpSubmitRequest) -> Self {
        Self {
            harness: request.harness,
            cwd: request.cwd.clone(),
        }
    }
}

struct LocalAcpWorkerRequest {
    request: LocalAcpSubmitRequest,
    stream_tx: async_channel::Sender<LocalAcpStreamEvent>,
    result_tx: oneshot::Sender<Result<LocalAcpSubmitResult>>,
}

struct LocalAcpWorkerSession {
    connection: Connection,
    session_id: acp::SessionId,
    config_options: Option<Vec<acp::SessionConfigOption>>,
    applied_model_id: Option<String>,
    applied_mode: Option<String>,
}

fn spawn_local_acp_worker() -> mpsc::UnboundedSender<LocalAcpWorkerRequest> {
    let (tx, rx) = mpsc::unbounded_channel();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let Ok(runtime) = runtime else {
            log::error!("failed to build local ACP runtime");
            return;
        };
        let local_set = tokio::task::LocalSet::new();
        runtime.block_on(local_set.run_until(run_local_acp_worker(rx)));
    });
    tx
}

async fn run_local_acp_worker(mut rx: mpsc::UnboundedReceiver<LocalAcpWorkerRequest>) {
    let mut session = None;

    while let Some(worker_request) = rx.recv().await {
        let result =
            submit_local_acp_query_on_worker(&mut session, worker_request.request, worker_request.stream_tx)
                .await;
        if result.is_err() {
            if let Some(session) = session.take() {
                if let Err(error) = session.connection.close().await {
                    log::debug!("Failed to close failed local ACP connection: {error:#}");
                }
            }
        }
        let _ = worker_request.result_tx.send(result);
    }

    if let Some(session) = session {
        if let Err(error) = session.connection.close().await {
            log::debug!("Failed to close local ACP connection: {error:#}");
        }
    }
}

async fn submit_local_acp_query_on_worker(
    session: &mut Option<LocalAcpWorkerSession>,
    request: LocalAcpSubmitRequest,
    stream_tx: async_channel::Sender<LocalAcpStreamEvent>,
) -> Result<LocalAcpSubmitResult> {
    if session.is_none() {
        *session = Some(start_local_acp_session(&request).await?);
    }

    let session = session
        .as_mut()
        .expect("local ACP session exists after initialization");
    apply_session_preferences(
        request.harness,
        request.model_id.as_deref(),
        session,
    )
    .await?;

    prompt_local_acp_session(session, request, stream_tx).await
}

async fn start_local_acp_session(request: &LocalAcpSubmitRequest) -> Result<LocalAcpWorkerSession> {
    let spec = registry::spec_for_harness(request.harness)
        .ok_or_else(|| anyhow!("{} does not support local ACP", request.harness))?;
    let program = path_search::resolve_command(spec.command).with_context(|| {
        format!(
            "{} ACP command '{}' was not found",
            request.harness, spec.command
        )
    })?;

    let mut command = Command::new(program);
    command.args(spec.args);
    command.current_dir(&request.cwd);
    command.env("PATH", path_search::augmented_path_env());
    for (key, value) in registry::process_env_for_harness(request.harness) {
        command.env(key, value);
    }

    let runtime = RuntimeContext::new(|task| {
        tokio::task::spawn_local(task);
    });
    let connection = Connection::spawn(&mut command, &runtime)?;
    let initialize_result = connection
        .initialize(
            acp::InitializeRequest::new(acp::ProtocolVersion::V1).client_info(
                acp::Implementation::new("warp", env!("CARGO_PKG_VERSION")).title("Warp"),
            ),
        )
        .await?;

    if registry::should_auto_authenticate(request.harness) {
        if let Some(auth_method) = initialize_result.auth_methods.first() {
            connection
                .authenticate(acp::AuthenticateRequest::new(auth_method.id().clone()))
                .await?;
        }
    }

    let session = connection
        .new_session(new_session_request(request))
        .await?;
    let session_id = session.session_id.clone();
    let config_options = session.config_options.clone();

    let mut worker_session = LocalAcpWorkerSession {
        connection,
        session_id,
        config_options,
        applied_model_id: None,
        applied_mode: None,
    };
    apply_session_preferences(
        request.harness,
        request.model_id.as_deref(),
        &mut worker_session,
    )
    .await?;

    Ok(worker_session)
}

async fn apply_session_preferences(
    harness: Harness,
    model_id: Option<&str>,
    session: &mut LocalAcpWorkerSession,
) -> Result<()> {
    if let Some(mode_id) = registry::default_session_mode(harness) {
        let should_apply_mode = harness == Harness::Cursor
            || session.applied_mode.as_deref() != Some(mode_id);
        if should_apply_mode {
            match session
                .connection
                .set_session_mode(acp::SetSessionModeRequest::new(
                    session.session_id.clone(),
                    mode_id,
                ))
                .await
            {
                Ok(_) => {
                    session.applied_mode = Some(mode_id.to_string());
                }
                Err(error) => {
                    log::debug!("ACP agent did not accept session mode via set_session_mode: {error:#}");
                    if let Err(error) = session
                        .connection
                        .set_session_config_option(acp::SetSessionConfigOptionRequest::new(
                            session.session_id.clone(),
                            "mode",
                            mode_id,
                        ))
                        .await
                    {
                        log::debug!(
                            "ACP agent did not accept session mode via config option: {error:#}"
                        );
                    } else {
                        session.applied_mode = Some(mode_id.to_string());
                    }
                }
            }
        }
    }

    let Some(model_id) = model_id.filter(|model_id| !model_id.is_empty()) else {
        return Ok(());
    };

    if session.applied_model_id.as_deref() == Some(model_id) {
        return Ok(());
    }

    if harness == Harness::Gemini {
        apply_gemini_session_model(&session.connection, &session.session_id, model_id).await?;
        session.applied_model_id = Some(model_id.to_string());
        return Ok(());
    }

    if let Some(model_config_id) = model_config_id(session.config_options.as_deref()) {
        if let Err(error) = session
            .connection
            .set_session_config_option(acp::SetSessionConfigOptionRequest::new(
                session.session_id.clone(),
                model_config_id,
                model_id.to_string(),
            ))
            .await
        {
            log::debug!("ACP agent did not accept model config selection: {error:#}");
        } else {
            session.applied_model_id = Some(model_id.to_string());
        }
    }

    Ok(())
}

async fn apply_gemini_session_model(
    connection: &Connection,
    session_id: &acp::SessionId,
    model_id: &str,
) -> Result<()> {
    let params = serde_json::value::to_raw_value(&json!({
        "sessionId": session_id,
        "modelId": model_id,
    }))
    .context("failed to serialize Gemini model selection params")?;
    connection
        .ext_method(acp::ExtRequest::new(
            "unstable_setSessionModel",
            Arc::from(params),
        ))
        .await
        .context("Gemini rejected model selection")?;
    Ok(())
}

fn new_session_request(request: &LocalAcpSubmitRequest) -> acp::NewSessionRequest {
    let mut new_session = acp::NewSessionRequest::new(request.cwd.clone());
    if request.harness == Harness::Cursor {
        if let Some(mode_id) = registry::default_session_mode(request.harness) {
            let mut meta = Map::new();
            meta.insert(
                "default_mode".to_string(),
                Value::String(mode_id.to_string()),
            );
            new_session = new_session.meta(meta);
        }
    }
    new_session
}

async fn prompt_local_acp_session(
    session: &LocalAcpWorkerSession,
    request: LocalAcpSubmitRequest,
    stream_tx: async_channel::Sender<LocalAcpStreamEvent>,
) -> Result<LocalAcpSubmitResult> {
    let mut notifications = session.connection.subscribe_session_updates();
    let mut tool_calls_by_id: HashMap<String, LocalAcpToolCallMessage> = HashMap::new();
    let mut prompt = Box::pin(session.connection.prompt(acp::PromptRequest::new(
        session.session_id.clone(),
        vec![acp::ContentBlock::Text(acp::TextContent::new(
            request.prompt,
        ))],
    )));

    loop {
        tokio::select! {
            result = &mut prompt => {
                result?;
                break;
            }
            notification = notifications.next() => {
                let Some(notification) = notification else {
                    continue;
                };
                match notification.update {
                    acp::SessionUpdate::ToolCall(tool_call) => {
                        let message = tool_calls::message_from_tool_call(tool_call);
                        tool_calls_by_id.insert(message.tool_call_id.clone(), message.clone());
                        let _ = stream_tx.send(LocalAcpStreamEvent::ToolCall(message)).await;
                    }
                    acp::SessionUpdate::ToolCallUpdate(update) => {
                        let tool_call_id = update.tool_call_id.to_string();
                        if let Some(existing) = tool_calls_by_id.get_mut(&tool_call_id) {
                            tool_calls::apply_tool_call_update(existing, &update);
                            let _ = stream_tx
                                .send(LocalAcpStreamEvent::ToolCall(existing.clone()))
                                .await;
                        }
                    }
                    update => {
                        if let Some(event) = stream_event_from_update(update) {
                            let _ = stream_tx.send(event).await;
                        }
                    }
                }
            }
        }
    }

    Ok(LocalAcpSubmitResult {
        session_id: session.session_id.to_string(),
    })
}

fn model_config_id(
    config_options: Option<&[acp::SessionConfigOption]>,
) -> Option<acp::SessionConfigId> {
    config_options?
        .iter()
        .find(|option| {
            option.category == Some(acp::SessionConfigOptionCategory::Model)
                || option.id.to_string().to_ascii_lowercase().contains("model")
                || option.name.to_ascii_lowercase().contains("model")
        })
        .map(|option| option.id.clone())
}

fn stream_event_from_update(update: acp::SessionUpdate) -> Option<LocalAcpStreamEvent> {
    match update {
        acp::SessionUpdate::AgentMessageChunk(chunk) => match chunk.content {
            acp::ContentBlock::Text(text) => Some(LocalAcpStreamEvent::Text(text.text)),
            _ => None,
        },
        acp::SessionUpdate::AgentThoughtChunk(chunk) => match chunk.content {
            acp::ContentBlock::Text(text) => Some(LocalAcpStreamEvent::Thought(text.text)),
            _ => None,
        },
        _ => None,
    }
}
