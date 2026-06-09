//! The bridge between the TUI and the real Warp agent.
//!
//! [`TuiAgentBridge`] is a model that owns the conversation transcript the UI
//! renders. The view subscribes to it and re-renders whenever it emits.
//!
//! It drives a single agent conversation through the lowest-level Multi-Agent
//! API client (`ServerApi::generate_multi_agent_output`): it builds a request
//! from the user's prompt, streams `api::ResponseEvent`s on the main thread,
//! applies the streamed client actions into the transcript, and emits `()` on
//! every change so the view re-renders. The PUBLIC SURFACE below (the transcript
//! types and the `entries`/`is_streaming`/`status_line` accessors and
//! `submit_user_input`/`new` signatures) is the contract the `components` child
//! agent renders against and MUST remain stable.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ai::api_keys::ApiKeyManager;
use warp_multi_agent_api as api;
use warp_multi_agent_api::client_action::{
    Action, AddMessagesToTask, AppendToMessageContent, CreateTask, UpdateTaskMessage,
};
use warp_multi_agent_api::message::Message as MessageBody;
use warp_multi_agent_api::response_event::stream_finished::Reason;
use warp_multi_agent_api::response_event::{StreamFinished, Type as ResponseEventType};
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::api::{
    ConversionParams, ConvertAPIMessageToClientOutputMessage, MaybeAIAgentOutputMessage,
};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentOutputMessageType};
use crate::ai::llms::LLMPreferences;
use crate::server::server_api::{AIApiError, ServerApiProvider};
use crate::settings::AISettings;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// How often to nudge a redraw while streaming so the spinner animates and
/// streamed text appears promptly (the TUI backend only repaints on events).
const SPINNER_TICK: Duration = Duration::from_millis(90);

/// Status of a tool call shown in the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiToolStatus {
    Running,
    Succeeded,
    Failed,
}

/// A single renderable entry in the conversation transcript.
#[derive(Debug, Clone)]
pub enum TuiTranscriptEntry {
    /// Something the user typed and submitted.
    User { text: String },
    /// Streamed assistant prose.
    Agent { text: String },
    /// A tool/command the agent invoked.
    ToolCall {
        title: String,
        detail: String,
        status: TuiToolStatus,
    },
    /// An informational notice (welcome message, errors, etc.).
    Notice { text: String },
}

/// Owns the transcript and streaming state for a single TUI agent conversation.
pub struct TuiAgentBridge {
    entries: Vec<TuiTranscriptEntry>,
    streaming: bool,
    status: String,
    /// Server-assigned conversation id, captured from the stream's init event
    /// and round-tripped on follow-up turns to continue the conversation.
    conversation_id: Option<String>,
    /// Task/message state mirrored from the streamed client actions and resent
    /// as request context so follow-up turns retain conversation history.
    tasks: Vec<api::Task>,
    /// Maps a streamed message id to the transcript entry rendering it, so
    /// streamed text/tool updates upsert in place instead of duplicating.
    entry_for_message: HashMap<String, usize>,
    /// Maps a tool call id to its transcript entry so a later result can flip
    /// its status.
    #[allow(dead_code)]
    tool_entry_for_call: HashMap<String, usize>,
}

impl Entity for TuiAgentBridge {
    type Event = ();
}

impl TuiAgentBridge {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            entries: vec![TuiTranscriptEntry::Notice {
                text: "Welcome to the Warp agent. Type a message and press Enter.".to_owned(),
            }],
            streaming: false,
            status: "Ready".to_owned(),
            conversation_id: None,
            tasks: Vec::new(),
            entry_for_message: HashMap::new(),
            tool_entry_for_call: HashMap::new(),
        }
    }

    /// Submit a line of user input to the agent, starting a streamed response.
    pub fn submit_user_input(&mut self, input: String, ctx: &mut ModelContext<Self>) {
        let input = input.trim().to_owned();
        // Ignore empty submissions and re-entrant submits while a turn streams.
        if input.is_empty() || self.streaming {
            return;
        }
        self.entries.push(TuiTranscriptEntry::User {
            text: input.clone(),
        });
        self.streaming = true;
        self.status = "Thinking…".to_owned();
        ctx.emit(());
        self.start_request(input, ctx);
        self.schedule_spinner_tick(ctx);
    }

    /// The full transcript, oldest first.
    pub fn entries(&self) -> &[TuiTranscriptEntry] {
        &self.entries
    }

    /// Whether the agent is currently producing output.
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// A short status string for the status bar (e.g. "Ready", "Thinking…").
    pub fn status_line(&self) -> &str {
        &self.status
    }

    /// Build the request and spawn the streamed agent response.
    fn start_request(&mut self, query: String, ctx: &mut ModelContext<Self>) {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let request = self.build_request(query, ctx);
        ctx.spawn(
            async move { server_api.generate_multi_agent_output(&request).await },
            |bridge, result, ctx| match result {
                Ok(stream) => {
                    ctx.spawn_stream_local(
                        stream,
                        |bridge, event, ctx| bridge.handle_stream_event(event, ctx),
                        |bridge, ctx| bridge.finish_stream(ctx),
                    );
                }
                Err(error) => bridge.fail(format!("Request failed: {error}"), ctx),
            },
        );
    }

    /// Assemble a single-turn (or continuation) request from the user's prompt.
    fn build_request(&self, query: String, ctx: &mut ModelContext<Self>) -> api::Request {
        let model_config = {
            let preferences = LLMPreferences::as_ref(ctx);
            api::request::settings::ModelConfig {
                base: preferences
                    .get_active_base_model(ctx, None)
                    .id
                    .clone()
                    .into(),
                coding: preferences
                    .get_active_coding_model(ctx, None)
                    .id
                    .clone()
                    .into(),
                cli_agent: preferences
                    .get_active_cli_agent_model(ctx, None)
                    .id
                    .clone()
                    .into(),
                computer_use_agent: preferences
                    .get_active_computer_use_model(ctx, None)
                    .id
                    .clone()
                    .into(),
                ..Default::default()
            }
        };

        let api_keys = {
            let workspaces = UserWorkspaces::as_ref(ctx);
            let manager = ApiKeyManager::as_ref(ctx);
            let byo = workspaces.is_byo_api_key_enabled(ctx);
            let aws = workspaces.is_aws_bedrock_credentials_enabled(ctx);
            manager.api_keys_for_request(byo, aws)
        };
        let allow_credits = *AISettings::as_ref(ctx).can_use_warp_credits_for_fallback;
        let api_keys = match api_keys {
            Some(mut keys) => {
                keys.allow_use_of_warp_credits = allow_credits;
                Some(keys)
            }
            None if allow_credits => Some(api::request::settings::ApiKeys {
                allow_use_of_warp_credits: true,
                ..Default::default()
            }),
            None => None,
        };

        let pwd = std::env::current_dir()
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();

        let user_query = api::request::input::UserQuery {
            query,
            referenced_attachments: Default::default(),
            mode: None,
            intended_agent: Default::default(),
        };
        let input = api::request::Input {
            context: Some(api::InputContext {
                directory: Some(api::input_context::Directory {
                    pwd,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            r#type: Some(api::request::input::Type::UserInputs(
                api::request::input::UserInputs {
                    inputs: vec![api::request::input::user_inputs::UserInput {
                        input: Some(
                            api::request::input::user_inputs::user_input::Input::UserQuery(
                                user_query,
                            ),
                        ),
                    }],
                },
            )),
        };

        let settings = api::request::Settings {
            model_config: Some(model_config),
            api_keys,
            // The TUI renders tool calls but can't execute them yet, so advertising
            // tools would hang the turn waiting on results we never return.
            // TODO: enable tools once the TUI can execute them and return results.
            supported_tools: Vec::new(),
            supports_parallel_tool_calls: true,
            supports_reasoning_message: true,
            should_preserve_file_content_in_history: true,
            web_context_retrieval_enabled: true,
            ..Default::default()
        };

        api::Request {
            task_context: Some(api::request::TaskContext {
                tasks: self.tasks.clone(),
            }),
            input: Some(input),
            settings: Some(settings),
            metadata: Some(api::request::Metadata {
                conversation_id: self.conversation_id.clone().unwrap_or_default(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Handle one streamed event, mutating the transcript and emitting on change.
    fn handle_stream_event(
        &mut self,
        event: Result<api::ResponseEvent, Arc<AIApiError>>,
        ctx: &mut ModelContext<Self>,
    ) {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                self.fail(format!("Request failed: {error}"), ctx);
                return;
            }
        };
        match event.r#type {
            Some(ResponseEventType::Init(init)) => {
                if !init.conversation_id.is_empty() {
                    self.conversation_id = Some(init.conversation_id);
                }
                self.status = "Responding…".to_owned();
                ctx.emit(());
            }
            Some(ResponseEventType::ClientActions(client_actions)) => {
                for action in client_actions.actions {
                    self.apply_client_action(action);
                }
                ctx.emit(());
            }
            Some(ResponseEventType::Finished(finished)) => {
                let (status, is_error) = finish_status(&finished);
                if is_error {
                    self.entries.push(TuiTranscriptEntry::Notice {
                        text: status.clone(),
                    });
                }
                self.status = status;
                self.streaming = false;
                ctx.emit(());
            }
            None => {}
        }
    }

    /// Apply a single streamed client action into the transcript and task store.
    fn apply_client_action(&mut self, action: api::ClientAction) {
        let Some(action) = action.action else {
            return;
        };
        match action {
            Action::CreateTask(CreateTask { task: Some(task) }) => {
                let messages = task.messages.clone();
                if !self.tasks.iter().any(|existing| existing.id == task.id) {
                    self.tasks.push(task);
                }
                for message in &messages {
                    self.render_message(message, false);
                }
            }
            Action::AddMessagesToTask(AddMessagesToTask { task_id, messages }) => {
                for message in messages {
                    self.record_message(&task_id, message, false);
                }
            }
            Action::UpdateTaskMessage(UpdateTaskMessage {
                task_id,
                message: Some(message),
                ..
            }) => self.record_message(&task_id, message, false),
            Action::AppendToMessageContent(AppendToMessageContent {
                task_id,
                message: Some(message),
                ..
            }) => self.record_message(&task_id, message, true),
            _ => {}
        }
    }

    /// Store a streamed message for continuation context and render it.
    fn record_message(&mut self, task_id: &str, message: api::Message, append: bool) {
        self.store_message(task_id, &message, append);
        self.render_message(&message, append);
    }

    /// Mirror a streamed message into the task store used as request context.
    fn store_message(&mut self, task_id: &str, message: &api::Message, append: bool) {
        if !self.tasks.iter().any(|task| task.id == task_id) {
            self.tasks.push(api::Task {
                id: task_id.to_owned(),
                ..Default::default()
            });
        }
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == task_id) else {
            return;
        };
        if let Some(existing) = task.messages.iter_mut().find(|m| m.id == message.id) {
            if append {
                append_message_content(existing, message);
            } else {
                *existing = message.clone();
            }
        } else {
            task.messages.push(message.clone());
        }
    }

    /// Update the transcript entries for a single streamed message.
    fn render_message(&mut self, message: &api::Message, append: bool) {
        match message.message.as_ref() {
            Some(MessageBody::AgentOutput(output)) => {
                self.upsert_agent_entry(&message.id, &output.text, append);
            }
            // Tools are disabled for now (see `build_request`), so don't render
            // tool calls/results; keep the surface prose-only.
            // TODO: render these once the TUI executes tools.
            _ => {}
        }
    }

    /// Upsert streamed assistant prose, appending deltas to the live entry.
    fn upsert_agent_entry(&mut self, message_id: &str, text: &str, append: bool) {
        if let Some(&index) = self.entry_for_message.get(message_id) {
            if let Some(TuiTranscriptEntry::Agent { text: existing }) = self.entries.get_mut(index)
            {
                if append {
                    existing.push_str(text);
                } else {
                    *existing = text.to_owned();
                }
                return;
            }
        }
        self.entries.push(TuiTranscriptEntry::Agent {
            text: text.to_owned(),
        });
        self.entry_for_message
            .insert(message_id.to_owned(), self.entries.len() - 1);
    }

    /// Upsert a tool-call entry, deriving a friendly title from the action.
    #[allow(dead_code)]
    fn upsert_tool_call_entry(&mut self, message: &api::Message, tool_call_id: &str) {
        let (title, detail) = tool_call_summary(message);
        if let Some(&index) = self.entry_for_message.get(&message.id) {
            if let Some(TuiTranscriptEntry::ToolCall {
                title: existing_title,
                detail: existing_detail,
                ..
            }) = self.entries.get_mut(index)
            {
                *existing_title = title;
                *existing_detail = detail;
            }
            return;
        }
        self.entries.push(TuiTranscriptEntry::ToolCall {
            title,
            detail,
            status: TuiToolStatus::Running,
        });
        let index = self.entries.len() - 1;
        self.entry_for_message.insert(message.id.clone(), index);
        self.tool_entry_for_call
            .insert(tool_call_id.to_owned(), index);
    }

    /// Flip a tool call's status once its result message arrives.
    #[allow(dead_code)]
    fn mark_tool_call_succeeded(&mut self, tool_call_id: &str) {
        if let Some(&index) = self.tool_entry_for_call.get(tool_call_id) {
            if let Some(TuiTranscriptEntry::ToolCall { status, .. }) = self.entries.get_mut(index) {
                *status = TuiToolStatus::Succeeded;
            }
        }
    }

    /// Record a failed request/stream as a notice and stop streaming.
    fn fail(&mut self, message: String, ctx: &mut ModelContext<Self>) {
        self.streaming = false;
        self.status = "Error".to_owned();
        self.entries
            .push(TuiTranscriptEntry::Notice { text: message });
        ctx.emit(());
    }

    /// Safety net invoked when the stream ends without a `Finished` event.
    fn finish_stream(&mut self, ctx: &mut ModelContext<Self>) {
        if self.streaming {
            self.streaming = false;
            self.status = "Ready".to_owned();
            ctx.emit(());
        }
    }

    /// While streaming, periodically emit so the view redraws (spinner ticks and
    /// streamed text appears even when no new event has arrived yet).
    fn schedule_spinner_tick(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async {
                Timer::after(SPINNER_TICK).await;
            },
            |bridge, _result, ctx| {
                if bridge.streaming {
                    ctx.emit(());
                    bridge.schedule_spinner_tick(ctx);
                }
            },
        );
    }
}

/// Appends streamed text deltas into an accumulating message of the same kind.
fn append_message_content(existing: &mut api::Message, delta: &api::Message) {
    match (existing.message.as_mut(), delta.message.as_ref()) {
        (Some(MessageBody::AgentOutput(current)), Some(MessageBody::AgentOutput(more))) => {
            current.text.push_str(&more.text);
        }
        (Some(MessageBody::AgentReasoning(current)), Some(MessageBody::AgentReasoning(more))) => {
            current.reasoning.push_str(&more.reasoning);
        }
        _ => existing.message = delta.message.clone(),
    }
}

/// Derive a concise `(title, detail)` for a tool-call message.
#[allow(dead_code)]
fn tool_call_summary(message: &api::Message) -> (String, String) {
    let task_id = TaskId::new(message.task_id.clone());
    let params = ConversionParams {
        task_id: &task_id,
        current_todo_list: None,
        active_code_review: None,
    };
    match message.clone().to_client_output_message(params) {
        Ok(MaybeAIAgentOutputMessage::Message(output)) => match output.message {
            AIAgentOutputMessageType::Action(action) => action_summary(&action),
            AIAgentOutputMessageType::Subagent(_) => ("Subagent".to_owned(), String::new()),
            _ => ("Tool call".to_owned(), String::new()),
        },
        _ => ("Tool call".to_owned(), String::new()),
    }
}

/// Map an agent action to a short, human-readable `(title, detail)`.
#[allow(dead_code)]
fn action_summary(action: &AIAgentAction) -> (String, String) {
    if let Some(command) = action.executable_command() {
        return ("Run command".to_owned(), command);
    }
    let title = if action.is_request_file_edit() {
        "Edit files"
    } else if action.is_get_specific_files() {
        "Read files"
    } else if action.is_get_relevant_files() {
        "Search codebase"
    } else if action.is_grep() {
        "Grep"
    } else if action.is_file_glob() {
        "Find files"
    } else {
        "Tool call"
    };
    (title.to_owned(), String::new())
}

/// Map a stream-finished reason to a `(status, is_error)` pair.
fn finish_status(finished: &StreamFinished) -> (String, bool) {
    match &finished.reason {
        Some(Reason::Done(_)) | None => ("Ready".to_owned(), false),
        Some(Reason::QuotaLimit(_)) => ("Quota limit reached".to_owned(), true),
        Some(Reason::ContextWindowExceeded(_)) => ("Context window exceeded".to_owned(), true),
        Some(Reason::MaxTokenLimit(_)) => ("Reached max token limit".to_owned(), true),
        Some(Reason::LlmUnavailable(_)) => ("Model unavailable".to_owned(), true),
        Some(Reason::InvalidApiKey(_)) => ("Invalid API key".to_owned(), true),
        Some(Reason::InternalError(_)) => ("Internal error".to_owned(), true),
        Some(Reason::Other(_)) => ("Finished unexpectedly".to_owned(), true),
    }
}
