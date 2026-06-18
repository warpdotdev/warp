//! The headless `warp-tui` front-end: a real (headless) Warp app whose root
//! window is a [`RootTuiView`] rendered through the `tui`-gated WarpUI backend.
//!
//! `RootTuiView` composes two child views — a [`TuiTranscriptView`] filling the
//! space above a bottom-anchored single-row [`TuiInputView`] — and routes the
//! input's submission events into the transcript. [`init`] is called from
//! `run_internal` once the headless app is up (see [`crate::run_tui`]). Ctrl-C
//! quit is handled by the runtime's input loop.

mod input_view;
mod transcript_view;

use std::collections::HashMap;
use std::env;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ai::skills::SkillPathOrigin;
use anyhow::{anyhow, Result};
use chrono::Local;
use input_view::{InputEvent, TuiInputView};
use transcript_view::TuiTranscriptView;
use warp_multi_agent_api::{AgentType, ToolType};
use warpui::r#async::Timer;
use warpui::{ModelContext, ModelHandle, SingletonEntity};
use warpui_core::elements::tui::{TuiChildView, TuiColumn, TuiConstrainedBox, TuiElement};
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{
    AddWindowOptions, AppContext, Entity, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::ai::agent::api::{self, RequestParams};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAttachment, AIAgentContext, AIAgentInput, AIAgentOutputStatus, CancellationReason,
    FinishedAIAgentOutput, UserQueryMode,
};
use crate::ai::blocklist::{
    AgentConversationEngine, AgentConversationEngineDelegate, AgentSessionOwnerId,
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, RequestInput, ResponseStream,
    ResponseStreamId, SessionContext,
};
use crate::ai::llms::LLMPreferences;
use crate::ai_assistant::execution_context::{WarpAiExecutionContext, WarpAiOsContext};

/// The bottom input frame's height: one text row inside a single-cell rounded
/// border (top + bottom), i.e. three rows total.
const INPUT_ROWS: u16 = 3;

/// App-level singleton owning the TUI app's single agent session.
pub struct CoreTuiModel {
    owner: Option<AgentSessionOwnerId>,
    active_conversation_id: Option<AIConversationId>,
    in_flight: Option<ModelHandle<ResponseStream>>,
}

#[allow(dead_code)]
impl CoreTuiModel {
    /// Creates the TUI core model and subscribes to active conversation updates.
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), |me, event, ctx| {
            let Some(conversation_id) = me.conversation_id_for_history_event(event) else {
                return;
            };
            if Some(conversation_id) == me.active_conversation_id {
                ctx.emit(CoreTuiModelEvent::ConversationUpdated { conversation_id });
            }
        });

        Self {
            owner: None,
            active_conversation_id: None,
            in_flight: None,
        }
    }

    /// Registers the single TUI session owner.
    pub fn register_session(&mut self, owner: AgentSessionOwnerId, _ctx: &mut ModelContext<Self>) {
        self.owner.get_or_insert(owner);
    }

    /// Sends a prompt in the single TUI session.
    pub fn send_prompt(
        &mut self,
        prompt: String,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(AIConversationId, ResponseStreamId)> {
        self.send_prompt_internal(prompt, false, ctx)
    }

    /// Sends a prompt using a no-network response stream for tests.
    #[cfg(test)]
    pub(crate) fn send_prompt_for_test(
        &mut self,
        prompt: String,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(AIConversationId, ResponseStreamId)> {
        self.send_prompt_internal(prompt, true, ctx)
    }

    fn send_prompt_internal(
        &mut self,
        prompt: String,
        use_fake_stream: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(AIConversationId, ResponseStreamId)> {
        if self.in_flight.is_some() {
            return Err(anyhow!("TUI agent request already in flight"));
        }
        let owner = self
            .owner
            .ok_or_else(|| anyhow!("TUI agent session is not registered"))?;

        let conversation_id = self.active_conversation_id.unwrap_or_else(|| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model.start_new_conversation(owner.entity_id(), false, false, false, ctx)
            })
        });

        let (task_id, conversation_data, parent_agent_id, agent_name) =
            conversation_request_data(owner, conversation_id, ctx)?;
        let context = TuiAgentContextBuilder::context(ctx);
        let request_input =
            tui_request_input(owner, conversation_id, task_id, prompt, context, ctx);
        let mut request_params = RequestParams::new(
            Some(owner.entity_id()),
            TuiAgentContextBuilder::session_context(ctx),
            &request_input,
            conversation_data.clone(),
            None,
            ctx,
        );
        request_params.parent_agent_id = parent_agent_id;
        request_params.agent_name = agent_name;

        #[cfg(test)]
        let (response_stream, response_stream_id) = if use_fake_stream {
            AgentConversationEngine::send_request_for_test(
                owner,
                request_input,
                request_params,
                conversation_data,
                ctx,
            )
        } else {
            AgentConversationEngine::send_request(
                owner,
                request_input,
                request_params,
                conversation_data,
                /*can_attempt_resume_on_error*/ true,
                ctx,
            )
        };
        #[cfg(not(test))]
        let (response_stream, response_stream_id) = {
            let _ = use_fake_stream;
            AgentConversationEngine::send_request(
                owner,
                request_input,
                request_params,
                conversation_data,
                /*can_attempt_resume_on_error*/ true,
                ctx,
            )
        };
        self.active_conversation_id = Some(conversation_id);
        self.in_flight = Some(response_stream);
        ctx.emit(CoreTuiModelEvent::PromptSubmitted { conversation_id });
        Ok((conversation_id, response_stream_id))
    }

    /// Cancels the active TUI request, if any.
    pub fn cancel_active_request(&mut self, ctx: &mut ModelContext<Self>) {
        let (Some(response_stream), Some(conversation_id)) =
            (self.in_flight.as_ref(), self.active_conversation_id)
        else {
            return;
        };
        response_stream.update(ctx, |stream, ctx| {
            stream.cancel(CancellationReason::ManuallyCancelled, conversation_id, ctx);
        });
    }

    /// Returns the active TUI conversation id, if one has started.
    pub fn active_conversation_id(&self) -> Option<AIConversationId> {
        self.active_conversation_id
    }

    /// Returns true while the single TUI session has an active request stream.
    pub fn has_in_flight_request(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Returns the in-flight response stream handle for request-construction tests.
    #[cfg(test)]
    pub(crate) fn in_flight_response_stream_for_test(&self) -> Option<ModelHandle<ResponseStream>> {
        self.in_flight.clone()
    }

    /// Extracts a conversation id from history events relevant to the active TUI conversation.
    fn conversation_id_for_history_event(
        &self,
        event: &BlocklistAIHistoryEvent,
    ) -> Option<AIConversationId> {
        match event {
            BlocklistAIHistoryEvent::StartedNewConversation {
                new_conversation_id,
                ..
            } => Some(*new_conversation_id),
            BlocklistAIHistoryEvent::CreatedSubtask {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::AppendedExchange {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::SetActiveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::ClearedActiveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationTitle {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::ConversationOwnershipTransferred {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::NewConversationRequestComplete {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::OrchestrationConfigUpdated {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                conversation_id, ..
            } => Some(*conversation_id),
            BlocklistAIHistoryEvent::ReassignedExchange {
                new_conversation_id,
                ..
            }
            | BlocklistAIHistoryEvent::SplitConversation {
                new_conversation_id,
                ..
            } => Some(*new_conversation_id),
            BlocklistAIHistoryEvent::RestoredConversations {
                conversation_ids, ..
            }
            | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView {
                cleared_conversation_ids: conversation_ids,
                ..
            } => self
                .active_conversation_id
                .filter(|active| conversation_ids.contains(active)),
            BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. } => self.active_conversation_id,
        }
    }
}

/// Sends a prompt through the TUI core model, prints the response, and exits.
pub fn run_prompt(prompt: String, ctx: &mut AppContext) {
    ctx.add_singleton_model(|ctx| TuiPromptRunner::new(prompt, ctx));
}

impl AgentConversationEngineDelegate for CoreTuiModel {
    fn skill_path_origin(&self, _ctx: &AppContext) -> SkillPathOrigin {
        SkillPathOrigin::Local
    }

    fn finished_receiving_output(
        &mut self,
        stream_id: ResponseStreamId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|stream| stream.as_ref(ctx).id() == &stream_id)
        {
            self.in_flight = None;
        }
        ctx.emit(CoreTuiModelEvent::RequestFinished { conversation_id });
    }
}

impl Entity for CoreTuiModel {
    type Event = CoreTuiModelEvent;
}

impl SingletonEntity for CoreTuiModel {}

#[allow(dead_code)]
pub enum CoreTuiModelEvent {
    /// A prompt was accepted and a request was sent.
    PromptSubmitted { conversation_id: AIConversationId },
    /// Streamed output mutated the conversation.
    ConversationUpdated { conversation_id: AIConversationId },
    /// The active request reached a terminal state.
    RequestFinished { conversation_id: AIConversationId },
}

struct TuiPromptRunner {
    conversation_id: AIConversationId,
    finished: bool,
    printed_output: String,
}

impl TuiPromptRunner {
    /// Sends a prompt through `CoreTuiModel` and exits after printing the final response.
    fn new(prompt: String, ctx: &mut ModelContext<Self>) -> Self {
        let owner = AgentSessionOwnerId::new(ctx.model_id());
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |runner, event, ctx| {
                let BlocklistAIHistoryEvent::UpdatedConversationStatus {
                    conversation_id,
                    new_status,
                    ..
                } = event
                else {
                    return;
                };
                if *conversation_id == runner.conversation_id && !new_status.is_in_progress() {
                    runner.print_response_and_exit(ctx);
                }
            },
        );
        ctx.subscribe_to_model(
            &CoreTuiModel::handle(ctx),
            |runner, event, ctx| match event {
                CoreTuiModelEvent::ConversationUpdated { conversation_id }
                    if *conversation_id == runner.conversation_id =>
                {
                    if let Err(error) = runner.print_incremental_response(ctx) {
                        eprintln!("{error:#}");
                    }
                }
                CoreTuiModelEvent::RequestFinished { conversation_id }
                    if *conversation_id == runner.conversation_id =>
                {
                    runner.print_response_and_exit(ctx);
                }
                CoreTuiModelEvent::PromptSubmitted { .. }
                | CoreTuiModelEvent::ConversationUpdated { .. }
                | CoreTuiModelEvent::RequestFinished { .. } => {}
            },
        );

        let conversation_id = match CoreTuiModel::handle(ctx).update(ctx, |model, ctx| {
            model.register_session(owner, ctx);
            model.send_prompt(prompt, ctx)
        }) {
            Ok((conversation_id, _)) => conversation_id,
            Err(error) => {
                eprintln!("{error:#}");
                ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
                AIConversationId::new()
            }
        };

        ctx.spawn(
            async move { Timer::after(Duration::from_secs(120)).await },
            |runner, _, ctx| {
                if !runner.finished {
                    let error = anyhow!("timed out waiting for TUI prompt response");
                    eprintln!("{error:#}");
                    ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
                    runner.finished = true;
                }
            },
        );

        Self {
            conversation_id,
            finished: false,
            printed_output: String::new(),
        }
    }

    fn print_incremental_response(&mut self, ctx: &AppContext) -> Result<()> {
        let Some(current_output) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&self.conversation_id)
            .and_then(current_response_text)
        else {
            return Ok(());
        };

        if current_output == self.printed_output {
            return Ok(());
        }

        if let Some(delta) = current_output.strip_prefix(&self.printed_output) {
            print!("{delta}");
        } else {
            if !self.printed_output.is_empty() && !self.printed_output.ends_with('\n') {
                println!();
            }
            print!("{current_output}");
        }
        io::stdout().flush()?;
        self.printed_output = current_output;
        Ok(())
    }

    fn print_response_and_exit(&mut self, ctx: &mut ModelContext<Self>) {
        if self.finished {
            return;
        }
        self.finished = true;
        let result = self.print_incremental_response(ctx).and_then(|()| {
            BlocklistAIHistoryModel::handle(ctx).read(ctx, |history_model, _| {
                let conversation = history_model
                    .conversation(&self.conversation_id)
                    .ok_or_else(|| {
                        anyhow!("TUI conversation {:?} was not found", self.conversation_id)
                    })?;
                final_response_text(conversation)
            })
        });

        match result {
            Ok(_) => {
                if !self.printed_output.is_empty() && !self.printed_output.ends_with('\n') {
                    println!();
                }
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            Err(error) => {
                eprintln!("{error:#}");
                ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
            }
        }
    }
}

impl Entity for TuiPromptRunner {
    type Event = ();
}

impl SingletonEntity for TuiPromptRunner {}

fn current_response_text(conversation: &AIConversation) -> Option<String> {
    let exchange = conversation.root_task_exchanges().last()?;
    let output = exchange.output_status.output()?.get();
    let text = output.format_for_copy(None);
    Some(if text.trim().is_empty() {
        output.to_string()
    } else {
        text
    })
}
fn final_response_text(conversation: &AIConversation) -> Result<String> {
    let exchange = conversation
        .root_task_exchanges()
        .last()
        .ok_or_else(|| anyhow!("TUI conversation finished without any exchanges"))?;

    match &exchange.output_status {
        AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Success { output },
        } => {
            let output = output.get();
            let text = output.format_for_copy(None);
            if text.trim().is_empty() {
                Ok(output.to_string())
            } else {
                Ok(text)
            }
        }
        output_status => Err(anyhow!(
            "TUI prompt did not finish successfully: status={:?}, conversation_status={:?}, output={}",
            output_status,
            conversation.status(),
            output_status,
        )),
    }
}

/// Builds request context for a TUI agent query.
#[allow(dead_code)]
pub struct TuiAgentContextBuilder;

#[allow(dead_code)]
impl TuiAgentContextBuilder {
    /// Builds phase-one TUI context.
    pub fn context(_app: &AppContext) -> Arc<[AIAgentContext]> {
        let pwd = env::current_dir()
            .ok()
            .map(|path| path.to_string_lossy().to_string());
        let home_dir = dirs::home_dir().map(|path| path.to_string_lossy().to_string());
        let shell_name = env::var("SHELL")
            .ok()
            .and_then(|shell| {
                Path::new(&shell)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "unknown".to_string());

        Arc::from([
            AIAgentContext::Directory {
                pwd,
                home_dir,
                are_file_symbols_indexed: false,
            },
            AIAgentContext::CurrentTime {
                current_time: Local::now(),
            },
            AIAgentContext::ExecutionEnvironment(WarpAiExecutionContext {
                os: WarpAiOsContext {
                    category: Some(env::consts::OS.to_string()),
                    distribution: None,
                },
                shell_name,
                shell_version: env::var("SHELL_VERSION").ok(),
            }),
        ])
    }

    /// Builds a local, non-terminal session context.
    pub fn session_context(_app: &AppContext) -> SessionContext {
        let cwd = env::current_dir()
            .ok()
            .map(|path| path.to_string_lossy().to_string());
        SessionContext::local(cwd)
    }
}

/// Builds request conversation data for the active TUI conversation.
#[allow(dead_code)]
fn conversation_request_data(
    owner: AgentSessionOwnerId,
    conversation_id: AIConversationId,
    ctx: &AppContext,
) -> Result<(
    TaskId,
    api::ConversationData,
    Option<String>,
    Option<String>,
)> {
    let history_model = BlocklistAIHistoryModel::as_ref(ctx);
    let conversation = history_model
        .conversation(&conversation_id)
        .ok_or_else(|| anyhow!("TUI conversation {conversation_id:?} does not exist"))?;
    let task_id = conversation.get_root_task_id().clone();
    let conversation_data = api::ConversationData {
        id: conversation.id(),
        tasks: conversation.compute_active_tasks(),
        server_conversation_token: conversation.server_conversation_token().cloned(),
        forked_from_conversation_token: conversation
            .forked_from_server_conversation_token()
            .cloned(),
        ambient_agent_task_id: None,
        existing_suggestions: history_model
            .existing_suggestions_for_conversation(conversation_id)
            .cloned(),
    };

    if !history_model
        .all_live_conversations_for_terminal_view(owner.entity_id())
        .any(|conversation| conversation.id() == conversation_id)
    {
        return Err(anyhow!(
            "TUI owner {:?} does not own conversation {:?}",
            owner,
            conversation_id
        ));
    }

    Ok((
        task_id,
        conversation_data,
        conversation.parent_agent_id().map(str::to_string),
        conversation.agent_name().map(str::to_string),
    ))
}

/// Builds the text-only TUI request input.
#[allow(dead_code)]
fn tui_request_input(
    owner: AgentSessionOwnerId,
    conversation_id: AIConversationId,
    task_id: TaskId,
    prompt: String,
    context: Arc<[AIAgentContext]>,
    app: &AppContext,
) -> RequestInput {
    let llm_prefs = LLMPreferences::as_ref(app);
    RequestInput {
        conversation_id,
        input_messages: HashMap::from([(
            task_id,
            vec![AIAgentInput::UserQuery {
                query: prompt,
                context,
                static_query_type: None,
                referenced_attachments: HashMap::<String, AIAgentAttachment>::new(),
                user_query_mode: UserQueryMode::Normal,
                running_command: None,
                intended_agent: Some(AgentType::Primary),
            }],
        )]),
        working_directory: TuiAgentContextBuilder::session_context(app)
            .current_working_directory()
            .clone(),
        model_id: llm_prefs
            .get_active_base_model(app, Some(owner.entity_id()))
            .id
            .clone(),
        coding_model_id: llm_prefs
            .get_active_coding_model(app, Some(owner.entity_id()))
            .id
            .clone(),
        cli_agent_model_id: llm_prefs
            .get_active_cli_agent_model(app, Some(owner.entity_id()))
            .id
            .clone(),
        computer_use_model_id: llm_prefs
            .get_active_computer_use_model(app, Some(owner.entity_id()))
            .id
            .clone(),
        shared_session_response_initiator: None,
        request_start_ts: Local::now(),
        supported_tools_override: Some(Vec::<ToolType>::new()),
    }
}

/// The root TUI view: a transcript that grows upward above a fixed,
/// bottom-anchored input. It owns both child views and forwards the input's
/// submissions into the transcript.
struct RootTuiView {
    transcript: ViewHandle<TuiTranscriptView>,
    input: ViewHandle<TuiInputView>,
}

impl RootTuiView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        // The transcript has no typed actions, so a plain TUI view suffices; the
        // input dispatches editing actions, so it must be a typed-action view.
        let transcript = ctx.add_tui_view(|_| TuiTranscriptView::default());
        let input = ctx.add_typed_action_tui_view(|_| TuiInputView::default());

        // On submission, append the text to the transcript. Routing through the
        // root (rather than wiring the transcript directly to the input) keeps
        // the view-ownership boundaries explicit and proves child-view
        // communication.
        ctx.subscribe_to_view(&input, |root, _input, event, ctx| match event {
            InputEvent::Submitted(text) => {
                let text = text.clone();
                root.transcript
                    .update(ctx, |transcript, ctx| transcript.append(text, ctx));
            }
        });

        ctx.focus(&input);

        Self { transcript, input }
    }
}

impl Entity for RootTuiView {
    type Event = ();
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let transcript = TuiChildView::new(&self.transcript, ctx);
        let input = TuiChildView::new(&self.input, ctx);

        // The transcript fills the space above the fixed-height input row.
        let column = TuiColumn::new()
            .flex_child(transcript)
            .child(TuiConstrainedBox::new(input).with_max_rows(INPUT_ROWS));

        Box::new(column)
    }
}

impl TypedActionView for RootTuiView {
    // The root handles no typed actions itself: editing actions are handled by
    // the input view, and Ctrl-C quit is handled by the runtime input loop.
    type Action = ();
}

/// Holds the live TUI session for the app's lifetime; dropping it on app
/// teardown restores the terminal.
struct TuiSession {
    _handle: TuiDriverHandle,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Creates the TUI root window and starts the headless draw + input driver.
/// Registered as a singleton so the session lives for the app's lifetime.
pub fn init(ctx: &mut AppContext) {
    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        RootTuiView::new,
    );
    CoreTuiModel::handle(ctx).update(ctx, |model, ctx| {
        model.register_session(AgentSessionOwnerId::new(root.id()), ctx);
    });

    match spawn_tui_driver(ctx, window_id, root) {
        Ok(handle) => {
            ctx.add_singleton_model(|_| TuiSession { _handle: handle });
        }
        Err(error) => {
            log::error!("failed to start the TUI driver: {error}");
            // Not in the alternate screen yet (entering it is what failed), so
            // print to stderr too — otherwise the process just exits instantly
            // with the reason buried in the log file.
            eprintln!(
                "warp-tui: could not start the terminal UI: {error}\n\
                 Run it directly in an interactive terminal (a real TTY), not piped or backgrounded."
            );
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        }
    }
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
