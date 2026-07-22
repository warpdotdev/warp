//! This module contains core business logic for Agent Mode, primarily sending input to an AI
//! model and receiving output.
//!
//! The `BlocklistAIController` orchestrates state updates and service calls to power the
//! Agent Mode UI.
pub mod input_context;
mod pending_response_streams;
pub mod response_stream;
pub(super) mod shared_session;
mod slash_command;
use std::collections::{HashMap, HashSet};
#[cfg(not(target_family = "wasm"))]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ai::skills::{ParsedSkill, SkillPathOrigin, SkillReference};
use anyhow::anyhow;
use chrono::{DateTime, Local};
use input_context::{input_context_for_request, parse_context_attachments};
use itertools::Itertools;
use parking_lot::FairMutex;
use pending_response_streams::PendingResponseStreams;
use session_sharing_protocol::common::ParticipantId;
pub use slash_command::*;
use warp_core::assertions::safe_assert;
use warp_errors::report_error;
use warp_multi_agent_api::{Task, ToolType, message};
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use self::response_stream::{ResponseStream, ResponseStreamEvent};
use super::action_model::{BlocklistAIActionEvent, BlocklistAIActionModel};
use super::context_model::{BlocklistAIContextModel, PendingAttachment, PendingFile};
use super::conversation_selection::{ConversationSelectionEvent, ConversationSelectionHandle};
use super::history_model::BlocklistAIHistoryModel;
use super::orchestration_event_streamer::{
    OrchestrationEventStreamer, OrchestrationEventStreamerEvent,
};
use super::orchestration_events::{OrchestrationEventService, OrchestrationEventServiceEvent};
use super::queued_query::{QueuedQueryId, QueuedQueryModel};
use super::{BlocklistAIInputModel, ResponseStreamId};
use crate::ai::AIRequestUsageModel;
use crate::ai::agent::api::{self, ServerConversationToken};
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionResult, AIAgentActionResultType, AIAgentAttachment, AIAgentContext,
    AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus, AIIdentifiers, CancellationOutcome,
    CancellationReason, DocumentContentAttachmentSource, EntrypointType, FileContext,
    FinishedAIAgentOutput, PassiveSuggestionResultType, PassiveSuggestionTrigger,
    PassiveSuggestionTriggerType, RenderableAIError, RequestCost, RequestMetadata, RunningCommand,
    StaticQueryType, TransientNetworkErrorKind, UserQueryMode, extract_user_query_mode,
};
use crate::ai::agent_events::AgentMessageEventMetadata;
#[cfg(not(target_family = "wasm"))]
use crate::ai::agent_sdk::ClaudeHarness;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::document::ai_document_model::{
    AIDocumentId, AIDocumentModel, AIDocumentUserEditStatus,
};
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::ai::skills::{ActiveSkillLookupError, SkillManager};
use crate::cloud_object::model::persistence::CloudModel;
use crate::features::FeatureFlag;
use crate::global_resource_handles::GlobalResourceHandlesProvider;
use crate::network::NetworkStatus;
use crate::notebooks::editor::model::FileLinkResolutionContext;
use crate::persistence::ModelEvent;
use crate::send_telemetry_from_ctx;
use crate::server::server_api::AIApiError;
#[cfg(not(target_family = "wasm"))]
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::TelemetryEvent;
use crate::terminal::ShellLaunchData;
use crate::terminal::model::block::{
    BlockId, CURSOR_MARKER, formatted_terminal_contents_for_input,
};
use crate::terminal::model::session::SessionType;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::TerminalModel;
use crate::terminal::view::inline_banner::ZeroStatePromptSuggestionType;
use crate::workspace::OneTimeModalModel;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::UserWorkspaces;

#[derive(Debug, Clone)]
pub struct SessionContext {
    session_type: Option<SessionType>,
    shell: Option<ShellLaunchData>,
    current_working_directory: Option<String>,
}

impl SessionContext {
    pub fn from_session(session: &ActiveSession, app: &AppContext) -> Self {
        SessionContext {
            session_type: session.session_type(app),
            shell: session.shell_launch_data(app),
            current_working_directory: session.current_working_directory().cloned(),
        }
    }

    pub fn session_type(&self) -> &Option<SessionType> {
        &self.session_type
    }

    pub fn shell(&self) -> &Option<ShellLaunchData> {
        &self.shell
    }

    pub fn current_working_directory(&self) -> &Option<String> {
        &self.current_working_directory
    }

    /// Returns the remote host ID if this is a `WarpifiedRemote` session with
    /// a connected `RemoteServerClient`.
    pub fn host_id(&self) -> Option<&warp_core::HostId> {
        match &self.session_type {
            Some(SessionType::WarpifiedRemote { host_id }) => host_id.as_ref(),
            Some(SessionType::Local) | None => None,
        }
    }

    /// Returns `true` if this is a remote session (regardless of whether
    /// the remote server client is connected).
    pub fn is_remote(&self) -> bool {
        matches!(self.session_type, Some(SessionType::WarpifiedRemote { .. }))
    }

    pub fn skill_path_origin(&self) -> SkillPathOrigin {
        match &self.session_type {
            Some(SessionType::WarpifiedRemote {
                host_id: Some(host_id),
            }) => SkillPathOrigin::Remote {
                host_id: host_id.clone(),
            },
            Some(SessionType::WarpifiedRemote { host_id: None }) => SkillPathOrigin::Unavailable,
            Some(SessionType::Local) | None => SkillPathOrigin::Local,
        }
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        SessionContext {
            session_type: None,
            shell: None,
            current_working_directory: None,
        }
    }

    #[cfg(test)]
    pub fn new_with_session_type_for_test(session_type: Option<SessionType>) -> Self {
        SessionContext {
            session_type,
            shell: None,
            current_working_directory: None,
        }
    }
}

pub enum BlocklistAIControllerEvent {
    /// Emitted when a request is sent to the AI agent API.
    SentRequest {
        contains_user_query: bool,
        /// True when this request is the first send of a previously queued prompt (e.g.
        /// via `/queue` or the auto-queue toggle) rather than a direct user submission.
        /// Subscribers that perform user-submission side effects (e.g. clearing the input
        /// buffer) should skip those effects when this is true — the user may have typed
        /// new input while the agent was busy and we don't want to wipe it.
        is_queued_prompt: bool,
        /// The model ID used for this request. None for slash commands that don't
        /// send a model request (e.g., /fork).
        model_id: LLMId,
        /// The ID of the response stream for this request.
        stream_id: ResponseStreamId,
    },

    /// Emitted when an AI output response is fully received, particularly relevant when output is
    /// being streamed.
    FinishedReceivingOutput {
        stream_id: ResponseStreamId,
        conversation_id: AIConversationId,
    },

    /// Emitted when the export-to-file slash command is executed.
    ExportConversationToFile {
        filename: Option<String>,
    },

    ExecuteLocalHarnessCommand {
        command: String,
    },
}

#[derive(Debug)]
pub struct RequestInput {
    pub conversation_id: AIConversationId,
    pub input_messages: HashMap<TaskId, Vec<AIAgentInput>>,
    pub working_directory: Option<String>,
    pub model_id: LLMId,
    pub coding_model_id: LLMId,
    pub cli_agent_model_id: LLMId,
    pub computer_use_model_id: LLMId,
    pub shared_session_response_initiator: Option<ParticipantId>,
    pub request_start_ts: DateTime<Local>,
    pub supported_tools_override: Option<Vec<ToolType>>,
}

impl RequestInput {
    fn for_task(
        inputs: Vec<AIAgentInput>,
        task_id: TaskId,
        active_session: &ModelHandle<ActiveSession>,
        shared_session_response_initiator: Option<ParticipantId>,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        app: &AppContext,
    ) -> Self {
        let mut me = Self::new_with_common_fields(
            conversation_id,
            active_session,
            shared_session_response_initiator,
            terminal_surface_id,
            app,
        );
        me.input_messages.insert(task_id, inputs);
        me
    }

    fn for_actions_results(
        action_results: Vec<AIAgentActionResult>,
        context: Arc<[AIAgentContext]>,
        active_session: &ModelHandle<ActiveSession>,
        shared_session_response_initiator: Option<ParticipantId>,
        conversation_id: AIConversationId,
        terminal_surface_id: EntityId,
        app: &AppContext,
    ) -> Self {
        let mut me = Self::new_with_common_fields(
            conversation_id,
            active_session,
            shared_session_response_initiator,
            terminal_surface_id,
            app,
        );
        for result in action_results.into_iter() {
            me.input_messages
                .entry(result.task_id.clone())
                .or_default()
                .push(AIAgentInput::ActionResult {
                    result,
                    context: context.clone(),
                });
        }
        me
    }

    pub fn all_inputs(&self) -> impl Iterator<Item = &AIAgentInput> {
        self.input_messages.values().flatten()
    }

    pub fn with_supported_tools(mut self, tools: Vec<ToolType>) -> Self {
        self.supported_tools_override = Some(tools);
        self
    }

    fn new_with_common_fields(
        conversation_id: AIConversationId,
        active_session: &ModelHandle<ActiveSession>,
        shared_session_response_initiator: Option<ParticipantId>,
        terminal_surface_id: EntityId,
        app: &AppContext,
    ) -> Self {
        let llm_prefs = LLMPreferences::as_ref(app);
        let model_id = llm_prefs
            .get_active_base_model(app, Some(terminal_surface_id))
            .id
            .clone();
        let coding_model_id = llm_prefs
            .get_active_coding_model(app, Some(terminal_surface_id))
            .id
            .clone();
        let cli_agent_model_id = llm_prefs
            .get_active_cli_agent_model(app, Some(terminal_surface_id))
            .id
            .clone();
        let computer_use_model_id = llm_prefs
            .get_active_computer_use_model(app, Some(terminal_surface_id))
            .id
            .clone();
        let working_directory = active_session
            .as_ref(app)
            .current_working_directory()
            .cloned();

        Self {
            conversation_id,
            input_messages: Default::default(),
            working_directory,
            model_id,
            coding_model_id,
            cli_agent_model_id,
            computer_use_model_id,
            shared_session_response_initiator,
            request_start_ts: Local::now(),
            supported_tools_override: None,
        }
    }
}

/// Controller for Blocklist AI.
///
/// This is responsible for managing and updating blocklist AI state for a single terminal surface.
pub struct BlocklistAIController {
    active_session: ModelHandle<ActiveSession>,
    input_model: ModelHandle<BlocklistAIInputModel>,
    context_model: ModelHandle<BlocklistAIContextModel>,
    action_model: ModelHandle<BlocklistAIActionModel>,
    terminal_model: Arc<FairMutex<TerminalModel>>,

    in_flight_response_streams: PendingResponseStreams,

    /// The ID of the terminal surface this controller is associated with.
    terminal_surface_id: EntityId,

    should_refresh_available_llms_on_stream_finish: bool,

    shared_session_state: shared_session::SharedSessionState,

    /// Ambient agent task ID attached to this controller. This is a property of the controller, and not an individual
    /// conversation, because the ambient agent task driver owns the entire Warp window working on a task, and any
    /// sessions within it. In the future, one task may span several sessions with background processes.
    ambient_agent_task_id: Option<AmbientAgentTaskId>,

    /// Per-session directory for downloading file attachments.
    /// Set by the agent driver based on the workspace directory (e.g. `{working_dir}/.warp/attachments`).
    attachments_download_dir: Option<std::path::PathBuf>,

    /// Pending auto-resume tasks that are waiting for network connectivity.
    /// These should be cancelled when a new request is sent for the same conversation.
    pending_auto_resume_handles: HashMap<AIConversationId, SpawnedFutureHandle>,
    /// Pending dormant Claude wake preparations for success-idle child conversations.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pending_local_claude_wakes: HashMap<AIConversationId, SpawnedFutureHandle>,
    /// Passive conversations explicitly requested to follow up after actions complete.
    pending_passive_follow_ups: HashSet<AIConversationId>,
    /// Passive suggestion results that should be included with the next request
    /// for a given conversation (e.g. accepted/iterated code diffs that weren't
    /// auto-resumed).
    pending_passive_suggestion_results: HashMap<
        AIConversationId,
        Vec<(
            PassiveSuggestionResultType,
            Option<PassiveSuggestionTrigger>,
        )>,
    >,
}

enum InputQueryType {
    /// The user submitted query from the input. This may map to [`AIAgentInput::UserQuery`] but may
    /// map to other `AIAgentInput` types depending on various factors.
    UserSubmittedQueryFromInput {
        query: String,
        static_query_type: Option<StaticQueryType>,
        running_command: Option<RunningCommand>,
    },
    /// A custom [`AIInputType`].
    AIInputType { ai_input: AIAgentInput },
}

enum WhichTask {
    NewConversation,
    Task {
        conversation_id: AIConversationId,
        task_id: TaskId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalClaudeWakeTrigger {
    PendingEvents,
    WakeOnlyStream {
        wake_message: AgentMessageEventMetadata,
    },
}

impl LocalClaudeWakeTrigger {
    #[cfg(not(target_family = "wasm"))]
    fn requires_pending_events(&self) -> bool {
        match self {
            Self::PendingEvents => true,
            Self::WakeOnlyStream { .. } => false,
        }
    }
}

struct InputQuery {
    which_task: WhichTask,
    input_query: InputQueryType,
    /// Additional referenced attachments to include in the query
    /// (e.g. file path references from shared session file uploads).
    additional_attachments: HashMap<String, AIAgentAttachment>,
    /// When `Some`, this submission is a fired queued-prompt row; the send path resolves the
    /// row's stored attachments by this id instead of the live input staging.
    queued_query_id: Option<QueuedQueryId>,
}

impl InputQuery {
    fn query(&self) -> String {
        match &self.input_query {
            InputQueryType::UserSubmittedQueryFromInput { query, .. } => query.clone(),
            InputQueryType::AIInputType { ai_input } => {
                ai_input.display_query().unwrap_or_default()
            }
        }
    }
}

impl BlocklistAIController {
    /// Returns the bundled-skill catalog origin for this controller's active session.
    pub fn skill_path_origin(&self, ctx: &AppContext) -> SkillPathOrigin {
        SessionContext::from_session(self.active_session.as_ref(ctx), ctx).skill_path_origin()
    }

    /// Creates a controller for a terminal surface.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        input_model: ModelHandle<BlocklistAIInputModel>,
        context_model: ModelHandle<BlocklistAIContextModel>,
        conversation_selection: ConversationSelectionHandle,
        action_model: ModelHandle<BlocklistAIActionModel>,
        active_session: ModelHandle<ActiveSession>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&action_model, move |me, _, event, ctx| {
            let BlocklistAIActionEvent::FinishedAction {
                conversation_id,
                cancellation_reason,
                ..
            } = event
            else {
                return;
            };
            // `FinalizedExternally` (e.g. shell exit) means the conversation status and message
            // is set elsewhere through a dedicated path, so we must not trigger a follow-up or update conversation status here.
            let cancellation_outcome =
                cancellation_reason.map(|reason| reason.conversation_outcome());
            if matches!(
                cancellation_outcome,
                Some(CancellationOutcome::FinalizedExternally)
            ) {
                return;
            }
            let action_model = me.action_model.as_ref(ctx);
            if action_model.has_unfinished_actions_for_conversation(*conversation_id) {
                return;
            }

            let history_model = BlocklistAIHistoryModel::handle(ctx);
            let Some((is_viewing_shared_session, is_entirely_passive_code_diff)) = history_model
                .as_ref(ctx)
                .conversation(conversation_id)
                .map(|conversation| {
                    (
                        conversation.is_viewing_shared_session(),
                        conversation.is_entirely_passive_code_diff(),
                    )
                })
            else {
                return;
            };

            // Viewer sessions should not send follow-ups.
            // They only act as passive viewers of the action stream.
            if is_viewing_shared_session {
                return;
            }

            let Some(finished_action_results) =
                action_model.get_finished_action_results(*conversation_id)
            else {
                return;
            };
            let is_passive_code_diff = is_entirely_passive_code_diff
                && finished_action_results.last().is_some_and(|result| {
                    matches!(result.result, AIAgentActionResultType::RequestFileEdits(_))
                });
            let has_manual_follow_up = me.pending_passive_follow_ups.contains(conversation_id);

            // A `Succeeded` cancellation (e.g. an optimistic long-running-command
            // completion or a revert) is itself the terminal result, so it neither
            // triggers a follow-up nor counts as a cancellation.
            let treat_as_success =
                matches!(cancellation_outcome, Some(CancellationOutcome::Succeeded));
            let should_trigger_follow_up_request = (!is_passive_code_diff
                && !treat_as_success
                && finished_action_results
                    .iter()
                    .any(|result| result.result.should_trigger_request_upon_completion()))
                || has_manual_follow_up;
            if !should_trigger_follow_up_request {
                if matches!(
                    cancellation_outcome,
                    Some(CancellationOutcome::KeepInProgress)
                ) {
                    return;
                }
                // We also check if there's an in-flight req, because it's possible that this
                // subscription callback was queued in response to auto-cancelling pending actions
                // in the process of constructing a request. In such cases, we don't want to update
                // conversation status to Cancelled/Success.
                if !me
                    .in_flight_response_streams
                    .has_active_stream_for_conversation(*conversation_id, ctx)
                {
                    // If the completed actions do not trigger a follow-up request, update conversation
                    // status based on the outcome of the actions.
                    //
                    // (It would otherwise remain `InProgress`, which would be correct, since we'd be
                    // immediately triggering a follow-up request).
                    //
                    // In practice, the only time where this codepath gets triggered is upon completion
                    // of a passive code diff action, where we don't autosend the next request.
                    //
                    // With passive code diffs, its most appropriate to mark the conversation
                    // successful if the passive diff was accepted. In practice, there's only ever
                    // one RequestFileEdits action, so `finished_action_results` at this point
                    // should only have a single element.
                    //
                    // If the user does end up following up on the passive diff-originated conversation,
                    // the status will once again be updated to `InProgress`.
                    let updated_conversation_status = if finished_action_results
                        .iter()
                        .all(|result| result.result.is_successful())
                        || treat_as_success
                    {
                        ConversationStatus::Success
                    } else {
                        // This is an imperfect heuristic that practically speaking should have no effect.
                        //
                        // If we actually need to differentiate between the state of a conversation
                        // where actions completed with mixed result statuses (e.g. a mix of
                        // cancelled, error, and success) _and_ we don't automatically send back action
                        // results to the agent, then it'd be worth considering adding a new status
                        // variant.
                        ConversationStatus::Cancelled
                    };
                    history_model.update(ctx, |history_model, ctx| {
                        history_model.update_conversation_status(
                            me.terminal_surface_id,
                            *conversation_id,
                            updated_conversation_status,
                            ctx,
                        );
                    });
                }
                // Unlock any pending-LRC row so it isn't left locked if the action
                // completes without triggering a follow-up request.
                QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                    model.unlock_pending_lrc_rows(*conversation_id, ctx);
                });
                return;
            }
            me.send_follow_up_for_conversation(*conversation_id, ctx);
            // Unlock any query queued during the pre-snapshot window now that the
            // snapshot has been sent.
            QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                model.unlock_pending_lrc_rows(*conversation_id, ctx);
            });
        });

        ctx.subscribe_to_model(&conversation_selection, |me, _, event, ctx| {
            let ConversationSelectionEvent::Deactivated {
                conversation_id,
                final_exchange_count,
                is_exit_before_new_entrance,
            } = event
            else {
                return;
            };
            if *is_exit_before_new_entrance || *final_exchange_count == 0 {
                return;
            }
            let history = BlocklistAIHistoryModel::handle(ctx);
            let Some(conversation) = history.as_ref(ctx).conversation(conversation_id) else {
                return;
            };
            if conversation.is_viewing_shared_session() {
                return;
            }
            if conversation.status().is_in_progress() {
                me.cancel_conversation_progress(
                    *conversation_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
            }
        });
        // Subscribe to the orchestration event service to inject events
        // (e.g. MessagesReceivedFromAgents) into conversations that receive inter-agent messages.
        let svc = OrchestrationEventService::handle(ctx);
        ctx.subscribe_to_model(&svc, move |me, _, event, ctx| {
            let OrchestrationEventServiceEvent::EventsReady { conversation_id } = event;
            me.handle_pending_events_ready(*conversation_id, ctx);
        });
        let streamer = OrchestrationEventStreamer::handle(ctx);
        ctx.subscribe_to_model(&streamer, move |me, _, event, ctx| match event {
            OrchestrationEventStreamerEvent::DormantClaudeWakeReady {
                conversation_id,
                wake_message,
            } => {
                me.handle_dormant_claude_wake_ready(*conversation_id, wake_message.clone(), ctx);
            }
            // Viewer-mode events are handled by `OrchestrationViewerModel`.
            OrchestrationEventStreamerEvent::ChildSpawned { .. }
            | OrchestrationEventStreamerEvent::ChildStatusChanged { .. }
            | OrchestrationEventStreamerEvent::WatchedRunStatusChanged { .. } => {}
        });
        Self {
            input_model,
            context_model,
            action_model,
            active_session,
            terminal_model,
            in_flight_response_streams: PendingResponseStreams::new(),
            terminal_surface_id,
            should_refresh_available_llms_on_stream_finish: false,
            shared_session_state: shared_session::SharedSessionState::default(),
            ambient_agent_task_id: None,
            attachments_download_dir: None,
            pending_auto_resume_handles: HashMap::new(),
            pending_local_claude_wakes: HashMap::new(),
            pending_passive_follow_ups: HashSet::new(),
            pending_passive_suggestion_results: HashMap::new(),
        }
    }

    /// Internal method to send a query to the AI model. External callers should use either
    /// `send_user_query_in_conversation`, `send_user_in_conversation`, or
    /// `send_custom_ai_input_query` instead.
    ///
    /// When the request is sent, a `BlocklistAIEvent::SentRequest` event is emitted containing the
    /// query itself as well as a oneshot `Receiver` that can be `await`-ed to receive the response
    /// from the AI.
    fn send_query(
        &mut self,
        input_query: InputQuery,
        entrypoint_type: EntrypointType,
        // The shared session participant who initiated this query
        // (None if this is not a shared session).
        shared_session_participant_id: Option<ParticipantId>,
        is_queued_prompt: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // Store the participant who initiated this query before sending
        // so that send_query can use it when creating the exchange.
        if let Some(participant_id) = shared_session_participant_id {
            self.set_current_response_initiator(participant_id);
        }

        let query = input_query.query().to_owned();
        let (conversation_id, task_id) = match input_query.which_task {
            WhichTask::NewConversation => {
                let conversation = self.start_new_conversation_for_request(ctx);
                (conversation.id(), conversation.get_root_task_id().clone())
            }
            WhichTask::Task {
                conversation_id,
                task_id,
            } => (conversation_id, task_id),
        };

        // Drain any queued passive suggestion results for this conversation
        // *before* cancelling progress, since cancel_conversation_progress
        // clears the pending map.
        let pending_passive_results = self
            .pending_passive_suggestion_results
            .remove(&conversation_id)
            .unwrap_or_default();

        let ai_history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let active_conversation_id =
            ai_history_model.active_conversation_id(self.terminal_surface_id);
        let cancellation_reason = CancellationReason::FollowUpSubmitted {
            is_for_same_conversation: active_conversation_id
                .is_some_and(|id| id == conversation_id),
        };
        if let Some(active_conversation_id) = active_conversation_id {
            self.cancel_conversation_progress(active_conversation_id, cancellation_reason, ctx);
        }

        if let Some(slash_command_request) = SlashCommandRequest::from_query(query.as_str()) {
            // Only fired queued rows carry `queued_query_id`. For those rows, keep slash commands
            // (e.g. queued `/compact`) on the conversation they were queued on; direct slash
            // submissions still re-derive their target from the current UI selection.
            let conversation_id_override = input_query
                .queued_query_id
                .is_some()
                .then_some(conversation_id);
            slash_command_request.send_request(
                self,
                input_query.queued_query_id,
                conversation_id_override,
                ctx,
            );
            return;
        }

        let (query, user_query_mode) = extract_user_query_mode(query);

        // Attribute /orchestrate queries to the slash-command entry surface.
        if matches!(user_query_mode, UserQueryMode::Orchestrate) {
            send_telemetry_from_ctx!(
                super::telemetry::BlocklistOrchestrationTelemetryEvent::OrchestrationEntered(
                    super::telemetry::OrchestrationEnteredEvent {
                        conversation_id,
                        plan_id: None,
                        entry_source:
                            super::telemetry::OrchestrationEntrySource::SlashCommandOrchestrate,
                    }
                ),
                ctx
            );
        }

        let should_prepend_finished_action_results = matches!(
            input_query.input_query,
            InputQueryType::UserSubmittedQueryFromInput { .. }
        );

        let completed_action_results = self.action_model.update(ctx, |action_model, ctx| {
            action_model.cancel_all_pending_actions(
                conversation_id,
                Some(cancellation_reason),
                ctx,
            );
            action_model.drain_finished_action_results(conversation_id)
        });

        let context = input_context_for_request(
            false,
            self.context_model.as_ref(ctx),
            self.active_session.as_ref(ctx),
            Some(conversation_id),
            vec![],
            ctx,
        );
        let mut inputs = if should_prepend_finished_action_results {
            completed_action_results
                .into_iter()
                .map(|result| AIAgentInput::ActionResult {
                    result,
                    context: context.clone(),
                })
                .collect_vec()
        } else {
            // Custom AI inputs like CodeReview are encoded as top-level request
            // variants (`request::input::Type::CodeReview`, etc.), and `convert_input`
            // only emits those variants in the single-input path.
            //
            // Tool call results are encoded differently: they only exist inside
            // `request::input::Type::UserInputs` as `user_input::Input::ToolCallResult`.
            // There is no proto request shape that can represent both a top-level
            // CodeReview-style input and a ToolCallResult in the same request.
            //
            // So if we prepend an ActionResult here, `convert_input` has to fall back
            // to the multi-input `UserInputs` path, where CodeReview is ignored
            // entirely. The stale tool result is preserved, but the custom AI input
            // disappears from the request.
            vec![]
        };

        // Append any queued passive suggestion results that were drained
        // earlier (before cancel_conversation_progress).
        for (suggestion, trigger) in pending_passive_results {
            inputs.push(AIAgentInput::PassiveSuggestionResult {
                trigger,
                suggestion,
                context: context.clone(),
            });
        }

        let additional_attachments = input_query.additional_attachments;
        let queued_query_id = input_query.queued_query_id;
        let ai_input = match input_query.input_query {
            InputQueryType::UserSubmittedQueryFromInput {
                static_query_type,
                running_command,
                ..
            } => {
                // Resolve the attachment set for this submission. The direct-send branch
                // preserves existing behavior: live input staging is still consumed by regular
                // submissions, but fired queued rows read from their row-owned attachment set.
                let prompt_attachments = match queued_query_id {
                    Some(query_id) => QueuedQueryModel::as_ref(ctx)
                        .attachments_for(conversation_id, query_id)
                        .to_vec(),
                    None => self
                        .context_model
                        .as_ref(ctx)
                        .pending_attachments()
                        .to_vec(),
                };

                input_for_query(
                    query,
                    &task_id,
                    conversation_id,
                    static_query_type,
                    user_query_mode,
                    running_command,
                    additional_attachments,
                    prompt_attachments,
                    self.context_model.as_ref(ctx),
                    self.active_session.as_ref(ctx),
                    ctx,
                )
            }
            InputQueryType::AIInputType { ai_input } => ai_input,
        };
        inputs.push(ai_input);

        // Piggyback any pending orchestration config updates for this conversation.
        let taken_dirty_events = AIDocumentModel::handle(ctx).update(ctx, |model, _| {
            model.take_dirty_orchestration_events(&conversation_id)
        });
        for dirty_event in &taken_dirty_events {
            inputs.push(AIAgentInput::OrchestrationConfigUpdate {
                plan_id: dirty_event.plan_id.clone(),
                config: dirty_event.config.clone(),
                status: dirty_event.status,
            });
        }

        let send_result = self.send_request_input(
            RequestInput::for_task(
                inputs,
                task_id,
                &self.active_session,
                self.get_current_response_initiator(),
                conversation_id,
                self.terminal_surface_id,
                ctx,
            ),
            Some(RequestMetadata {
                is_autodetected_user_query: !self.input_model.as_ref(ctx).is_input_type_locked(),
                entrypoint: entrypoint_type,
                is_auto_resume_after_error: false,
            }),
            /*can_attempt_resume_on_error*/ true,
            is_queued_prompt,
            ctx,
        );

        // If the request failed, re-insert the dirty events so they aren't
        // silently lost.
        if let Err(e) = &send_result {
            report_error!(e);
            if !taken_dirty_events.is_empty() {
                AIDocumentModel::handle(ctx).update(ctx, |model, _| {
                    model.set_dirty_orchestration_events(conversation_id, taken_dirty_events);
                });
            }
        }
    }

    /// Populates plan documents from user query to AIDocumentModel if not already present.
    /// Parses attachments from query and creates AI documents for any user-attached plans.
    /// This is split from parse_context_attachments to run later in the pipeline when new conversations are created.
    fn maybe_populate_plans_for_ai_document_model(
        &self,
        referenced_attachments: &HashMap<String, AIAgentAttachment>,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Get file link resolution context from active session
        let session = self.active_session.as_ref(ctx);
        let file_link_resolution_context =
            session
                .current_working_directory()
                .cloned()
                .map(|working_directory| FileLinkResolutionContext {
                    working_directory,
                    shell_launch_data: session.shell_launch_data(ctx),
                });

        for attachment in referenced_attachments.values() {
            let AIAgentAttachment::DocumentContent {
                document_id,
                content,
                source,
                ..
            } = attachment
            else {
                continue;
            };
            if !matches!(*source, DocumentContentAttachmentSource::UserAttached) {
                continue;
            }
            let document_id = match AIDocumentId::try_from(document_id.as_str()) {
                Ok(id) => id,
                Err(_) => {
                    log::warn!("Invalid ai_document_id in document content: {document_id}");
                    continue;
                }
            };

            // Skip if document already exists in the model
            let ai_document_model = AIDocumentModel::as_ref(ctx);
            if ai_document_model
                .get_current_document(&document_id)
                .is_some()
            {
                continue;
            }

            // Look up notebook to get title and sync_id
            let cloud_model = CloudModel::as_ref(ctx);
            let notebook_data = cloud_model
                .get_all_active_notebooks()
                .find(|nb| nb.model().ai_document_id.as_ref() == Some(&document_id))
                .map(|nb| (nb.model().title.clone(), nb.id));

            if let Some((title, sync_id)) = notebook_data {
                AIDocumentModel::handle(ctx).update(ctx, |model, model_ctx| {
                    model.create_document_from_notebook(
                        document_id,
                        sync_id,
                        title,
                        content,
                        conversation_id,
                        file_link_resolution_context.clone(),
                        model_ctx,
                    );
                });
            } else {
                log::warn!("Notebook not found for ai_document_id: {document_id}");
            }
        }
    }

    pub fn send_user_query_in_new_conversation(
        &mut self,
        query: String,
        static_query_type: Option<StaticQueryType>,
        entrypoint_type: EntrypointType,
        participant_id: Option<ParticipantId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_user_query_in_new_conversation_internal(
            query,
            static_query_type,
            entrypoint_type,
            participant_id,
            /*is_queued_prompt*/ false,
            /*queued_query_id*/ None,
            ctx,
        );
    }

    /// Sends the first submission of a previously queued user prompt into a new conversation.
    /// Same as [`Self::send_user_query_in_new_conversation`] but marks the emitted
    /// `SentRequest` event so UI subscribers (e.g. the input editor) know not to treat
    /// this as a direct user submission and therefore not clear the input buffer.
    pub fn send_queued_user_query_in_new_conversation(
        &mut self,
        query: String,
        static_query_type: Option<StaticQueryType>,
        entrypoint_type: EntrypointType,
        participant_id: Option<ParticipantId>,
        queued_query_id: QueuedQueryId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_user_query_in_new_conversation_internal(
            query,
            static_query_type,
            entrypoint_type,
            participant_id,
            /*is_queued_prompt*/ true,
            Some(queued_query_id),
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn send_user_query_in_new_conversation_internal(
        &mut self,
        query: String,
        static_query_type: Option<StaticQueryType>,
        entrypoint_type: EntrypointType,
        participant_id: Option<ParticipantId>,
        is_queued_prompt: bool,
        queued_query_id: Option<QueuedQueryId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let participant_id = participant_id.or_else(|| self.get_sharer_participant_id());
        let running_command = {
            let terminal_model = self.terminal_model.lock();
            get_running_command(&terminal_model)
        };
        if let Some(running_command) = running_command {
            let conversation_id = self.start_new_conversation_for_request(ctx).id();
            let history_model = BlocklistAIHistoryModel::handle(ctx);
            let task_id = match history_model.update(ctx, |history_model, ctx| {
                history_model.create_cli_subagent_task_for_conversation(
                    running_command.block_id.clone(),
                    conversation_id,
                    self.terminal_surface_id,
                    ctx,
                )
            }) {
                Ok(task_id) => task_id,
                Err(e) => {
                    report_error!(
                        anyhow::Error::new(e)
                            .context("Could not create CLI subagent task optimistically")
                    );
                    return;
                }
            };
            self.send_query(
                InputQuery {
                    which_task: WhichTask::Task {
                        conversation_id,
                        task_id,
                    },
                    input_query: InputQueryType::UserSubmittedQueryFromInput {
                        query,
                        static_query_type,
                        running_command: Some(running_command),
                    },
                    additional_attachments: HashMap::new(),
                    queued_query_id,
                },
                entrypoint_type,
                participant_id,
                is_queued_prompt,
                ctx,
            );
        } else {
            self.send_query(
                InputQuery {
                    which_task: WhichTask::NewConversation,
                    input_query: InputQueryType::UserSubmittedQueryFromInput {
                        query,
                        static_query_type,
                        running_command: None,
                    },
                    additional_attachments: HashMap::new(),
                    queued_query_id,
                },
                entrypoint_type,
                participant_id,
                is_queued_prompt,
                ctx,
            );
        }
    }

    /// Sends a query into an existing conversation as an agent-initiated request.
    /// This is the agent-initiated counterpart to `send_user_query_in_conversation`.
    pub fn send_agent_query_in_conversation(
        &mut self,
        query: String,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_user_query_in_conversation_internal(
            query,
            conversation_id,
            None,
            false,
            HashMap::new(),
            EntrypointType::AgentInitiated,
            /*is_queued_prompt*/ false,
            /*queued_query_id*/ None,
            ctx,
        );
    }

    /// Sends the given user query to the AI model, returning whether it
    /// reached the shared request dispatch path.
    pub fn send_user_query_in_conversation(
        &mut self,
        query: String,
        conversation_id: AIConversationId,
        participant_id: Option<ParticipantId>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        self.send_user_query_in_conversation_internal(
            query,
            conversation_id,
            participant_id,
            false, // skip_running_command_detection
            HashMap::new(),
            EntrypointType::UserInitiated,
            /*is_queued_prompt*/ false,
            /*queued_query_id*/ None,
            ctx,
        )
    }

    /// Sends the first submission of a previously queued user prompt into an existing conversation.
    /// Same as [`Self::send_user_query_in_conversation`] but marks the emitted `SentRequest`
    /// event so UI subscribers (e.g. the input editor) know not to treat this as a direct
    /// user submission and therefore not clear the input buffer.
    pub fn send_queued_user_query_in_conversation(
        &mut self,
        query: String,
        conversation_id: AIConversationId,
        participant_id: Option<ParticipantId>,
        queued_query_id: QueuedQueryId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_user_query_in_conversation_internal(
            query,
            conversation_id,
            participant_id,
            false, // skip_running_command_detection
            HashMap::new(),
            EntrypointType::UserInitiated,
            /*is_queued_prompt*/ true,
            Some(queued_query_id),
            ctx,
        );
    }

    /// Sends the given user query to the AI model, with additional referenced attachments.
    pub fn send_user_query_in_conversation_with_attachments(
        &mut self,
        query: String,
        conversation_id: AIConversationId,
        participant_id: Option<ParticipantId>,
        additional_attachments: HashMap<String, AIAgentAttachment>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_user_query_in_conversation_internal(
            query,
            conversation_id,
            participant_id,
            false, // skip_running_command_detection
            additional_attachments,
            EntrypointType::UserInitiated,
            /*is_queued_prompt*/ false,
            /*queued_query_id*/ None,
            ctx,
        );
    }

    /// Sends the given user query to the AI model, skipping long running command detection.
    /// We use this when we fork a conversation and immediately send an initial query, to avoid
    /// a race condition where restored command blocks may appear long running when the initial query is sent,
    /// causing the query to go to the lrc subagent.
    pub fn send_user_query_in_conversation_no_lrc_subagent(
        &mut self,
        query: String,
        conversation_id: AIConversationId,
        participant_id: Option<ParticipantId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_user_query_in_conversation_internal(
            query,
            conversation_id,
            participant_id,
            true, // skip_running_command_detection
            HashMap::new(),
            EntrypointType::UserInitiated,
            /*is_queued_prompt*/ false,
            /*queued_query_id*/ None,
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn send_user_query_in_conversation_internal(
        &mut self,
        query: String,
        conversation_id: AIConversationId,
        participant_id: Option<ParticipantId>,
        skip_running_command_detection: bool,
        additional_attachments: HashMap<String, AIAgentAttachment>,
        entrypoint_type: EntrypointType,
        is_queued_prompt: bool,
        queued_query_id: Option<QueuedQueryId>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let is_viewer = self
            .terminal_model
            .lock()
            .shared_session_status()
            .is_viewer();
        if is_viewer {
            report_error!("Viewers should never attempt to send queries directly");
        }

        // Ensure we capture all pending context blocks before promoting and attaching them to the conversation.
        let context_block_ids = self
            .context_model
            .as_ref(ctx)
            .pending_context_block_ids()
            .clone();

        let (promoted_blocks, task_id, running_command) = {
            let mut terminal_model = self.terminal_model.lock();
            terminal_model
                .block_list_mut()
                .associate_blocks_with_conversation(context_block_ids.iter(), conversation_id);

            // Promote all blocks that are pending for this conversation to attached.
            // This happens at query submission time, making blocks permanently associated with the conversation.
            let promoted_blocks = terminal_model
                .block_list_mut()
                .promote_blocks_to_attached_from_conversation(conversation_id);

            let active_block = terminal_model.block_list().active_block();
            let running_command_opt = if !skip_running_command_detection {
                get_running_command(&terminal_model)
            } else {
                None
            };

            let (task_id, running_command) = if let Some(running_command) = running_command_opt {
                let history_model = BlocklistAIHistoryModel::handle(ctx);
                match history_model.update(ctx, |history_model, ctx| {
                    history_model.create_cli_subagent_task_for_conversation(
                        running_command.block_id.clone(),
                        conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    )
                }) {
                    Ok(task_id) => (task_id, Some(running_command)),
                    Err(e) => {
                        report_error!(
                            anyhow::Error::new(e)
                                .context("Could not create CLI subagent task optimistically")
                        );
                        return false;
                    }
                }
            } else if let Some(task_id) = active_block
                .is_agent_monitoring()
                .then(|| active_block.agent_interaction_metadata())
                .flatten()
                .filter(|metadata| metadata.conversation_id() == &conversation_id)
                .and_then(|metadata| metadata.subagent_task_id().cloned())
            {
                (task_id, None)
            } else {
                let history_model = BlocklistAIHistoryModel::as_ref(ctx);
                let Some(conversation) = history_model.conversation(&conversation_id) else {
                    report_error!(
                        "Tried to send follow-up query for non-existent conversation",
                        extra: { "conversation_id" => ?conversation_id }
                    );
                    return false;
                };

                (conversation.get_root_task_id().clone(), None)
            };

            (promoted_blocks, task_id, running_command)
        };

        // Persist the updated visibility for each promoted block
        if !promoted_blocks.is_empty()
            && let Some(sender) = GlobalResourceHandlesProvider::as_ref(ctx)
                .get()
                .model_event_sender
                .as_ref()
        {
            for (block_id, agent_view_visibility) in promoted_blocks {
                if let Err(e) = sender.send(ModelEvent::UpdateBlockAgentViewVisibility {
                    block_id: block_id.to_string(),
                    agent_view_visibility: agent_view_visibility.into(),
                }) {
                    report_error!(
                        anyhow::Error::new(e)
                            .context("Error sending UpdateBlockAgentViewVisibility event")
                    );
                }
            }
        }

        let participant_id = participant_id.or_else(|| self.get_sharer_participant_id());
        self.send_query(
            InputQuery {
                which_task: WhichTask::Task {
                    conversation_id,
                    task_id,
                },
                input_query: InputQueryType::UserSubmittedQueryFromInput {
                    query,
                    static_query_type: None,
                    running_command,
                },
                additional_attachments,
                queued_query_id,
            },
            entrypoint_type,
            participant_id,
            is_queued_prompt,
            ctx,
        );
        true
    }

    /// Sends a request triggered by a zero-state prompt suggestion.
    pub fn send_zero_state_prompt_suggestion(
        &mut self,
        query_type: ZeroStatePromptSuggestionType,
        ctx: &mut ModelContext<Self>,
    ) {
        let participant_id = self.get_sharer_participant_id();
        self.send_query(
            InputQuery {
                which_task: WhichTask::NewConversation,
                input_query: InputQueryType::UserSubmittedQueryFromInput {
                    query: query_type.query().to_string(),
                    static_query_type: query_type.static_query_type(),
                    running_command: None,
                },
                additional_attachments: HashMap::new(),
                queued_query_id: None,
            },
            EntrypointType::ZeroStateAgentModePromptSuggestion,
            participant_id,
            /*is_queued_prompt*/ false,
            ctx,
        );
    }

    /// Sends a custom [`AIAgentInput`] query.
    pub fn send_custom_ai_input_query(
        &mut self,
        ai_input: AIAgentInput,
        ctx: &mut ModelContext<Self>,
    ) {
        let participant_id = self.get_sharer_participant_id();
        let which_task = match self.context_model.as_ref(ctx).selected_conversation_id(ctx) {
            Some(id) => {
                let Some(conversation) = BlocklistAIHistoryModel::as_ref(ctx).conversation(&id)
                else {
                    report_error!(
                        "Tried to send custom AI input query as follow-up in non-existent conversation"
                    );
                    return;
                };
                WhichTask::Task {
                    conversation_id: conversation.id(),
                    task_id: conversation.get_root_task_id().clone(),
                }
            }
            None => WhichTask::NewConversation,
        };
        self.send_query(
            InputQuery {
                which_task,
                input_query: InputQueryType::AIInputType { ai_input },
                additional_attachments: HashMap::new(),
                queued_query_id: None,
            },
            EntrypointType::UserInitiated,
            participant_id,
            /*is_queued_prompt*/ false,
            ctx,
        )
    }

    pub fn send_slash_command_request(
        &mut self,
        slash_command: SlashCommandRequest,
        ctx: &mut ModelContext<Self>,
    ) {
        slash_command.send_request(self, None, None, ctx);
    }
    /// Starts the create-project agent flow with the supplied project description.
    pub fn send_create_new_project_request(&mut self, query: String, ctx: &mut ModelContext<Self>) {
        self.send_slash_command_request(SlashCommandRequest::CreateNewProject { query }, ctx);
    }

    /// Resolves a skill reference against this controller's active execution host.
    pub(crate) fn resolve_skill_for_invocation(
        &self,
        reference: &SkillReference,
        ctx: &AppContext,
    ) -> Result<ParsedSkill, ActiveSkillLookupError> {
        let path_origin = self.skill_path_origin(ctx);
        SkillManager::handle(ctx)
            .as_ref(ctx)
            .active_skill_by_reference_with_origin(reference, &path_origin, ctx)
            .cloned()
    }

    /// Sends an already-resolved skill invocation through the shared slash-command request path.
    pub(crate) fn send_resolved_skill_invocation(
        &mut self,
        skill: ParsedSkill,
        user_query: Option<String>,
        queued_query_id: Option<QueuedQueryId>,
        conversation_id: Option<AIConversationId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let request = SlashCommandRequest::InvokeSkill { skill, user_query };
        if let Some(query_id) = queued_query_id {
            self.send_queued_slash_command_request(request, query_id, conversation_id, ctx);
        } else {
            self.send_slash_command_request(request, ctx);
        }
    }

    /// Resolves and sends a skill invocation for surfaces that do not need intermediate UI work.
    pub fn send_invoke_skill_request(
        &mut self,
        reference: SkillReference,
        user_query: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), ActiveSkillLookupError> {
        let skill = self.resolve_skill_for_invocation(&reference, ctx)?;
        self.send_resolved_skill_invocation(skill, user_query, None, None, ctx);
        Ok(())
    }

    /// Same as [`Self::send_slash_command_request`] but marks the emitted `SentRequest`
    /// event as a queued prompt submission so UI subscribers (e.g. the input editor)
    /// don't clear the input buffer on the auto-send.
    pub fn send_queued_slash_command_request(
        &mut self,
        slash_command: SlashCommandRequest,
        queued_query_id: QueuedQueryId,
        conversation_id: Option<AIConversationId>,
        ctx: &mut ModelContext<Self>,
    ) {
        slash_command.send_request(self, Some(queued_query_id), conversation_id, ctx);
    }

    /// Mark a conversation to follow up after its actions complete and attempt to send immediately
    /// if results are already available.
    pub fn request_follow_up_after_actions(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.pending_passive_follow_ups.insert(conversation_id);

        if self
            .in_flight_response_streams
            .has_active_stream_for_conversation(conversation_id, ctx)
        {
            return;
        }

        let has_pending_actions = self
            .action_model
            .as_ref(ctx)
            .get_pending_actions_for_conversation(&conversation_id)
            .next()
            .is_some();
        if has_pending_actions {
            return;
        }

        let finished_action_results = self
            .action_model
            .as_ref(ctx)
            .get_finished_action_results(conversation_id);
        if finished_action_results.is_some_and(|results| !results.is_empty()) {
            self.send_follow_up_for_conversation(conversation_id, ctx);
        }
    }

    /// Sends a custom AI input, building context from the current session.
    pub fn send_ai_input_with_context(
        &mut self,
        build_input: impl FnOnce(Arc<[AIAgentContext]>) -> AIAgentInput,
        ctx: &mut ModelContext<Self>,
    ) {
        let context = input_context_for_request(
            false,
            self.context_model.as_ref(ctx),
            self.active_session.as_ref(ctx),
            None,
            vec![],
            ctx,
        );
        self.send_custom_ai_input_query(build_input(context), ctx);
    }

    /// Sends the result of a passive suggestion (accepted/rejected code diff or
    /// prompt) back to the model so it can continue with accurate context.
    pub fn send_passive_suggestion_result(
        &mut self,
        conversation_id: Option<AIConversationId>,
        suggestion: PassiveSuggestionResultType,
        trigger: Option<PassiveSuggestionTrigger>,
        ctx: &mut ModelContext<Self>,
    ) {
        let which_task = match conversation_id {
            Some(id) => {
                let Some(conversation) = BlocklistAIHistoryModel::as_ref(ctx).conversation(&id)
                else {
                    report_error!(
                        "[passive-suggestion-result] conversation not found",
                        extra: { "id" => ?id }
                    );
                    return;
                };
                WhichTask::Task {
                    conversation_id: conversation.id(),
                    task_id: conversation.get_root_task_id().clone(),
                }
            }
            None => WhichTask::NewConversation,
        };

        let context = input_context_for_request(
            false,
            self.context_model.as_ref(ctx),
            self.active_session.as_ref(ctx),
            conversation_id,
            vec![],
            ctx,
        );

        let participant_id = self.get_sharer_participant_id();
        let trigger_type = trigger.as_ref().map(PassiveSuggestionTriggerType::from);
        log::debug!(
            "[passive-suggestions] sending result: trigger={}, trigger_type={:?}",
            if trigger.is_some() { "Some" } else { "None" },
            trigger_type,
        );
        self.send_query(
            InputQuery {
                which_task,
                input_query: InputQueryType::AIInputType {
                    ai_input: AIAgentInput::PassiveSuggestionResult {
                        trigger,
                        suggestion,
                        context,
                    },
                },
                additional_attachments: HashMap::new(),
                queued_query_id: None,
            },
            EntrypointType::TriggerPassiveSuggestion {
                trigger: trigger_type,
            },
            participant_id,
            /*is_queued_prompt*/ false,
            ctx,
        );
    }

    /// Queues a passive suggestion result to be included with the next request
    /// for the given conversation. Use this instead of `send_passive_suggestion_result`
    /// when the result should not trigger an immediate server request (e.g. the user
    /// accepted a code diff without auto-resuming).
    pub fn queue_passive_suggestion_result(
        &mut self,
        conversation_id: AIConversationId,
        suggestion: PassiveSuggestionResultType,
        trigger: Option<PassiveSuggestionTrigger>,
    ) {
        self.pending_passive_suggestion_results
            .entry(conversation_id)
            .or_default()
            .push((suggestion, trigger));
    }

    fn send_follow_up_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .in_flight_response_streams
            .has_active_stream_for_conversation(conversation_id, ctx)
        {
            return;
        }

        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.mark_active_conversation_id(conversation_id, self.terminal_surface_id, ctx);
        });

        let finished_results = self.action_model.update(ctx, |action_model, _| {
            action_model.drain_finished_action_results(conversation_id)
        });
        if finished_results.is_empty() {
            return;
        }

        // Check whether any result will trigger a server-side subagent (e.g. CLI
        // subagent for LRC), or if one is already active. If so, we must not
        // piggyback orchestration events because the subagent cannot interpret
        // them and inserting events breaks tool_use/tool_result ordering.
        let will_trigger_server_subagent = finished_results
            .iter()
            .any(|r| r.result.triggers_server_subagent());
        let has_active_subagent = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|c| c.has_active_subagent());

        let context = input_context_for_request(
            false,
            self.context_model.as_ref(ctx),
            self.active_session.as_ref(ctx),
            Some(conversation_id),
            vec![],
            ctx,
        );
        let mut request_input = RequestInput::for_actions_results(
            finished_results,
            context,
            &self.active_session,
            self.get_current_response_initiator(),
            conversation_id,
            self.terminal_surface_id,
            ctx,
        );

        // Include any pending orchestration events in this follow-up rather
        // than waiting for a separate idle injection turn. Skip when a server
        // subagent is or will be active — events will be delivered via the idle
        // path once the subagent session ends.
        let mut has_piggybacked_events = false;
        if will_trigger_server_subagent || has_active_subagent {
            log::debug!(
                "Skipping event piggyback for conversation {conversation_id:?}: \
                 {}",
                if will_trigger_server_subagent {
                    "results will trigger a server-side subagent"
                } else {
                    "a subagent is currently active"
                }
            );
        } else if let Some((event_inputs, task_id)) = OrchestrationEventService::handle(ctx)
            .update(ctx, |svc, ctx| {
                svc.drain_events_for_request(conversation_id, ctx)
            })
        {
            has_piggybacked_events = true;
            request_input
                .input_messages
                .entry(task_id)
                .or_default()
                .extend(event_inputs);
        }

        let result = self.send_request_input(
            request_input,
            None,
            /*can_attempt_resume_on_error*/ true,
            /*is_queued_prompt*/ false,
            ctx,
        );

        if has_piggybacked_events && result.is_err() {
            OrchestrationEventService::handle(ctx).update(ctx, |svc, ctx| {
                svc.requeue_awaiting_events(conversation_id, ctx);
            });
        }

        self.pending_passive_follow_ups.remove(&conversation_id);
    }

    fn conversation_ready_for_pending_events(
        &self,
        conversation_id: AIConversationId,
        ctx: &ModelContext<Self>,
    ) -> bool {
        let owns = BlocklistAIHistoryModel::as_ref(ctx)
            .all_live_conversations_for_terminal_surface(self.terminal_surface_id)
            .any(|conversation| conversation.id() == conversation_id);
        let has_active_stream = self
            .in_flight_response_streams
            .has_active_stream_for_conversation(conversation_id, ctx);
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            log::info!(
                "Pending events are not ready: conversation_id={conversation_id:?} reason=conversation_missing owns_conversation={owns} has_active_stream={has_active_stream}"
            );
            return false;
        };
        // WaitingForEvents is treated as Success here: pending events
        // drain via the next outbound request and the server-side
        // supersede emits the resume signal.
        let is_ready_status = matches!(
            conversation.status(),
            ConversationStatus::Success | ConversationStatus::WaitingForEvents,
        );
        if !owns || has_active_stream || !is_ready_status {
            log::info!(
                "Pending events are not ready: conversation_id={conversation_id:?} owns_conversation={owns} has_active_stream={has_active_stream} status={:?}",
                conversation.status()
            );
            return false;
        }

        true
    }

    #[cfg(target_family = "wasm")]
    fn maybe_prepare_local_claude_wake(
        &mut self,
        _conversation_id: AIConversationId,
        _trigger: LocalClaudeWakeTrigger,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        false
    }

    #[cfg(not(target_family = "wasm"))]
    fn maybe_prepare_local_claude_wake(
        &mut self,
        conversation_id: AIConversationId,
        trigger: LocalClaudeWakeTrigger,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self
            .pending_local_claude_wakes
            .contains_key(&conversation_id)
        {
            log::info!("Dormant Claude wake already pending: conversation_id={conversation_id:?}");
            return true;
        }
        if trigger.requires_pending_events() {
            let has_pending_events = OrchestrationEventService::handle(ctx)
                .update(ctx, |svc, _| svc.has_pending_events(conversation_id));
            if !has_pending_events {
                return false;
            }
        }

        if !self.conversation_ready_for_pending_events(conversation_id, ctx) {
            return false;
        }

        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let Some(conversation) = history_model.conversation(&conversation_id).cloned() else {
            log::info!(
                "Skipping dormant Claude wake preparation: conversation_id={conversation_id:?} reason=conversation_missing"
            );
            return false;
        };
        let parent_conversation = conversation
            .parent_conversation_id()
            .and_then(|parent_conversation_id| history_model.conversation(&parent_conversation_id))
            .cloned();
        let working_dir = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned()
            .map(PathBuf::from);
        let task_id = conversation.task_id();
        let wake_message_for_prepare = match &trigger {
            LocalClaudeWakeTrigger::PendingEvents => None,
            LocalClaudeWakeTrigger::WakeOnlyStream { wake_message } => Some(wake_message.clone()),
        };
        let trigger_for_callback = trigger.clone();

        let server_api = ServerApiProvider::as_ref(ctx).get();
        let handle = ctx.spawn(
            async move {
                log::info!(
                    "Preparing dormant Claude wake command: conversation_id={conversation_id:?} task_id={task_id:?}"
                );
                ClaudeHarness::wake_dormant_session(
                    server_api.clone(),
                    conversation,
                    parent_conversation,
                    working_dir,
                    wake_message_for_prepare,
                )
                .await
            },
            move |me, result, ctx| {
                me.pending_local_claude_wakes.remove(&conversation_id);
                match result {
                    Ok(Some(command)) => {
                        if let LocalClaudeWakeTrigger::WakeOnlyStream { wake_message } =
                            &trigger_for_callback
                        {
                            OrchestrationEventStreamer::handle(ctx).update(
                                ctx,
                                |streamer, ctx| {
                                    streamer.persist_dormant_claude_wake_cursor(
                                        conversation_id,
                                        wake_message,
                                        ctx,
                                    );
                                },
                            );
                        }
                        log::info!(
                            "Executing dormant Claude wake command: conversation_id={conversation_id:?} task_id={task_id:?}"
                        );
                        BlocklistAIHistoryModel::handle(ctx).update(
                            ctx,
                            |history_model, ctx| {
                                history_model.update_conversation_status(
                                    me.terminal_surface_id,
                                    conversation_id,
                                    ConversationStatus::InProgress,
                                    ctx,
                                );
                            },
                        );
                        ctx.emit(BlocklistAIControllerEvent::ExecuteLocalHarnessCommand {
                            command,
                        });
                    }
                    Ok(None) => {
                        match &trigger_for_callback {
                            LocalClaudeWakeTrigger::PendingEvents => {
                                log::info!(
                                    "Falling back to generic pending-event injection after dormant Claude wake eligibility check: conversation_id={conversation_id:?} task_id={task_id:?}"
                                );
                                me.inject_pending_events_for_request(conversation_id, ctx);
                            }
                            LocalClaudeWakeTrigger::WakeOnlyStream { wake_message } => {
                                log::info!(
                                    "Retrying wake-only dormant Claude eligibility check: conversation_id={conversation_id:?} task_id={task_id:?}"
                                );
                                me.schedule_dormant_claude_wake_ready_retry(
                                    conversation_id,
                                    wake_message.clone(),
                                    ctx,
                                );
                            }
                        }
                    }
                    Err(err) => {
                        log::warn!(
                            "Failed to prepare dormant Claude wake command for {conversation_id:?} task_id={task_id:?}: {err:#}"
                        );
                        match &trigger_for_callback {
                            LocalClaudeWakeTrigger::PendingEvents => {
                                me.schedule_pending_events_ready_retry(conversation_id, ctx);
                            }
                            LocalClaudeWakeTrigger::WakeOnlyStream { wake_message } => {
                                me.schedule_dormant_claude_wake_ready_retry(
                                    conversation_id,
                                    wake_message.clone(),
                                    ctx,
                                );
                            }
                        }
                    }
                }
            },
        );
        self.pending_local_claude_wakes
            .insert(conversation_id, handle);
        true
    }

    #[cfg(not(target_family = "wasm"))]
    fn schedule_pending_events_ready_retry(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn(
            async move { Timer::after(Duration::from_secs(2)).await },
            move |me, _, ctx| {
                me.handle_pending_events_ready(conversation_id, ctx);
            },
        );
    }

    #[cfg(not(target_family = "wasm"))]
    fn schedule_dormant_claude_wake_ready_retry(
        &mut self,
        conversation_id: AIConversationId,
        wake_message: AgentMessageEventMetadata,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn(
            async move { Timer::after(Duration::from_secs(2)).await },
            move |me, _, ctx| {
                me.handle_dormant_claude_wake_ready(conversation_id, wake_message.clone(), ctx);
            },
        );
    }

    fn inject_pending_events_for_request(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.conversation_ready_for_pending_events(conversation_id, ctx) {
            return;
        }

        let Some((inputs, task_id)) = OrchestrationEventService::handle(ctx)
            .update(ctx, |svc, ctx| {
                svc.drain_events_for_request(conversation_id, ctx)
            })
        else {
            return;
        };

        // The resume request supersedes any in-flight wait_for_events.
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.cancel_wait_for_events_for_conversation(conversation_id, ctx);
        });

        if self
            .send_request_input(
                RequestInput::for_task(
                    inputs,
                    task_id,
                    &self.active_session,
                    self.get_current_response_initiator(),
                    conversation_id,
                    self.terminal_surface_id,
                    ctx,
                ),
                None,
                /*can_attempt_resume_on_error*/ true,
                /*is_queued_prompt*/ false,
                ctx,
            )
            .is_err()
        {
            // TODO: surface retry exhaustion. The existing requeue
            // re-emits `EventsReady` until `MAX_RETRY_ATTEMPTS` is hit,
            // after which events are dropped silently and the wait has
            // already been cancelled — the conversation can end up stuck
            // with no executor pending entry, no watchdog, and no
            // in-flight stream. Follow-up: park-on-exhaust the events
            // and transition the conversation to `Error` so the next
            // user resume can carry them along.
            OrchestrationEventService::handle(ctx).update(ctx, |svc, ctx| {
                svc.requeue_awaiting_events(conversation_id, ctx);
            });
        }
    }

    /// Handles the EventsReady signal. Checks readiness, drains
    /// pending events from the service, and injects them into the conversation.
    fn handle_pending_events_ready(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.conversation_ready_for_pending_events(conversation_id, ctx) {
            return;
        }

        if self.maybe_prepare_local_claude_wake(
            conversation_id,
            LocalClaudeWakeTrigger::PendingEvents,
            ctx,
        ) {
            return;
        }

        self.inject_pending_events_for_request(conversation_id, ctx);
    }

    fn handle_dormant_claude_wake_ready(
        &mut self,
        conversation_id: AIConversationId,
        wake_message: AgentMessageEventMetadata,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.maybe_prepare_local_claude_wake(
            conversation_id,
            LocalClaudeWakeTrigger::WakeOnlyStream { wake_message },
            ctx,
        ) {
            log::info!(
                "Ignoring dormant Claude wake-ready signal: conversation_id={conversation_id:?}"
            );
        }
    }

    pub fn resume_conversation(
        &mut self,
        conversation_id: AIConversationId,
        can_attempt_resume_on_error: bool,
        is_auto_resume_after_error: bool,
        additional_context: Vec<AIAgentContext>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            report_error!(
                "Tried to resume non-existent conversation",
                extra: { "conversation_id" => ?conversation_id }
            );
            return;
        };
        let task_id = {
            let terminal_model = self.terminal_model.lock();
            let active_block = terminal_model.block_list().active_block();
            if let Some(agent_interaction_metadata) = active_block
                .agent_interaction_metadata()
                .filter(|metadata| {
                    metadata.conversation_id() == &conversation_id && metadata.is_agent_in_control()
                })
            {
                agent_interaction_metadata
                    .subagent_task_id()
                    .cloned()
                    .unwrap_or_else(|| conversation.get_root_task_id().clone())
            } else {
                conversation.get_root_task_id().clone()
            }
        };

        let context = input_context_for_request(
            false,
            self.context_model.as_ref(ctx),
            self.active_session.as_ref(ctx),
            Some(conversation_id),
            additional_context,
            ctx,
        );

        let inputs = vec![AIAgentInput::ResumeConversation { context }];
        let metadata = if is_auto_resume_after_error {
            Some(RequestMetadata {
                is_autodetected_user_query: false,
                entrypoint: EntrypointType::ResumeConversation,
                is_auto_resume_after_error: true,
            })
        } else {
            None
        };
        let _ = self.send_request_input(
            RequestInput::for_task(
                inputs,
                task_id,
                &self.active_session,
                self.get_current_response_initiator(),
                conversation_id,
                self.terminal_surface_id,
                ctx,
            ),
            metadata,
            can_attempt_resume_on_error,
            /*is_queued_prompt*/ false,
            ctx,
        );
    }

    /// Schedules an auto-resume-after-error for the conversation once the network is online
    /// and the auto-handoff sleep modal is closed, so the resume doesn't race the user's
    /// enable/dismiss decision on wake.
    fn schedule_auto_resume_after_error(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let wait_for_online = NetworkStatus::as_ref(ctx).wait_until_online();
        let wait_for_modal_closed =
            OneTimeModalModel::as_ref(ctx).wait_until_auto_handoff_sleep_modal_closed();
        let wait = async move {
            wait_for_online.await;
            // Await the modal second: the future reads live modal state at
            // poll time, so a modal surfaced on wake (after connectivity
            // returns) is still observed.
            wait_for_modal_closed.await;
        };
        let handle = ctx.spawn(wait, move |me, _, ctx| {
            // Clean up the pending handle now that the resume is executing.
            me.pending_auto_resume_handles.remove(&conversation_id);
            me.resume_conversation(
                conversation_id,
                // Don't allow a second resume-on-error to prevent a persistent loop.
                /*can_attempt_resume_on_error*/
                false,
                /*is_auto_resume_after_error*/ true,
                vec![],
                ctx,
            );
        });
        self.pending_auto_resume_handles
            .insert(conversation_id, handle);
    }

    pub fn send_passive_code_diff_request(
        &mut self,
        query: String,
        block_id: &BlockId,
        file_contexts: Vec<FileContext>,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<(AIConversationId, ResponseStreamId)> {
        let mut input_context = file_contexts
            .into_iter()
            .map(AIAgentContext::File)
            .collect_vec();
        if let Some(block_context) = self
            .context_model
            .as_ref(ctx)
            .transform_block_to_context(block_id, false)
        {
            input_context.push(block_context);
        }

        let new_conversation = self.start_new_conversation_for_request(ctx);
        self.send_request_input(
            RequestInput::for_task(
                vec![AIAgentInput::AutoCodeDiffQuery {
                    query,
                    context: input_context.into(),
                }],
                new_conversation.get_root_task_id().clone(),
                &self.active_session,
                self.get_current_response_initiator(),
                new_conversation.id(),
                self.terminal_surface_id,
                ctx,
            ),
            Some(RequestMetadata {
                is_autodetected_user_query: false,
                entrypoint: EntrypointType::PromptSuggestion {
                    is_static: false,
                    is_coding: true,
                },
                is_auto_resume_after_error: false,
            }),
            /*can_attempt_resume_on_error*/ true,
            /*is_queued_prompt*/ false,
            ctx,
        )
    }

    /// Builds request params for an out-of-band passive suggestions request.
    ///
    /// This reads conversation state read-only and does NOT create exchanges,
    /// register response streams, or modify conversation status. The caller
    /// is responsible for spawning the API call and handling the response.
    ///
    /// If `followup_conversation_id` is provided, the conversation's task context
    /// and server token are included so the server can use prior context.
    /// Otherwise, a new conversation is created to anchor the request.
    /// Builds request params for an out-of-band passive suggestions request.
    ///
    /// This is read-only and does NOT create exchanges, register response
    /// streams, or modify conversation history. The caller is responsible for
    /// spawning the API call and handling the response.
    ///
    /// If `followup_conversation_id` is provided, the conversation's task
    /// context and server token are included so the server can use prior
    /// context. Otherwise a fresh, ephemeral conversation ID is generated
    /// without touching the history model.
    pub fn build_passive_suggestions_request_params(
        &self,
        followup_conversation_id: Option<AIConversationId>,
        trigger: PassiveSuggestionTrigger,
        supported_tools: Vec<ToolType>,
        ctx: &ModelContext<Self>,
    ) -> anyhow::Result<(AIConversationId, api::RequestParams)> {
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);

        // Resolve conversation state. For follow-ups we read from history;
        // for new triggers we generate a fresh ID without persisting anything.
        let (conversation_id, task_id, conversation_data) = if let Some(conversation_id) =
            followup_conversation_id
        {
            let Some(conversation) = history_model.conversation(&conversation_id) else {
                return Err(anyhow!(
                    "Tried to build passive suggestions request params for non-existent conversation with ID {conversation_id:?}"
                ));
            };
            let task_id = conversation.get_root_task_id().clone();
            let conversation_data = api::ConversationData {
                id: conversation_id,
                tasks: conversation.compute_active_tasks(),
                server_conversation_token: conversation.server_conversation_token().cloned(),
                forked_from_conversation_token: conversation
                    .forked_from_server_conversation_token()
                    .cloned(),
                // Do not tie passive suggestion requests to the cloud agent task, since they are
                // separate, read-only requests.
                ambient_agent_task_id: None,
                existing_suggestions: None,
            };
            (conversation_id, task_id, conversation_data)
        } else if !matches!(
            trigger,
            PassiveSuggestionTrigger::AgentResponseCompleted { .. }
        ) {
            // Generate a fresh, ephemeral conversation ID without mutating history.
            let conversation_id = AIConversationId::new();
            let task_id = TaskId::new(uuid::Uuid::new_v4().to_string());
            let conversation_data = api::ConversationData {
                id: conversation_id,
                tasks: vec![],
                server_conversation_token: None,
                forked_from_conversation_token: None,
                // Do not tie passive suggestion requests to the cloud agent task, since they are
                // separate, read-only requests.
                ambient_agent_task_id: None,
                existing_suggestions: None,
            };
            (conversation_id, task_id, conversation_data)
        } else {
            return Err(anyhow!(
                "Tried to use agent response completed trigger to generate passive suggestions without a conversation ID"
            ));
        };

        let inputs = vec![AIAgentInput::TriggerPassiveSuggestion {
            context: input_context_for_request(
                false,
                self.context_model.as_ref(ctx),
                self.active_session.as_ref(ctx),
                Some(conversation_id),
                vec![],
                ctx,
            ),
            attachments: vec![],
            trigger: trigger.clone(),
        }];

        let request_input = RequestInput::for_task(
            inputs,
            task_id,
            &self.active_session,
            self.get_current_response_initiator(),
            conversation_id,
            self.terminal_surface_id,
            ctx,
        )
        .with_supported_tools(supported_tools);

        let metadata = Some(RequestMetadata {
            is_autodetected_user_query: false,
            entrypoint: EntrypointType::TriggerPassiveSuggestion {
                trigger: Some((&trigger).into()),
            },
            is_auto_resume_after_error: false,
        });

        let request_params = api::RequestParams::new(
            Some(self.terminal_surface_id),
            SessionContext::from_session(self.active_session.as_ref(ctx), ctx),
            &request_input,
            conversation_data,
            metadata,
            ctx,
        );

        Ok((conversation_id, request_params))
    }

    pub fn send_unit_test_suggestions_request(
        &mut self,
        block_output: String,
        trigger: PassiveSuggestionTrigger,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<(AIConversationId, ResponseStreamId)> {
        let attachments = vec![AIAgentAttachment::PlainText(block_output.to_string())];
        let trigger_type = (&trigger).into();
        let inputs = vec![AIAgentInput::TriggerPassiveSuggestion {
            context: input_context_for_request(
                false,
                self.context_model.as_ref(ctx),
                self.active_session.as_ref(ctx),
                None,
                vec![],
                ctx,
            ),
            attachments,
            trigger,
        }];

        let new_conversation = self.start_new_conversation_for_request(ctx);
        self.send_request_input(
            RequestInput::for_task(
                inputs,
                new_conversation.get_root_task_id().clone(),
                &self.active_session,
                self.get_current_response_initiator(),
                new_conversation.id(),
                self.terminal_surface_id,
                ctx,
            ),
            Some(RequestMetadata {
                is_autodetected_user_query: false,
                entrypoint: EntrypointType::TriggerPassiveSuggestion {
                    trigger: Some(trigger_type),
                },
                is_auto_resume_after_error: false,
            }),
            /*can_attempt_resume_on_error*/ true,
            /*is_queued_prompt*/ false,
            ctx,
        )
    }

    /// Set the ID of the ambient agent task which owns this controller and its backing session.
    pub fn set_ambient_agent_task_id(
        &mut self,
        id: Option<AmbientAgentTaskId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.ambient_agent_task_id = id;
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.set_ambient_agent_task_id(id, ctx);
        });
    }

    #[cfg(test)]
    pub fn get_ambient_agent_task_id(&self) -> Option<AmbientAgentTaskId> {
        self.ambient_agent_task_id
    }

    /// Set the per-session directory for downloading file attachments.
    pub fn set_attachments_download_dir(&mut self, dir: std::path::PathBuf) {
        self.attachments_download_dir = Some(dir);
    }

    fn start_new_conversation_for_request<'a>(
        &self,
        ctx: &'a mut ModelContext<Self>,
    ) -> &'a AIConversation {
        let is_autoexecute_override = self
            .context_model
            .as_ref(ctx)
            .pending_query_autoexecute_override(ctx)
            .is_autoexecute_any_action();
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let id = history_model.update(ctx, |history_model, ctx| {
            // We don't mark passive conversations as "the active conversation" (at least when they first appear).
            history_model.start_new_conversation(
                self.terminal_surface_id,
                is_autoexecute_override,
                false,
                false,
                ctx,
            )
        });
        history_model
            .as_ref(ctx)
            .conversation(&id)
            .expect("Conversation exists- was just created.")
    }

    /// Attempts to send a request to the AI model API. Adds context to the input if it
    /// contains a user query. Returns `Err` if the AI input was not able to be sent due to an
    /// existing in-flight request. Emits an event containing a receiver for the AI's output.
    /// If conversation_id is Some, we follow up in that conversation.
    /// If it's None or we can't find a conversation with that ID, we start a new one.
    /// Returns the conversation ID of affected conversation and response stream ID.
    ///
    ///  This function does not handle cancelling any in flight requests (and sending them back as
    /// input) for an existing conversation. Consider calling [`Self::send_custom_ai_input_query`] if
    /// you're trying to send a query with a custom [`AIAgentInput`] type where you'd like the "normal"
    /// flow that handles existing conversations properly.
    fn send_request_input(
        &mut self,
        request_input: RequestInput,
        query_metadata: Option<RequestMetadata>,
        can_attempt_resume_on_error: bool,
        is_queued_prompt: bool,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<(AIConversationId, ResponseStreamId)> {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let (
            conversation_id,
            conversation_server_token,
            conversation_forked_from_token,
            active_tasks,
            parent_agent_id,
            agent_name,
        ) = {
            let Some(conversation) = history_model
                .as_ref(ctx)
                .conversation(&request_input.conversation_id)
            else {
                return Err(anyhow!(
                    "Tried to send request for non-existent conversation with ID {:?}",
                    request_input.conversation_id
                ));
            };

            let active_tasks = conversation.compute_active_tasks();

            (
                conversation.id(),
                conversation.server_conversation_token().cloned(),
                conversation
                    .forked_from_server_conversation_token()
                    .cloned(),
                active_tasks,
                conversation.parent_agent_id().map(str::to_string),
                conversation.agent_name().map(str::to_string),
            )
        };

        // Cancel any pending auto-resume for this conversation, since the user is sending a new
        // request.
        if let Some(handle) = self
            .pending_auto_resume_handles
            .remove(&request_input.conversation_id)
        {
            handle.abort();
        }

        // Passive background requests never auto-resume: a resume would issue a fresh
        // turn on a conversation the user never sees.
        let is_passive_request = request_input
            .all_inputs()
            .any(|input| input.is_passive_request());
        let can_attempt_resume_on_error = can_attempt_resume_on_error && !is_passive_request;

        // Make sure there's no existing response stream for the conversation. If
        // there is, something has gone wrong.
        if self
            .in_flight_response_streams
            .has_active_stream_for_conversation(conversation_id, ctx)
        {
            send_telemetry_from_ctx!(
                TelemetryEvent::AIInputNotSent {
                    entrypoint: query_metadata.map(|metadata| metadata.entrypoint),
                    inputs: request_input
                        .all_inputs()
                        .cloned()
                        .map(|input| input.into())
                        .collect(),
                    active_server_conversation_id: conversation_server_token.clone(),
                    active_client_conversation_id: Some(conversation_id),
                },
                ctx
            );
            const AI_INPUT_NOT_SENT_ERROR_STR: &str =
                "Not sending AI input because there is an in-flight request";
            safe_assert!(false, "{}", AI_INPUT_NOT_SENT_ERROR_STR);
            return Err(anyhow::anyhow!(AI_INPUT_NOT_SENT_ERROR_STR));
        }

        let conversation_data = api::ConversationData {
            id: conversation_id,
            tasks: active_tasks,
            server_conversation_token: conversation_server_token,
            forked_from_conversation_token: conversation_forked_from_token,
            ambient_agent_task_id: self.ambient_agent_task_id,
            existing_suggestions: history_model
                .as_ref(ctx)
                .existing_suggestions_for_conversation(conversation_id)
                .cloned(),
        };

        // Log an error if tool call results do not have corresponding tool calls in task context
        validate_tool_call_results(
            request_input.all_inputs(),
            &conversation_data.tasks,
            &conversation_data.server_conversation_token,
        );

        // Safety net: re-arm the Gemini Enterprise (GEAP) credential refresh
        // chain if it was parked or never armed, so upcoming requests can
        // authenticate. The connected Grok subscription's request-time OAuth
        // refresh is handled in the response stream's send path
        // (`ResponseStream::spawn_request`).
        #[cfg(not(target_family = "wasm"))]
        {
            use ::ai::api_keys::ApiKeyManager;

            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                crate::ai::geap_credentials::refresh_geap_credentials_if_needed(manager, ctx);
            });
        }

        let mut request_params = api::RequestParams::new(
            Some(self.terminal_surface_id),
            SessionContext::from_session(self.active_session.as_ref(ctx), ctx),
            &request_input,
            conversation_data.clone(),
            query_metadata,
            ctx,
        );
        request_params.parent_agent_id = parent_agent_id;
        request_params.agent_name = agent_name;

        let server_conversation_token_for_identifiers =
            conversation_data.server_conversation_token.clone();

        let response_stream = ctx.add_model(|ctx| {
            // Create AIIdentifiers for the response stream
            let ai_identifiers = AIIdentifiers {
                server_output_id: None, // Will be populated by the successful response
                server_conversation_id: server_conversation_token_for_identifiers.map(Into::into),
                client_conversation_id: Some(conversation_data.id),
                client_exchange_id: None,
                model_id: Some(request_params.model.clone()),
            };
            ResponseStream::new(
                request_params.clone(),
                ai_identifiers,
                can_attempt_resume_on_error,
                ctx,
            )
        });
        let response_stream_id = response_stream.as_ref(ctx).id().clone();
        let response_stream_clone = response_stream.clone();
        let input_contains_user_query = request_input
            .all_inputs()
            .any(|input| input.is_user_query());
        ctx.subscribe_to_model(&response_stream, move |me, _, event, ctx| {
            me.handle_response_stream_event(
                input_contains_user_query,
                event,
                &response_stream_clone,
                ctx,
            );
        });

        for input in request_input.all_inputs() {
            if let AIAgentInput::UserQuery {
                referenced_attachments,
                ..
            } = input
            {
                self.maybe_populate_plans_for_ai_document_model(
                    referenced_attachments,
                    conversation_data.id,
                    ctx,
                );
            }
        }

        history_model.update(ctx, |history_model, ctx| {
            match history_model.update_conversation_for_new_request_input(
                request_input,
                response_stream_id.clone(),
                self.terminal_surface_id,
                ctx,
            ) {
                Ok(_) => {
                    history_model.update_conversation_status(
                        self.terminal_surface_id,
                        conversation_data.id,
                        ConversationStatus::InProgress,
                        ctx,
                    );
                }
                Err(e) => {
                    log::warn!("Failed to push new exchange to AI conversation: {e:?}");
                }
            }
        });

        self.in_flight_response_streams.register_new_stream(
            response_stream_id.clone(),
            conversation_data.id,
            response_stream,
            CancellationReason::FollowUpSubmitted {
                is_for_same_conversation: true,
            },
            ctx,
        );

        // Skip the context reset for a fired queued-prompt row (`is_queued_prompt`): its
        // attachments came from the row, not the live staging, so the live `pending_attachments`
        // belong to the user's next prompt and must be preserved.
        if input_contains_user_query && !is_queued_prompt {
            // Get the pending document ID before clearing context
            let pending_document_id = self.context_model.as_ref(ctx).pending_document_id();

            // Reset the context state to the default.
            self.context_model.update(ctx, |context_model, ctx| {
                context_model.reset_context_to_default(ctx);
            });

            // Update the document status to UpToDate after query submission
            if let Some(doc_id) = pending_document_id {
                AIDocumentModel::handle(ctx).update(ctx, |model, mctx| {
                    model.set_user_edit_status(&doc_id, AIDocumentUserEditStatus::UpToDate, mctx);
                });
            }
        }

        ctx.emit(BlocklistAIControllerEvent::SentRequest {
            contains_user_query: input_contains_user_query,
            is_queued_prompt,
            model_id: request_params.model.clone(),
            stream_id: response_stream_id.clone(),
        });
        if !is_passive_request {
            history_model.update(ctx, |history_model, ctx| {
                history_model.mark_active_conversation_id(
                    conversation_data.id,
                    self.terminal_surface_id,
                    ctx,
                );
            });
        }

        // Trigger a snapshot save to persist the agent view state when a user query is sent.
        // This ensures the agent view is restored if the app restarts.
        if input_contains_user_query {
            ctx.dispatch_global_action("workspace:save_app", ());
        }

        Ok((conversation_data.id, response_stream_id))
    }

    /// Cancels a pending AI request response stream, given the exchange ID, if it exists.
    /// Returns true if a pending stream was found and canceled, false otherwise.
    pub fn try_cancel_pending_response_stream(
        &mut self,
        stream_id: &ResponseStreamId,
        reason: CancellationReason,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        self.in_flight_response_streams
            .try_cancel_stream(stream_id, reason, ctx)
    }

    pub fn has_active_stream_for_conversation(
        &self,
        conversation_id: AIConversationId,
        app: &AppContext,
    ) -> bool {
        self.in_flight_response_streams
            .has_active_stream_for_conversation(conversation_id, app)
    }

    #[cfg(test)]
    pub fn register_mock_stream_for_test(
        &mut self,
        stream_id: ResponseStreamId,
        conversation_id: AIConversationId,
        stream: ModelHandle<ResponseStream>,
        ctx: &mut ModelContext<Self>,
    ) {
        let stream_clone = stream.clone();
        ctx.subscribe_to_model(&stream, move |me, _, event, ctx| {
            me.handle_response_stream_event(false, event, &stream_clone, ctx);
        });
        self.in_flight_response_streams.register_new_stream(
            stream_id,
            conversation_id,
            stream,
            CancellationReason::ManuallyCancelled,
            ctx,
        );
    }

    /// Cancels 'progress' for the active conversation if there is one:
    ///  * If there is an in-flight request, cancels it.
    ///  * Else, if the request finished, but actions from the response are pending or mid-execution, cancels all of them.
    pub fn cancel_conversation_progress(
        &mut self,
        conversation_id: AIConversationId,
        reason: CancellationReason,
        ctx: &mut ModelContext<Self>,
    ) {
        // Restore the user's keyboard focus if a background computer-use session is still active
        // for this conversation. ctrl-c / stop / pane-close all funnel through here, and on
        // cancellation the computer-use subagent never produces a normal SubagentResult, so the
        // normal-completion teardown in `Conversation` is skipped. Scoped to this conversation so a
        // concurrent background session in another conversation is left intact; idempotent and a
        // no-op when this conversation has no active background session.
        computer_use::end_background_session(&conversation_id.to_string());

        // Cancel any pending auto-resume for this conversation.
        if let Some(handle) = self.pending_auto_resume_handles.remove(&conversation_id) {
            handle.abort();
        }

        // Discard any queued passive suggestion results for this conversation.
        self.pending_passive_suggestion_results
            .remove(&conversation_id);

        // Remove any locked pending-LRC queries so they don't linger after cancellation.
        QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
            model.remove_pending_lrc_rows(conversation_id, ctx);
        });

        if !self
            .in_flight_response_streams
            .try_cancel_streams_for_conversation(conversation_id, reason, ctx)
        {
            // No active stream whose cancellation would mark the conversation `Cancelled`.
            // A parked auto-resume was aborted above; nothing else will move the
            // conversation out of TransientError, so surface the cancellation directly.
            //
            // TODO(REMOTE-1950): Track the parked auto-resume as a first-class cancelable so its
            // cancellation flows through the same `AfterStreamFinished` path, dropping this special case.
            if matches!(
                reason.conversation_outcome(),
                CancellationOutcome::Cancelled
            ) {
                let history_model = BlocklistAIHistoryModel::handle(ctx);
                let is_recovering = history_model
                    .as_ref(ctx)
                    .conversation(&conversation_id)
                    .is_some_and(|conversation| conversation.status().is_transient_error());
                if is_recovering {
                    history_model.update(ctx, |history_model, ctx| {
                        history_model.update_conversation_status(
                            self.terminal_surface_id,
                            conversation_id,
                            ConversationStatus::Cancelled,
                            ctx,
                        );
                    });
                }
            }

            // Otherwise, cancel pending actions and update the input state.
            self.action_model.update(ctx, |action_model, ctx| {
                action_model.cancel_all_pending_actions(conversation_id, Some(reason), ctx);
            });
            self.set_input_mode_for_cancellation(ctx);
        }
    }

    /// Finalizes a conversation as a terminal failure because an agent-issued
    /// command caused the shell process to exit (e.g. it ran `exit`, or ran a
    /// failing command after enabling `set -e`).
    ///
    /// Invoked from the terminal view's shell-exit handler before the pane is
    /// torn down. The conversation is moved into a terminal `Error` state with a
    /// shell-exit message so that the Oz run reports `FAILED` (with an
    /// explanation) instead of "Cancelled by user", and so the subsequent
    /// pane-close cancellation — which is guarded by `is_in_progress` — becomes a
    /// no-op and cannot overwrite the failure.
    pub fn fail_conversation_due_to_shell_exit(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_surface_id = self.terminal_surface_id;
        let history_model = BlocklistAIHistoryModel::handle(ctx);

        // Only act on conversations that are still running. A finished
        // conversation (e.g. the agent already completed) must not be
        // retroactively marked as failed.
        let is_in_progress = history_model
            .as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| conversation.status().is_in_progress());
        if !is_in_progress {
            return;
        }

        // Finish any in-flight response stream(s) with the shell-exit error. This
        // marks the streaming exchange(s) as errored, sets the conversation to
        // `Error` (with a message), and renders the failure inline. We then cancel
        // the underlying request so it stops streaming; the cancellation does not
        // overwrite the status because `mark_request_cancelled` ignores the
        // `AgentExitedShell` reason.
        let stream_ids = self
            .in_flight_response_streams
            .stream_ids_for_conversation(conversation_id, ctx);
        let had_in_flight_stream = !stream_ids.is_empty();
        for stream_id in &stream_ids {
            history_model.update(ctx, |history_model, ctx| {
                history_model.mark_response_stream_completed_with_error(
                    RenderableAIError::AgentExitedShell,
                    /* recovery_pending */ false,
                    stream_id,
                    conversation_id,
                    terminal_surface_id,
                    ctx,
                );
            });
            self.try_cancel_pending_response_stream(
                stream_id,
                CancellationReason::AgentExitedShell,
                ctx,
            );
        }

        // Stop any pending or mid-execution actions so a queued action result
        // can't subsequently move the conversation back to Success/Cancelled.
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.cancel_all_pending_actions(
                conversation_id,
                Some(CancellationReason::AgentExitedShell),
                ctx,
            );
        });

        // If there was no in-flight stream to attach the error to (e.g. the agent
        // ran `exit` and no follow-up request was in flight), set the terminal
        // `Error` status directly, recording the structured shell-exit error so
        // status consumers (Oz task sync and the ambient SDK driver) classify it
        // as FAILED rather than a generic ERROR.
        if !had_in_flight_stream {
            history_model.update(ctx, |history_model, ctx| {
                history_model.update_conversation_status_with_error(
                    terminal_surface_id,
                    conversation_id,
                    ConversationStatus::Error,
                    Some(RenderableAIError::AgentExitedShell),
                    ctx,
                );
            });
        }
    }

    /// Clears finished action results for a conversation. Used when reverting.
    pub fn clear_finished_action_results(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.action_model.update(ctx, |action_model, _| {
            action_model.clear_finished_action_results(conversation_id);
        });
    }

    /// Cancels the in-flight request for the given conversation, if there is one.
    ///
    /// Returns `true` if a request was actually cancelled.
    pub fn cancel_request(
        &mut self,
        response_stream_id: &ResponseStreamId,
        reason: CancellationReason,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        self.in_flight_response_streams
            .try_cancel_stream(response_stream_id, reason, ctx)
    }

    fn handle_response_stream_event(
        &mut self,
        did_input_contain_user_query: bool,
        event: &ResponseStreamEvent,
        response_stream: &ModelHandle<ResponseStream>,
        ctx: &mut ModelContext<Self>,
    ) {
        let stream_id = response_stream.as_ref(ctx).id().clone();

        match event {
            ResponseStreamEvent::ReceivedEvent(event) => {
                // Dynamic lookup handles conversation splits mid-stream.
                let Some(conversation_id) = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation_for_response_stream(&stream_id)
                else {
                    log::warn!("Could not find conversation for response stream: {stream_id:?}");
                    return;
                };
                let Some(event) = event.consume() else {
                    debug_assert!(
                        false,
                        "This model should only have a single subscriber that takes ownership over the event."
                    );
                    return;
                };
                let history_model = BlocklistAIHistoryModel::handle(ctx);
                match event {
                    Ok(event) => {
                        // If this controller is part of a shared session, forward the entire response event to viewers first.
                        if FeatureFlag::AgentSharedSessions.is_enabled() {
                            let mut model = self.terminal_model.lock();
                            if model.shared_session_status().is_sharer() {
                                // Get the participant who initiated this response, falling back to the sharer if needed.
                                let participant_id = self
                                    .get_current_response_initiator()
                                    .or_else(|| self.get_sharer_participant_id());

                                // For forked conversations (e.g. when loading from cloud), include
                                // the original conversation token so viewers can link the new
                                // server-assigned token to their existing conversation.
                                //
                                // This token is cleared after the first Init event (see below),
                                // so it's only sent once per forked conversation.
                                let forked_from_token = history_model
                                    .as_ref(ctx)
                                    .conversation(&conversation_id)
                                    .and_then(|conv| {
                                        conv.forked_from_server_conversation_token()
                                            .map(|t| t.as_str().to_string())
                                    });

                                model.send_agent_response_for_shared_session(
                                    &event,
                                    participant_id,
                                    forked_from_token,
                                );
                            }
                        }
                        let Some(event) = event.r#type else {
                            return;
                        };
                        match event {
                            warp_multi_agent_api::response_event::Type::Init(init_event) => {
                                history_model.update(ctx, |history_model, ctx| {
                                    history_model.initialize_output_for_response_stream(
                                        &stream_id,
                                        conversation_id,
                                        self.terminal_surface_id,
                                        init_event,
                                        ctx,
                                    );

                                    // Clear the forked_from token after the first Init event.
                                    // For forked conversations, we only need to send this once so
                                    // viewers can update their conversation's server token. After
                                    // that, the viewer's conversation uses the new token directly.
                                    if let Some(conversation) =
                                        history_model.conversation_mut(&conversation_id)
                                    {
                                        conversation.clear_forked_from_server_conversation_token();
                                    }
                                });
                            }
                            warp_multi_agent_api::response_event::Type::Finished(
                                finished_event,
                            ) => {
                                self.handle_response_stream_finished(
                                    &stream_id,
                                    finished_event,
                                    conversation_id,
                                    did_input_contain_user_query,
                                    ctx,
                                );
                            }
                            warp_multi_agent_api::response_event::Type::ClientActions(actions) => {
                                let client_actions = actions.actions;
                                let skill_path_origin = SessionContext::from_session(
                                    self.active_session.as_ref(ctx),
                                    ctx,
                                )
                                .skill_path_origin();
                                let apply_result =
                                    history_model.update(ctx, |history_model, ctx| {
                                        history_model.apply_client_actions(
                                            &stream_id,
                                            client_actions,
                                            conversation_id,
                                            self.terminal_surface_id,
                                            &skill_path_origin,
                                            ctx,
                                        )
                                    });
                                if let Err(e) = apply_result {
                                    report_error!(
                                        anyhow::Error::new(e).context(
                                            "Failed to apply client actions to conversation"
                                        )
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if matches!(e.as_ref(), AIApiError::QuotaLimit { .. }) {
                            // If the error is a quota limit, we want to refresh workspace metadata
                            // So the current state of AI overages is immediately up to date.
                            TeamUpdateManager::handle(ctx).update(
                                ctx,
                                |team_update_manager, ctx| {
                                    std::mem::drop(
                                        team_update_manager.refresh_workspace_metadata(ctx),
                                    );
                                },
                            );
                            AIRequestUsageModel::handle(ctx).update(ctx, |model, ctx| {
                                model.enable_buy_credits_banner(ctx);
                            });
                        }

                        // A resume scheduled for this failure keeps the conversation in
                        // the non-terminal TransientError status instead of Error.
                        let recovery_pending = response_stream
                            .as_ref(ctx)
                            .should_resume_conversation_after_stream_finished();
                        let mut renderable_error: RenderableAIError = (&e).into();
                        if let RenderableAIError::Other {
                            will_attempt_resume,
                            waiting_for_network,
                            ..
                        }
                        | RenderableAIError::TransientNetworkError {
                            will_attempt_resume,
                            waiting_for_network,
                            ..
                        } = &mut renderable_error
                        {
                            // Rendering-only hints; state machine consumers key off the
                            // TransientError conversation status instead.
                            *will_attempt_resume |= recovery_pending;
                            if recovery_pending {
                                let network_status = NetworkStatus::as_ref(ctx);
                                *waiting_for_network = !network_status.is_online();
                            }
                        }

                        history_model.update(ctx, |history_model, ctx| {
                            history_model.mark_response_stream_completed_with_error(
                                renderable_error,
                                recovery_pending,
                                &stream_id,
                                conversation_id,
                                self.terminal_surface_id,
                                ctx,
                            );
                        });
                    }
                }
            }
            ResponseStreamEvent::WaitingForNetwork { waiting } => {
                let Some(conversation_id) = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation_for_response_stream(&stream_id)
                else {
                    log::warn!("Could not find conversation for response stream: {stream_id:?}");
                    return;
                };
                // Mirror the parked-retry state on the conversation: TransientError while
                // waiting for connectivity, back to InProgress when the retry fires.
                // This event is only emitted after a recoverable request failure parks a
                // retry while offline (see `defer_retry_until_online`), so treating
                // `waiting` as a transient-error state is always correct here.
                let status = if *waiting {
                    ConversationStatus::TransientError
                } else {
                    ConversationStatus::InProgress
                };
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.update_conversation_status(
                        self.terminal_surface_id,
                        conversation_id,
                        status,
                        ctx,
                    );
                });
            }
            ResponseStreamEvent::AfterStreamFinished { cancellation } => {
                // Cancellations provide conversation_id (survives truncation); otherwise use dynamic lookup.
                let conversation_id = match &cancellation {
                    Some(stream_cancellation) => stream_cancellation.conversation_id,
                    None => {
                        let Some(id) = BlocklistAIHistoryModel::as_ref(ctx)
                            .conversation_for_response_stream(&stream_id)
                        else {
                            log::warn!(
                                "Could not find conversation for response stream: {stream_id:?}"
                            );
                            return;
                        };
                        id
                    }
                };

                let history_model = BlocklistAIHistoryModel::handle(ctx);
                let Some(conversation) = history_model.as_ref(ctx).conversation(&conversation_id)
                else {
                    log::warn!("Conversation not found.");
                    return;
                };
                let new_exchange_ids = conversation.new_exchange_ids_for_response(&stream_id);
                let mut was_passive_request = false;
                let mut is_any_exchange_unfinished = false;
                let mut actions_to_queue = vec![];

                for new_exchange_id in new_exchange_ids {
                    let Some(exchange) = conversation.exchange_with_id(new_exchange_id) else {
                        log::warn!("Exchange not found.");
                        return;
                    };
                    was_passive_request |= exchange.has_passive_request();
                    is_any_exchange_unfinished |= !exchange.output_status.is_finished();

                    if let AIAgentOutputStatus::Finished {
                        finished_output: FinishedAIAgentOutput::Success { output },
                        ..
                    } = &exchange.output_status
                    {
                        actions_to_queue.extend(output.get().actions().cloned());
                    }
                }

                if let Some(stream_cancellation) = &cancellation {
                    // If this is a shared session, send a synthetic StreamFinished event to notify viewers
                    // of any user-initiated cancellation. We skip internal cancellations that preserve
                    // the conversation's InProgress status, such as follow-ups and CLI subagent user
                    // takeover, because those do not end the conversation.
                    if FeatureFlag::AgentSharedSessions.is_enabled()
                        && !matches!(
                            stream_cancellation.reason.conversation_outcome(),
                            CancellationOutcome::KeepInProgress
                        )
                    {
                        // For any terminal status (not just canceled), we need to inform viewers the stream has stopped.
                        self.send_cancellation_to_viewers(ctx);
                    }

                    history_model.update(ctx, |history_model, ctx| {
                        history_model.mark_response_stream_cancelled(
                            &stream_id,
                            conversation_id,
                            self.terminal_surface_id,
                            stream_cancellation.reason,
                            ctx,
                        );
                    });

                    if !was_passive_request
                        && matches!(
                            stream_cancellation.reason.conversation_outcome(),
                            CancellationOutcome::Cancelled
                        )
                    {
                        self.set_input_mode_for_cancellation(ctx);
                    }
                } else if is_any_exchange_unfinished {
                    // Defensive: truncated streams are detected inside `ResponseStream`,
                    // so an unfinished exchange here means an unexpected completion path.
                    log::warn!(
                        "Response stream completed with an unfinished exchange and no error event."
                    );

                    history_model.update(ctx, |history_model, ctx| {
                        history_model.mark_response_stream_completed_with_error(
                            RenderableAIError::transient_network_error(
                                false,
                                false,
                                TransientNetworkErrorKind::UnfinishedExchange,
                            ),
                            /*recovery_pending*/ false,
                            &stream_id,
                            conversation_id,
                            self.terminal_surface_id,
                            ctx,
                        );
                    });
                } else if !actions_to_queue.is_empty() {
                    self.action_model.update(ctx, |action_model, ctx| {
                        action_model.queue_actions(actions_to_queue, conversation_id, ctx);
                    });
                }

                // Cancelled streams will handle pending_response_stream updates synchronously.
                if cancellation.is_none() {
                    self.in_flight_response_streams.cleanup_stream(&stream_id);

                    // Now that the stream is cleaned up, re-check for pending
                    // orchestration events that couldn't be drained earlier.
                    self.handle_pending_events_ready(conversation_id, ctx);
                }

                // Before cleaning up the response stream, check if we should attempt to resume.
                if response_stream
                    .as_ref(ctx)
                    .should_resume_conversation_after_stream_finished()
                {
                    self.schedule_auto_resume_after_error(conversation_id, ctx);
                }

                // Clean up the response stream tracking entry now that the stream is complete.
                history_model.update(ctx, |history_model, _| {
                    if let Some(conversation) = history_model.conversation_mut(&conversation_id) {
                        conversation.cleanup_completed_response_stream(&stream_id);
                    }
                });
                ctx.unsubscribe_from_model(response_stream);

                if self.should_refresh_available_llms_on_stream_finish {
                    self.should_refresh_available_llms_on_stream_finish = false;
                    LLMPreferences::handle(ctx).update(ctx, |llm_preferences, ctx| {
                        llm_preferences.refresh_authed_models(ctx);
                    });
                }
                ctx.emit(BlocklistAIControllerEvent::FinishedReceivingOutput {
                    stream_id,
                    conversation_id,
                });
                AIRequestUsageModel::handle(ctx).update(ctx, |request_usage_model, ctx| {
                    request_usage_model.refresh_request_usage_async(ctx);
                });

                self.maybe_refresh_ai_overages(ctx);
            }
        }
    }

    /// Sets the terminal input state after an AI request is cancelled.
    /// From the user perspective, we downgrade the level of autonomy so:
    /// * Executing a task automatically -> interactive AI input
    /// * Interactive AI input -> interactive shell input
    fn set_input_mode_for_cancellation(&mut self, ctx: &mut ModelContext<Self>) {
        // If the request was cancelled, default to shell mode with autodetection
        // enabled.
        self.input_model.update(ctx, |input_model, ctx| {
            input_model.set_input_config_for_classic_mode(
                input_model
                    .input_config()
                    .with_shell_type()
                    .unlocked_if_autodetection_enabled(false, ctx),
                ctx,
            );
        });
    }

    /// Checks if we should refresh AI overage information after an AI request completes.
    /// This is used to ensure the UI matches the state of the workspace,
    /// especially because overages are not real-time communicated to clients.
    fn maybe_refresh_ai_overages(&mut self, ctx: &mut ModelContext<Self>) {
        let workspace = UserWorkspaces::as_ref(ctx).current_workspace();
        let Some(workspace) = workspace else {
            return;
        };

        // We want to minimize the number of times we ping our backend for updated usage information;
        // doing it after every AI query finishes would be very expensive.

        // If a user is below their personal limits, then we know that they won't eat into overages,
        // so we don't need to refresh.
        let has_no_requests_remaining = !AIRequestUsageModel::as_ref(ctx).has_requests_remaining();
        // If overages aren't enabled, we're not going to reap the benefit of refreshing at all anyway.
        let are_overages_enabled = workspace.are_overages_enabled();

        if are_overages_enabled && has_no_requests_remaining {
            // Give a one second delay to ensure that Stripe has been charged and the database is completely updated,
            // before syncing new AI overages data.
            ctx.spawn(
                async move { Timer::after(Duration::from_secs(1)).await },
                |_, _, ctx| {
                    UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                        user_workspaces.refresh_ai_overages(ctx);
                    });
                },
            );
        }
    }

    pub(super) fn handle_response_stream_finished(
        &mut self,
        stream_id: &ResponseStreamId,
        mut finished_event: warp_multi_agent_api::response_event::StreamFinished,
        conversation_id: AIConversationId,
        did_request_contain_user_query: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        history_model.update(ctx, |history_model, ctx| {
            // Update conversation cost and usage information before updating and
            // persisting the conversation.
            history_model.update_conversation_cost_and_usage_for_request(
                conversation_id,
                finished_event.request_cost.map(|cost| {
                    // Total credits charged for this request = inference (`exact`) + platform.
                    RequestCost::new(f64::from(cost.exact) + f64::from(cost.platform_credits))
                }),
                finished_event.token_usage,
                finished_event.conversation_usage_metadata.take(),
                did_request_contain_user_query,
                ctx,
            );
        });

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        match finished_event.reason {
            Some(warp_multi_agent_api::response_event::stream_finished::Reason::Done(_)) | None => {
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_successfully(
                        stream_id,
                        conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    );
                });
            }
            Some(warp_multi_agent_api::response_event::stream_finished::Reason::Other(_)) => {
                let error_message = "Response stream finished unexpectedly (with finish reason `Other`).";
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::Other {
                            error_message: error_message.to_owned(),
                            will_attempt_resume: false,
                            waiting_for_network: false,
                            is_user_error: false,
                        },
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    );
                });
            }
            Some(warp_multi_agent_api::response_event::stream_finished::Reason::ContextWindowExceeded(_)) => {
                let error_message = "Input exceeded context window limit.";
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::ContextWindowExceeded(error_message.to_owned()),
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    );
                });
            }
            Some(warp_multi_agent_api::response_event::stream_finished::Reason::QuotaLimit(_)) => {
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::QuotaLimit {
                            user_display_message: None,
                        },
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    );
                });
            }
            Some(warp_multi_agent_api::response_event::stream_finished::Reason::LlmUnavailable(_)) => {
                let error_message = "The LLM is currently unavailable.";
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::Other {
                            error_message: error_message.to_owned(),
                            will_attempt_resume: false,
                            waiting_for_network: false,
                            is_user_error: false,
                        },
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    );
                });
            }
            Some(warp_multi_agent_api::response_event::stream_finished::Reason::InvalidApiKey(details)) => {
                use warp_multi_agent_api::LlmProvider;
                let is_aws_bedrock = details
                    .provider
                    .try_into()
                    .ok()
                    .is_some_and(|p: LlmProvider| p == LlmProvider::AwsBedrock);

                let error = if is_aws_bedrock {
                    RenderableAIError::AwsBedrockCredentialsExpiredOrInvalid {
                        model_name: details.model_name,
                    }
                } else {
                    let provider = details.provider.try_into().ok().and_then(|p| match p {
                        LlmProvider::Google => Some("Google"),
                        LlmProvider::Anthropic => Some("Anthropic"),
                        LlmProvider::Openai => Some("OpenAI"),
                        LlmProvider::Xai => Some("xAI"),
                        LlmProvider::Openrouter => Some("OpenRouter"),
                        LlmProvider::AwsBedrock | LlmProvider::Unknown => None,
                    });
                    RenderableAIError::InvalidApiKey {
                        provider: provider.unwrap_or("Unknown").to_string(),
                        model_name: details.model_name,
                    }
                };

                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        error,
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    );
                });
            }
            Some(warp_multi_agent_api::response_event::stream_finished::Reason::InternalError(
                warp_multi_agent_api::response_event::stream_finished::InternalError{ message})) => {
                let error_message = format!(
                    "Response stream finished unexpectedly with internal error: {message}",
                );
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::Other {
                            error_message,
                            will_attempt_resume: false,
                            waiting_for_network: false,
                            is_user_error: false,
                        },
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    );
                });
            }
            Some(warp_multi_agent_api::response_event::stream_finished::Reason::MaxTokenLimit(_)) => {
                let error_message = "Input exceeded context window limit.";
                history_model.update(ctx, |history_model, ctx| {
                    history_model.mark_response_stream_completed_with_error(
                        RenderableAIError::ContextWindowExceeded(error_message.to_owned()),
                        /*recovery_pending*/ false,
                        stream_id,
                        conversation_id,
                        self.terminal_surface_id,
                        ctx,
                    );
                });
            }
        }

        if finished_event.should_refresh_model_config {
            LLMPreferences::handle(ctx).update(ctx, |llm_preferences, ctx| {
                llm_preferences.refresh_authed_models(ctx);
            });
        }
    }
}

impl Entity for BlocklistAIController {
    type Event = BlocklistAIControllerEvent;
}

#[derive(Clone)]
pub struct ClientIdentifiers {
    pub conversation_id: AIConversationId,
    pub client_exchange_id: AIAgentExchangeId,
    /// Not populated for restored AI blocks.
    pub response_stream_id: Option<ResponseStreamId>,
}

#[allow(clippy::too_many_arguments)]
fn input_for_query(
    query: String,
    task_id: &TaskId,
    conversation_id: AIConversationId,
    static_query_type: Option<StaticQueryType>,
    user_query_mode: UserQueryMode,
    running_command: Option<RunningCommand>,
    additional_attachments: HashMap<String, AIAgentAttachment>,
    prompt_attachments: Vec<PendingAttachment>,
    context_model: &BlocklistAIContextModel,
    active_session: &ActiveSession,
    app: &AppContext,
) -> AIAgentInput {
    // Split the resolved attachment set into image context (sent inline) and file references.
    let mut image_context = Vec::new();
    let mut file_attachments = Vec::new();
    for attachment in prompt_attachments {
        match attachment {
            PendingAttachment::Image(image) => image_context.push(AIAgentContext::Image(image)),
            PendingAttachment::File(file) => file_attachments.push(file),
        }
    }

    let context = input_context_for_request(
        true,
        context_model,
        active_session,
        Some(conversation_id),
        image_context,
        app,
    );
    let intended_agent = BlocklistAIHistoryModel::as_ref(app)
        .conversation(&conversation_id)
        .and_then(|c| c.get_task(task_id))
        .and_then(|task| {
            if task.is_root_task() {
                Some(warp_multi_agent_api::AgentType::Primary)
            } else if task.is_cli_subagent() {
                Some(warp_multi_agent_api::AgentType::Cli)
            } else {
                None
            }
        });
    let mut referenced_attachments = parse_context_attachments(&query, context_model, app);
    referenced_attachments.extend(additional_attachments);
    add_pending_file_attachments(&mut referenced_attachments, file_attachments);

    AIAgentInput::UserQuery {
        query,
        context,
        static_query_type,
        referenced_attachments,
        user_query_mode,
        running_command,
        intended_agent,
    }
}

pub(super) fn add_pending_file_attachments(
    referenced_attachments: &mut HashMap<String, AIAgentAttachment>,
    file_attachments: Vec<PendingFile>,
) {
    for file in file_attachments {
        let attachment = AIAgentAttachment::FilePathReference {
            file_id: uuid::Uuid::new_v4().to_string(),
            file_name: file.file_name.clone(),
            file_path: file.file_path.to_string_lossy().to_string(),
        };
        let mut key = file.file_name.clone();
        if referenced_attachments.contains_key(&key) {
            let mut suffix = 1;
            loop {
                key = format!("{} ({suffix})", file.file_name);
                if !referenced_attachments.contains_key(&key) {
                    break;
                }
                suffix += 1;
            }
        }
        referenced_attachments.insert(key, attachment);
    }
}

/// Validates that tool call results have corresponding tool calls in the task context, otherwise
/// logs a warning.
fn validate_tool_call_results<'a>(
    inputs: impl Iterator<Item = &'a AIAgentInput>,
    tasks: &[Task],
    server_conversation_token: &Option<ServerConversationToken>,
) {
    // Create a mapping from tool call IDs to their task IDs
    let mut tool_call_to_task_map: HashMap<String, String> = HashMap::new();
    for task in tasks {
        for message in &task.messages {
            if let Some(message::Message::ToolCall(tool_call)) = &message.message {
                tool_call_to_task_map
                    .insert(tool_call.tool_call_id.clone(), message.task_id.clone());
            }
        }
    }

    // Check each input for tool call results and validate they have corresponding tool calls
    for input in inputs {
        if let AIAgentInput::ActionResult { result, .. } = input {
            let action_id_str = result.id.to_string();
            let server_conversation_id = server_conversation_token
                .as_ref()
                .map(|token| token.as_str())
                .unwrap_or("None");

            if !tool_call_to_task_map.contains_key(&action_id_str) {
                log::warn!(
                    "Found tool call result with ID '{action_id_str}' but no corresponding tool \
                    call in task context. Server conversation ID: '{server_conversation_id}'"
                );
            }
        }
    }
}

fn get_running_command(terminal_model: &TerminalModel) -> Option<RunningCommand> {
    let active_block = terminal_model.block_list().active_block();
    if !active_block.is_active_and_long_running() || active_block.is_agent_monitoring() {
        return None;
    }
    let is_alt_screen_active = terminal_model.is_alt_screen_active();
    Some(RunningCommand {
        block_id: active_block.id().clone(),
        command: active_block.command_to_string(),
        grid_contents: if is_alt_screen_active {
            formatted_terminal_contents_for_input(
                terminal_model.alt_screen().grid_handler(),
                None,
                CURSOR_MARKER,
            )
        } else {
            formatted_terminal_contents_for_input(
                active_block.output_grid().grid_handler(),
                // TODO(vorporeal): This is probably too large.
                Some(1000),
                CURSOR_MARKER,
            )
        },
        cursor: CURSOR_MARKER.to_owned(),
        requested_command_id: active_block.requested_command_action_id().cloned(),
        is_alt_screen_active,
    })
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
