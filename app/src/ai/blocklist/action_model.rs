//! The `BlocklistAIActionModel` is responsible for managing state related to `AIAgentAction`s
//! received in AI responses.
//!
//! Notably, this model manages the "action queue", which is used to support receiving multiple
//! actions in a single AI response.
//!
//! Actions are executed, one by one, either initiated by the user or auto-executed, if the user's
//! AI permissions permit. Action execution is handled by `BlocklistAIActionExecutor`, which
//! consumes the action to be executed and emits an event when execution is complete.
//!
//! Action state also has indirect implications for various parts of the terminal UI -- for
//! example, the input should be hidden if there is a pending AI requested command that requires
//! action from the user.

mod execute;
mod preprocess;
mod scheduler;
mod tool_action_model;

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Local;
pub(crate) use execute::{
    apply_edits, coerce_integer_args, ActionExecution, AgentToolExecutionContext,
    AgentToolExecutor, AnyActionExecution, ExecuteActionInput, FileReadResult,
    MalformedFinalLineProxyEvent, PreprocessActionInput, SurfaceSpecificToolExecutor,
};
#[cfg(test)]
pub(crate) use execute::{compose_run_agents_child_prompt, run_agents_to_start_agent_mode};
pub use execute::{
    read_local_file_context, EditAcceptAndContinueClickedEvent, EditAcceptClickedEvent,
    EditResolvedEvent, EditStats, NewConversationDecision, PromptSuggestionExecutor,
    ReadFileContextResult, RequestFileEditsExecutor, RequestFileEditsFormatKind,
    RequestFileEditsTelemetryEvent, RunAgentsExecutor, RunAgentsExecutorEvent,
    RunAgentsSpawningSnapshot, ShellCommandExecutor, ShellCommandExecutorEvent, StartAgentExecutor,
    StartAgentExecutorEvent, StartAgentRequest, StartAgentRequestId,
};
use futures::future::{join_all, BoxFuture};
use itertools::Itertools;
use parking_lot::FairMutex;
use scheduler::StartedAction;
pub(crate) use scheduler::{AgentToolScheduleHost, AgentToolScheduler};
pub(crate) use tool_action_model::AgentToolActionModel;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use self::execute::ask_user_question::AskUserQuestionExecutor;
use self::execute::search_codebase::SearchCodebaseExecutor;
use self::execute::{BlocklistAIActionExecutor, BlocklistAIActionExecutorEvent, NotExecutedReason};
pub(crate) use self::execute::{RunningActionPhase, TryExecuteResult};
use super::BlocklistAIHistoryModel;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AIAgentActionTypeDiscriminants, AIAgentExchange, CancellationReason,
    CreateDocumentsResult, EditDocumentsResult,
};
use crate::ai::ai_document_view::DEFAULT_PLANNING_DOCUMENT_TITLE;
use crate::ai::blocklist::action_model::execute::suggest_new_conversation::SuggestNewConversationExecutor;
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::TerminalModel;
use crate::{send_telemetry_from_ctx, TelemetryEvent};

/// The status of an action from an AI output.
#[derive(Clone, Debug)]
pub enum AIActionStatus {
    /// The action is preprocessing and has yet to be started.
    Preprocessing,

    /// The action is queued, but isn't yet actionable by the user (there is another action that
    /// was queued prior that the user must act on first).
    Queued,

    // The action is next up for execution, but is blocked by the completion of another action
    // and/or user confirmation.
    Blocked,

    /// The action is running asynchronously.
    ///
    /// This is never the status for actions that are executed synchronously.
    RunningAsync,

    /// The action has either been cancelled or completed.
    Finished(Arc<AIAgentActionResult>),
}

impl AIActionStatus {
    /// Returns whether the action is currently preprocessing.
    pub fn is_preprocessing(&self) -> bool {
        matches!(self, AIActionStatus::Preprocessing)
    }

    pub fn is_queued(&self) -> bool {
        matches!(self, AIActionStatus::Queued)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, AIActionStatus::Blocked)
    }

    pub fn is_done(&self) -> bool {
        matches!(self, AIActionStatus::Finished(..))
    }

    pub fn is_running(&self) -> bool {
        matches!(self, AIActionStatus::RunningAsync)
    }

    pub fn is_success(&self) -> bool {
        let AIActionStatus::Finished(result) = self else {
            return false;
        };
        result.result.is_successful()
    }

    pub fn is_failed(&self) -> bool {
        let AIActionStatus::Finished(result) = self else {
            return false;
        };
        result.result.is_failed()
    }

    pub fn is_cancelled(&self) -> bool {
        let AIActionStatus::Finished(result) = self else {
            return false;
        };
        result.result.is_cancelled()
    }

    pub fn is_cancelled_during_requested_command_execution(&self) -> bool {
        let AIActionStatus::Finished(result) = self else {
            return false;
        };
        result
            .result
            .is_cancelled_during_requested_command_execution()
    }

    pub fn finished_result(&self) -> Option<&AIAgentActionResult> {
        let AIActionStatus::Finished(result) = self else {
            return None;
        };
        Some(result.as_ref())
    }
}

#[derive(Debug, Clone)]
struct RunningActions {
    /// The execution phase for this batch of actions.
    phase: RunningActionPhase,

    /// The specific action IDs still running within the phase.
    /// If the phase is serial, there is only at most one action in here.
    /// For parallel phases, there can be several action IDs present at once,
    /// or there can be 0 or 1 actions; actions are added and removed as
    /// they are produced and completed, respectively.
    action_ids: Vec<AIAgentActionId>,
}

impl RunningActions {
    fn new(phase: RunningActionPhase, action_id: AIAgentActionId) -> Self {
        Self {
            phase,
            action_ids: vec![action_id],
        }
    }

    fn add_action(&mut self, action_id: AIAgentActionId) {
        self.action_ids.push(action_id);
    }

    fn remove_action(&mut self, action_id: &AIAgentActionId) {
        self.action_ids.retain(|id| id != action_id);
    }

    fn contains(&self, action_id: &AIAgentActionId) -> bool {
        self.action_ids.iter().any(|id| id == action_id)
    }

    fn first_action_id(&self) -> Option<&AIAgentActionId> {
        self.action_ids.first()
    }

    fn is_empty(&self) -> bool {
        self.action_ids.is_empty()
    }
}

pub struct BlocklistAIActionModel {
    executor: ModelHandle<BlocklistAIActionExecutor>,
    tools: AgentToolActionModel,

    /// The ID of the terminal view this controller is associated with.
    terminal_view_id: EntityId,

    /// In view-only mode, we never block on user acceptance and avoid any interactive controls.
    /// This is used for agent session sharing to avoid any tools blocking on the viewer's acceptance.
    is_view_only: bool,

    /// The ID of the ambient agent task which owns this action model, if any.
    ambient_agent_task_id: Option<crate::ai::ambient_agents::AmbientAgentTaskId>,
}

impl BlocklistAIActionModel {
    pub fn new(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        active_session: ModelHandle<ActiveSession>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let executor = ctx.add_model(|ctx| {
            BlocklistAIActionExecutor::new(
                terminal_model,
                active_session.clone(),
                model_event_dispatcher,
                get_relevant_files_controller,
                terminal_view_id,
                ctx,
            )
        });
        ctx.subscribe_to_model(&executor, move |me, event, ctx| match event {
            BlocklistAIActionExecutorEvent::ExecutingAction { action_id } => {
                ctx.emit(BlocklistAIActionEvent::ExecutingAction(action_id.clone()));
            }
            BlocklistAIActionExecutorEvent::FinishedAction {
                result,
                conversation_id,
                cancellation_reason,
            } => AgentToolScheduler::finish_action(
                me,
                *conversation_id,
                result.clone(),
                *cancellation_reason,
                ctx,
            ),
            BlocklistAIActionExecutorEvent::InitProject(id) => {
                ctx.emit(BlocklistAIActionEvent::InitProject(id.clone()))
            }
            BlocklistAIActionExecutorEvent::OpenCodeReview(id) => {
                ctx.emit(BlocklistAIActionEvent::ToggleCodeReview(id.clone()))
            }
            BlocklistAIActionExecutorEvent::InsertCodeReviewComments {
                action_id,
                repo_path,
                comments,
                base_branch,
            } => {
                ctx.emit(BlocklistAIActionEvent::InsertCodeReviewComments {
                    action_id: action_id.clone(),
                    repo_path: repo_path.clone(),
                    comments: comments.clone(),
                    base_branch: base_branch.clone(),
                });
            }
        });

        Self {
            executor,
            tools: AgentToolActionModel::new(),
            terminal_view_id,
            is_view_only: false,
            ambient_agent_task_id: None,
        }
    }

    /// Enable or disable view-only mode (for use in agent session sharing).
    pub fn set_view_only(&mut self, is_view_only: bool) {
        self.is_view_only = is_view_only;
    }

    /// Marks an action as remotely executing on the viewer side.
    /// This is called when a viewer receives a CommandExecutionStarted event from the sharer,
    /// allowing the viewer's UI to show the action as running even though it's not executing locally.
    pub fn mark_action_as_remotely_executing(
        &mut self,
        action_id: &AIAgentActionId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Only applicable for viewers
        if !self.is_view_only {
            return;
        }

        // Remove the action from pending_actions for the specific conversation
        // so that we can correctly show the command as running.
        if let Some(pending_actions) = self.tools.pending_actions.get_mut(&conversation_id) {
            pending_actions.retain(|a| &a.id != action_id);
        }

        self.tools.record_running_action(
            conversation_id,
            action_id.clone(),
            RunningActionPhase::Serial,
        );
        ctx.emit(BlocklistAIActionEvent::ExecutingAction(action_id.clone()));
    }

    /// Returns true if the action model is operating in view-only mode (used for shared-session viewers).
    pub fn is_view_only(&self) -> bool {
        self.is_view_only
    }

    pub fn shell_command_executor(&self, app: &AppContext) -> ModelHandle<ShellCommandExecutor> {
        self.executor.as_ref(app).shell_command_executor().clone()
    }

    pub fn suggest_new_conversation_executor(
        &self,
        app: &AppContext,
    ) -> ModelHandle<SuggestNewConversationExecutor> {
        self.executor
            .as_ref(app)
            .suggest_new_conversation_executor()
            .clone()
    }

    pub fn request_file_edits_executor(
        &self,
        app: &AppContext,
    ) -> ModelHandle<RequestFileEditsExecutor> {
        self.executor
            .as_ref(app)
            .request_file_edits_executor()
            .clone()
    }

    pub fn search_codebase_executor<'a>(
        &'a self,
        app: &'a AppContext,
    ) -> &'a ModelHandle<SearchCodebaseExecutor> {
        self.executor.as_ref(app).search_codebase_executor()
    }

    pub fn suggest_prompt_executor(
        &self,
        app: &AppContext,
    ) -> ModelHandle<PromptSuggestionExecutor> {
        self.executor.as_ref(app).suggest_prompt_executor().clone()
    }

    pub fn start_agent_executor(&self, app: &AppContext) -> ModelHandle<StartAgentExecutor> {
        self.executor.as_ref(app).start_agent_executor().clone()
    }

    pub fn run_agents_executor(&self, app: &AppContext) -> ModelHandle<RunAgentsExecutor> {
        self.executor.as_ref(app).run_agents_executor().clone()
    }

    pub fn ask_user_question_executor(
        &self,
        app: &AppContext,
    ) -> ModelHandle<AskUserQuestionExecutor> {
        self.executor
            .as_ref(app)
            .ask_user_question_executor()
            .clone()
    }

    pub fn set_ambient_agent_task_id(
        &mut self,
        id: Option<crate::ai::ambient_agents::AmbientAgentTaskId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.ambient_agent_task_id = id;
        self.executor.update(ctx, |executor, ctx| {
            executor.set_ambient_agent_task_id(id, ctx);
        });
    }

    /// Returns all pending actions for all conversations.
    pub fn get_pending_actions(&self) -> Vec<&AIAgentAction> {
        self.tools.get_pending_actions()
    }

    /// Returns all pending actions for a specific conversation.
    pub fn get_pending_actions_for_conversation(
        &self,
        conversation_id: &AIConversationId,
    ) -> impl Iterator<Item = &AIAgentAction> {
        self.tools
            .get_pending_actions_for_conversation(conversation_id)
    }

    /// Returns the next pending action
    pub fn get_pending_action(&self, app: &AppContext) -> Option<&AIAgentAction> {
        let conversation_id = self.active_conversation_id(app)?;
        self.tools.blocked_action_for_conversation(&conversation_id)
    }

    /// Returns a pending action by its ID, searching across all conversations.
    pub fn get_pending_action_by_id(&self, action_id: &AIAgentActionId) -> Option<&AIAgentAction> {
        self.tools.get_pending_action_by_id(action_id)
    }

    /// Returns the next pending or running action ID, for the active conversation, if any.
    pub fn get_pending_or_running_action_id<'a>(
        &'a self,
        app: &'a AppContext,
    ) -> Option<&'a AIAgentActionId> {
        let conversation_id = self.active_conversation_id(app)?;
        self.tools
            .blocked_action_for_conversation(&conversation_id)
            .map(|action| &action.id)
            .or_else(|| {
                self.tools
                    .running_actions
                    .get(&conversation_id)
                    .and_then(RunningActions::first_action_id)
            })
    }

    /// Returns one of the currently asynchronously-executing actions, if any.
    ///
    /// When multiple actions run in parallel, only the first is returned. This is
    /// sufficient for callers that need a single status indicator (e.g., "Searching
    /// codebase...") or just need to know whether *something* is running.
    pub fn get_async_running_action<'a>(
        &'a self,
        app: &'a AppContext,
    ) -> Option<&'a AIAgentAction> {
        let conversation_id = self.active_conversation_id(app)?;
        self.tools
            .running_actions
            .get(&conversation_id)
            .and_then(RunningActions::first_action_id)
            .and_then(|action_id| self.executor.as_ref(app).async_executing_action(action_id))
    }

    /// Returns whether there is a pending or running action for the active conversation.
    pub fn has_unfinished_actions(&self, app: &AppContext) -> bool {
        let Some(conversation_id) = self.active_conversation_id(app) else {
            return false;
        };
        self.tools
            .has_unfinished_actions_for_conversation(conversation_id)
    }

    pub fn has_unfinished_actions_for_conversation(
        &self,
        conversation_id: AIConversationId,
    ) -> bool {
        self.tools
            .has_unfinished_actions_for_conversation(conversation_id)
    }

    /// Returns finished action results received from the most recent AI output for the active conversation.
    pub fn get_finished_action_results(
        &self,
        conversation_id: AIConversationId,
    ) -> Option<&Vec<Arc<AIAgentActionResult>>> {
        self.tools.get_finished_action_results(conversation_id)
    }

    /// Returns the `AIActionStatus` for the action corresponding to the given `id`, if any.
    pub fn get_action_status(&self, id: &AIAgentActionId) -> Option<AIActionStatus> {
        self.tools.get_action_status(id, self.is_view_only)
    }

    pub fn get_action_result(&self, id: &AIAgentActionId) -> Option<&Arc<AIAgentActionResult>> {
        self.tools.get_action_result(id)
    }

    /// Bulk restore action results from a list of exchanges (used when loading conversations from tasks)
    pub fn restore_action_results_from_exchanges(&mut self, exchanges: Vec<&AIAgentExchange>) {
        self.tools.restore_action_results_from_exchanges(exchanges);
    }

    /// Dispatches a `RunAgents` action with the user-edited request
    /// from the confirmation card.
    pub fn execute_run_agents(
        &mut self,
        action_id: &AIAgentActionId,
        request: ai::agent::action::RunAgentsRequest,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut found = None;
        for (conv_id, queue) in self.tools.pending_actions.iter_mut() {
            if let Some(action) = queue.iter_mut().find(|action| &action.id == action_id) {
                found = Some((*conv_id, action));
                break;
            }
        }
        let Some((conversation_id, action)) = found else {
            log::warn!(
                "BlocklistAIActionModel::execute_run_agents: no pending action for {action_id:?}"
            );
            return;
        };
        if !matches!(action.action, AIAgentActionType::RunAgents(_)) {
            log::warn!(
                "BlocklistAIActionModel::execute_run_agents: pending action {action_id:?} is not RunAgents"
            );
            return;
        }
        action.action = AIAgentActionType::RunAgents(request);
        self.execute_action(action_id, conversation_id, ctx);
    }

    /// Removes a pending `RunAgents` action and records a `Denied`
    /// result. Used when the orchestration config is disapproved at
    /// the time the action becomes blocked on user confirmation.
    pub fn deny_run_agents(
        &mut self,
        action_id: &AIAgentActionId,
        reason: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut found: Option<(AIConversationId, AIAgentAction)> = None;
        for (conv_id, queue) in self.tools.pending_actions.iter_mut() {
            if let Some(idx) = queue.iter().position(|a| &a.id == action_id) {
                if let Some(action) = queue.remove(idx) {
                    found = Some((*conv_id, action));
                }
                break;
            }
        }
        let Some((conversation_id, action)) = found else {
            log::warn!(
                "BlocklistAIActionModel::deny_run_agents: no pending action for {action_id:?}"
            );
            return;
        };
        let result = Arc::new(AIAgentActionResult {
            id: action.id,
            task_id: action.task_id,
            result: AIAgentActionResultType::RunAgents(
                ai::agent::action_result::RunAgentsResult::Denied { reason },
            ),
        });
        AgentToolScheduler::finish_action(self, conversation_id, result, None, ctx);
    }

    /// Attempts to execute the next pending action for the active conversation.
    pub fn execute_next_action_for_user(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(pending_action_id) = self
            .tools
            .pending_actions
            .get(&conversation_id)
            .and_then(|queue| queue.front())
            .map(|action| action.id.clone())
        else {
            return;
        };

        if AgentToolScheduler::start_pending_action_by_id(
            self,
            &pending_action_id,
            conversation_id,
            true,
            ctx,
        )
        .is_some_and(|result| matches!(result, StartedAction::Sync))
        {
            AgentToolScheduler::try_to_execute_available_actions(self, conversation_id, ctx);
        }
    }

    /// Attempts to execute the pending action with the given `action_id` for the given conversation.
    pub fn execute_action(
        &mut self,
        action_id: &AIAgentActionId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if AgentToolScheduler::start_pending_action_by_id(
            self,
            action_id,
            conversation_id,
            true,
            ctx,
        )
        .is_some_and(|result| matches!(result, StartedAction::Sync))
        {
            AgentToolScheduler::try_to_execute_available_actions(self, conversation_id, ctx);
        }
    }

    /// Gets the active conversation ID for this terminal view.
    fn active_conversation_id(&self, app: &AppContext) -> Option<AIConversationId> {
        BlocklistAIHistoryModel::as_ref(app).active_conversation_id(self.terminal_view_id)
    }

    fn update_conversation_in_progress_status(
        &self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
            history_model.update_conversation_status(
                self.terminal_view_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
        });
    }

    fn handle_not_executed_action(
        &self,
        action: &AIAgentAction,
        reason: NotExecutedReason,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if reason.needs_confirmation() {
            ctx.emit(BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(
                action.id.clone(),
            ));
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                let blocked_action_user_friendly_str = action.action.user_friendly_name();
                history_model.update_conversation_status(
                    self.terminal_view_id,
                    conversation_id,
                    ConversationStatus::Blocked {
                        blocked_action: format!("{blocked_action_user_friendly_str:?}"),
                    },
                    ctx,
                );
            });
        }
    }

    /// Queues the `actions` in the given iterator for the given conversation,
    /// to be dispatched in the order in which they appear in the iterator.
    pub(super) fn queue_actions(
        &mut self,
        actions: Vec<AIAgentAction>,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        AgentToolScheduler::queue_actions(self, actions, conversation_id, ctx);
    }

    /// Apply a finished action result to the conversation.
    /// This is used in agent session sharing to apply finished action results
    /// received from the action stream.
    pub fn apply_finished_action_result(
        &mut self,
        conversation_id: AIConversationId,
        mut action_result: AIAgentActionResult,
        ctx: &mut ModelContext<Self>,
    ) {
        let action_id = action_result.id.clone();
        if let Some(queue) = self.tools.pending_actions.get_mut(&conversation_id) {
            if let Some(idx) = queue.iter().position(|a| a.id == action_id) {
                queue.remove(idx);
            }
        }

        // For shared session viewers, take in any document action results
        // and apply the associated actions to the local document version
        // (or create a new document if the given doc does not exist).
        self.maybe_sync_view_only_documents_with_local_model(
            conversation_id,
            &mut action_result,
            ctx,
        );

        AgentToolScheduler::finish_action(
            self,
            conversation_id,
            Arc::new(action_result),
            None,
            ctx,
        );
    }

    pub(super) fn cancel_action_with_id(
        &mut self,
        conversation_id: AIConversationId,
        action_id: &AIAgentActionId,
        reason: CancellationReason,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .tools
            .running_actions
            .get(&conversation_id)
            .is_some_and(|running| running.contains(action_id))
        {
            self.executor.update(ctx, |executor, ctx| {
                executor.cancel_running_async_action(action_id, Some(reason), ctx)
            });
        } else {
            let Some(pending_actions_for_conversation) =
                self.tools.pending_actions.get_mut(&conversation_id)
            else {
                return;
            };
            if let Some((idx, _)) = pending_actions_for_conversation
                .iter()
                .find_position(|action| action.id == *action_id)
            {
                if let Some(action) = pending_actions_for_conversation.remove(idx) {
                    self.cancel_pending_action(conversation_id, action, Some(reason), ctx);
                }
            }
        }
    }

    /// Cancels any in-flight WaitForEvents action for the given conversation.
    pub fn cancel_wait_for_events_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let action_id = self.executor.update(ctx, |executor, _| {
            executor.find_running_wait_for_events(conversation_id)
        });
        if let Some(action_id) = action_id {
            self.cancel_action_with_id(
                conversation_id,
                &action_id,
                CancellationReason::FollowUpSubmitted {
                    is_for_same_conversation: true,
                },
                ctx,
            );
        }
    }

    pub(super) fn cancel_all_pending_actions(
        &mut self,
        conversation_id: AIConversationId,
        reason: Option<CancellationReason>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.executor.update(ctx, |executor, ctx| {
            executor.cancel_all_running_async_actions_for_conversation(conversation_id, reason, ctx)
        });

        let Some(actions_to_cancel) = self.tools.pending_actions.get_mut(&conversation_id) else {
            return;
        };
        for action in actions_to_cancel.drain(..).collect_vec() {
            log::info!(
                "Canceling pending action of type {:?} conversation_id={conversation_id:?} action_id={:?}, reason={:?}, backtrace=\n{}",
                AIAgentActionTypeDiscriminants::from(&action.action),
                action.id,
                reason,
                std::backtrace::Backtrace::force_capture()
            );
            self.cancel_pending_action(conversation_id, action, reason, ctx);
        }
    }

    fn cancel_pending_action(
        &mut self,
        conversation_id: AIConversationId,
        pending_action: AIAgentAction,
        reason: Option<CancellationReason>,
        ctx: &mut ModelContext<Self>,
    ) {
        if matches!(
            pending_action.action,
            AIAgentActionType::RequestComputerUse(_)
        ) {
            let server_conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .and_then(|c| c.server_conversation_token())
                .map(|t| t.as_str().to_string());
            send_telemetry_from_ctx!(
                TelemetryEvent::ComputerUseCancelled {
                    client_conversation_id: conversation_id,
                    server_conversation_id,
                    ambient_agent_task_id: self.ambient_agent_task_id,
                },
                ctx
            );
        }

        let result = Arc::new(AIAgentActionResult {
            id: pending_action.id,
            task_id: pending_action.task_id,
            result: pending_action.action.cancelled_result(),
        });
        AgentToolScheduler::finish_action(self, conversation_id, result, reason, ctx);
    }

    /// Returns all finished action results from the given conversation, moving them to the
    /// `past_action_results` in the process.
    pub(super) fn drain_finished_action_results(
        &mut self,
        conversation_id: AIConversationId,
    ) -> Vec<AIAgentActionResult> {
        self.tools.drain_finished_results(conversation_id)
    }

    /// Clears finished action results for a conversation. Used when reverting.
    pub(super) fn clear_finished_action_results(&mut self, conversation_id: AIConversationId) {
        self.tools.action_order.remove(&conversation_id);
        self.tools.finished_action_results.remove(&conversation_id);
    }

    /// The control flow for initiating cancellations across suggested plans, requested commands,
    /// and code diff views are identical, and thus should be handled directly by the [`AIBlock`]'s
    /// respective functions.
    pub fn handle_requested_command_accepted(
        &mut self,
        action_id: &AIAgentActionId,
        command: String,
        ctx: &mut ModelContext<Self>,
    ) {
        // Search through all pending conversations to find the action and conversation ID
        let mut found_conversation_id = None;
        for (conversation_id, pending_actions_for_conversation) in
            self.tools.pending_actions.iter_mut()
        {
            if let Some(action) = pending_actions_for_conversation
                .iter_mut()
                .find(|action| action.id == *action_id)
            {
                if let AIAgentActionType::RequestCommandOutput {
                    command: original_command,
                    ..
                } = &mut action.action
                {
                    *original_command = command;
                    found_conversation_id = Some(*conversation_id);
                    break;
                }
            }
        }

        let Some(conversation_id) = found_conversation_id else {
            debug_assert!(false, "Expected action to be requested command.");
            return;
        };

        self.execute_action(action_id, conversation_id, ctx);
    }

    /// In shared-session viewer (view-only) mode, ensure document-related action results
    /// are backed by documents in the local `AIDocumentModel` and that their
    /// `DocumentContext` versions match. For CreateDocuments, restore missing documents
    /// (using titles from the original action); for EditDocuments, apply edits to local
    /// documents and align versions, so headers and "View" buttons stay accurate.
    fn maybe_sync_view_only_documents_with_local_model(
        &self,
        conversation_id: AIConversationId,
        result: &mut AIAgentActionResult,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.is_view_only {
            return;
        }

        match &mut result.result {
            AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Success {
                created_documents,
            }) => {
                let history = BlocklistAIHistoryModel::handle(ctx);
                let Some(conversation) = history.as_ref(ctx).conversation(&conversation_id) else {
                    return;
                };
                let titles = conversation.get_document_titles_for_action(&result.id);

                let doc_model = AIDocumentModel::handle(ctx);
                doc_model.update(ctx, |doc_model, doc_ctx| {
                    for (index, doc_context) in created_documents.iter_mut().enumerate() {
                        // If a user is re-opening a shared session that they previously closed in the current warp session,
                        // we should delete the previously created document so that the verseion history doesn't get messed up.
                        doc_model.delete_document(&doc_context.document_id);

                        let title = titles
                            .as_ref()
                            .and_then(|t| t.get(index))
                            .cloned()
                            .unwrap_or_else(|| DEFAULT_PLANNING_DOCUMENT_TITLE.to_string());

                        doc_model.restore_document(
                            doc_context.document_id,
                            conversation_id,
                            &title,
                            doc_context.content.clone(),
                            Local::now(),
                            doc_ctx,
                        );
                    }
                });
            }
            AIAgentActionResultType::EditDocuments(EditDocumentsResult::Success {
                updated_documents,
            }) => {
                let doc_model = AIDocumentModel::handle(ctx);
                doc_model.update(ctx, |doc_model, doc_ctx| {
                    for doc_context in updated_documents.iter_mut() {
                        if doc_model
                            .get_current_document(&doc_context.document_id)
                            .is_none()
                        {
                            // You can't make edits to a doc that does not exist.
                            continue;
                        }

                        if let Some(new_version) = doc_model.restore_document_edit(
                            &doc_context.document_id,
                            doc_context.content.clone(),
                            Local::now(),
                            doc_ctx,
                        ) {
                            // Align the header's version with the locally restored doc
                            // so the viewer sees the correct bumped version.
                            doc_context.document_version = new_version;
                        }
                    }
                });
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
pub enum BlocklistAIActionEvent {
    /// Emitted when the action with the given ID is enqueued for execution.
    QueuedAction(AIAgentActionId),
    /// Emitted when the action with the given ID requires user confirmation to execute.
    ActionBlockedOnUserConfirmation(AIAgentActionId),
    /// Emitted when the action with the given ID begins execution.
    ExecutingAction(AIAgentActionId),
    /// Emitted when the action with the given ID has finished.
    FinishedAction {
        action_id: AIAgentActionId,
        conversation_id: AIConversationId,
        cancellation_reason: Option<CancellationReason>,
    },
    InitProject(AIAgentActionId),
    ToggleCodeReview(AIAgentActionId),
    InsertCodeReviewComments {
        action_id: AIAgentActionId,
        repo_path: PathBuf,
        comments: Vec<ai::agent::action::InsertReviewComment>,
        base_branch: Option<String>,
    },
}

impl BlocklistAIActionEvent {
    pub fn action_id(&self) -> &AIAgentActionId {
        match self {
            BlocklistAIActionEvent::QueuedAction(action_id) => action_id,
            BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(action_id) => action_id,
            BlocklistAIActionEvent::ExecutingAction(action_id) => action_id,
            BlocklistAIActionEvent::FinishedAction { action_id, .. } => action_id,
            BlocklistAIActionEvent::InitProject(action_id) => action_id,
            BlocklistAIActionEvent::ToggleCodeReview(action_id) => action_id,
            BlocklistAIActionEvent::InsertCodeReviewComments { action_id, .. } => action_id,
        }
    }
}

impl Entity for BlocklistAIActionModel {
    type Event = BlocklistAIActionEvent;
}

impl AgentToolScheduleHost for BlocklistAIActionModel {
    type Context<'a> = ModelContext<'a, Self>;

    fn app_context<'a, 'b>(ctx: &'a Self::Context<'b>) -> &'a AppContext {
        ctx
    }

    fn tools(&mut self) -> &mut AgentToolActionModel {
        &mut self.tools
    }

    fn tools_ref(&self) -> &AgentToolActionModel {
        &self.tools
    }

    fn preprocess(
        &mut self,
        action: &AIAgentAction,
        conversation_id: AIConversationId,
        ctx: &mut Self::Context<'_>,
    ) -> BoxFuture<'static, ()> {
        self.executor.update(ctx, |e, ctx| {
            e.preprocess_action(action, conversation_id, ctx)
        })
    }

    fn try_execute(
        &mut self,
        action: AIAgentAction,
        conversation_id: AIConversationId,
        is_user_initiated: bool,
        ctx: &mut Self::Context<'_>,
    ) -> TryExecuteResult {
        self.executor.update(ctx, |e, ctx| {
            e.try_to_execute_action(action, conversation_id, is_user_initiated, ctx)
        })
    }

    fn can_autoexecute(
        &mut self,
        action: &AIAgentAction,
        conversation_id: AIConversationId,
        ctx: &mut Self::Context<'_>,
    ) -> bool {
        self.executor.update(ctx, |e, ctx| {
            e.can_autoexecute_action(action, conversation_id, ctx)
        })
    }

    fn action_phase(&self, action: &AIAgentAction, ctx: &AppContext) -> RunningActionPhase {
        self.executor.as_ref(ctx).action_phase(action, ctx)
    }

    fn spawn_after_preprocess(
        &mut self,
        futures: Vec<BoxFuture<'static, ()>>,
        ctx: &mut Self::Context<'_>,
        then: impl FnOnce(&mut Self, &mut Self::Context<'_>) + 'static,
    ) {
        ctx.spawn(join_all(futures), move |me, _, ctx| then(me, ctx));
    }

    fn should_enqueue(
        &self,
        conversation_id: AIConversationId,
        action_id: &AIAgentActionId,
        _ctx: &AppContext,
    ) -> bool {
        // In view-only mode, skip actions already marked as running (can happen if
        // CommandExecutionStarted arrives before the action is queued).
        !(self.is_view_only
            && self
                .tools
                .running_actions
                .get(&conversation_id)
                .is_some_and(|r| r.contains(action_id)))
    }

    fn on_action_enqueued(
        &mut self,
        _conversation_id: AIConversationId,
        action_id: &AIAgentActionId,
        ctx: &mut Self::Context<'_>,
    ) {
        ctx.emit(BlocklistAIActionEvent::QueuedAction(action_id.clone()));
    }

    fn on_action_started(
        &mut self,
        conversation_id: AIConversationId,
        is_wait_for_events: bool,
        ctx: &mut Self::Context<'_>,
    ) {
        if !is_wait_for_events {
            self.update_conversation_in_progress_status(conversation_id, ctx);
        }
    }

    fn on_action_not_executed(
        &mut self,
        action: &AIAgentAction,
        reason: NotExecutedReason,
        conversation_id: AIConversationId,
        ctx: &mut Self::Context<'_>,
    ) {
        self.handle_not_executed_action(action, reason, conversation_id, ctx);
    }

    fn on_action_finished(
        &mut self,
        conversation_id: AIConversationId,
        result: &Arc<AIAgentActionResult>,
        cancellation_reason: Option<CancellationReason>,
        ctx: &mut Self::Context<'_>,
    ) {
        ctx.emit(BlocklistAIActionEvent::FinishedAction {
            action_id: result.id.clone(),
            conversation_id,
            cancellation_reason,
        });
    }

    fn on_phase_drained(
        &mut self,
        conversation_id: AIConversationId,
        cancellation_reason: Option<CancellationReason>,
        ctx: &mut Self::Context<'_>,
    ) {
        if !cancellation_reason.is_some_and(|r| r.should_preserve_in_progress_status()) {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                let status = if self
                    .tools
                    .finished_action_results
                    .get(&conversation_id)
                    .is_some_and(|results| results.iter().all(|r| r.result.is_cancelled()))
                {
                    ConversationStatus::Cancelled
                } else {
                    ConversationStatus::InProgress
                };
                history_model.update_conversation_status(
                    self.terminal_view_id,
                    conversation_id,
                    status,
                    ctx,
                );
            });
        }
    }
}

#[cfg(test)]
#[path = "action_model_tests.rs"]
mod tests;
