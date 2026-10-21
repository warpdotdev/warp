pub(super) mod ask_user_question;
pub(super) mod call_mcp_tool;
pub(super) mod create_documents;
pub(super) mod edit_documents;
pub(super) mod fetch_conversation;
pub(super) mod file_glob;
pub(super) mod grep;
pub(super) mod lrc_activity;
pub(super) mod read_documents;
pub(super) mod read_files;
pub(super) mod read_mcp_resource;
pub(super) mod read_skill;
pub(super) mod request_computer_use;
pub(super) mod request_file_edits;
pub(super) mod run_agents;
pub(super) mod search_codebase;
pub(super) mod send_message;
pub(super) mod shell_command;
pub(super) mod start_agent;
pub(super) mod start_recording;
pub(super) mod stop_recording;
pub(super) mod suggest_new_conversation;
pub(super) mod suggest_prompt;
pub(super) mod upload_artifact;
pub(super) mod use_computer;
pub(super) mod wait_for_events;

use std::any::Any;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use ai::agent::action::FileEdit;
use ai::agent::action_result::{InsertReviewCommentsResult, RequestCommandOutputResult};
use ai::hooks::{PermissionDecision, PermissionFloors, WarpPermissionResult};
pub use ask_user_question::AskUserQuestionExecutor;
use call_mcp_tool::CallMCPToolExecutor;
pub(crate) use call_mcp_tool::coerce_integer_args;
use create_documents::CreateDocumentsExecutor;
use edit_documents::EditDocumentsExecutor;
use fetch_conversation::FetchConversationExecutor;
use file_glob::FileGlobExecutor;
#[cfg(feature = "local_fs")]
use futures::AsyncReadExt;
use futures::FutureExt;
use futures::future::BoxFuture;
use grep::GrepExecutor;
#[cfg(feature = "local_fs")]
use mime_guess::from_path;
use parking_lot::FairMutex;
use read_documents::ReadDocumentsExecutor;
pub(super) use read_files::ReadFilesExecutor;
use read_mcp_resource::ReadMCPResourceExecutor;
use read_skill::ReadSkillExecutor;
use request_computer_use::RequestComputerUseExecutor;
pub use request_file_edits::{
    EditAcceptAndContinueClickedEvent, EditAcceptClickedEvent, EditResolvedEvent, EditStats,
    RequestFileEditsExecutor, RequestFileEditsFormatKind, RequestFileEditsTelemetryEvent,
};
pub(crate) use request_file_edits::{FileReadResult, MalformedFinalLineProxyEvent, apply_edits};
pub use run_agents::{RunAgentsExecutor, RunAgentsExecutorEvent, RunAgentsSpawningSnapshot};
#[cfg(test)]
pub use run_agents::{compose_run_agents_child_prompt, run_agents_to_start_agent_mode};
pub use send_message::SendMessageToAgentExecutor;
use serde::{Deserialize, Serialize};
pub use shell_command::{ShellCommandExecutor, ShellCommandExecutorEvent, ShellRecoveryResult};
pub use start_agent::{
    StartAgentExecutor, StartAgentExecutorEvent, StartAgentOutcome, StartAgentRequest,
    StartAgentRequestId, TEAM_CHANGED_DURING_CHILD_LAUNCH_ERROR,
};
use start_recording::StartRecordingExecutor;
use stop_recording::StopRecordingExecutor;
pub use suggest_new_conversation::NewConversationDecision;
use suggest_new_conversation::SuggestNewConversationExecutor;
pub use suggest_prompt::PromptSuggestionExecutor;
use upload_artifact::UploadArtifactExecutor;
use use_computer::UseComputerExecutor;
use wait_for_events::WaitForEventsExecutor;
use warp_core::execution_mode::AppExecutionMode;
#[cfg(feature = "local_fs")]
use warp_files::{FileModel, TextFileReadResult};
#[cfg(feature = "local_fs")]
use warp_util::file::FileLoadError;
#[cfg(feature = "local_fs")]
use warp_util::file_type::is_buffer_binary;
use warp_util::path::ShellFamily;
use warpui::r#async::{Spawnable, SpawnableOutput};
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use self::search_codebase::SearchCodebaseExecutor;
use crate::BlocklistAIHistoryModel;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AIAgentActionTypeDiscriminants, CancellationReason, FileContext,
    FileLocations, ReadFilesFailedFile, ServerOutputId,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::blocklist::action_model::recording_controller::RecordingController;
use crate::ai::blocklist::hooks::denied_result::denied_result_for;
use crate::ai::blocklist::hooks::pre_tool::{self, PendingDispatch, PreToolLookup};
use crate::ai::blocklist::hooks::tool_result;
use crate::ai::blocklist::permissions::{
    CommandExecutionPermission, CommandExecutionPermissionDeniedReason, FileWritePermission,
    FileWritePermissionDeniedReason,
};
use crate::ai::blocklist::telemetry::send_run_agents_completed_telemetry;
use crate::ai::execution_profiles::ActionPermission;
use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
#[cfg(not(target_family = "wasm"))]
use crate::ai::mcp::TemplatableMCPServerManager;
#[cfg(feature = "local_fs")]
use crate::ai::{agent::AnyFileContent, paths::host_native_absolute_path};
use crate::server::team_scope::RequestTeamScope;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::command_executor::shell_quote_arg;
use crate::terminal::model::session::{ExecuteCommandOptions, Session};
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::shell::ShellType;
use crate::terminal::{ShellLaunchData, TerminalModel};
#[cfg(feature = "local_fs")]
use crate::util::image::{
    ProcessImageResult, is_supported_image_mime_type, process_image_for_agent,
};
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::is_binary_file;
use crate::workspaces::user_workspaces::TeamContextResolver;

/// Types of actions that can be executed in parallel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParallelExecutionPolicy {
    /// Read-only actions that only inspect local context and may be safely coalesced into the
    /// same execution phase when the underlying runtime supports it.
    ReadOnlyLocalContext,
}

/// Whether an action is running serially or in parallel with other actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunningActionPhase {
    /// A barrier action that must run by itself.
    Serial,
    /// A phase where several actions from the same compatibility group may be in flight together.
    Parallel(ParallelExecutionPolicy),
}

#[derive(Debug, Clone, Copy)]
struct ExecuteActionInput<'a> {
    action: &'a AIAgentAction,
    conversation_id: AIConversationId,
}

#[derive(Debug, Clone, Copy)]
struct PreprocessActionInput<'a> {
    action: &'a AIAgentAction,
    conversation_id: AIConversationId,
}

type AsyncExecuteActionFn<T> = Pin<Box<dyn Spawnable<Output = T>>>;
type OnCompleteFn<T> = Box<dyn FnOnce(T, &mut AppContext) -> AIAgentActionResultType>;

enum ActionExecution<T: SpawnableOutput> {
    Async {
        execute_future: AsyncExecuteActionFn<T>,
        on_complete: OnCompleteFn<T>,
    },
    Sync(AIAgentActionResultType),
    NotReady,
    InvalidAction,
}

impl<T: SpawnableOutput> ActionExecution<T> {
    fn new_async(
        execute_future: impl Spawnable<Output = T>,
        on_complete: impl FnOnce(T, &mut AppContext) -> AIAgentActionResultType + 'static,
    ) -> Self {
        Self::Async {
            execute_future: Box::pin(execute_future),
            on_complete: Box::new(on_complete),
        }
    }
}

/// A trait implemented by all types that implement [`Any`] and [`SpawnableOutput`].
trait AnySpawnableOutput: Any + SpawnableOutput {}
impl<T> AnySpawnableOutput for T where T: Any + SpawnableOutput {}

type AnyAsyncExecuteActionFn = Pin<Box<dyn Spawnable<Output = Box<dyn AnySpawnableOutput>>>>;
type AnyOnCompleteFn = Box<dyn FnOnce(Box<dyn Any>, &mut AppContext) -> AIAgentActionResultType>;

enum AnyActionExecution {
    Async {
        execute_future: AnyAsyncExecuteActionFn,
        on_complete: AnyOnCompleteFn,
    },
    Sync(AIAgentActionResultType),
    NotReady,
    InvalidAction,
}

impl<T> From<ActionExecution<T>> for AnyActionExecution
where
    T: Send + 'static,
{
    fn from(value: ActionExecution<T>) -> Self {
        match value {
            ActionExecution::Async {
                execute_future,
                on_complete,
            } => AnyActionExecution::Async {
                execute_future: Box::pin(async move {
                    let result = execute_future.await;
                    Box::new(result) as Box<dyn AnySpawnableOutput>
                }),
                on_complete: Box::new(move |result, app| {
                    on_complete(*result.downcast::<T>().expect("Type is correct."), app)
                }),
            },
            ActionExecution::Sync(result) => AnyActionExecution::Sync(result),
            ActionExecution::NotReady => AnyActionExecution::NotReady,
            ActionExecution::InvalidAction => AnyActionExecution::InvalidAction,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NotExecutedReason {
    NotReady,
    NeedsConfirmation,
    WaitingOnSharer,
    /// A pre-tool hook is still deciding. The action is re-queued and retried when the verdict
    /// arrives.
    WaitingOnHooks,
}

/// Result type for `BlocklistAIActionExecutor::try_to_execute_action`.
#[derive(Debug)]
pub(super) enum TryExecuteResult {
    ExecutedSync,
    ExecutedAsync,
    NotExecuted {
        reason: NotExecutedReason,
        action: Box<AIAgentAction>,
    },
}

/// How an MCP action names its server when it carries no installation id.
enum McpLookup<'a> {
    Tool { name: &'a str },
    Resource { name: &'a str, uri: Option<&'a str> },
}

#[derive(Clone)]
struct AsyncExecutingAction {
    action: AIAgentAction,
    /// The conversation this action belongs to so cancellation and follow-up scheduling remain
    /// scoped even when several conversations have async actions in flight.
    conversation_id: AIConversationId,
}

impl AsyncExecutingAction {
    fn is_shell_command_action(&self) -> bool {
        matches!(
            self.action.action,
            AIAgentActionType::RequestCommandOutput { .. }
                | AIAgentActionType::WriteToLongRunningShellCommand { .. }
                | AIAgentActionType::ReadShellCommandOutput { .. }
        )
    }
}

pub struct BlocklistAIActionExecutor {
    shell_command_executor: ModelHandle<ShellCommandExecutor>,
    read_files_executor: ModelHandle<ReadFilesExecutor>,
    upload_artifact_executor: ModelHandle<UploadArtifactExecutor>,
    search_codebase_executor: ModelHandle<SearchCodebaseExecutor>,
    request_file_edits_executor: ModelHandle<RequestFileEditsExecutor>,
    grep_executor: ModelHandle<GrepExecutor>,
    file_glob_executor: ModelHandle<FileGlobExecutor>,
    read_mcp_resource_executor: ModelHandle<ReadMCPResourceExecutor>,
    call_mcp_tool_executor: ModelHandle<CallMCPToolExecutor>,
    suggest_new_conversation_executor: ModelHandle<SuggestNewConversationExecutor>,
    suggest_prompt_executor: ModelHandle<PromptSuggestionExecutor>,
    read_documents_executor: ModelHandle<ReadDocumentsExecutor>,
    edit_documents_executor: ModelHandle<EditDocumentsExecutor>,
    create_documents_executor: ModelHandle<CreateDocumentsExecutor>,
    use_computer_executor: ModelHandle<UseComputerExecutor>,
    request_computer_use_executor: ModelHandle<RequestComputerUseExecutor>,
    start_recording_executor: ModelHandle<StartRecordingExecutor>,
    stop_recording_executor: ModelHandle<StopRecordingExecutor>,
    read_skill_executor: ModelHandle<ReadSkillExecutor>,
    fetch_conversation_executor: ModelHandle<FetchConversationExecutor>,
    start_agent_executor: ModelHandle<StartAgentExecutor>,
    run_agents_executor: ModelHandle<RunAgentsExecutor>,
    send_message_executor: ModelHandle<SendMessageToAgentExecutor>,
    ask_user_question_executor: ModelHandle<AskUserQuestionExecutor>,
    wait_for_events_executor: ModelHandle<WaitForEventsExecutor>,
    /// The actions currently executing asynchronously, keyed by action ID.
    /// We track them per action rather than as a single slot so multiple actions from the same
    /// parallel phase can complete independently.
    async_executing_actions: std::collections::HashMap<AIAgentActionId, AsyncExecutingAction>,

    /// Reference to the terminal model for checking session sharing state.
    terminal_model: Arc<FairMutex<TerminalModel>>,
    team_context_resolver: TeamContextResolver,
    /// The session actions run in, used to describe their working directory to hooks.
    active_session: ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
}

impl BlocklistAIActionExecutor {
    pub fn new(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        active_session: ModelHandle<ActiveSession>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,
        terminal_view_id: EntityId,
        team_context_resolver: TeamContextResolver,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let read_files_executor =
            ctx.add_model(|_| ReadFilesExecutor::new(active_session.clone(), terminal_view_id));
        let upload_artifact_executor = ctx
            .add_model(|_| UploadArtifactExecutor::new(active_session.clone(), terminal_view_id));
        let search_codebase_executor = ctx.add_model(|ctx| {
            SearchCodebaseExecutor::new(
                active_session.clone(),
                get_relevant_files_controller,
                terminal_view_id,
                ctx,
            )
        });
        let shell_command_executor = ctx.add_model(|ctx| {
            ShellCommandExecutor::new(
                active_session.clone(),
                terminal_model.clone(),
                model_event_dispatcher,
                terminal_view_id,
                ctx,
            )
        });
        let request_file_edits_executor = ctx.add_model(|ctx| {
            RequestFileEditsExecutor::new(active_session.clone(), terminal_view_id, ctx)
        });
        let grep_executor =
            ctx.add_model(|_| GrepExecutor::new(active_session.clone(), terminal_view_id));
        let file_glob_executor =
            ctx.add_model(|_| FileGlobExecutor::new(active_session.clone(), terminal_view_id));
        let read_mcp_resource_executor = ctx
            .add_model(|_| ReadMCPResourceExecutor::new(active_session.clone(), terminal_view_id));
        let call_mcp_tool_executor =
            ctx.add_model(|_| CallMCPToolExecutor::new(active_session.clone(), terminal_view_id));
        let suggest_new_conversation_executor =
            ctx.add_model(|_| SuggestNewConversationExecutor::new());
        let suggest_prompt_executor = ctx.add_model(|_| PromptSuggestionExecutor::new());
        let read_documents_executor = ctx.add_model(|_| ReadDocumentsExecutor::new());
        let edit_documents_executor = ctx.add_model(|_| EditDocumentsExecutor::new());
        let create_documents_executor = ctx
            .add_model(|_| CreateDocumentsExecutor::new(active_session.clone(), terminal_view_id));
        let use_computer_executor = ctx.add_model(|_| UseComputerExecutor::new());
        let request_computer_use_executor =
            ctx.add_model(|_| RequestComputerUseExecutor::new(terminal_view_id));
        let start_recording_executor = ctx.add_model(|_| StartRecordingExecutor::new());
        let stop_recording_executor = ctx.add_model(|_| StopRecordingExecutor::new());
        let read_skill_executor = ctx.add_model(|_| ReadSkillExecutor::new(active_session.clone()));
        let fetch_conversation_executor = ctx.add_model(|_| FetchConversationExecutor::new());
        let start_agent_executor = ctx.add_model(StartAgentExecutor::new);
        let team_context_resolver_for_run_agents = team_context_resolver.clone();
        let run_agents_executor = ctx.add_model(|_| {
            RunAgentsExecutor::new(
                start_agent_executor.clone(),
                terminal_view_id,
                team_context_resolver_for_run_agents,
            )
        });
        let send_message_executor = ctx.add_model(|_| SendMessageToAgentExecutor::new());
        let ask_user_question_executor =
            ctx.add_model(|_| AskUserQuestionExecutor::new(terminal_view_id));
        let wait_for_events_executor =
            ctx.add_model(|ctx| WaitForEventsExecutor::new(terminal_view_id, ctx));
        Self {
            shell_command_executor,
            read_files_executor,
            upload_artifact_executor,
            search_codebase_executor,
            request_file_edits_executor,
            grep_executor,
            file_glob_executor,
            read_mcp_resource_executor,
            call_mcp_tool_executor,
            suggest_new_conversation_executor,
            suggest_prompt_executor,
            read_documents_executor,
            edit_documents_executor,
            create_documents_executor,
            use_computer_executor,
            request_computer_use_executor,
            start_recording_executor,
            stop_recording_executor,
            async_executing_actions: Default::default(),
            terminal_model,
            team_context_resolver,
            active_session,
            terminal_view_id,
            read_skill_executor,
            fetch_conversation_executor,
            start_agent_executor,
            run_agents_executor,
            send_message_executor,
            ask_user_question_executor,
            wait_for_events_executor,
        }
    }

    pub fn async_executing_action(&self, action_id: &AIAgentActionId) -> Option<&AIAgentAction> {
        self.async_executing_actions
            .get(action_id)
            .map(|running| &running.action)
    }

    /// Returns the action_id of any running WaitForEvents action for the
    /// given conversation. There is at most one (wait_for_events is
    /// documented as exclusive within a turn).
    pub(super) fn find_running_wait_for_events(
        &self,
        conversation_id: AIConversationId,
    ) -> Option<AIAgentActionId> {
        self.async_executing_actions
            .iter()
            .find_map(|(action_id, running)| {
                if running.conversation_id == conversation_id
                    && matches!(
                        running.action.action,
                        AIAgentActionType::WaitForEvents { .. }
                    )
                {
                    Some(action_id.clone())
                } else {
                    None
                }
            })
    }

    pub fn shell_command_executor(&self) -> &ModelHandle<ShellCommandExecutor> {
        &self.shell_command_executor
    }

    pub(super) fn active_session(&self) -> &ModelHandle<ActiveSession> {
        &self.active_session
    }

    pub fn request_file_edits_executor(&self) -> &ModelHandle<RequestFileEditsExecutor> {
        &self.request_file_edits_executor
    }

    pub fn search_codebase_executor(&self) -> &ModelHandle<SearchCodebaseExecutor> {
        &self.search_codebase_executor
    }

    pub fn suggest_new_conversation_executor(
        &self,
    ) -> &ModelHandle<SuggestNewConversationExecutor> {
        &self.suggest_new_conversation_executor
    }

    pub fn suggest_prompt_executor(&self) -> &ModelHandle<PromptSuggestionExecutor> {
        &self.suggest_prompt_executor
    }

    pub fn start_agent_executor(&self) -> &ModelHandle<StartAgentExecutor> {
        &self.start_agent_executor
    }

    pub fn run_agents_executor(&self) -> &ModelHandle<RunAgentsExecutor> {
        &self.run_agents_executor
    }

    pub fn action_phase(&self, action: &AIAgentAction, ctx: &AppContext) -> RunningActionPhase {
        match &action.action {
            AIAgentActionType::ReadFiles(..)
            | AIAgentActionType::SearchCodebase(..)
            | AIAgentActionType::ReadSkill(_) => {
                RunningActionPhase::Parallel(ParallelExecutionPolicy::ReadOnlyLocalContext)
            }
            AIAgentActionType::Grep { .. }
                if self.grep_executor.as_ref(ctx).can_execute_in_parallel(ctx) =>
            {
                RunningActionPhase::Parallel(ParallelExecutionPolicy::ReadOnlyLocalContext)
            }
            AIAgentActionType::FileGlob { .. } | AIAgentActionType::FileGlobV2 { .. }
                if self
                    .file_glob_executor
                    .as_ref(ctx)
                    .can_execute_in_parallel(ctx) =>
            {
                RunningActionPhase::Parallel(ParallelExecutionPolicy::ReadOnlyLocalContext)
            }
            _ => RunningActionPhase::Serial,
        }
    }

    pub fn ask_user_question_executor(&self) -> &ModelHandle<AskUserQuestionExecutor> {
        &self.ask_user_question_executor
    }

    pub fn set_ambient_agent_task_id(
        &self,
        id: Option<AmbientAgentTaskId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_message_executor.update(ctx, |executor, _| {
            executor.set_ambient_agent_task_id(id);
        });
        self.request_computer_use_executor
            .update(ctx, |executor, _| {
                executor.set_ambient_agent_task_id(id);
            });
    }

    pub fn preprocess_action(
        &self,
        action: &AIAgentAction,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        // In view-only mode, we do not need to perform any preprocessing work.
        if self.is_shared_session_viewer() {
            return futures::future::ready(()).boxed();
        }

        let input = PreprocessActionInput {
            action,
            conversation_id,
        };

        match &action.action {
            AIAgentActionType::RequestCommandOutput { .. }
            | AIAgentActionType::WriteToLongRunningShellCommand { .. }
            | AIAgentActionType::ReadShellCommandOutput { .. }
            | AIAgentActionType::TransferShellCommandControlToUser { .. } => self
                .shell_command_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::ReadFiles(..) => self
                .read_files_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::UploadArtifact(..) => self
                .upload_artifact_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::SearchCodebase(..) => self
                .search_codebase_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::Grep { .. } => self
                .grep_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::FileGlob { .. } | AIAgentActionType::FileGlobV2 { .. } => self
                .file_glob_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::CallMCPTool { .. } => self
                .call_mcp_tool_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::ReadMCPResource { .. } => self
                .read_mcp_resource_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            // Normally, requested file edits are not handled by the executor. However, when performing a task autonomously,
            // the executor is responsible for auto-approving diffs.
            AIAgentActionType::RequestFileEdits { .. } => self
                .request_file_edits_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::InitProject => futures::future::ready(()).boxed(),
            AIAgentActionType::OpenCodeReview => futures::future::ready(()).boxed(),
            AIAgentActionType::InsertCodeReviewComments { .. } => {
                futures::future::ready(()).boxed()
            }
            AIAgentActionType::SuggestNewConversation { .. } => self
                .suggest_new_conversation_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::SuggestPrompt { .. } => self
                .suggest_prompt_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::ReadDocuments(_) => self
                .read_documents_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::EditDocuments(_) => self
                .edit_documents_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::CreateDocuments(_) => self
                .create_documents_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::UseComputer(_) => self
                .use_computer_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::RequestComputerUse(_) => self
                .request_computer_use_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::StartRecording { .. } => self
                .start_recording_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::StopRecording { .. } => self
                .stop_recording_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::ReadSkill(_) => self
                .read_skill_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::FetchConversation { .. } => self
                .fetch_conversation_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::SendMessageToAgent { .. } => self
                .send_message_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::AskUserQuestion { .. } => self
                .ask_user_question_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::RunAgents(_) => self
                .run_agents_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::WaitForEvents { .. } => self
                .wait_for_events_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
        }
    }

    /// Returns `None` if the action was executed (and thereby consumed).
    ///
    /// If the executor cannot execute the action at this time, returns a result indicating why.
    pub fn try_to_execute_action(
        &mut self,
        action: AIAgentAction,
        conversation_id: AIConversationId,
        is_user_initiated: bool,
        ctx: &mut ModelContext<Self>,
    ) -> TryExecuteResult {
        // We should never actually execute actions in view-only mode.
        if self.is_shared_session_viewer() {
            return TryExecuteResult::NotExecuted {
                reason: NotExecutedReason::WaitingOnSharer,
                action: Box::new(action),
            };
        }

        let hook_effects = match pre_tool::begin_or_lookup(
            &action,
            conversation_id,
            is_user_initiated,
            &self.active_session,
            ctx,
        ) {
            PreToolLookup::Inactive => None,
            PreToolLookup::Pending { dispatch } => {
                if let Some(dispatch) = dispatch {
                    self.spawn_hook_dispatch(dispatch, ctx);
                }
                return TryExecuteResult::NotExecuted {
                    reason: NotExecutedReason::WaitingOnHooks,
                    action: Box::new(action),
                };
            }
            PreToolLookup::Resolved(merged) => Some(merged),
        };

        let input = ExecuteActionInput {
            action: &action,
            conversation_id,
        };
        let can_auto_execute = self.should_autoexecute(input, ctx);
        let is_agent_autonomous = AppExecutionMode::as_ref(ctx).is_autonomous();
        let is_autonomous_shell_exception =
            is_agent_autonomous && action.action.is_request_command_output();

        // A hook denial ends the action here and a hook's request for confirmation holds it;
        // execution falls through to the checks below, which are what the policy started from.
        if let Some(merged) = hook_effects {
            let floors = self.permission_floors(input, can_auto_execute, ctx);
            let decision = pre_tool::decide(
                &merged,
                WarpPermissionResult {
                    can_auto_execute,
                    is_user_initiated,
                    is_autonomous_shell_exception,
                },
                floors,
                &action,
                conversation_id,
                ctx,
            );
            match decision {
                PermissionDecision::Deny { reason, .. } => {
                    let result = denied_result_for(&action.action, &reason.text);
                    send_run_agents_completed_telemetry(
                        conversation_id,
                        &action.action,
                        &result,
                        ctx,
                    );
                    self.finish_sync(action, result, conversation_id, ctx);
                    return TryExecuteResult::ExecutedSync;
                }
                PermissionDecision::RequireConfirmation { .. } => {
                    return TryExecuteResult::NotExecuted {
                        action: Box::new(action),
                        reason: NotExecutedReason::NeedsConfirmation,
                    };
                }
                PermissionDecision::Execute => {}
            }
        }

        // The agent cannot auto execute and either:
        // - the agent is interactive, OR
        // - the agent is autonomous and the action was not requesting command output
        let needs_confirmation =
            !(is_user_initiated || can_auto_execute || is_autonomous_shell_exception);
        if needs_confirmation {
            return TryExecuteResult::NotExecuted {
                action: Box::new(action),
                reason: NotExecutedReason::NeedsConfirmation,
            };
        } else if !is_user_initiated && !can_auto_execute && is_agent_autonomous {
            // It must be the case that the autonomous agent is requesting a denylisted command.
            if let AIAgentActionType::RequestCommandOutput { command, .. } = &action.action {
                let result = AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::Denylisted {
                        command: command.clone(),
                    },
                );
                self.finish_sync(action, result, conversation_id, ctx);
                return TryExecuteResult::ExecutedSync;
            }
        }

        let action_clone = action.clone();
        let execution = match &action.action {
            AIAgentActionType::RequestCommandOutput { .. }
            | AIAgentActionType::WriteToLongRunningShellCommand { .. }
            | AIAgentActionType::ReadShellCommandOutput { .. }
            | AIAgentActionType::TransferShellCommandControlToUser { .. } => self
                .shell_command_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::InitProject => {
                ctx.emit(BlocklistAIActionExecutorEvent::InitProject(action.id));
                ActionExecution::<()>::Sync(AIAgentActionResultType::InitProject).into()
            }
            AIAgentActionType::OpenCodeReview => {
                ctx.emit(BlocklistAIActionExecutorEvent::OpenCodeReview(action.id));
                ActionExecution::<()>::Sync(AIAgentActionResultType::OpenCodeReview).into()
            }
            AIAgentActionType::InsertCodeReviewComments {
                repo_path,
                comments,
                base_branch,
            } => {
                ctx.emit(BlocklistAIActionExecutorEvent::InsertCodeReviewComments {
                    action_id: action.id,
                    repo_path: repo_path.clone(),
                    comments: comments.clone(),
                    base_branch: base_branch.clone(),
                });
                ActionExecution::<()>::Sync(AIAgentActionResultType::InsertReviewComments(
                    InsertReviewCommentsResult::Success {
                        repo_path: repo_path.to_string_lossy().to_string(),
                    },
                ))
                .into()
            }
            AIAgentActionType::ReadFiles(..) => self
                .read_files_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::UploadArtifact(..) => self
                .upload_artifact_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx)),
            AIAgentActionType::SearchCodebase(..) => {
                let team_context_resolver = self.team_context_resolver.clone();
                self.search_codebase_executor
                    .update(ctx, |executor, ctx| {
                        let team_scope = RequestTeamScope::from_scope(&team_context_resolver(ctx));
                        executor.execute(input, team_scope, ctx)
                    })
                    .into()
            }
            AIAgentActionType::Grep { .. } => self
                .grep_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::FileGlob { .. } | AIAgentActionType::FileGlobV2 { .. } => self
                .file_glob_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::CallMCPTool { .. } => self
                .call_mcp_tool_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::ReadMCPResource { .. } => self
                .read_mcp_resource_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            // Normally, requested file edits are not handled by the executor. However, when performing a task autonomously,
            // the executor is responsible for auto-approving diffs.
            AIAgentActionType::RequestFileEdits { .. } => self
                .request_file_edits_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::SuggestNewConversation { .. } => self
                .suggest_new_conversation_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::SuggestPrompt { .. } => self
                .suggest_prompt_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::ReadDocuments(_) => self
                .read_documents_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::EditDocuments(_) => self
                .edit_documents_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::CreateDocuments(_) => self
                .create_documents_executor
                .update(ctx, |executor, ctx| {
                    executor.execute(input, conversation_id, ctx)
                })
                .into(),
            AIAgentActionType::UseComputer(_) => self
                .use_computer_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::RequestComputerUse(_) => self
                .request_computer_use_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::StartRecording { .. } => self
                .start_recording_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::StopRecording { .. } => self
                .stop_recording_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx)),
            AIAgentActionType::ReadSkill(_) => self
                .read_skill_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::FetchConversation { .. } => self
                .fetch_conversation_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::SendMessageToAgent { .. } => self
                .send_message_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx)),
            AIAgentActionType::AskUserQuestion { .. } => self
                .ask_user_question_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::RunAgents(_) => self
                .run_agents_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::WaitForEvents { .. } => self
                .wait_for_events_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
        };

        let action_id = action_clone.id.clone();
        match execution {
            AnyActionExecution::NotReady => TryExecuteResult::NotExecuted {
                reason: NotExecutedReason::NotReady,
                action: Box::new(action_clone),
            },
            AnyActionExecution::InvalidAction => {
                debug_assert!(false, "Tried to execute AIAgentAction with wrong executor.");
                TryExecuteResult::NotExecuted {
                    reason: NotExecutedReason::NotReady,
                    action: Box::new(action_clone),
                }
            }
            AnyActionExecution::Async {
                execute_future,
                on_complete,
            } => {
                self.async_executing_actions.insert(
                    action_id.clone(),
                    AsyncExecutingAction {
                        action: action_clone,
                        conversation_id,
                    },
                );
                ctx.emit(BlocklistAIActionExecutorEvent::ExecutingAction {
                    action_id: action_id.clone(),
                });
                ctx.spawn(execute_future, move |me, result, ctx| {
                    let Some(running) = me.async_executing_actions.remove(&action_id) else {
                        return;
                    };
                    let result = on_complete(result, ctx);
                    tool_result::observe_terminal_result(
                        &running.action,
                        &result,
                        None,
                        running.conversation_id,
                        &me.active_session,
                        ctx,
                    );
                    ctx.emit(BlocklistAIActionExecutorEvent::FinishedAction {
                        result: Arc::new(AIAgentActionResult {
                            id: action_id,
                            task_id: running.action.task_id,
                            result,
                        }),
                        conversation_id: running.conversation_id,
                        cancellation_reason: None,
                    });
                });
                TryExecuteResult::ExecutedAsync
            }
            AnyActionExecution::Sync(action_result) => {
                self.finish_sync(action_clone, action_result, conversation_id, ctx);
                TryExecuteResult::ExecutedSync
            }
        }
    }

    /// Reports a result that is already known, with no asynchronous work to wait for.
    fn finish_sync(
        &mut self,
        action: AIAgentAction,
        result: AIAgentActionResultType,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        tool_result::observe_terminal_result(
            &action,
            &result,
            None,
            conversation_id,
            &self.active_session,
            ctx,
        );
        ctx.emit(BlocklistAIActionExecutorEvent::ExecutingAction {
            action_id: action.id.clone(),
        });
        ctx.emit(BlocklistAIActionExecutorEvent::FinishedAction {
            result: Arc::new(AIAgentActionResult {
                id: action.id,
                task_id: action.task_id,
                result,
            }),
            conversation_id,
            cancellation_reason: None,
        });
    }

    /// Runs a pre-tool hook dispatch in the background and announces the verdict when it lands.
    fn spawn_hook_dispatch(&self, dispatch: PendingDispatch, ctx: &mut ModelContext<Self>) {
        let PendingDispatch { ticket, future } = dispatch;
        ctx.spawn(future, move |_, outcome, ctx| {
            if let Some(resumption) = pre_tool::on_resolved(&ticket, outcome, ctx) {
                ctx.emit(BlocklistAIActionExecutorEvent::HookVerdictResolved {
                    action_id: ticket.action_id,
                    conversation_id: resumption.conversation_id,
                    is_user_initiated: resumption.is_user_initiated,
                });
            }
        });
    }

    pub fn can_autoexecute_action(
        &self,
        action: &AIAgentAction,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        self.should_autoexecute(
            ExecuteActionInput {
                action,
                conversation_id,
            },
            ctx,
        )
    }

    pub fn cancel_running_async_action(
        &mut self,
        action_id: &AIAgentActionId,
        reason: Option<CancellationReason>,
        ctx: &mut ModelContext<Self>,
    ) {
        // A viewer should not be able to cancel an action.
        if self.is_shared_session_viewer() {
            return;
        }
        if let Some(running) = self.async_executing_actions.remove(action_id) {
            let action_kind = AIAgentActionTypeDiscriminants::from(&running.action.action);
            log::info!(
                "Canceling running async action of type {action_kind:?} action_id={action_id:?}, reason={reason:?}, backtrace=\n{}",
                std::backtrace::Backtrace::force_capture()
            );
            if running.is_shell_command_action() {
                self.shell_command_executor.update(ctx, |executor, ctx| {
                    executor.cancel_execution(&running.action.id, ctx);
                });
            } else if matches!(running.action.action, AIAgentActionType::SearchCodebase(..)) {
                self.search_codebase_executor.update(ctx, |executor, ctx| {
                    executor.cancel_execution(&running.action.id, ctx);
                });
            } else if matches!(running.action.action, AIAgentActionType::RunAgents(..)) {
                self.run_agents_executor.update(ctx, |executor, ctx| {
                    executor.cancel_execution(&running.action.id, ctx);
                });
            } else if matches!(
                running.action.action,
                AIAgentActionType::StartRecording { .. }
            ) {
                RecordingController::handle(ctx).update(ctx, |controller, _| {
                    controller.abort_start(running.conversation_id);
                });
            } else if let AIAgentActionType::WaitForEvents { tool_call_id, .. } =
                &running.action.action
            {
                // Drop the executor's pending entry; the shared cancel
                // path emits FinishedAction(Cancelled).
                let tool_call_id = tool_call_id.clone();
                self.wait_for_events_executor.update(ctx, |executor, _| {
                    executor.cancel_execution(&tool_call_id);
                });
            }
            let result = running.action.action.cancelled_result();
            send_run_agents_completed_telemetry(
                running.conversation_id,
                &running.action.action,
                &result,
                ctx,
            );
            tool_result::observe_terminal_result(
                &running.action,
                &result,
                reason,
                running.conversation_id,
                &self.active_session,
                ctx,
            );
            ctx.emit(BlocklistAIActionExecutorEvent::FinishedAction {
                result: Arc::new(AIAgentActionResult {
                    id: running.action.id.clone(),
                    task_id: running.action.task_id,
                    result,
                }),
                conversation_id: running.conversation_id,
                cancellation_reason: reason,
            });
        }
    }

    /// Drops executor-held per-action state now that the action has reached a
    /// terminal result. Called from the action model's terminal-result choke
    /// point (`handle_action_result`), which every outcome — success, failure,
    /// or cancellation via any path — funnels through.
    ///
    /// Participation is per-executor opt-in: an executor may only be added
    /// here if its per-action state is never read after the action's terminal
    /// result. Prepared file edits qualify (consumed by execute, dead
    /// otherwise). Do NOT add state that outlives completion, e.g. the
    /// search-codebase executor's root repo paths (read at render time after
    /// the action finishes) or long-running-command state (the command
    /// outlives its action's snapshot result).
    pub(super) fn discard_action_state(
        &mut self,
        action_id: &AIAgentActionId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.request_file_edits_executor.update(ctx, |executor, _| {
            executor.discard_pending(action_id);
        });
    }

    pub fn cancel_all_running_async_actions_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        reason: Option<CancellationReason>,
        ctx: &mut ModelContext<Self>,
    ) {
        let action_ids = self
            .async_executing_actions
            .iter()
            .filter_map(|(action_id, running)| {
                (running.conversation_id == conversation_id).then_some(action_id.clone())
            })
            .collect::<Vec<_>>();
        for action_id in action_ids {
            self.cancel_running_async_action(&action_id, reason, ctx);
        }
    }

    /// The facts behind Warp's own permission answer that no hook may loosen. An action Warp
    /// auto-executes has nothing to loosen, so it has no floors; neither has any family other
    /// than the shell, file edit, and MCP ones. The floors are read from the same permission
    /// functions the per-family `should_autoexecute` checks call, so a denylist bypass or a
    /// sandbox exemption is applied exactly as Warp applies it. The functions are called again
    /// here rather than read from the executor that ran them; where that executor answered
    /// before reaching them, the floors still describe the settings and lists as they stand.
    fn permission_floors(
        &self,
        input: ExecuteActionInput,
        can_auto_execute: bool,
        ctx: &ModelContext<Self>,
    ) -> PermissionFloors {
        if can_auto_execute {
            return PermissionFloors::default();
        }
        match &input.action.action {
            AIAgentActionType::RequestCommandOutput {
                command,
                is_read_only,
                is_risky,
                ..
            } => self.shell_floors(
                &input.conversation_id,
                command,
                *is_read_only,
                *is_risky,
                ctx,
            ),
            AIAgentActionType::RequestFileEdits { file_edits, .. } => {
                self.file_write_floors(&input.conversation_id, file_edits, ctx)
            }
            AIAgentActionType::CallMCPTool {
                server_id, name, ..
            } => self.mcp_floors(server_id.as_ref(), McpLookup::Tool { name }, ctx),
            AIAgentActionType::ReadMCPResource {
                server_id,
                name,
                uri,
            } => self.mcp_floors(
                server_id.as_ref(),
                McpLookup::Resource {
                    name,
                    uri: uri.as_deref(),
                },
                ctx,
            ),
            AIAgentActionType::WriteToLongRunningShellCommand { .. }
            | AIAgentActionType::ReadShellCommandOutput { .. }
            | AIAgentActionType::TransferShellCommandControlToUser { .. }
            | AIAgentActionType::ReadFiles(_)
            | AIAgentActionType::UploadArtifact(_)
            | AIAgentActionType::SearchCodebase(_)
            | AIAgentActionType::Grep { .. }
            | AIAgentActionType::FileGlob { .. }
            | AIAgentActionType::FileGlobV2 { .. }
            | AIAgentActionType::SuggestNewConversation { .. }
            | AIAgentActionType::SuggestPrompt(_)
            | AIAgentActionType::InitProject
            | AIAgentActionType::OpenCodeReview
            | AIAgentActionType::ReadDocuments(_)
            | AIAgentActionType::EditDocuments(_)
            | AIAgentActionType::CreateDocuments(_)
            | AIAgentActionType::UseComputer(_)
            | AIAgentActionType::InsertCodeReviewComments { .. }
            | AIAgentActionType::RequestComputerUse(_)
            | AIAgentActionType::StartRecording { .. }
            | AIAgentActionType::StopRecording { .. }
            | AIAgentActionType::ReadSkill(_)
            | AIAgentActionType::FetchConversation { .. }
            | AIAgentActionType::SendMessageToAgent { .. }
            | AIAgentActionType::AskUserQuestion { .. }
            | AIAgentActionType::RunAgents(_)
            | AIAgentActionType::WaitForEvents { .. } => PermissionFloors::default(),
        }
    }

    /// The floors for a shell command, from `can_autoexecute_command`'s reason. Without a shell
    /// type the command cannot be taken apart for the denylist and Warp does not auto-execute
    /// it; the setting is still a fact.
    fn shell_floors(
        &self,
        conversation_id: &AIConversationId,
        command: &str,
        is_read_only: Option<bool>,
        is_risky: Option<bool>,
        ctx: &ModelContext<Self>,
    ) -> PermissionFloors {
        let permissions = BlocklistAIPermissions::as_ref(ctx);
        let terminal_view_id = Some(self.terminal_view_id);
        let scope = (self.team_context_resolver)(ctx);
        let Some(escape_char) = self
            .active_session
            .as_ref(ctx)
            .shell_type(ctx)
            .map(|shell| ShellFamily::from(shell).escape_char())
        else {
            return PermissionFloors {
                always_ask_setting: permissions.get_execute_commands_setting(
                    terminal_view_id,
                    &scope,
                    ctx,
                ) == ActionPermission::AlwaysAsk,
                ..Default::default()
            };
        };
        match permissions.can_autoexecute_command(
            conversation_id,
            command,
            escape_char,
            is_read_only.unwrap_or(false),
            is_risky,
            terminal_view_id,
            &scope,
            ctx,
        ) {
            CommandExecutionPermission::Denied(
                CommandExecutionPermissionDeniedReason::ExplicitlyDenylisted,
            ) => PermissionFloors {
                command_denylist_hit: true,
                ..Default::default()
            },
            CommandExecutionPermission::Denied(
                CommandExecutionPermissionDeniedReason::AlwaysAskEnabled,
            ) => PermissionFloors {
                always_ask_setting: true,
                ..Default::default()
            },
            CommandExecutionPermission::Denied(
                CommandExecutionPermissionDeniedReason::AutonomyForceDisabled
                | CommandExecutionPermissionDeniedReason::ContainsRedirection
                | CommandExecutionPermissionDeniedReason::Inconclusive
                | CommandExecutionPermissionDeniedReason::AgentDecided,
            )
            | CommandExecutionPermission::Allowed(_) => PermissionFloors::default(),
        }
    }

    /// The floors for a file edit, from `can_write_files`'s reason.
    fn file_write_floors(
        &self,
        conversation_id: &AIConversationId,
        file_edits: &[FileEdit],
        ctx: &ModelContext<Self>,
    ) -> PermissionFloors {
        let paths = file_edits
            .iter()
            .filter_map(|edit| edit.file().map(PathBuf::from))
            .collect::<Vec<_>>();
        let scope = (self.team_context_resolver)(ctx);
        match BlocklistAIPermissions::as_ref(ctx).can_write_files(
            conversation_id,
            &paths,
            Some(self.terminal_view_id),
            &scope,
            ctx,
        ) {
            FileWritePermission::Denied(FileWritePermissionDeniedReason::ProtectedPath) => {
                PermissionFloors {
                    protected_write_path: true,
                    ..Default::default()
                }
            }
            FileWritePermission::Denied(FileWritePermissionDeniedReason::AlwaysAskEnabled) => {
                PermissionFloors {
                    always_ask_setting: true,
                    ..Default::default()
                }
            }
            FileWritePermission::Denied(
                FileWritePermissionDeniedReason::AutonomyForceDisabled
                | FileWritePermissionDeniedReason::Inconclusive
                | FileWritePermissionDeniedReason::AgentDecided,
            )
            | FileWritePermission::Allowed(_) => PermissionFloors::default(),
        }
    }

    /// The floors for an MCP call. The server is found the way `can_call_mcp_tool` and
    /// `can_read_mcp_resource` find it (by installation id, else by the tool or resource name),
    /// because those checks answer only yes or no and do not say whether the denylist was why.
    #[cfg(not(target_family = "wasm"))]
    fn mcp_floors(
        &self,
        server_id: Option<&uuid::Uuid>,
        lookup: McpLookup<'_>,
        ctx: &ModelContext<Self>,
    ) -> PermissionFloors {
        let permissions = BlocklistAIPermissions::as_ref(ctx);
        let manager = TemplatableMCPServerManager::as_ref(ctx);
        let terminal_view_id = Some(self.terminal_view_id);
        let template = server_id
            .and_then(|id| manager.get_template_uuid(*id))
            .or_else(|| {
                let installation = match lookup {
                    McpLookup::Tool { name } => manager.server_from_tool(name.to_owned()),
                    McpLookup::Resource { name, uri } => manager.server_from_resource(name, uri),
                };
                installation
                    .copied()
                    .and_then(|id| manager.get_template_uuid(id))
            });
        let mcp_server_denylisted = template.is_some_and(|template| {
            permissions
                .get_mcp_denylist(ctx, terminal_view_id)
                .contains(&template)
        });
        let always_ask_setting = permissions.get_mcp_permissions_setting(ctx, terminal_view_id)
            == ActionPermission::AlwaysAsk;
        PermissionFloors {
            mcp_server_denylisted,
            always_ask_setting,
            ..Default::default()
        }
    }

    /// MCP calls never run on wasm, so the check that would establish a floor never runs either.
    #[cfg(target_family = "wasm")]
    fn mcp_floors(
        &self,
        _server_id: Option<&uuid::Uuid>,
        _lookup: McpLookup<'_>,
        _ctx: &ModelContext<Self>,
    ) -> PermissionFloors {
        PermissionFloors::default()
    }

    fn should_autoexecute(&self, input: ExecuteActionInput, ctx: &mut ModelContext<Self>) -> bool {
        let team_context_resolver = self.team_context_resolver.clone();
        match input.action.action {
            AIAgentActionType::RequestCommandOutput { .. }
            | AIAgentActionType::WriteToLongRunningShellCommand { .. }
            | AIAgentActionType::ReadShellCommandOutput { .. }
            | AIAgentActionType::TransferShellCommandControlToUser { .. } => {
                self.shell_command_executor.update(ctx, |executor, ctx| {
                    executor.should_autoexecute(input, &team_context_resolver(ctx), ctx)
                })
            }
            AIAgentActionType::ReadFiles(_) => {
                self.read_files_executor.update(ctx, |executor, ctx| {
                    executor.should_autoexecute(input, &team_context_resolver(ctx), ctx)
                })
            }
            AIAgentActionType::UploadArtifact(_) => {
                self.upload_artifact_executor.update(ctx, |executor, ctx| {
                    executor.should_autoexecute(input, &team_context_resolver(ctx), ctx)
                })
            }
            AIAgentActionType::SearchCodebase(_) => {
                self.search_codebase_executor.update(ctx, |executor, ctx| {
                    executor.should_autoexecute(input, &team_context_resolver(ctx), ctx)
                })
            }
            AIAgentActionType::RequestFileEdits { .. } => {
                self.request_file_edits_executor
                    .update(ctx, |executor, ctx| {
                        executor.should_autoexecute(input, &team_context_resolver(ctx), ctx)
                    })
            }
            AIAgentActionType::Grep { .. } => self.grep_executor.update(ctx, |executor, ctx| {
                executor.should_autoexecute(input, &team_context_resolver(ctx), ctx)
            }),
            AIAgentActionType::FileGlob { .. } | AIAgentActionType::FileGlobV2 { .. } => {
                self.file_glob_executor.update(ctx, |executor, ctx| {
                    executor.should_autoexecute(input, &team_context_resolver(ctx), ctx)
                })
            }
            AIAgentActionType::CallMCPTool { .. } => self
                .call_mcp_tool_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::ReadMCPResource { .. } => self
                .read_mcp_resource_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::InitProject => true,
            AIAgentActionType::OpenCodeReview => true,
            AIAgentActionType::InsertCodeReviewComments { .. } => true,
            AIAgentActionType::SuggestNewConversation { .. } => self
                .suggest_new_conversation_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::SuggestPrompt { .. } => self
                .suggest_prompt_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::ReadDocuments(_) => self
                .read_documents_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::EditDocuments(_) => self
                .edit_documents_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::CreateDocuments(_) => self
                .create_documents_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::UseComputer(_) => self
                .use_computer_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::RequestComputerUse(_) => {
                self.request_computer_use_executor
                    .update(ctx, |executor, ctx| {
                        executor.should_autoexecute(input, &team_context_resolver(ctx), ctx)
                    })
            }
            AIAgentActionType::StartRecording { .. } => self
                .start_recording_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::StopRecording { .. } => self
                .stop_recording_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::ReadSkill(_) => self
                .read_skill_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::FetchConversation { .. } => self
                .fetch_conversation_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::SendMessageToAgent { .. } => self
                .send_message_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::AskUserQuestion { .. } => self
                .ask_user_question_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::RunAgents(_) => self
                .run_agents_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::WaitForEvents { .. } => self
                .wait_for_events_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
        }
    }

    fn is_shared_session_viewer(&self) -> bool {
        self.terminal_model.lock().is_shared_session_viewer()
    }
}
impl Entity for BlocklistAIActionExecutor {
    type Event = BlocklistAIActionExecutorEvent;
}

pub enum BlocklistAIActionExecutorEvent {
    /// Emitted when an action is execution starts.
    ExecutingAction {
        action_id: AIAgentActionId,
    },

    /// Emitted when an action has finished.
    FinishedAction {
        result: Arc<AIAgentActionResult>,
        conversation_id: AIConversationId,
        /// The reason for cancellation, if this action was cancelled.
        cancellation_reason: Option<CancellationReason>,
    },

    /// Emitted when a pre-tool hook verdict for a queued action has arrived, so the action can be
    /// tried again.
    HookVerdictResolved {
        action_id: AIAgentActionId,
        conversation_id: AIConversationId,
        /// Whether the user had already approved the action before the verdict was requested.
        is_user_initiated: bool,
    },

    InitProject(AIAgentActionId),
    OpenCodeReview(AIAgentActionId),
    InsertCodeReviewComments {
        action_id: AIAgentActionId,
        repo_path: PathBuf,
        comments: Vec<ai::agent::action::InsertReviewComment>,
        base_branch: Option<String>,
    },
}

/// Per-file byte limit for [`read_local_file_context`]. Binary files larger
/// than this are skipped; text files are truncated at this limit.
#[cfg(feature = "local_fs")]
const MAX_FILE_READ_BYTES: usize = 1_000_000;

/// The results of a [`read_local_file_context`] call.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ReadFileContextResult {
    /// [`FileContext`] data for all files that could be read.
    pub file_contexts: Vec<FileContext>,
    /// Requested files that could not be read, each paired with a reason-specific
    /// failure message (missing, too large, or unprocessable).
    pub failed_files: Vec<ReadFilesFailedFile>,
}

/// Builds a single, reason-accurate summary from a batch of file-read failures,
/// one `path: reason` entry per file. Shared by the agent-tool consumers
/// (`read_files`, `get_files`, `search_codebase`) so they all surface the same
/// per-file reason instead of a flat "do not exist" list.
pub fn describe_failed_files(failed_files: &[ReadFilesFailedFile]) -> String {
    failed_files
        .iter()
        .map(|failed_file| format!("{}: {}", failed_file.path, failed_file.message))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Reads the content of the given files at the given `FileLocations`.
///
/// If any files do not exist, they are included in the `failed_files` field of the result.
///
/// Binary files larger than the per-file byte limit are skipped and reported as
/// too large. Text files are truncated at the per-file limit via line streaming.
/// If `max_file_bytes` is provided, it overrides the default per-file limit
/// ([`MAX_FILE_READ_BYTES`]). Pass `None` to use the default.
/// If `max_batch_bytes` is provided, the cumulative content of all files is capped at that
/// budget; once exceeded, remaining files are reported as too large.
#[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
pub async fn read_local_file_context(
    file_names: &[FileLocations],
    current_working_directory: Option<String>,
    shell: Option<ShellLaunchData>,
    max_file_bytes: Option<usize>,
    max_batch_bytes: Option<usize>,
) -> anyhow::Result<ReadFileContextResult> {
    #[cfg(not(feature = "local_fs"))]
    return Err(anyhow::anyhow!(
        "Can't read files when not on a local filesystem"
    ));

    #[cfg(feature = "local_fs")]
    {
        let mut result = ReadFileContextResult {
            file_contexts: Vec::new(),
            failed_files: Vec::new(),
        };

        let mut batch_bytes_remaining = max_batch_bytes;

        for file in file_names {
            let absolute_file_path = PathBuf::from(host_native_absolute_path(
                &file.name,
                &shell,
                &current_working_directory,
            ));

            let metadata = match async_fs::metadata(&absolute_file_path).await {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    result.failed_files.push(ReadFilesFailedFile {
                        path: absolute_file_path.to_string_lossy().to_string(),
                        message: "File does not exist".to_string(),
                    });
                    continue;
                }
                Err(e) => return Err(anyhow::anyhow!(e)),
            };
            let last_modified = metadata.modified().ok();
            let file_size = metadata.len() as usize;
            let path_str = absolute_file_path.to_string_lossy().to_string();

            // Effective byte budget: the tighter of per-file and remaining batch.
            let per_file_limit = max_file_bytes.unwrap_or(MAX_FILE_READ_BYTES);
            let effective_max = match batch_bytes_remaining {
                Some(remaining) => per_file_limit.min(remaining),
                None => per_file_limit,
            };

            // Decide text vs binary before opening the file. Extension-based
            // detection alone is wrong for extensionless text files (e.g. shell
            // scripts named `bundle`, `run`), so for the ambiguous case we fall
            // back to content-based inspection of the first chunk. The binary
            // path below still acts as a safety net via
            // `TextFileReadResult::NotText` if the text reader trips on invalid
            // UTF-8.
            if !should_read_as_binary(&absolute_file_path).await {
                match FileModel::read_text_file(
                    &absolute_file_path,
                    effective_max,
                    &file.lines,
                    last_modified,
                )
                .await?
                {
                    TextFileReadResult::Segments {
                        segments,
                        bytes_read,
                    } => {
                        if let Some(remaining) = &mut batch_bytes_remaining {
                            *remaining = remaining.saturating_sub(bytes_read);
                        }
                        result
                            .file_contexts
                            .extend(segments.into_iter().map(|seg| FileContext {
                                file_name: seg.file_name,
                                content: AnyFileContent::StringContent(seg.content),
                                line_range: seg.line_range,
                                last_modified: seg.last_modified,
                                line_count: seg.line_count,
                            }));
                        continue;
                    }
                    TextFileReadResult::NotText => {
                        // Fall through to binary path below.
                    }
                }
            }

            // Binary path (either detected as binary, or text reading failed).
            match read_binary_file_context(
                &absolute_file_path,
                effective_max,
                file_size,
                last_modified,
            )
            .await?
            {
                BinaryFileReadResult::Context {
                    file_context,
                    bytes_read,
                } => {
                    if let Some(remaining) = &mut batch_bytes_remaining {
                        *remaining = remaining.saturating_sub(bytes_read);
                    }
                    result.file_contexts.push(file_context);
                }
                BinaryFileReadResult::NotFound => result.failed_files.push(ReadFilesFailedFile {
                    path: path_str,
                    message: "File does not exist".to_string(),
                }),
                BinaryFileReadResult::TooLarge {
                    size_bytes,
                    limit_bytes,
                } => result.failed_files.push(ReadFilesFailedFile {
                    path: path_str,
                    message: format!(
                        "File is too large to read ({} > {} limit). Downscale/compress it or read a smaller copy.",
                        format_mb(size_bytes),
                        format_mb(limit_bytes)
                    ),
                }),
                BinaryFileReadResult::ProcessingFailed { detail } => {
                    result.failed_files.push(ReadFilesFailedFile {
                        path: path_str,
                        message: format!("File could not be processed as an image: {detail}"),
                    })
                }
            }
        }

        Ok(result)
    }
}

/// Returns `true` if the file at `path` should be read via the binary code
/// path in [`read_local_file_context`], `false` if it should be read as text.
///
/// Uses extension-based detection as a fast path and falls back to content
/// inspection (reading the first 1 KiB) for extensionless files whose names
/// don't match any known text or binary pattern. Without the content-based
/// fallback, extensionless text files (e.g. shell scripts named `bundle`)
/// would be incorrectly classified as binary and returned to the agent as
/// raw bytes instead of UTF-8 text.
#[cfg(feature = "local_fs")]
async fn should_read_as_binary(path: &std::path::Path) -> bool {
    // Fast path: extension/filename clearly indicates text.
    if !is_binary_file(path) {
        return false;
    }
    // Fast path: file has a known binary extension (e.g. `.png`, `.exe`).
    if path.extension().is_some() {
        return true;
    }
    // Extensionless file with an unknown basename. Inspect the first chunk of
    // the file to decide. Treat open/read errors as binary so the binary path
    // takes over and reports a consistent error.
    is_file_content_binary_async(path).await
}

/// Async sibling of [`warp_util::file_type::is_file_content_binary`]. Reads
/// the first 1 KiB of `path` asynchronously and returns `true` if the content
/// looks binary according to [`is_buffer_binary`]. Returns `true` on any I/O
/// error so callers default to the binary code path. Kept local to this
/// module so `warp_util` doesn't need to grow an `async_fs` dependency.
#[cfg(feature = "local_fs")]
async fn is_file_content_binary_async(path: &std::path::Path) -> bool {
    const CHUNK_SIZE: usize = 1024;

    let Ok(mut file) = async_fs::File::open(path).await else {
        return true;
    };
    let mut buffer = [0u8; CHUNK_SIZE];
    let Ok(n) = file.read(&mut buffer).await else {
        return true;
    };
    is_buffer_binary(&buffer[..n])
}

/// Renders a byte count in megabytes with one decimal place (e.g. `3.5 MB`),
/// matching the units used in the "too large" failure message.
#[cfg(feature = "local_fs")]
fn format_mb(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(feature = "local_fs")]
enum BinaryFileReadResult {
    /// Successfully read as binary.
    Context {
        file_context: FileContext,
        bytes_read: usize,
    },
    /// The file does not exist on disk.
    NotFound,
    /// The file exists but exceeds the effective byte limit.
    TooLarge {
        size_bytes: usize,
        limit_bytes: usize,
    },
    /// The file could not be processed (e.g. an image decode/resize failed, or
    /// the processed image is still too large to send).
    ProcessingFailed { detail: String },
}

/// Reads a binary file, applying image processing when applicable.
#[cfg(feature = "local_fs")]
async fn read_binary_file_context(
    path: &std::path::Path,
    max_bytes: usize,
    file_size: usize,
    last_modified: Option<std::time::SystemTime>,
) -> anyhow::Result<BinaryFileReadResult> {
    if file_size > max_bytes {
        return Ok(BinaryFileReadResult::TooLarge {
            size_bytes: file_size,
            limit_bytes: max_bytes,
        });
    }

    let content = match read_file_as_binary(path).await {
        Ok(content) => content,
        Err(FileLoadError::DoesNotExist) => return Ok(BinaryFileReadResult::NotFound),
        Err(FileLoadError::IOError(e)) => return Err(anyhow::anyhow!(e)),
    };

    let mime_type = from_path(path).first_or_octet_stream().to_string();
    let processed_content = if is_supported_image_mime_type(&mime_type) {
        match process_image_for_agent(&content) {
            ProcessImageResult::Success { data } => Some(data),
            ProcessImageResult::TooLarge => {
                log::warn!("Image file too large after processing: {}", path.display());
                return Ok(BinaryFileReadResult::ProcessingFailed {
                    detail: "the image is too large to send even after resizing".to_string(),
                });
            }
            ProcessImageResult::Error(err) => {
                log::warn!("Error processing image file {}: {err:?}", path.display());
                return Ok(BinaryFileReadResult::ProcessingFailed {
                    detail: err.to_string(),
                });
            }
        }
    } else {
        None
    };

    let final_content = processed_content.unwrap_or(content);
    if final_content.len() > max_bytes {
        return Ok(BinaryFileReadResult::TooLarge {
            size_bytes: final_content.len(),
            limit_bytes: max_bytes,
        });
    }

    let bytes_read = final_content.len();
    Ok(BinaryFileReadResult::Context {
        file_context: FileContext::new(
            path.to_string_lossy().to_string(),
            AnyFileContent::BinaryContent(final_content),
            None,
            last_modified,
        ),
        bytes_read,
    })
}

fn build_is_file_path_command(path: &str, shell_type: ShellType) -> String {
    let escaped_path = shell_quote_arg(path, shell_type);
    if shell_type == ShellType::PowerShell {
        format!("if (Test-Path -PathType Leaf {escaped_path}) {{ exit 0 }} else {{ exit 1 }}")
    } else {
        format!("test -f {escaped_path}")
    }
}

fn build_is_git_repository_command(absolute_path: &str, shell_type: ShellType) -> String {
    format!(
        "git -C {} rev-parse",
        shell_quote_arg(absolute_path, shell_type)
    )
}

/// Returns true if the given path is a regular file on the session's filesystem.
/// Runs a shell command on the session so it works for both local and remote sessions.
async fn is_file_path(path: &str, session: &Session) -> bool {
    let command = build_is_file_path_command(path, session.shell().shell_type());
    session
        .execute_command(&command, None, None, ExecuteCommandOptions::default())
        .await
        .map(|output| output.success())
        .unwrap_or(false)
}

/// Returns true if git is installed and the given path is in a git repository.
async fn is_git_repository(absolute_path: &str, session: &Session) -> anyhow::Result<bool> {
    let git_command = build_is_git_repository_command(absolute_path, session.shell().shell_type());
    let command_output = session
        .execute_command(
            git_command.as_str(),
            None,
            None,
            ExecuteCommandOptions::default(),
        )
        .await?;
    Ok(command_output.success())
}

fn get_server_output_id(
    conversation_id: AIConversationId,
    ctx: &mut AppContext,
) -> Option<ServerOutputId> {
    BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&conversation_id)?
        .latest_exchange()?
        .output_status
        .server_output_id()
}

#[cfg(feature = "local_fs")]
async fn read_file_as_binary(file_path: &std::path::Path) -> Result<Vec<u8>, FileLoadError> {
    if !FileModel::file_exists(file_path).await {
        return Err(FileLoadError::DoesNotExist);
    }

    async_fs::read(file_path).await.map_err(FileLoadError::from)
}

#[cfg(all(test, feature = "local_fs"))]
#[path = "execute_tests.rs"]
mod tests;
