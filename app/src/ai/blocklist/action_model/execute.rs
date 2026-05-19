pub(super) mod ask_user_question;
pub(super) mod call_mcp_tool;
pub(super) mod create_documents;
pub(super) mod edit_documents;
pub(super) mod fetch_conversation;
pub(super) mod file_glob;
pub(super) mod grep;
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
pub(super) mod suggest_new_conversation;
pub(super) mod suggest_prompt;
pub(super) mod upload_artifact;
pub(super) mod use_computer;

use ai::agent::action_result::{InsertReviewCommentsResult, RequestCommandOutputResult};
pub use ask_user_question::AskUserQuestionExecutor;
pub(crate) use call_mcp_tool::coerce_integer_args;
use call_mcp_tool::CallMCPToolExecutor;
use create_documents::CreateDocumentsExecutor;
use edit_documents::EditDocumentsExecutor;
use fetch_conversation::FetchConversationExecutor;
use file_glob::FileGlobExecutor;
use futures::{future::BoxFuture, FutureExt};
use grep::GrepExecutor;
use parking_lot::FairMutex;
use read_documents::ReadDocumentsExecutor;
pub(super) use read_files::ReadFilesExecutor;
use read_mcp_resource::ReadMCPResourceExecutor;
use read_skill::ReadSkillExecutor;
use request_computer_use::RequestComputerUseExecutor;
pub(crate) use request_file_edits::apply_edits;
pub(crate) use request_file_edits::FileReadResult;
pub(crate) use request_file_edits::MalformedFinalLineProxyEvent;
pub use request_file_edits::{
    EditAcceptAndContinueClickedEvent, EditAcceptClickedEvent, EditResolvedEvent, EditStats,
    RequestFileEditsExecutor, RequestFileEditsFormatKind, RequestFileEditsTelemetryEvent,
};
#[cfg(test)]
pub use run_agents::{compose_run_agents_child_prompt, run_agents_to_start_agent_mode};
pub use run_agents::{RunAgentsExecutor, RunAgentsExecutorEvent, RunAgentsSpawningSnapshot};
pub use send_message::SendMessageToAgentExecutor;
use serde::{Deserialize, Serialize};
pub use shell_command::{ShellCommandExecutor, ShellCommandExecutorEvent};
pub use start_agent::{
    StartAgentExecutor, StartAgentExecutorEvent, StartAgentRequest, StartAgentRequestId,
};
pub use suggest_new_conversation::NewConversationDecision;
use suggest_new_conversation::SuggestNewConversationExecutor;
pub use suggest_prompt::PromptSuggestionExecutor;
use upload_artifact::UploadArtifactExecutor;
use use_computer::UseComputerExecutor;
use warp_core::{execution_mode::AppExecutionMode, features::FeatureFlag};

#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::is_binary_file;
#[cfg(feature = "local_fs")]
use futures::AsyncReadExt;
use std::{any::Any, path::PathBuf, pin::Pin, sync::Arc};
#[cfg(feature = "local_fs")]
use warp_files::{FileModel, TextFileReadResult};
#[cfg(feature = "local_fs")]
use warp_util::file::FileLoadError;
#[cfg(feature = "local_fs")]
use warp_util::file_type::is_buffer_binary;
use warpui::{
    r#async::{Spawnable, SpawnableOutput},
    AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity,
};

#[cfg(feature = "local_fs")]
use crate::util::image::{
    is_supported_image_mime_type, process_image_for_agent, ProcessImageResult,
};
#[cfg(feature = "local_fs")]
use mime_guess::from_path;

use self::search_codebase::SearchCodebaseExecutor;
#[cfg(feature = "local_fs")]
use crate::ai::{agent::AnyFileContent, paths::host_native_absolute_path};
use crate::{
    ai::{
        agent::{
            conversation::AIConversationId, task::TaskId, AIAgentAction, AIAgentActionId,
            AIAgentActionResult, AIAgentActionResultType, AIAgentActionType,
            AIAgentActionTypeDiscriminants, CancellationReason, FileContext, FileLocations,
            ServerOutputId,
        },
        ambient_agents::AmbientAgentTaskId,
        get_relevant_files::controller::GetRelevantFilesController,
    },
    terminal::{
        model::session::{active_session::ActiveSession, ExecuteCommandOptions, Session},
        model_events::ModelEventDispatcher,
        shell::ShellType,
        ShellLaunchData, TerminalModel,
    },
    BlocklistAIHistoryModel,
};

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

#[derive(Debug, Copy, Clone)]
pub enum NotExecutedReason {
    NotReady,
    NeedsConfirmation,
    WaitingOnSharer,
}

impl NotExecutedReason {
    pub fn needs_confirmation(&self) -> bool {
        matches!(self, Self::NeedsConfirmation)
    }
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
    read_skill_executor: ModelHandle<ReadSkillExecutor>,
    fetch_conversation_executor: ModelHandle<FetchConversationExecutor>,
    start_agent_executor: ModelHandle<StartAgentExecutor>,
    run_agents_executor: ModelHandle<RunAgentsExecutor>,
    send_message_executor: ModelHandle<SendMessageToAgentExecutor>,
    ask_user_question_executor: ModelHandle<AskUserQuestionExecutor>,
    /// The actions currently executing asynchronously, keyed by action ID.
    /// We track them per action rather than as a single slot so multiple actions from the same
    /// parallel phase can complete independently.
    async_executing_actions: std::collections::HashMap<AIAgentActionId, AsyncExecutingAction>,

    /// Reference to the terminal model for checking session sharing state.
    terminal_model: Arc<FairMutex<TerminalModel>>,
}

impl BlocklistAIActionExecutor {
    pub fn new(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        active_session: ModelHandle<ActiveSession>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,
        terminal_view_id: EntityId,
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
        let read_skill_executor = ctx.add_model(|_| ReadSkillExecutor::new());
        let fetch_conversation_executor = ctx.add_model(|_| FetchConversationExecutor::new());
        let start_agent_executor = ctx.add_model(StartAgentExecutor::new);
        let run_agents_executor =
            ctx.add_model(|_| RunAgentsExecutor::new(start_agent_executor.clone()));
        let send_message_executor = ctx.add_model(|_| SendMessageToAgentExecutor::new());
        let ask_user_question_executor =
            ctx.add_model(|_| AskUserQuestionExecutor::new(terminal_view_id));
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
            async_executing_actions: Default::default(),
            terminal_model,
            read_skill_executor,
            fetch_conversation_executor,
            start_agent_executor,
            run_agents_executor,
            send_message_executor,
            ask_user_question_executor,
        }
    }

    pub fn async_executing_action(&self, action_id: &AIAgentActionId) -> Option<&AIAgentAction> {
        self.async_executing_actions
            .get(action_id)
            .map(|running| &running.action)
    }

    pub fn shell_command_executor(&self) -> &ModelHandle<ShellCommandExecutor> {
        &self.shell_command_executor
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
            AIAgentActionType::ReadSkill(_) => self
                .read_skill_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::FetchConversation { .. } => self
                .fetch_conversation_executor
                .update(ctx, |executor, ctx| executor.preprocess_action(input, ctx)),
            AIAgentActionType::StartAgent { .. } => self
                .start_agent_executor
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

        let input = ExecuteActionInput {
            action: &action,
            conversation_id,
        };
        let can_auto_execute = self.should_autoexecute(input, ctx);
        let is_agent_autonomous = AppExecutionMode::as_ref(ctx).is_autonomous();

        // The agent cannot auto execute and either:
        // - the agent is interactive, OR
        // - the agent is autonomous and the action was not requesting command output
        let needs_confirmation = !(is_user_initiated
            || can_auto_execute
            || (is_agent_autonomous && action.action.is_request_command_output()));
        if needs_confirmation {
            return TryExecuteResult::NotExecuted {
                action: Box::new(action),
                reason: NotExecutedReason::NeedsConfirmation,
            };
        } else if !is_user_initiated && !can_auto_execute && is_agent_autonomous {
            // It must be the case that the autonomous agent is requesting a denylisted command.
            if let AIAgentActionType::RequestCommandOutput { command, .. } = &action.action {
                let action_id = action.id.clone();
                let result = AIAgentActionResultType::RequestCommandOutput(
                    RequestCommandOutputResult::Denylisted {
                        command: command.clone(),
                    },
                );

                ctx.emit(BlocklistAIActionExecutorEvent::ExecutingAction {
                    action_id: action_id.clone(),
                });
                ctx.emit(BlocklistAIActionExecutorEvent::FinishedAction {
                    result: Arc::new(AIAgentActionResult {
                        id: action_id,
                        task_id: action.task_id.clone(),
                        result,
                    }),
                    conversation_id,
                    cancellation_reason: None,
                });

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
                if FeatureFlag::PRCommentsSlashCommand.is_enabled() {
                    ctx.emit(BlocklistAIActionExecutorEvent::InsertCodeReviewComments {
                        action_id: action.id,
                        repo_path: repo_path.clone(),
                        comments: comments.clone(),
                        base_branch: base_branch.clone(),
                    });
                }
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
            AIAgentActionType::SearchCodebase(..) => self
                .search_codebase_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
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
            AIAgentActionType::ReadSkill(_) => self
                .read_skill_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::FetchConversation { .. } => self
                .fetch_conversation_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::StartAgent { .. } => self
                .start_agent_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            AIAgentActionType::SendMessageToAgent { .. } => self
                .send_message_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx)),
            AIAgentActionType::AskUserQuestion { .. } => self
                .ask_user_question_executor
                .update(ctx, |executor, ctx| executor.execute(input, ctx))
                .into(),
            // Standard executor path (un-edited request). The card
            // view's Accept uses `execute_run_agents` instead.
            AIAgentActionType::RunAgents(_) => self
                .run_agents_executor
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
                ctx.emit(BlocklistAIActionExecutorEvent::ExecutingAction {
                    action_id: action_id.clone(),
                });
                ctx.emit(BlocklistAIActionExecutorEvent::FinishedAction {
                    result: Arc::new(AIAgentActionResult {
                        id: action_id,
                        task_id: action.task_id,
                        result: action_result,
                    }),
                    conversation_id,
                    cancellation_reason: None,
                });
                TryExecuteResult::ExecutedSync
            }
        }
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
            }
            ctx.emit(BlocklistAIActionExecutorEvent::FinishedAction {
                result: Arc::new(AIAgentActionResult {
                    id: running.action.id.clone(),
                    task_id: running.action.task_id,
                    result: running.action.action.cancelled_result(),
                }),
                conversation_id: running.conversation_id,
                cancellation_reason: reason,
            });
        }
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

    /// Dispatches a `RunAgents` action with a user-edited request
    /// (from the confirmation card's Accept handler).
    pub fn execute_run_agents(
        &mut self,
        action_id: AIAgentActionId,
        request: ai::agent::action::RunAgentsRequest,
        conversation_id: AIConversationId,
        task_id: TaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.is_shared_session_viewer() {
            log::warn!("RunAgents dispatch attempted in shared-session-viewer mode; ignoring");
            return;
        }
        if self.async_executing_actions.contains_key(&action_id) {
            log::warn!("RunAgents dispatch reentered for {action_id:?}; ignoring");
            return;
        }

        let receiver = self.run_agents_executor.update(ctx, |executor, exec_ctx| {
            executor.dispatch_run_agents(
                action_id.clone(),
                request.clone(),
                conversation_id,
                exec_ctx,
            )
        });

        // Synthesize an AIAgentAction for cancellation and task_id
        // plumbing.
        let action = AIAgentAction {
            id: action_id.clone(),
            task_id,
            action: AIAgentActionType::RunAgents(request),
            requires_result: true,
        };
        self.async_executing_actions.insert(
            action_id.clone(),
            AsyncExecutingAction {
                action,
                conversation_id,
            },
        );
        ctx.emit(BlocklistAIActionExecutorEvent::ExecutingAction {
            action_id: action_id.clone(),
        });

        ctx.spawn(
            async move { receiver.recv().await },
            move |me, result, ctx| {
                let Some(running) = me.async_executing_actions.remove(&action_id) else {
                    return;
                };
                let result_type = match result {
                    Ok(r) => AIAgentActionResultType::RunAgents(r),
                    Err(_) => AIAgentActionResultType::RunAgents(
                        ai::agent::action_result::RunAgentsResult::Cancelled,
                    ),
                };
                ctx.emit(BlocklistAIActionExecutorEvent::FinishedAction {
                    result: Arc::new(AIAgentActionResult {
                        id: action_id,
                        task_id: running.action.task_id,
                        result: result_type,
                    }),
                    conversation_id: running.conversation_id,
                    cancellation_reason: None,
                });
            },
        );
    }

    fn should_autoexecute(&self, input: ExecuteActionInput, ctx: &mut ModelContext<Self>) -> bool {
        match input.action.action {
            AIAgentActionType::RequestCommandOutput { .. }
            | AIAgentActionType::WriteToLongRunningShellCommand { .. }
            | AIAgentActionType::ReadShellCommandOutput { .. }
            | AIAgentActionType::TransferShellCommandControlToUser { .. } => self
                .shell_command_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::ReadFiles(_) => self
                .read_files_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::UploadArtifact(_) => self
                .upload_artifact_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::SearchCodebase(_) => self
                .search_codebase_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::RequestFileEdits { .. } => self
                .request_file_edits_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::Grep { .. } => self
                .grep_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::FileGlob { .. } | AIAgentActionType::FileGlobV2 { .. } => self
                .file_glob_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
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
            AIAgentActionType::RequestComputerUse(_) => self
                .request_computer_use_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::ReadSkill(_) => self
                .read_skill_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::FetchConversation { .. } => self
                .fetch_conversation_executor
                .update(ctx, |executor, ctx| executor.should_autoexecute(input, ctx)),
            AIAgentActionType::StartAgent { .. } => self
                .start_agent_executor
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

    /// Expected absolute paths of requested files that did not exist or could
    /// not be read (e.g. binary files that exceed the size limit).
    pub missing_files: Vec<String>,
}

/// Reads the content of the given files at the given `FileLocations`.
///
/// If any files do not exist, they are included in the `missing_files` field of the result.
///
/// Binary files larger than the per-file byte limit are skipped and reported as oversized.
/// Text files are truncated at the per-file limit via line streaming.
/// If `max_file_bytes` is provided, it overrides the default per-file limit
/// ([`MAX_FILE_READ_BYTES`]). Pass `None` to use the default.
/// If `max_batch_bytes` is provided, the cumulative content of all files is capped at that
/// budget; once exceeded, remaining files are reported as oversized.
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
            missing_files: Vec::new(),
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
                    result
                        .missing_files
                        .push(absolute_file_path.to_string_lossy().to_string());
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
                BinaryFileReadResult::Missing => result.missing_files.push(path_str),
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

#[cfg(feature = "local_fs")]
enum BinaryFileReadResult {
    /// Successfully read as binary.
    Context {
        file_context: FileContext,
        bytes_read: usize,
    },
    /// File doesn't exist, exceeds the size limit, or couldn't be processed.
    Missing,
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
        return Ok(BinaryFileReadResult::Missing);
    }

    let content = match read_file_as_binary(path).await {
        Ok(content) => content,
        Err(FileLoadError::DoesNotExist) => return Ok(BinaryFileReadResult::Missing),
        Err(FileLoadError::IOError(e)) => return Err(anyhow::anyhow!(e)),
    };

    let mime_type = from_path(path).first_or_octet_stream().to_string();
    let processed_content = if is_supported_image_mime_type(&mime_type) {
        match process_image_for_agent(&content) {
            ProcessImageResult::Success { data } => Some(data),
            ProcessImageResult::TooLarge => {
                log::warn!("Image file too large after processing: {}", path.display());
                return Ok(BinaryFileReadResult::Missing);
            }
            ProcessImageResult::Error(err) => {
                log::warn!("Error processing image file {}: {err:?}", path.display());
                return Ok(BinaryFileReadResult::Missing);
            }
        }
    } else {
        None
    };

    let final_content = processed_content.unwrap_or(content);
    if final_content.len() > max_bytes {
        return Ok(BinaryFileReadResult::Missing);
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

/// Returns true if the given path is a regular file on the session's filesystem.
/// Runs a shell command on the session so it works for both local and remote sessions.
async fn is_file_path(path: &str, session: &Session) -> bool {
    let command = if session.shell().shell_type() == ShellType::PowerShell {
        format!("if (Test-Path -PathType Leaf \"{path}\") {{ exit 0 }} else {{ exit 1 }}")
    } else {
        format!("test -f \"{path}\"")
    };
    session
        .execute_command(&command, None, None, ExecuteCommandOptions::default())
        .await
        .map(|output| output.success())
        .unwrap_or(false)
}

/// Returns true if git is installed and the given path is in a git repository.
async fn is_git_repository(absolute_path: &str, session: &Session) -> anyhow::Result<bool> {
    let git_command = format!("git -C \"{absolute_path}\" rev-parse");
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
