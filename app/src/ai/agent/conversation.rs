use std::collections::{HashMap, HashSet};
use std::fmt::Display;

use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationConfigStatus};
use ai::document::AIDocumentId;
use ai::skills::SkillPathOrigin;
use chrono::{DateTime, Local, TimeZone};
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vec1::{Size0Error, Vec1};
use warp_cli::agent::Harness;
use warp_core::command::ExitCode;
use warp_core::execution_mode::AppExecutionMode;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::WarpTheme;
use warp_multi_agent_api::response_event::stream_finished;
use warp_multi_agent_api::response_event::stream_finished::TokenUsage;
use warp_multi_agent_api::{self as api};
use warpui::color::ColorU;
use warpui::{AppContext, EntityId, ModelContext, SingletonEntity};

use super::api::ServerConversationToken;
use super::task::helper::*;
use super::task::transaction::{SavedTask, Transaction};
use super::task::{
    derive_todo_lists_from_root_task, ExtractMessagesError, Task, TaskId, TaskMessageContext,
    UpdateTaskError, UpgradeOptimisticTaskError,
};
use super::task_store::TaskStore;
use super::{
    AIAgentAction, AIAgentActionId, AIAgentContext, AIAgentExchange, AIAgentExchangeId,
    AIAgentInput, AIAgentOutput, AIAgentOutputStatus, AIAgentTodo, AIAgentTodoId,
    FinishedAIAgentOutput, MessageId, OutputModelInfo, RenderableAIError, RequestCost,
    ServerOutputId, Shared, SuggestedLoggingId, Suggestions,
};
use crate::ai::agent::api::convert_conversation::{
    compute_time_to_first_token_ms_from_messages, proto_timestamp_to_local_datetime,
    ConvertToExchanges,
};
use crate::ai::agent::comment::CodeReview;
use crate::ai::agent::icons::{
    failed_icon, gray_stop_icon, in_progress_icon, succeeded_icon, yellow_stop_icon,
};
use crate::ai::agent::linearization::compute_task_depths;
use crate::ai::agent::todos::AIAgentTodoList;
use crate::ai::agent::{
    AIAgentOutputMessage, AIAgentOutputMessageType, AIIdentifiers, CancellationReason,
    MessageToAIAgentOutputMessageError, SummarizationType,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, ConversationStatusUpdate, RequestInput, ResponseStreamId,
    SerializedBlockListItem,
};
use crate::ai::llms::LLMPreferences;
use crate::ai::skills::SkillDescriptor;
use crate::code_review::CodeReviewTelemetryEvent;
use crate::notebooks::NotebookId;
use crate::persistence::model::{
    AgentConversationData, ConversationUsageMetadata, ModelTokenUsage, PersistedAutoexecuteMode,
    ToolUsageMetadata,
};
use crate::persistence::ModelEvent;
use crate::server::ids::ServerId;
use crate::terminal::general_settings::GeneralSettings;
use crate::terminal::model::block::{
    AgentInteractionMetadata, AgentViewVisibility, BlockId, SerializedAIMetadata, SerializedBlock,
};
use crate::ui_components::icons::Icon;
use crate::workspaces::user_profiles::UserProfileWithUID;
use crate::{BlocklistAIHistoryModel, GlobalResourceHandlesProvider};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
    Stopped,
}

impl TodoStatus {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, TodoStatus::Cancelled)
    }
}

fn footer_model_token_usage(
    usage_metadata: &stream_finished::ConversationUsageMetadata,
    llm_preferences: &LLMPreferences,
) -> Vec<ModelTokenUsage> {
    // warp + byok rows merge on their server-known model id. Custom endpoint
    // rows live in a separate bucket keyed by their upstream `config_key` so
    // they never collide with a warp/byok row that happens to share the same
    // resolved alias. The `config_key` itself is not retained on
    // `ModelTokenUsage`; it is translated to an alias up front and only the
    // alias flows downstream (display + shared-session replay).
    let mut standard_usage: HashMap<String, ModelTokenUsage> = HashMap::new();
    for (model_id, usage) in &usage_metadata.warp_token_usage {
        let entry = standard_usage
            .entry(model_id.clone())
            .or_insert_with(|| ModelTokenUsage {
                model_id: model_id.clone(),
                ..Default::default()
            });
        entry.warp_tokens += usage.total_tokens;
        for (category, tokens) in &usage.token_usage_by_category {
            *entry
                .warp_token_usage_by_category
                .entry(category.clone())
                .or_default() += *tokens;
        }
    }
    for (model_id, usage) in &usage_metadata.byok_token_usage {
        let entry = standard_usage
            .entry(model_id.clone())
            .or_insert_with(|| ModelTokenUsage {
                model_id: model_id.clone(),
                ..Default::default()
            });
        entry.byok_tokens += usage.total_tokens;
        for (category, tokens) in &usage.token_usage_by_category {
            *entry
                .byok_token_usage_by_category
                .entry(category.clone())
                .or_default() += *tokens;
        }
    }

    let mut custom_usage: HashMap<String, ModelTokenUsage> = HashMap::new();
    for (config_key, usage) in &usage_metadata.custom_endpoint_token_usage {
        let label = llm_preferences.custom_endpoint_usage_display_label(config_key);
        let entry = custom_usage
            .entry(config_key.clone())
            .or_insert_with(|| ModelTokenUsage {
                model_id: label,
                ..Default::default()
            });
        entry.custom_endpoint_tokens += usage.total_tokens;
        for (category, tokens) in &usage.token_usage_by_category {
            *entry
                .custom_endpoint_token_usage_by_category
                .entry(category.clone())
                .or_default() += *tokens;
        }
    }

    standard_usage
        .into_values()
        .chain(custom_usage.into_values())
        .collect()
}

// basic info for creating a dummy command block based on an exchange's inputs
pub(crate) struct CommandBlockInfo {
    pub(crate) command: String,
    pub(crate) output: String,
    pub(crate) exit_code: ExitCode,
    pub(crate) ai_metadata: Option<String>,
    /// The api message ID of the tool call that initiated this command.
    /// Used to find the corresponding exchange for PWD and start_ts fallback.
    pub(crate) message_id: String,
    /// Estimated timestamp when the command started.
    /// Note that this may not be perfectly accurate, because it may come from the tool call timestamp
    /// which is when the agent made the tool call, before the command actually started.
    pub(crate) start_ts: Option<DateTime<Local>>,
    /// Estimated timestamp when the command finished.
    /// Note that this may not be perfectly accurate, because it may come from the tool call result timestamp
    /// which is when the server receives the result, after the command actually finished.
    pub(crate) completed_ts: Option<DateTime<Local>>,
}

#[derive(Debug, Clone)]
struct AddedExchange {
    #[allow(dead_code)]
    task_id: TaskId,
    exchange_id: AIAgentExchangeId,
}

#[derive(thiserror::Error, Debug)]
pub enum RestoreConversationError {
    #[error("Restored conversation has no root task")]
    NoRootTask,
}

#[derive(thiserror::Error, Debug)]
#[error("Subagent task not found")]
pub struct SubagentTaskNotFound;

/// An Agent Mode conversation.
#[derive(Debug, Clone)]
pub struct AIConversation {
    /// Unique ID for this conversation.
    id: AIConversationId,

    /// Whether this conversation is being shared from a different warp instance
    /// (i.e. is not a local conversation).
    is_viewing_shared_session: bool,
    task_store: TaskStore,
    optimistic_cli_subagent_subtask_id: Option<TaskId>,

    /// TODO lists created during the conversation, ordered by creation time. The last list (if any) is the active list.
    todo_lists: Vec<AIAgentTodoList>,

    /// Current the code review in this conversation, `None` if the has never tried to address
    /// comments in this conversation.
    code_review: Option<CodeReview>,

    status: ConversationStatus,
    /// Optional detail for the current error status.
    status_error_message: Option<String>,

    /// Tracks whether the code review has been opened at least once for this conversation.
    has_opened_code_review: bool,

    /// Usage metadata for this conversation, including summarization status, context window usage,
    /// credits spent, token usage, and tool usage.
    conversation_usage_metadata: ConversationUsageMetadata,

    /// The server-generated unique "token" for this conversation.
    ///
    /// This must be roundtripped to the server when sending follow-ups within a given conversation.
    server_conversation_token: Option<ServerConversationToken>,

    /// The server-assigned task/run identifier (`ai_tasks.id`) for this
    /// conversation, used for v2 orchestration.
    ///
    /// For local conversations, parsed from `StreamInit.run_id` on the first
    /// response. For remote child agents spawned via `POST /agent/run`, set
    /// from `SpawnAgentResponse.task_id`.
    ///
    /// Used for messaging API, events API, poller self-filtering, lifecycle
    /// reports, parent↔child agent identity, and task status reporting.
    /// The string form (for APIs that accept a run_id) is obtained via
    /// `run_id()` which calls `.to_string()` on this field.
    task_id: Option<AmbientAgentTaskId>,

    /// The server conversation ID of the source conversation if this conversation was forked.
    forked_from_server_conversation_token: Option<ServerConversationToken>,

    /// Metadata from the server for this conversation (permissions, timestamps, etc.).
    /// This is None for new conversations and gets populated after the first response completes.
    /// TODO (roland): server_conversation_token, conversation_usage_metadata, and artifacts are duplicated in here.
    /// Those are updated via stream events on init and finished respectively, while this is fetched via graphQL
    /// Consider consolidating by having the stream events return this whole metadata
    server_metadata: Option<ServerAIConversationMetadata>,

    /// The active transaction for this conversation, if any.
    transaction: Option<Transaction>,

    /// The per-conversation override on the user's usual autonomy settings.
    autoexecute_override: AIConversationAutoexecuteMode,

    /// Map of new exchanges added keyed by ID of response stream corresponding to the MAA API
    /// request.
    added_exchanges_by_response: HashMap<ResponseStreamId, Vec1<AddedExchange>>,

    /// A set of the hidden exchanges.
    /// This is stored here instead of the AIAgentExchange because this is a view specific field.
    /// We cache this here because we don't have access to the block everywhere we are updating the
    /// persisted exchanges.
    hidden_exchanges: HashSet<AIAgentExchangeId>,

    /// A set of action IDs that have been reverted by the user.
    reverted_action_ids: HashSet<AIAgentActionId>,

    /// Accumulated suggestions received in the course of this conversation.
    existing_suggestions: Option<Suggestions>,

    /// A set of suggestion logging IDs that have been dismissed for this conversation.
    dismissed_suggestion_ids: HashSet<SuggestedLoggingId>,

    total_request_cost: RequestCost,
    total_token_usage_by_model: HashMap<String, TokenUsage>,

    /// Fallback title used when no task description or initial query exists.
    fallback_display_title: Option<String>,

    /// Artifacts created during this conversation (plans, PRs, etc.).
    artifacts: Vec<Artifact>,

    /// Whether the AIConversation is being used as a vehicle for a CLI conversation, that
    /// doesn't have a full internal representation but uses an AIConversationId to render
    /// in the agent view.
    is_cli_agent_transcript: bool,

    // TODO(advait): Group child-agent-only fields (parent_agent_id,
    // agent_name, orchestration_harness_type, parent_conversation_id,
    // is_remote_child, pinned) into a ChildAgentState sub-struct. See
    // PR #10777 review.
    /// Server-side identifier of the parent agent that spawned this child, if any.
    /// For current orchestration, this holds the parent's `run_id`. Persisted as
    /// `parent_agent_id` for serde compatibility with older conversation data.
    parent_agent_id: Option<String>,
    /// The display name for this agent (e.g. "Agent 1"), assigned by the orchestrator.
    agent_name: Option<String>,
    /// Harness metadata associated with this child agent in orchestration flows.
    orchestration_harness_type: Option<String>,
    /// The local conversation ID of the parent that spawned this child, if any.
    parent_conversation_id: Option<AIConversationId>,
    /// True when this conversation is a placeholder for a child agent executing
    /// on a remote worker. The parent's client does not drive execution for
    /// these conversations — the remote worker's own client handles status
    /// reporting.
    is_remote_child: bool,

    /// The last event sequence number observed from the v2 orchestration
    /// event log. Used on restore to resume event delivery without
    /// re-delivering already-processed events.
    last_event_sequence: Option<i64>,

    /// Per-plan orchestration configs hydrated from
    /// `OrchestrationConfigSnapshot` messages in the conversation's task list.
    /// Keyed by `plan_id`; snapshots with empty `plan_id` are ignored.
    orchestration_configs: HashMap<String, (OrchestrationConfig, OrchestrationConfigStatus)>,

    /// Whether the user has pinned this child agent in the orchestration
    /// pill bar. Persisted via `AgentConversationData.pinned`.
    pinned: bool,
}

pub(crate) fn artifact_from_fork_proto(
    proto_artifact: &api::message::artifact_event::ConversationArtifact,
) -> Option<Artifact> {
    use api::message::artifact_event::conversation_artifact::Artifact as ProtoArtifact;

    match &proto_artifact.artifact {
        Some(ProtoArtifact::PullRequest(pr)) => Some(Artifact::from(pr.clone())),
        Some(ProtoArtifact::Screenshot(ss)) => Some(Artifact::from(ss.clone())),
        Some(ProtoArtifact::Plan(plan)) => Some(Artifact::from(plan.clone())),
        Some(ProtoArtifact::File(file)) => Some(Artifact::from(file.clone())),
        None => None,
    }
}

impl AIConversation {
    pub fn new(is_viewing_shared_session: bool, is_cli_agent_transcript: bool) -> Self {
        let root_task = Task::new_optimistic_root();
        Self {
            id: AIConversationId::new(),
            task_store: TaskStore::with_root_task(root_task),
            optimistic_cli_subagent_subtask_id: None,
            code_review: None,
            is_viewing_shared_session,
            is_cli_agent_transcript,
            todo_lists: vec![],
            status: ConversationStatus::InProgress,
            status_error_message: None,
            has_opened_code_review: false,
            conversation_usage_metadata: ConversationUsageMetadata::default(),
            server_conversation_token: None,
            task_id: None,
            forked_from_server_conversation_token: None,
            server_metadata: None,
            transaction: None,
            autoexecute_override: Default::default(),
            added_exchanges_by_response: Default::default(),
            hidden_exchanges: Default::default(),
            reverted_action_ids: Default::default(),
            existing_suggestions: None,
            dismissed_suggestion_ids: Default::default(),
            total_request_cost: RequestCost::new(0.),
            total_token_usage_by_model: Default::default(),
            fallback_display_title: None,
            artifacts: Vec::new(),
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            last_event_sequence: None,
            orchestration_configs: HashMap::new(),
            pinned: false,
        }
    }

    /// Strict restore: returns `Err(NoRootTask)` if `tasks` is empty. Use
    /// for cloud-restore and fork-insert paths, where an empty payload is
    /// malformed input rather than a not-yet-populated child.
    pub fn new_restored(
        id: AIConversationId,
        tasks: Vec<api::Task>,
        conversation_data: Option<AgentConversationData>,
    ) -> Result<Self, RestoreConversationError> {
        if tasks.is_empty() {
            return Err(RestoreConversationError::NoRootTask);
        }
        Self::new_restored_synthesizing_on_empty(id, tasks, conversation_data)
    }

    // TODO: derive todo list state from tasks instead of taking args. This
    // would make it possible to fully restore a convo from tasks, instead of
    // having to persist this additional data.
    /// Lenient restore: when `tasks` is empty, synthesizes a fresh in-memory
    /// conversation with a new `Optimistic(Root)` root task and the persisted
    /// overlay metadata applied (mirroring the shape `AIConversation::new()`
    /// produces). Use for the local-DB restore path, where an empty
    /// `agent_tasks` set is the normal shape of a child conversation
    /// persisted before its first server response.
    pub fn new_restored_synthesizing_on_empty(
        id: AIConversationId,
        tasks: Vec<api::Task>,
        conversation_data: Option<AgentConversationData>,
    ) -> Result<Self, RestoreConversationError> {
        let (task_store, todo_lists, status) = if tasks.is_empty() {
            // Bypass `derive_status_from_root_task`: it would return `Success`
            // for a root with no exchanges, silently misclassifying a restored
            // "child waiting on server response" as done.
            let root_task = Task::new_optimistic_root();
            let task_store = TaskStore::with_root_task(root_task);
            (task_store, Vec::new(), ConversationStatus::InProgress)
        } else {
            let api_tasks_by_id: HashMap<String, api::Task> =
                tasks.into_iter().map(|t| (t.id.clone(), t)).collect();

            // To process a task, we need to reference some of the data in its parent task.  To
            // avoid cloning, we process the task tree from deepest tasks to shallowest tasks.  This
            // ensures that children are always processed before their parents, avoiding any need to
            // clone task data to ensure the parent is available when processing the child.
            let depths = compute_task_depths(&api_tasks_by_id);
            let mut task_ids: Vec<String> = api_tasks_by_id.keys().cloned().collect();
            task_ids.sort_by(|a, b| {
                depths
                    .get(b.as_str())
                    .unwrap_or(&0)
                    .cmp(depths.get(a.as_str()).unwrap_or(&0))
            });

            let mut api_tasks_and_exchanges_by_id: HashMap<_, _> = api_tasks_by_id
                .into_iter()
                .map(|(id, task)| {
                    let exchanges = task.into_exchanges();
                    (id, (task, exchanges))
                })
                .collect();

            let mut tasks_by_id = HashMap::new();
            // Defer root selection until we've seen every parentless task so
            // we can deterministically prefer a candidate with non-empty
            // messages. Heals legacy DB rows that contain an orphan
            // optimistic-UUID stub alongside the real server root; without
            // this dedupe, `HashMap` iteration order picks between them
            // non-deterministically. See QUALITY-774.
            let mut parentless_candidates: Vec<(api::Task, Vec<AIAgentExchange>)> = Vec::new();
            for task_id in task_ids {
                let Some((task, exchanges)) = api_tasks_and_exchanges_by_id.remove(&task_id) else {
                    continue;
                };

                if let Some(parent_id) = task.parent_id() {
                    if let Some((parent_task, _)) = api_tasks_and_exchanges_by_id.get(parent_id) {
                        tasks_by_id.insert(
                            TaskId::new(task.id.clone()),
                            Task::new_restored_subtask(task, parent_task, exchanges),
                        );
                    } else {
                        log::error!(
                            "Could not find parent task (id: {}) for task (id: {})",
                            parent_id,
                            task.id
                        );
                    }
                } else {
                    parentless_candidates.push((task, exchanges));
                }
            }

            // Prefer the parentless candidate with non-empty messages (the
            // real server root) over an empty stub. If multiple have messages
            // or none have messages, fall back to the first-encountered
            // candidate.
            let root_task_pick = parentless_candidates
                .iter()
                .position(|(task, _)| !task.messages.is_empty())
                .or_else(|| (!parentless_candidates.is_empty()).then_some(0))
                .map(|idx| parentless_candidates.swap_remove(idx));

            let Some((root_api_task, root_exchanges)) = root_task_pick else {
                return Err(RestoreConversationError::NoRootTask);
            };
            let root_task = Task::new_restored_root(root_api_task, root_exchanges.into_iter());

            // Derive todo lists from tasks by replaying UpdateTodos operations
            let todo_lists = derive_todo_lists_from_root_task(&root_task);
            let root_task_id = root_task.id().clone();
            tasks_by_id.insert(root_task.id().clone(), root_task);

            // Determine the correct status based on the exchanges before constructing
            let status = Self::derive_status_from_root_task(&tasks_by_id.get(&root_task_id));

            let task_store = TaskStore::from_tasks(tasks_by_id, root_task_id);
            (task_store, todo_lists, status)
        };

        let (
            server_conversation_token,
            forked_from_server_conversation_token,
            conversation_usage_metadata,
            reverted_action_ids,
            artifacts,
            parent_agent_id,
            agent_name,
            orchestration_harness_type,
            parent_conversation_id,
            is_remote_child,
            run_id,
            autoexecute_override,
            last_event_sequence,
            pinned,
        ) = if let Some(data) = conversation_data {
            let server_conversation_token = data
                .server_conversation_token
                .map(ServerConversationToken::new);
            let conversation_usage_metadata = data.conversation_usage_metadata.unwrap_or_default();
            let reverted_action_ids: HashSet<AIAgentActionId> = data
                .reverted_action_ids
                .unwrap_or_default()
                .into_iter()
                .map_into()
                .collect();
            let forked_from_server_conversation_token = data
                .forked_from_server_conversation_token
                .map(ServerConversationToken::new);
            let artifacts: Vec<Artifact> = data
                .artifacts_json
                .and_then(|json| {
                    serde_json::from_str(&json)
                        .map_err(|e| log::error!("Failed to deserialize artifacts: {e}"))
                        .ok()
                })
                .unwrap_or_default();
            let parent_conversation_id = data
                .parent_conversation_id
                .and_then(|id| AIConversationId::try_from(id).ok());
            let autoexecute_override = if FeatureFlag::RememberFastForwardState.is_enabled() {
                data.autoexecute_override
                    .map(Into::into)
                    .unwrap_or_default()
            } else {
                AIConversationAutoexecuteMode::default()
            };
            (
                server_conversation_token,
                forked_from_server_conversation_token,
                conversation_usage_metadata,
                reverted_action_ids,
                artifacts,
                data.parent_agent_id,
                data.agent_name,
                data.orchestration_harness_type,
                parent_conversation_id,
                data.is_remote_child,
                data.run_id,
                autoexecute_override,
                data.last_event_sequence,
                data.pinned,
            )
        } else {
            (
                None,
                None,
                ConversationUsageMetadata::default(),
                HashSet::new(),
                Vec::new(),
                None,
                None,
                None,
                None,
                false,
                None,
                AIConversationAutoexecuteMode::default(),
                None,
                false,
            )
        };

        Ok(Self {
            id,
            is_viewing_shared_session: false,
            is_cli_agent_transcript: false,
            task_store,
            status,
            status_error_message: None,
            todo_lists,
            // TODO(alokedesai): Support session restoration for code review comments.
            code_review: None,
            has_opened_code_review: false,
            conversation_usage_metadata,
            server_conversation_token,
            task_id: run_id.as_deref().and_then(|id| id.parse().ok()),
            forked_from_server_conversation_token,
            server_metadata: None,
            transaction: None,
            autoexecute_override,
            added_exchanges_by_response: Default::default(),
            existing_suggestions: None,
            hidden_exchanges: Default::default(),
            reverted_action_ids,
            dismissed_suggestion_ids: Default::default(),
            total_request_cost: RequestCost::new(0.),
            total_token_usage_by_model: Default::default(),
            optimistic_cli_subagent_subtask_id: None,
            fallback_display_title: None,
            artifacts,
            parent_agent_id,
            agent_name,
            orchestration_harness_type,
            parent_conversation_id,
            is_remote_child,
            last_event_sequence,
            orchestration_configs: HashMap::new(),
            pinned,
        })
    }

    pub fn id(&self) -> AIConversationId {
        self.id
    }

    /// Assigns fresh exchange IDs to all exchanges in this conversation.
    /// Used when forking conversations to avoid ID collisions with persisted blocks.
    pub fn reassign_exchange_ids(&mut self) {
        let task_ids: Vec<TaskId> = self.task_store.tasks().map(|t| t.id().clone()).collect();
        for task_id in task_ids {
            self.task_store.modify_task(&task_id, |task| {
                task.reassign_exchange_ids();
            });
        }
    }

    pub fn is_viewing_shared_session(&self) -> bool {
        self.is_viewing_shared_session
    }

    pub fn set_is_viewing_shared_session(&mut self, is_viewing_shared_session: bool) {
        self.is_viewing_shared_session = is_viewing_shared_session;
    }

    pub fn is_cli_agent_transcript(&self) -> bool {
        self.is_cli_agent_transcript
    }

    pub fn was_summarized(&self) -> bool {
        self.conversation_usage_metadata.was_summarized
    }

    /// Returns true if the conversation is currently being summarized.
    pub fn is_summarizing(&self) -> bool {
        let Some(exchange) = self.latest_visible_exchange() else {
            return false;
        };
        let Some(output) = exchange.output_status.output() else {
            return false;
        };
        output.get().messages.last().is_some_and(|m| {
            matches!(
                m.message,
                AIAgentOutputMessageType::Summarization {
                    finished_duration: None,
                    summarization_type: SummarizationType::ConversationSummary,
                    ..
                }
            )
        })
    }

    pub fn context_window_usage(&self) -> f32 {
        self.conversation_usage_metadata.context_window_usage
    }

    /// Total credits spent in the conversation, including both LLM inference
    /// and platform credits.
    pub fn credits_spent(&self) -> f32 {
        let total = self.conversation_usage_metadata.credits_spent
            + self.conversation_usage_metadata.platform_credits_spent;
        (total * 10.0).round() / 10.0
    }

    pub fn inference_credits_spent(&self) -> f32 {
        self.conversation_usage_metadata.credits_spent
    }

    pub fn platform_credits_spent(&self) -> f32 {
        self.conversation_usage_metadata.platform_credits_spent
    }

    /// Test-only helper that sets the conversation's credit total directly.
    /// Used by unit tests that exercise downstream credit-aware logic
    /// (e.g. the orchestration credit rollup) without having to wire up a
    /// full `StreamFinished` event.
    #[cfg(test)]
    pub(crate) fn set_credits_spent_for_test(&mut self, credits: f32) {
        self.conversation_usage_metadata.credits_spent = credits;
        self.conversation_usage_metadata.platform_credits_spent = 0.0;
    }

    /// Test-only helper that simulates the root-task upgrade performed by the
    /// `Action::CreateTask` branch of `apply_client_action` when the server
    /// confirms the root for a newly started conversation. Replaces the
    /// in-memory `Optimistic(Root)` root with a server-backed `Task` carrying
    /// `server_task`'s id.
    ///
    /// Unlike the production `Action::CreateTask` path, this helper does NOT
    /// update `added_exchanges_by_response`; it is only safe to call when no
    /// in-flight response stream references the optimistic root.
    #[cfg(test)]
    pub(crate) fn upgrade_optimistic_root_to_server_task_for_test(
        &mut self,
        server_task: api::Task,
    ) {
        let root_task_id = self.task_store.root_task_id().clone();
        let root_task = self
            .task_store
            .remove(&root_task_id)
            .expect("root task should exist for upgrade-in-place test helper");
        let server_root = root_task
            .into_server_created_task(server_task, None, None, None, &SkillPathOrigin::Unavailable)
            .expect("upgrading optimistic root to a server-backed task should succeed");
        self.task_store.set_root_task(server_root);
    }

    // Credits spent over the last block, where the block comprises
    // all agent outputs since the most recent user input.
    pub fn credits_spent_for_last_block(&self) -> Option<f32> {
        self.conversation_usage_metadata
            .credits_spent_for_last_block
            .map(|credits| (credits * 10.0).round() / 10.0)
    }

    /// Time to first token for the last completed set of agent responses
    /// since the most recent user query
    pub fn time_to_first_token_for_last_user_query_ms(&self) -> i64 {
        let exchanges = self.all_exchanges();
        if exchanges.is_empty() {
            return 0;
        }

        // Walk backwards from the end to find all exchanges in the last block
        // (everything since the last user query).
        for exchange in exchanges.iter().rev() {
            if exchange.has_user_query() {
                return exchange.time_to_first_token_ms.unwrap_or(0);
            }
        }

        // If we never found a user query, return the time_to_first_token_ms from the first exchange
        exchanges
            .first()
            .and_then(|ex| ex.time_to_first_token_ms)
            .unwrap_or(0)
    }

    /// Helper to derive an exchange's finish time from its associated task messages.
    fn finish_time_from_exchange_messages(
        task: &Task,
        exchange: &AIAgentExchange,
    ) -> Option<DateTime<Local>> {
        task.messages()
            .filter(|m| !m.id.is_empty())
            .filter(|m| {
                let id = MessageId::new(m.id.clone());
                exchange.added_message_ids.contains(&id)
            })
            .filter_map(|m| {
                m.timestamp.as_ref().and_then(|ts| {
                    let nanos = if ts.nanos < 0 { 0 } else { ts.nanos as u32 };
                    Local.timestamp_opt(ts.seconds, nanos).single()
                })
            })
            .max()
    }

    /// Derive an exchange's start time from the latest input's context.
    fn start_time_from_exchange_messages(exchange: &AIAgentExchange) -> Option<DateTime<Local>> {
        exchange
            .input
            .last()
            .and_then(|input| input.context())
            .and_then(|contexts| {
                contexts.iter().find_map(|context| match context {
                    AIAgentContext::CurrentTime { current_time } => Some(*current_time),
                    _ => None,
                })
            })
    }

    /// Derive the conversation status from the root task's exchanges.
    /// Used when restoring conversations to determine if they were cancelled or completed successfully.
    fn derive_status_from_root_task(root_task: &Option<&Task>) -> ConversationStatus {
        let Some(root_task) = root_task else {
            return ConversationStatus::Success;
        };

        // Check the last exchange's output status
        if let Some(last_exchange) = root_task.last_exchange() {
            match &last_exchange.output_status {
                AIAgentOutputStatus::Finished {
                    finished_output: FinishedAIAgentOutput::Cancelled { .. },
                } => return ConversationStatus::Cancelled,
                AIAgentOutputStatus::Finished {
                    finished_output: FinishedAIAgentOutput::Error { .. },
                } => return ConversationStatus::Error,
                _ => {}
            }
        }

        // If not cancelled or errored, it's successful
        ConversationStatus::Success
    }

    /// Total agent response time for the last completed set of agent responses
    /// since the most recent user query.
    pub fn total_agent_response_time_since_last_user_query_ms(&self) -> i64 {
        let exchanges = self.all_exchanges();
        if exchanges.is_empty() {
            return 0;
        }

        // Walk backwards, accumulating durations until we find a user query
        let mut total_ms: i64 = 0;
        for exchange in exchanges.iter().rev() {
            total_ms += exchange
                .duration()
                .map(|duration| duration.num_milliseconds())
                .unwrap_or(0);

            if exchange.has_user_query() {
                break;
            }
        }

        total_ms
    }

    /// Wall-to-wall response time for the last completed set of agent responses.
    pub fn wall_to_wall_response_time_since_last_query(&self) -> Option<i64> {
        let exchanges = self.all_exchanges();
        let last_exchange = exchanges.last().copied()?;
        let finish_time = last_exchange.finish_time?;

        // Walk backwards to find the most recent exchange with a user query
        let start_time = exchanges.iter().rev().find_map(|exchange| {
            if exchange.has_user_query() {
                Some(exchange.start_time)
            } else {
                None
            }
        })?;

        let duration = finish_time.signed_duration_since(start_time);
        Some(duration.num_milliseconds())
    }

    pub fn token_usage(&self) -> &[ModelTokenUsage] {
        &self.conversation_usage_metadata.token_usage
    }

    pub fn tool_usage_metadata(&self) -> &ToolUsageMetadata {
        &self.conversation_usage_metadata.tool_usage_metadata
    }

    pub fn usage_metadata(&self) -> ConversationUsageMetadata {
        self.conversation_usage_metadata.clone()
    }

    pub fn status(&self) -> &ConversationStatus {
        &self.status
    }

    /// Test-only setter for driving status-dependent logic directly.
    #[cfg(test)]
    pub(crate) fn set_status_for_test(&mut self, status: ConversationStatus) {
        self.status = status;
    }
    pub fn status_error_message(&self) -> Option<&str> {
        self.status_error_message.as_deref()
    }

    pub fn update_status(
        &mut self,
        status: ConversationStatus,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) {
        self.update_status_with_error_message(status, None, terminal_view_id, ctx);
    }

    pub fn update_status_with_error_message(
        &mut self,
        status: ConversationStatus,
        error_message: Option<String>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) {
        self.status_error_message = if matches!(
            &status,
            ConversationStatus::Error | ConversationStatus::TransientError
        ) {
            error_message.filter(|message| !message.trim().is_empty())
        } else {
            None
        };
        let prev_status = self.status.clone();
        let new_status = status.clone();
        self.status = status;
        ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationStatus {
            conversation_id: self.id,
            terminal_view_id,
            update: ConversationStatusUpdate::Changed { prev_status },
            new_status,
        });
    }

    pub fn is_processing_response_stream(&self, stream_id: &ResponseStreamId) -> bool {
        self.added_exchanges_by_response.contains_key(stream_id)
    }

    /// Removes the response stream tracking entry after the stream has fully completed.
    pub fn cleanup_completed_response_stream(&mut self, stream_id: &ResponseStreamId) {
        self.added_exchanges_by_response.remove(stream_id);
    }

    pub fn new_exchange_ids_for_response(
        &self,
        stream_id: &ResponseStreamId,
    ) -> impl Iterator<Item = AIAgentExchangeId> + '_ {
        self.added_exchanges_by_response
            .get(stream_id)
            .into_iter()
            .flat_map(|added_exchanges| {
                added_exchanges
                    .iter()
                    .map(|new_exchange| new_exchange.exchange_id)
            })
    }

    pub fn server_conversation_token(&self) -> Option<&ServerConversationToken> {
        self.server_conversation_token.as_ref()
    }

    /// Returns the server-assigned run identifier as a string.
    pub fn run_id(&self) -> Option<String> {
        self.task_id.map(|id| id.to_string())
    }

    /// Sets the task ID by parsing a run_id string.
    pub fn set_run_id(&mut self, id: String) {
        self.task_id = id.parse().ok();
    }

    /// Returns the server-assigned task ID, if available.
    pub fn task_id(&self) -> Option<AmbientAgentTaskId> {
        self.task_id
    }

    /// Sets the task ID directly (used for child agents spawned via `SpawnAgentResponse`).
    pub fn set_task_id(&mut self, id: AmbientAgentTaskId) {
        self.task_id = Some(id);
    }

    /// Returns the server-side agent identifier for orchestration.
    pub fn orchestration_agent_id(&self) -> Option<String> {
        self.run_id()
    }

    /// Updates the server conversation token for this conversation.
    ///
    /// This is used internally for session sharing when a forked conversation receives
    /// its new server-assigned token. The viewer needs to update the conversation's token
    /// from the original (forked-from) token to the new token so subsequent messages can
    /// be matched to the correct conversation.
    ///
    /// This should only be called by session sharing viewer logic when linking forked conversations.
    pub(crate) fn set_server_conversation_token(&mut self, token: String) {
        self.server_conversation_token = Some(ServerConversationToken::new(token));
    }

    pub fn forked_from_server_conversation_token(&self) -> Option<&ServerConversationToken> {
        self.forked_from_server_conversation_token.as_ref()
    }

    /// Clears the forked_from token after the first Init event has been sent to viewers.
    /// This ensures we only send the forked_from token once during session sharing.
    pub(crate) fn clear_forked_from_server_conversation_token(&mut self) {
        self.forked_from_server_conversation_token = None;
    }

    pub fn server_id(&self) -> Option<ServerId> {
        self.server_metadata
            .as_ref()
            .map(|metadata| metadata.metadata.uid)
    }

    pub fn server_metadata(&self) -> Option<&ServerAIConversationMetadata> {
        self.server_metadata.as_ref()
    }

    pub fn set_server_metadata(&mut self, metadata: ServerAIConversationMetadata) {
        self.server_metadata = Some(metadata);
    }

    pub fn parent_agent_id(&self) -> Option<&str> {
        self.parent_agent_id.as_deref()
    }

    pub fn set_parent_agent_id(&mut self, id: String) {
        self.parent_agent_id = Some(id);
    }

    pub fn agent_name(&self) -> Option<&str> {
        self.agent_name.as_deref()
    }

    pub fn set_agent_name(&mut self, name: String) {
        self.agent_name = Some(name);
    }

    pub fn orchestration_harness_type(&self) -> Option<&str> {
        self.orchestration_harness_type.as_deref()
    }

    pub fn orchestration_harness(&self) -> Option<Harness> {
        self.orchestration_harness_type
            .as_deref()
            .map(parse_orchestration_harness_type)
            .or_else(|| {
                self.server_metadata
                    .as_ref()
                    .map(|metadata| Harness::from(metadata.harness))
            })
    }

    pub fn set_orchestration_harness(&mut self, harness: Harness) {
        self.orchestration_harness_type = Some(harness.config_name().to_string());
    }

    pub fn parent_conversation_id(&self) -> Option<AIConversationId> {
        self.parent_conversation_id
    }

    pub fn set_parent_conversation_id(&mut self, id: AIConversationId) {
        self.parent_conversation_id = Some(id);
    }

    /// Returns the last observed v2 orchestration event sequence number,
    /// if any. The cursor is per-conversation: the highest sequence the
    /// streamer has seen on the run-ids this conversation watches
    /// (`watched_run_ids` for owner-side conversations, the ancestor
    /// subtree for viewer-mode orchestrator placeholders).
    pub fn last_event_sequence(&self) -> Option<i64> {
        self.last_event_sequence
    }

    /// Updates the last observed v2 orchestration event sequence number.
    pub fn set_last_event_sequence(&mut self, sequence: i64) {
        self.last_event_sequence = Some(sequence);
    }

    /// Returns whether the user has pinned this conversation in the
    /// orchestration pill bar.
    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// Sets the persisted pin state. Callers must follow up with
    /// `write_updated_conversation_state` to push the change to SQLite.
    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    /// Returns true if this conversation was spawned by a parent orchestrator agent.
    pub fn is_child_agent_conversation(&self) -> bool {
        self.parent_conversation_id.is_some() || self.parent_agent_id.is_some()
    }

    /// True iff this conversation knows about a parent agent — either via a
    /// local parent placeholder (`parent_conversation_id`, set in the GUI
    /// parent) or via the parent's server-side run identifier
    /// (`parent_agent_id`, stamped in driver-hosted processes).
    pub fn has_parent_agent(&self) -> bool {
        self.parent_conversation_id.is_some() || self.parent_agent_id.is_some()
    }

    /// Returns true if this is a placeholder for a child agent executing on a
    /// remote worker. The parent's client should not report task status for
    /// these — the remote worker handles it.
    pub fn is_remote_child(&self) -> bool {
        self.is_remote_child
    }

    /// Marks this conversation as a remote child placeholder.
    pub fn mark_as_remote_child(&mut self) {
        self.is_remote_child = true;
    }

    /// Returns the orchestration config and status for a specific plan,
    /// or `None` if no config has been hydrated for that plan.
    pub fn orchestration_config_for_plan(
        &self,
        plan_id: &str,
    ) -> Option<(&OrchestrationConfig, OrchestrationConfigStatus)> {
        self.orchestration_configs
            .get(plan_id)
            .map(|(config, status)| (config, *status))
    }

    /// Returns `true` if at least one plan has an orchestration config.
    pub fn has_any_orchestration_config(&self) -> bool {
        !self.orchestration_configs.is_empty()
    }

    /// Inserts or replaces the orchestration config for a specific plan.
    /// Returns `true` if the value actually changed.
    pub fn set_orchestration_config_for_plan(
        &mut self,
        plan_id: String,
        config: OrchestrationConfig,
        status: OrchestrationConfigStatus,
    ) -> bool {
        use std::collections::hash_map::Entry;
        match self.orchestration_configs.entry(plan_id) {
            Entry::Occupied(mut entry) => {
                let existing = entry.get();
                if existing.0 != config || existing.1 != status {
                    entry.insert((config, status));
                    true
                } else {
                    false
                }
            }
            Entry::Vacant(entry) => {
                entry.insert((config, status));
                true
            }
        }
    }

    /// Returns a reference to the full per-plan config map.
    pub fn orchestration_configs(
        &self,
    ) -> &HashMap<String, (OrchestrationConfig, OrchestrationConfigStatus)> {
        &self.orchestration_configs
    }

    /// Bulk-replaces all orchestration configs (used during hydration).
    /// Returns `true` if the map actually changed.
    pub fn set_orchestration_configs(
        &mut self,
        configs: HashMap<String, (OrchestrationConfig, OrchestrationConfigStatus)>,
    ) -> bool {
        if self.orchestration_configs != configs {
            self.orchestration_configs = configs;
            true
        } else {
            false
        }
    }

    /// Returns a flat list of linearized messages across all tasks, interpolating subtask messages
    /// in between subagent tool calls and results, effectively corresponding to the order in which
    /// the messages were created and added to the conversation.
    pub fn all_linearized_messages(&self) -> Vec<&api::Message> {
        self.task_store.all_linearized_messages()
    }

    /// Returns all the tasks in this conversation.
    ///
    /// Note that until we've fully migrated to the multi-agent endpoint, in reality, each
    /// conversation is comprised of a single task (the legacy endpoint `GenerateAIAgentOutput` does
    /// not support multiple tasks within a conversation).
    pub fn all_tasks(&self) -> impl Iterator<Item = &Task> {
        self.task_store.tasks()
    }

    /// Returns the set of tasks that are still active (relevant to the agent).
    ///
    /// This filters the full task list using DFS linearization to determine
    /// which tasks have open subagent tool calls without corresponding results.
    pub fn compute_active_tasks(&self) -> Vec<warp_multi_agent_api::Task> {
        use std::collections::HashMap;

        let root_task_id = self.get_root_task_id().to_string();
        let all_tasks: HashMap<&str, &warp_multi_agent_api::Task> = self
            .all_tasks()
            .filter_map(|task| {
                let source = task.source()?;
                Some((source.id.as_str(), source))
            })
            .collect();
        let active_task_ids =
            crate::ai::agent::linearization::compute_active_task_ids(&root_task_id, &all_tasks);
        all_tasks
            .into_values()
            .filter(|task| active_task_ids.contains(task.id.as_str()))
            .cloned()
            .collect()
    }

    /// Returns the titles from the CreateDocuments request corresponding to the given action ID (if any).
    /// This is used by shared-session viewers to use the correct document titles from the original CreateDocuments action.
    pub fn get_document_titles_for_action(
        &self,
        action_id: &AIAgentActionId,
    ) -> Option<Vec<String>> {
        for exchange in self.all_exchanges() {
            let Some(output) = exchange.output_status.output() else {
                continue;
            };

            for message in &output.get().messages {
                if let AIAgentOutputMessage {
                    message: AIAgentOutputMessageType::Action(action),
                    ..
                } = message
                {
                    if &action.id == action_id {
                        if let super::AIAgentActionType::CreateDocuments(
                            super::CreateDocumentsRequest { documents },
                        ) = &action.action
                        {
                            let titles = documents
                                .iter()
                                .map(|doc| doc.title.clone())
                                .collect::<Vec<_>>();
                            return Some(titles);
                        }
                    }
                }
            }
        }

        None
    }

    /// Returns the start timestamp of the earliest [`AIAgentExchange`] in the conversation, if
    /// any.
    pub fn start_ts(&self) -> Option<DateTime<Local>> {
        self.root_task_exchanges()
            .next()
            .map(|exchange| exchange.start_time)
    }

    pub fn has_opened_code_review(&self) -> bool {
        self.has_opened_code_review
    }

    pub fn mark_code_review_as_opened(&mut self) {
        self.has_opened_code_review = true;
    }

    /// Returns the IDs of comments that have been addressed in this conversation.
    pub fn addressed_comment_ids(&self) -> HashSet<crate::code_review::comments::CommentId> {
        self.code_review
            .as_ref()
            .map(|cr| cr.addressed_comments.iter().map(|c| c.id).collect())
            .unwrap_or_default()
    }

    pub fn is_entirely_passive_code_diff(&self) -> bool {
        let mut has_passive_code_diff_exchange = false;
        for exchange in self.root_task_exchanges() {
            has_passive_code_diff_exchange |= exchange.has_passive_code_diff();
            if exchange.has_user_query() {
                return false;
            }
        }
        has_passive_code_diff_exchange
    }

    pub fn is_entirely_passive(&self) -> bool {
        let mut has_passive_exchange = false;
        for exchange in self.root_task_exchanges() {
            has_passive_exchange |= exchange.has_passive_request();
            if exchange.has_user_query() {
                return false;
            }
        }
        has_passive_exchange
    }

    /// True if the conversation consists of just one exchange
    /// and that exchange is a passive suggestion.
    pub fn is_single_passive_exchange(&self) -> bool {
        self.task_store.task_count() == 1
            && self.is_entirely_passive()
            && self
                .get_root_task()
                .is_some_and(|task| task.exchanges_len() == 1)
    }

    /// True if the conversation started with a CLI subagent and was never continued.
    /// These conversations only have CLI subagent exchanges with no user queries,
    /// meaning they never hit the primary agent.
    pub fn is_orphaned_cli_subagent_conversation(&self) -> bool {
        // Check if conversation has only 1 task (root task) and it's a CLI subagent
        let started_with_cli_subagent = self.task_store.task_count() == 1
            && self
                .get_root_task()
                .is_some_and(|task| task.is_cli_subagent());

        if !started_with_cli_subagent {
            return false;
        }

        // Check if conversation was never continued (no user queries in any exchange)
        let never_continued = self
            .root_task_exchanges()
            .all(|exchange| !exchange.has_user_query());

        never_continued
    }

    /// Returns true if this conversation should be unconditionally excluded
    /// from conversation navigation and history.
    pub fn should_exclude_from_navigation(&self) -> bool {
        // Passive-only suggestions without any follow-up requests shouldn't be presented as
        // conversations.
        self.is_entirely_passive()
            // Orphaned CLI subagent conversations (invoked from within a terminal block) are
            // internal and shouldn't appear in navigation.
            || self.is_orphaned_cli_subagent_conversation()
            // Shared session viewer conversations are excluded because the shared session itself
            // is visible/represented elsewhere.
            || self.is_viewing_shared_session()
            // 3p transcript viewers create an internal conversation only so agent-view
            // filtering can associate the restored block snapshot with an active conversation.
            || self.is_cli_agent_transcript()
            // Child agent conversations spawned by an orchestrator are managed via the parent's
            // status card and shouldn't clutter the navigation list.
            || self.is_child_agent_conversation()
    }

    pub fn existing_suggestions(&self) -> Option<&Suggestions> {
        self.existing_suggestions.as_ref()
    }

    pub fn dismissed_suggestion_ids(&self) -> &HashSet<SuggestedLoggingId> {
        &self.dismissed_suggestion_ids
    }

    pub fn dismiss_current_suggestions(&mut self) {
        if let Some(suggestions) = &self.existing_suggestions {
            self.dismissed_suggestion_ids
                .extend(suggestions.rules.iter().map(|r| r.logging_id.clone()));
            self.dismissed_suggestion_ids.extend(
                suggestions
                    .agent_mode_workflows
                    .iter()
                    .map(|w| w.logging_id.clone()),
            );
        }
    }

    pub fn is_exchange_hidden(&self, exchange_id: AIAgentExchangeId) -> bool {
        self.hidden_exchanges.contains(&exchange_id)
    }

    pub fn set_is_exchange_hidden(
        &mut self,
        exchange_id: AIAgentExchangeId,
        is_hidden: bool,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) {
        // If the status is not being modified, return.
        if is_hidden == self.hidden_exchanges.contains(&exchange_id) {
            return;
        }

        if is_hidden {
            self.hidden_exchanges.insert(exchange_id);
        } else {
            self.hidden_exchanges.remove(&exchange_id);
        }

        // If the status is being toggled, set the persisted exchange hidden status.
        // Find the exchange and the terminal view ID for the exchange and emit an event to update
        // the exchange hidden state.
        ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
            exchange_id,
            terminal_view_id,
            conversation_id: self.id,
            is_hidden,
        });
    }

    /// Returns an iterator over all exchanges in all tasks in this conversation.
    pub fn all_exchanges(&self) -> Vec<&AIAgentExchange> {
        self.task_store.all_exchanges().collect()
    }

    /// Returns a vector of vectors of exchanges, in linearized order as they appeared in the
    /// conversation, grouped by task ID.
    pub fn all_exchanges_by_task(&self) -> Vec<(TaskId, Vec<&AIAgentExchange>)> {
        self.task_store.all_exchanges_by_task()
    }

    pub fn root_task_exchanges(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.task_store
            .root_task()
            .into_iter()
            .flat_map(|task| task.exchanges())
    }

    pub fn exchange_count(&self) -> usize {
        self.task_store.exchange_count()
    }

    pub fn is_empty(&self) -> bool {
        self.exchange_count() == 0
    }

    pub fn exchanges_reversed(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.task_store
            .root_task()
            .into_iter()
            .flat_map(|task| task.exchanges_reversed())
    }

    #[cfg_attr(target_family = "wasm", allow(unused))]
    pub fn exchange_with_id(&self, exchange_id: AIAgentExchangeId) -> Option<&AIAgentExchange> {
        for task in self.task_store.tasks() {
            if let Some(exchange) = task.exchanges().find(|exchange| exchange.id == exchange_id) {
                return Some(exchange);
            }
        }
        None
    }

    /// Returns the exchange that preceded the exchange with the given id, if there is one.
    pub fn previous_exchange(&self, exchange_id: &AIAgentExchangeId) -> Option<&AIAgentExchange> {
        self.exchanges_reversed()
            .skip_while(|e| e.id != *exchange_id)
            .nth(1)
    }

    /// Returns the last exchange that didn't contain a passive request.
    pub fn last_non_passive_exchange(&self) -> Option<&AIAgentExchange> {
        self.exchanges_reversed().find(|e| !e.has_passive_request())
    }

    /// Returns the latest root task exchange that has a visible AI block.
    /// Passive exchanges do not render conversation-level controls, and hidden exchanges have
    /// been removed from the blocklist.
    pub fn latest_visible_exchange(&self) -> Option<&AIAgentExchange> {
        self.exchanges_reversed()
            .find(|e| !e.has_passive_request() && !self.is_exchange_hidden(e.id))
    }

    pub fn first_exchange(&self) -> Option<&AIAgentExchange> {
        self.task_store.first_exchange()
    }

    pub fn latest_exchange(&self) -> Option<&AIAgentExchange> {
        self.task_store.latest_exchange()
    }

    pub fn latest_skills(&self) -> Option<Vec<SkillDescriptor>> {
        self.task_store.latest_skills()
    }

    /// Get the title of the given conversation.
    /// Priority: task description > initial query > fallback_display_title.
    pub fn title(&self) -> Option<String> {
        self.task_store
            .root_task()
            .and_then(|task| {
                if task.description().is_empty() {
                    self.initial_query()
                } else {
                    Some(task.description().to_owned())
                }
            })
            .or_else(|| self.fallback_display_title.clone())
    }

    /// Updates the conversation title and persists the conversation.
    pub(crate) fn update_conversation_title(
        &mut self,
        title: String,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) {
        let title_for_metadata = title.clone();
        self.task_store
            .modify_root_task(|root_task| root_task.update_description(title));
        if let Some(metadata) = self.server_metadata.as_mut() {
            metadata.title = title_for_metadata;
        }
        self.write_updated_conversation_state(ctx);
    }

    /// Restores a previous title snapshot and persists the conversation.
    pub(crate) fn restore_conversation_title(
        &mut self,
        root_task_description: String,
        server_metadata_title: Option<String>,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) {
        self.task_store
            .modify_root_task(|root_task| root_task.update_description(root_task_description));
        if let (Some(metadata), Some(title)) =
            (self.server_metadata.as_mut(), server_metadata_title)
        {
            metadata.title = title;
        }
        self.write_updated_conversation_state(ctx);
    }

    /// Sets a fallback title used when no task description or initial query exists.
    pub fn set_fallback_display_title(&mut self, title: String) {
        self.fallback_display_title = Some(title);
    }

    /// Returns the last time this conversation was modified (i.e., when the latest exchange was started).
    pub fn last_modified_at(&self) -> Option<DateTime<Local>> {
        self.latest_exchange()
            .map(|e| e.finish_time.unwrap_or(e.start_time))
    }

    /// Returns artifacts created during this conversation.
    pub fn artifacts(&self) -> &[Artifact] {
        &self.artifacts
    }

    /// Adds an artifact to this conversation and persists the change.
    pub fn add_artifact(
        &mut self,
        artifact: Artifact,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) {
        self.artifacts.push(artifact.clone());
        self.write_updated_conversation_state(ctx);
        ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
            terminal_view_id,
            conversation_id: self.id,
            artifact,
        });
    }

    /// Updates the notebook_uid for a plan artifact when it's synced to Warp Drive.
    pub fn update_plan_notebook_uid(
        &mut self,
        document_uid: AIDocumentId,
        notebook_uid: NotebookId,
        terminal_view_id: Option<EntityId>,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) {
        let document_uid = document_uid.to_string();
        for artifact in &mut self.artifacts {
            if let Artifact::Plan {
                document_uid: doc_uid,
                notebook_uid: ref mut nb_uid,
                ..
            } = artifact
            {
                if doc_uid == &document_uid {
                    *nb_uid = Some(notebook_uid);
                    let updated_artifact = artifact.clone();
                    self.write_updated_conversation_state(ctx);
                    if let Some(terminal_view_id) = terminal_view_id {
                        ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                            terminal_view_id,
                            conversation_id: self.id,
                            artifact: updated_artifact,
                        });
                    }
                    return;
                }
            }
        }
    }

    pub fn initial_query(&self) -> Option<String> {
        self.root_task_exchanges()
            .flat_map(|exchange| exchange.input.iter())
            .find_map(|input| {
                AIAgentInput::user_query(input)
                    .or_else(|| AIAgentInput::auto_code_diff_query(input).map(|s| s.to_string()))
                    .or_else(|| AIAgentInput::prompt_suggestion_result(input).cloned())
            })
    }

    pub fn initial_user_query(&self) -> Option<String> {
        self.root_task_exchanges()
            .flat_map(|exchange| exchange.input.iter())
            .find_map(AIAgentInput::user_query)
    }

    /// Export the conversation to markdown format.
    /// This is used by both clipboard export and file export.
    pub fn export_to_markdown(
        &self,
        action_model: Option<&crate::ai::blocklist::BlocklistAIActionModel>,
    ) -> String {
        let mut result = Vec::new();
        for exchange in self.all_exchanges() {
            let formatted_exchange = exchange.format_for_copy(action_model);
            if !formatted_exchange.is_empty() {
                result.push(formatted_exchange);
            }
        }
        result.join("\n\n")
    }

    pub fn has_auto_code_diff_query(&self) -> bool {
        self.root_task_exchanges()
            .flat_map(|exchange| exchange.input.iter())
            .any(|input| input.is_auto_code_diff_query())
    }

    pub fn latest_user_query(&self) -> Option<String> {
        self.exchanges_reversed().find_map(|exchange| {
            exchange.input.iter().rev().find_map(|input| {
                AIAgentInput::user_query(input)
                    .map(|query| query.trim().to_owned())
                    .filter(|query| !query.is_empty())
            })
        })
    }

    /// Returns an iterator over the IDs of all UseComputer actions across all exchanges
    /// in this conversation.
    pub fn use_computer_action_ids(&self) -> impl Iterator<Item = AIAgentActionId> + '_ {
        self.all_exchanges().into_iter().flat_map(|exchange| {
            exchange
                .output_status
                .output()
                .into_iter()
                .flat_map(|output| {
                    output
                        .get()
                        .actions()
                        .filter(|a| matches!(a.action, super::AIAgentActionType::UseComputer(_)))
                        .map(|a| a.id.clone())
                        .collect::<Vec<_>>()
                })
        })
    }

    pub fn contains_action(&self, action_id: &AIAgentActionId) -> bool {
        self.task_store.tasks().any(|task| {
            task.exchanges()
            .any(|exchange| {
                let Some(output) = exchange.output_status.output()
                else {
                    return false;
                };
                output.get().messages.iter().any(|step| {
                    matches!(step, AIAgentOutputMessage{ message: AIAgentOutputMessageType::Action(AIAgentAction { id, .. }), .. } if id == action_id)
                })
            })
        })
    }

    /// Returns the exchange ID that contains the given action ID, if any.
    pub fn exchange_id_for_action(&self, action_id: &AIAgentActionId) -> Option<AIAgentExchangeId> {
        for task in self.task_store.tasks() {
            for exchange in task.exchanges() {
                let Some(output) = exchange.output_status.output() else {
                    continue;
                };
                let contains_action = output.get().messages.iter().any(|step| {
                    matches!(step, AIAgentOutputMessage{ message: AIAgentOutputMessageType::Action(AIAgentAction { id, .. }), .. } if id == action_id)
                });
                if contains_action {
                    return Some(exchange.id);
                }
            }
        }
        None
    }

    /// Returns the `AIAgentContext` objects attached to the exchange with the given ID, if any.
    pub fn context_for_exchange(
        &self,
        exchange_id: AIAgentExchangeId,
    ) -> impl Iterator<Item = &AIAgentContext> {
        context_in_exchanges(self.exchange_with_id(exchange_id).into_iter())
    }

    pub fn update_for_new_request_input(
        &mut self,
        request_input: RequestInput,
        stream_id: ResponseStreamId,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<(), UpdateConversationError> {
        if let Some(request_info) = self.added_exchanges_by_response.remove(&stream_id) {
            log::error!(
                "Existing response stream info for stream id {stream_id:?}: {request_info:?}"
            );
        }

        let RequestInput {
            input_messages,
            working_directory,
            model_id,
            coding_model_id,
            cli_agent_model_id,
            computer_use_model_id,
            shared_session_response_initiator,
            request_start_ts,
            ..
        } = request_input;

        for (task_id, inputs) in input_messages.into_iter() {
            let should_hide = inputs
                .iter()
                .any(|input| input.is_passive_suggestion_trigger());

            let new_exchange = AIAgentExchange {
                id: AIAgentExchangeId::new(),
                input: inputs,
                output_status: AIAgentOutputStatus::Streaming { output: None },
                added_message_ids: HashSet::new(),
                start_time: request_start_ts,
                finish_time: None,
                time_to_first_token_ms: None,
                working_directory: working_directory.clone(),
                // TODO(CORE-3546): fetch shell launch data from active session
                model_id: model_id.clone(),
                coding_model_id: coding_model_id.clone(),
                cli_agent_model_id: cli_agent_model_id.clone(),
                computer_use_model_id: computer_use_model_id.clone(),
                request_cost: None,
                // This will be None for non-shared sessions
                response_initiator: shared_session_response_initiator.clone(),
            };

            let new_exchange_id = new_exchange.id;
            self.append_exchange_to_task(&task_id, new_exchange)?;

            self.added_exchanges_by_response.insert(
                stream_id.clone(),
                Vec1::new(AddedExchange {
                    task_id: task_id.clone(),
                    exchange_id: new_exchange_id,
                }),
            );

            if should_hide {
                self.hidden_exchanges.insert(new_exchange_id);
            }

            ctx.emit(BlocklistAIHistoryEvent::AppendedExchange {
                exchange_id: new_exchange_id,
                task_id,
                terminal_view_id,
                conversation_id: self.id,
                is_hidden: should_hide,
                response_stream_id: Some(stream_id.clone()),
            });
        }
        Ok(())
    }

    pub fn append_reassigned_exchange(
        &mut self,
        response_stream_id: &ResponseStreamId,
        exchange: AIAgentExchange,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<(), UpdateConversationError> {
        let root_task_id = self.task_store.root_task_id().clone();
        let exchange_id = exchange.id;
        if exchange.output_status.is_streaming() {
            if let Some(added_exchanges) =
                self.added_exchanges_by_response.get_mut(response_stream_id)
            {
                added_exchanges.push(AddedExchange {
                    task_id: root_task_id.clone(),
                    exchange_id,
                });
            } else {
                self.added_exchanges_by_response.insert(
                    response_stream_id.clone(),
                    Vec1::new(AddedExchange {
                        task_id: root_task_id.clone(),
                        exchange_id,
                    }),
                );
            }
        }

        self.append_exchange_to_task(&root_task_id, exchange)?;

        ctx.emit(BlocklistAIHistoryEvent::ReassignedExchange {
            exchange_id,
            terminal_view_id,
            new_task_id: root_task_id,
            new_conversation_id: self.id,
        });
        Ok(())
    }

    fn append_exchange_to_task(
        &mut self,
        task_id: &TaskId,
        exchange: AIAgentExchange,
    ) -> Result<(), UpdateConversationError> {
        for input in exchange.input.iter() {
            if let AIAgentInput::CodeReview {
                review_comments, ..
            } = input
            {
                let review_comments = review_comments
                    .comments
                    .clone()
                    .into_iter()
                    .map(|c| c.into())
                    .collect();

                if let Some(code_review) = self.code_review.as_mut() {
                    code_review.pending_comments.extend(review_comments);
                } else {
                    self.code_review = Some(CodeReview::new_with_pending_comments(review_comments));
                }
            }
        }

        if self.task_store.append_exchange(task_id, exchange) {
            Ok(())
        } else {
            Err(UpdateConversationError::NoActiveTask)
        }
    }

    pub fn remove_exchange(
        &mut self,
        exchange_id: AIAgentExchangeId,
    ) -> Result<AIAgentExchange, UpdateConversationError> {
        let mut response_entries_to_remove = vec![];
        for (stream_id, added_exchanges) in self.added_exchanges_by_response.iter_mut() {
            if let Some(idx) = added_exchanges
                .iter()
                .position(|new_exchange| new_exchange.exchange_id == exchange_id)
            {
                if let Err(Size0Error) = added_exchanges.remove(idx) {
                    response_entries_to_remove.push(stream_id.clone());
                }
            }
        }
        for response_id in response_entries_to_remove.into_iter() {
            self.added_exchanges_by_response.remove(&response_id);
        }

        // Find which task contains this exchange
        let task_id = self.task_store.tasks().find_map(|task| {
            task.exchanges()
                .any(|e| e.id == exchange_id)
                .then(|| task.id().clone())
        });

        if let Some(task_id) = task_id {
            if let Some(exchange) = self.task_store.remove_task_exchange(&task_id, exchange_id) {
                return Ok(exchange);
            }
        }
        Err(UpdateConversationError::ExchangeNotFound)
    }

    pub fn initialize_output_for_response_stream(
        &mut self,
        stream_id: &ResponseStreamId,
        init_event: warp_multi_agent_api::response_event::StreamInit,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<(), UpdateConversationError> {
        let Some(new_exchanges) = self.added_exchanges_by_response.get(stream_id).cloned() else {
            return Err(UpdateConversationError::NoPendingRequest);
        };

        let request_id = init_event.request_id.clone();
        for new_exchange_info in new_exchanges.iter() {
            self.get_exchange_to_update(new_exchange_info.exchange_id)?
                .init_output(ServerOutputId::new(request_id.clone()))?;
            ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                exchange_id: new_exchange_info.exchange_id,
                terminal_view_id,
                conversation_id: self.id,
                is_hidden: self
                    .hidden_exchanges
                    .contains(&new_exchange_info.exchange_id),
            });
        }

        self.server_conversation_token =
            Some(ServerConversationToken::new(init_event.conversation_id));
        let run_id = Some(init_event.run_id).filter(|s| !s.is_empty());
        self.task_id = run_id.as_deref().and_then(|id| id.parse().ok());
        Ok(())
    }

    pub fn update_cost_and_usage_for_request(
        &mut self,
        request_cost: Option<RequestCost>,
        token_usage: Vec<TokenUsage>,
        usage_metadata: Option<stream_finished::ConversationUsageMetadata>,
        was_user_initiated_request: bool,
        ctx: &AppContext,
    ) -> Result<(), UpdateConversationError> {
        for usage in token_usage.into_iter() {
            let entry = self
                .total_token_usage_by_model
                .entry(usage.model_id.clone())
                .or_insert_with(|| TokenUsage {
                    model_id: usage.model_id.clone(),
                    total_input: 0,
                    output: 0,
                    input_cache_read: 0,
                    input_cache_write: 0,
                    cost_in_cents: 0.0,
                });

            entry.total_input += usage.total_input;
            entry.output += usage.output;
            entry.input_cache_read += usage.input_cache_read;
            entry.input_cache_write += usage.input_cache_write;
            entry.cost_in_cents += usage.cost_in_cents;
        }

        if let Some(request_cost) = request_cost {
            let credits_spent_for_last_block = self
                .conversation_usage_metadata
                .credits_spent_for_last_block
                .get_or_insert(0.0);

            // If this exchange begins with a user input (implying it is initiating a new response),
            // reset credits spent to only include credits for this new response.
            if was_user_initiated_request {
                *credits_spent_for_last_block = 0.;
            }

            // Accumulate response credit usage.
            *credits_spent_for_last_block += request_cost.value() as f32;
            self.total_request_cost += request_cost;
        }

        if let Some(usage_metadata) = usage_metadata {
            self.conversation_usage_metadata.context_window_usage =
                usage_metadata.context_window_usage;
            self.conversation_usage_metadata.credits_spent = usage_metadata.credits_spent;
            self.conversation_usage_metadata.platform_credits_spent =
                usage_metadata.platform_credits_spent;
            let llm_preferences = LLMPreferences::as_ref(ctx);
            self.conversation_usage_metadata.token_usage =
                footer_model_token_usage(&usage_metadata, llm_preferences);

            self.conversation_usage_metadata.tool_usage_metadata = usage_metadata
                .tool_usage_metadata
                .as_ref()
                .map(Into::into)
                .unwrap_or_default();

            // A conversation can never go from summarized to un-summarized,
            // so we only update the summarized flag if it's going from false to true.
            if usage_metadata.summarized && !self.conversation_usage_metadata.was_summarized {
                self.conversation_usage_metadata.was_summarized = usage_metadata.summarized;
            }
        }
        Ok(())
    }

    pub fn mark_request_completed(
        &mut self,
        stream_id: &ResponseStreamId,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<(), UpdateConversationError> {
        let Some(new_exchanges) = self.added_exchanges_by_response.get(stream_id).cloned() else {
            log::error!("No pending request info for completed request.");
            return Err(UpdateConversationError::NoPendingRequest);
        };

        let mut has_new_actions = false;
        for AddedExchange {
            exchange_id,
            task_id,
        } in new_exchanges.into_iter()
        {
            let completed_exchange = self.mark_exchange_completed(&task_id, exchange_id)?;
            let output = completed_exchange
                .output_status
                .output()
                .map(Shared::get_owned);
            if let Some(output_shared) = output {
                let output = output_shared.get();
                has_new_actions |= output.actions().next().is_some();

                if let Some(new_suggestions) = output.suggestions.clone() {
                    if let Some(existing_suggestions) = self.existing_suggestions.as_mut() {
                        existing_suggestions.rules.extend(new_suggestions.rules);
                        existing_suggestions
                            .agent_mode_workflows
                            .extend(new_suggestions.agent_mode_workflows);
                    } else {
                        self.existing_suggestions = Some(new_suggestions);
                    }
                }
            }

            ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                exchange_id,
                terminal_view_id,
                conversation_id: self.id,
                is_hidden: self.is_exchange_hidden(exchange_id),
            });
        }
        self.write_updated_conversation_state(ctx);

        if !has_new_actions {
            // Update conversation-level status to success if the output has no actions.
            self.update_status(ConversationStatus::Success, terminal_view_id, ctx);
        }

        Ok(())
    }

    pub fn mark_completed_after_successful_split(
        &mut self,
        stream_id: &ResponseStreamId,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<(), UpdateConversationError> {
        // Remove the mapping between the response stream and this conversation, as the response stream is
        // now associated with a different one.
        if let Some(added_exchanges) = self.added_exchanges_by_response.remove(stream_id) {
            for AddedExchange {
                exchange_id,
                task_id,
            } in added_exchanges.into_iter()
            {
                let completed_exchange = self.mark_exchange_completed(&task_id, exchange_id)?;
                let output = completed_exchange
                    .output_status
                    .output()
                    .map(Shared::get_owned);
                if let Some(output_shared) = output {
                    let output = output_shared.get();

                    if let Some(new_suggestions) = output.suggestions.clone() {
                        if let Some(existing_suggestions) = self.existing_suggestions.as_mut() {
                            existing_suggestions.rules.extend(new_suggestions.rules);
                            existing_suggestions
                                .agent_mode_workflows
                                .extend(new_suggestions.agent_mode_workflows);
                        } else {
                            self.existing_suggestions = Some(new_suggestions);
                        }
                    }
                }

                ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                    exchange_id,
                    terminal_view_id,
                    conversation_id: self.id,
                    is_hidden: self.is_exchange_hidden(exchange_id),
                });
            }
        }
        self.write_updated_conversation_state(ctx);

        // Update conversation-level status to success if the output has no actions.
        self.update_status(ConversationStatus::Success, terminal_view_id, ctx);
        Ok(())
    }

    pub fn mark_request_cancelled(
        &mut self,
        stream_id: &ResponseStreamId,
        terminal_view_id: EntityId,
        reason: CancellationReason,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<(), UpdateConversationError> {
        let Some(added_exchanges) = self.added_exchanges_by_response.get(stream_id).cloned() else {
            log::error!("No pending request info for completed request.");
            return Err(UpdateConversationError::NoPendingRequest);
        };
        if self.transaction.is_some() {
            self.commit_transaction()
        }

        for AddedExchange {
            exchange_id,
            task_id,
        } in added_exchanges.into_iter()
        {
            let is_viewing_shared_session = self.is_viewing_shared_session;
            let task = self
                .task_store
                .get(&task_id)
                .ok_or(UpdateConversationError::TaskNotFound)?
                .clone();
            let exchange = self.get_exchange_to_update(exchange_id)?;
            let AIAgentOutputStatus::Streaming { output } = &exchange.output_status else {
                // Skip exchanges that are already finished (e.g., a root task exchange
                // that completed before a subagent exchange was cancelled).
                continue;
            };
            exchange.output_status = AIAgentOutputStatus::Finished {
                finished_output: FinishedAIAgentOutput::Cancelled {
                    output: output.as_ref().map(Shared::get_owned),
                    reason,
                },
            };

            let finish_time = Self::finish_time_from_exchange_messages(&task, exchange)
                .unwrap_or_else(Local::now);

            // For shared-session viewers, derive start time and time to first token from server messages
            // (in the same way we do when restoring/forking conversations).
            if is_viewing_shared_session {
                if let Some(start_time) = Self::start_time_from_exchange_messages(exchange) {
                    exchange.start_time = start_time;
                }

                exchange.time_to_first_token_ms = compute_time_to_first_token_ms_from_messages(
                    exchange.start_time,
                    task.messages().filter(|m| {
                        let id = MessageId::new(m.id.clone());
                        exchange.added_message_ids.contains(&id)
                    }),
                );
            }

            exchange.finish_time = Some(finish_time);

            let is_hidden = self.is_exchange_hidden(exchange_id);
            ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                exchange_id,
                terminal_view_id,
                conversation_id: self.id,
                is_hidden,
            });
        }

        self.write_updated_conversation_state(ctx);

        // Don't mark the conversation as Cancelled if we're just cancelling to send a follow-up
        // on the same conversation (it will be immediately set back to InProgress), or if the
        // user manually took over the long-running command (the conversation remains in progress
        // and will resume once the command finishes or control is handed back).
        if !reason.should_preserve_in_progress_status() {
            self.update_status(ConversationStatus::Cancelled, terminal_view_id, ctx);
        }
        Ok(())
    }

    pub fn mark_request_cancelled_due_to_revert(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<(), UpdateConversationError> {
        if self.transaction.is_some() {
            self.commit_transaction();
        }
        self.update_status(ConversationStatus::Success, terminal_view_id, ctx);
        Ok(())
    }

    /// Marks the in-flight request's exchanges as finished with `error`.
    ///
    /// `recovery_pending` moves the conversation to the non-terminal `TransientError`
    /// status instead of `Error`, so consumers don't treat it as dead while an
    /// automatic recovery is in flight.
    pub fn mark_request_completed_with_error(
        &mut self,
        stream_id: &ResponseStreamId,
        error: RenderableAIError,
        recovery_pending: bool,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<(), UpdateConversationError> {
        let Some(added_exchanges) = self.added_exchanges_by_response.get(stream_id).cloned() else {
            log::error!("No pending request info for completed request.");
            return Err(UpdateConversationError::NoPendingRequest);
        };
        if self.transaction.is_some() {
            self.commit_transaction()
        }

        let AddedExchange {
            exchange_id: initial_exchange_id,
            ..
        } = added_exchanges.first();
        let identifiers = AIIdentifiers {
            server_output_id: None,
            server_conversation_id: self.server_conversation_token.clone().map(Into::into),
            client_conversation_id: Some(self.id),
            client_exchange_id: Some(*initial_exchange_id),
            model_id: None,
        };

        send_telemetry_from_ctx!(
            crate::TelemetryEvent::AgentModeError {
                identifiers,
                error: error.to_string(),
                is_user_visible: true,
                will_attempt_to_resume: recovery_pending,
            },
            ctx
        );

        for AddedExchange {
            exchange_id,
            task_id,
        } in added_exchanges.into_iter()
        {
            let is_viewing_shared_session = self.is_viewing_shared_session;
            let task = self
                .task_store
                .get(&task_id)
                .ok_or(UpdateConversationError::TaskNotFound)?
                .clone();
            let exchange = self.get_exchange_to_update(exchange_id)?;
            let AIAgentOutputStatus::Streaming { output } = &exchange.output_status else {
                return Err(UpdateConversationError::OutputAlreadyFinished);
            };
            exchange.output_status = AIAgentOutputStatus::Finished {
                finished_output: FinishedAIAgentOutput::Error {
                    output: output.as_ref().map(Shared::get_owned),
                    error: error.clone(),
                },
            };

            let finish_time = Self::finish_time_from_exchange_messages(&task, exchange)
                .unwrap_or_else(Local::now);

            // For shared-session viewers, derive start time and time to first token from server messages
            // (in the same way we do when restoring/forking conversations).
            if is_viewing_shared_session {
                if let Some(start_time) = Self::start_time_from_exchange_messages(exchange) {
                    exchange.start_time = start_time;
                }

                exchange.time_to_first_token_ms = compute_time_to_first_token_ms_from_messages(
                    exchange.start_time,
                    task.messages().filter(|m| {
                        let id = MessageId::new(m.id.clone());
                        exchange.added_message_ids.contains(&id)
                    }),
                );
            }

            exchange.finish_time = Some(finish_time);

            let is_hidden = self.is_exchange_hidden(exchange_id);
            ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                exchange_id,
                terminal_view_id,
                conversation_id: self.id,
                is_hidden,
            });
        }

        self.write_updated_conversation_state(ctx);
        let status = if recovery_pending {
            ConversationStatus::TransientError
        } else {
            ConversationStatus::Error
        };
        self.update_status_with_error_message(
            status,
            Some(error.to_string()),
            terminal_view_id,
            ctx,
        );
        Ok(())
    }

    fn mark_exchange_completed(
        &mut self,
        task_id: &TaskId,
        exchange_id: AIAgentExchangeId,
    ) -> Result<&AIAgentExchange, UpdateConversationError> {
        let task = self
            .task_store
            .get(task_id)
            .ok_or(UpdateConversationError::TaskNotFound)?
            .clone();
        let is_viewing_shared_session = self.is_viewing_shared_session;
        let exchange = self.get_exchange_to_update(exchange_id)?;
        let AIAgentOutputStatus::Streaming {
            output: Some(output),
        } = &exchange.output_status
        else {
            return Err(UpdateConversationError::OutputAlreadyFinished);
        };

        let output = output.get_owned();
        exchange.output_status = AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Success { output },
        };

        // Record finish time for this exchange based on the latest message timestamp associated
        // with this exchange. Fallback to `Local::now()` if no timestamps are present so that
        // duration calculations always have a sensible value.
        let finish_time =
            Self::finish_time_from_exchange_messages(&task, exchange).unwrap_or_else(Local::now);

        // For shared-session viewers, derive start time and time to first token from server messages
        // (in the same way we do when restoring/forking conversations).
        if is_viewing_shared_session {
            if let Some(start_time) = Self::start_time_from_exchange_messages(exchange) {
                exchange.start_time = start_time;
            }

            exchange.time_to_first_token_ms = compute_time_to_first_token_ms_from_messages(
                exchange.start_time,
                task.messages().filter(|m| {
                    let id = MessageId::new(m.id.clone());
                    exchange.added_message_ids.contains(&id)
                }),
            );
        }
        exchange.finish_time = Some(finish_time);

        let exchange = self
            .exchange_with_id(exchange_id)
            .ok_or(UpdateConversationError::ExchangeNotFound)?;
        #[cfg(feature = "agent_mode_evals")]
        {
            // When running evals, log exchanges as they finish so there's a record if the container is killed due to timeout
            // and there's no chance to gracefully export the whole conversation at the end.
            let exchange_number = self.all_exchanges().len();
            let token_usage = self.total_token_usage();
            let token_usage_json: Vec<serde_json::Value> = token_usage
                .iter()
                .map(|usage| {
                    serde_json::json!({
                        "model_id": usage.model_id,
                        "total_input": usage.total_input,
                        "output": usage.output,
                        "input_cache_read": usage.input_cache_read,
                        "input_cache_write": usage.input_cache_write,
                        "cost_in_cents": usage.cost_in_cents
                    })
                })
                .collect();
            println!(
                "===== Exchange {exchange_number} - token_usage={}",
                serde_json::to_string(&token_usage_json).unwrap_or_default()
            );
            for input in &exchange.input {
                println!("\nInput:\n\n{input}\n");
            }
            println!("Output:\n{}\n", &exchange.output_status);
        }
        Ok(exchange)
    }

    pub fn apply_client_action(
        &mut self,
        response_stream_id: &ResponseStreamId,
        terminal_view_id: EntityId,
        action: warp_multi_agent_api::client_action::Action,
        skill_path_origin: &SkillPathOrigin,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<(), UpdateConversationError> {
        use warp_multi_agent_api::client_action::*;
        match action {
            Action::BeginTransaction(_) => {
                self.begin_transaction();
            }
            Action::CommitTransaction(_) => {
                self.commit_transaction();
            }
            Action::RollbackTransaction(_) => {
                log::debug!("Rollback transaction.");
                self.rollback_transaction(response_stream_id);
            }
            Action::CreateTask(CreateTask { task: Some(task) }) => {
                let task_id = TaskId::new(task.id.clone());
                // Save an empty task to the transaction
                self.checkpoint_task(&task_id);

                if let Some(parent_id) = task.parent_id() {
                    // If we're expecting a server-created CLI subagent subtask, instead of creating
                    // a net-new subtask, we convert the optimistically-created CLI subtask into a
                    // server-backed one.
                    let optimistic_cli_subagent_subtask = self
                        .optimistic_cli_subagent_subtask_id
                        .as_ref()
                        .and_then(|id| self.task_store.remove(id));
                    let Some(parent_task) = self.task_store.get(&TaskId::new(parent_id.to_owned()))
                    else {
                        log::error!(
                            "Attempted to create task with parent id {parent_id} but no parent task found"
                        );
                        return Err(UpdateConversationError::TaskNotFound);
                    };

                    if let Some(optimistic_subtask) = optimistic_cli_subagent_subtask {
                        log::debug!(
                            "Upgrading optimistically created subtask with ID {:?} to server task with ID {:?}",
                            optimistic_subtask.id(),
                            task.id
                        );
                        self.optimistic_cli_subagent_subtask_id = None;
                        let optimistic_id = optimistic_subtask.id().clone();
                        let server_subtask = optimistic_subtask.into_server_created_task(
                            task,
                            parent_task.source(),
                            self.todo_lists.last(),
                            self.code_review.as_ref(),
                            skill_path_origin,
                        )?;
                        ctx.emit(BlocklistAIHistoryEvent::UpgradedTask {
                            optimistic_id: optimistic_id.clone(),
                            server_id: server_subtask.id().clone(),
                            terminal_view_id,
                        });

                        for new_exchange in self
                            .added_exchanges_by_response
                            .get_mut(response_stream_id)
                            .into_iter()
                            .flat_map(|new_exchanges| new_exchanges.iter_mut())
                        {
                            if new_exchange.task_id == optimistic_id {
                                new_exchange.task_id = server_subtask.id().clone();
                            }
                        }
                        self.task_store.insert(server_subtask);
                    } else if let Some(existing_exchange) = self
                        .added_exchanges_by_response
                        .get(response_stream_id)
                        .map(|new_exchanges| new_exchanges.first())
                        .and_then(|new_exchange| {
                            self.task_store
                                .get(&new_exchange.task_id)
                                .and_then(|t| t.exchange(new_exchange.exchange_id))
                        })
                    {
                        let subtask = Task::new_subtask(
                            task,
                            parent_task
                                .source()
                                .ok_or(UpdateConversationError::TaskNotInitialized)?,
                            existing_exchange,
                            self.todo_lists.last(),
                            self.code_review.as_ref(),
                            skill_path_origin,
                            // In shared-session viewers, we have to reconstruct what the original user input
                            // was using subsequent conversation messages (as the original input was not
                            // sent on this client). Once we reconstruct these inputs, we will insert them
                            // to mimic the normal conversation flow. (If this is not a shared session, the
                            // exchange inputs will already be populated).
                            self.is_viewing_shared_session,
                        );

                        // Subtasks can come pre-populated with messages (for example: an advice subagent
                        // or computer use subagent task created with an initial tool call already present
                        // in its task messages).
                        //
                        // In those cases, we need to ensure an AI block is created for the subtask's
                        // initial exchange; otherwise the first tool call/result can be "lost" from the
                        // block list because we only create AI blocks on AppendedExchange events.
                        //
                        // TODO(QUALITY-276): We should check if we can generally add exchanges from any
                        // subtask, or if that breaks things (e.g. in the CLI subagent).
                        let initial_exchange_ids: Vec<_> = if subtask.is_advice_subagent()
                            || subtask.is_computer_use_subagent()
                            || subtask.is_conversation_search_subagent()
                        {
                            subtask.exchanges().map(|e| e.id).collect()
                        } else {
                            Vec::new()
                        };

                        if self.is_viewing_shared_session {
                            // shared session viewers should move the current stream's new exchange from the root to the
                            // newly created subtask so there's exactly one "new" exchange and it
                            // belongs to the subtask (mirrors sharer semantics after optimistic upgrade).
                            let last_subtask_exchange_id = subtask
                                .exchanges()
                                .last()
                                .map(|e| e.id)
                                .ok_or(UpdateConversationError::ExchangeNotFound)?;

                            let new_exchanges = self
                                .added_exchanges_by_response
                                .get_mut(response_stream_id)
                                .ok_or(UpdateConversationError::NoPendingRequest)?;

                            let first = new_exchanges.first_mut();
                            // we're updating first's id is because it should correspond with the newly generated subtask's new exchange
                            first.task_id = task_id.clone();
                            first.exchange_id = last_subtask_exchange_id;
                        } else {
                            let new_exchanges = self
                                .added_exchanges_by_response
                                .get_mut(response_stream_id)
                                .ok_or(UpdateConversationError::NoPendingRequest)?;
                            new_exchanges.extend(subtask.exchanges().map(|exchange| {
                                AddedExchange {
                                    task_id: task_id.clone(),
                                    exchange_id: exchange.id,
                                }
                            }));
                        }

                        self.task_store.insert(subtask);
                        ctx.emit(BlocklistAIHistoryEvent::CreatedSubtask {
                            conversation_id: self.id,
                            terminal_view_id,
                            task_id: task_id.clone(),
                        });

                        for exchange_id in initial_exchange_ids {
                            let is_hidden = self.is_exchange_hidden(exchange_id);
                            ctx.emit(BlocklistAIHistoryEvent::AppendedExchange {
                                exchange_id,
                                task_id: task_id.clone(),
                                terminal_view_id,
                                conversation_id: self.id,
                                is_hidden,
                                response_stream_id: Some(response_stream_id.clone()),
                            });
                        }
                    }
                } else {
                    let root_task_id = self.task_store.root_task_id().clone();
                    if let Some(mut root_task) = self.task_store.remove(&root_task_id) {
                        let old_id = root_task.id().clone();
                        root_task = root_task.into_server_created_task(
                            task,
                            None,
                            self.todo_lists.last(),
                            self.code_review.as_ref(),
                            skill_path_origin,
                        )?;
                        ctx.emit(BlocklistAIHistoryEvent::UpgradedTask {
                            optimistic_id: old_id,
                            server_id: root_task.id().clone(),
                            terminal_view_id,
                        });

                        for AddedExchange {
                            ref mut task_id, ..
                        } in self
                            .added_exchanges_by_response
                            .get_mut(response_stream_id)
                            .ok_or(UpdateConversationError::NoPendingRequest)?
                            .iter_mut()
                        {
                            if *task_id == root_task_id {
                                *task_id = root_task.id().clone();
                            }
                        }
                        self.task_store.set_root_task(root_task);
                    }
                }
            }
            Action::UpdateTaskDescription(UpdateTaskDescription {
                task_id,
                description,
            }) => {
                let task_id = TaskId::new(task_id);
                self.checkpoint_task(&task_id);
                self.task_store
                    .modify_task(&task_id, |task| task.update_description(description))
                    .ok_or(UpdateConversationError::TaskNotFound)?;
            }
            Action::AddMessagesToTask(AddMessagesToTask { task_id, messages }) => {
                for message in messages.iter() {
                    match message.message.as_ref() {
                        Some(api::message::Message::UpdateTodos(update)) => {
                            if let Some(todos_op) = update.operation.as_ref() {
                                update_todo_list_from_todo_op(
                                    &mut self.todo_lists,
                                    todos_op.clone(),
                                );
                                ctx.emit(BlocklistAIHistoryEvent::UpdatedTodoList {
                                    terminal_view_id,
                                });
                            }
                        }
                        Some(api::message::Message::UpdateReviewComments(comments)) => {
                            if let Some(comments_op) = comments.operation.as_ref() {
                                if let Some(active_code_review) = self.code_review.as_mut() {
                                    let resolved_count = update_comment_from_comment_operation(
                                        active_code_review,
                                        comments_op.clone(),
                                    );
                                    if resolved_count > 0 {
                                        send_telemetry_from_ctx!(
                                            CodeReviewTelemetryEvent::CommentResolved {
                                                resolved_count
                                            },
                                            ctx
                                        );
                                    }
                                } else {
                                    log::error!(
                                        "Received an UpdateReviewComments message but there's no active code review state"
                                    );
                                }
                            }
                        }
                        Some(api::message::Message::ArtifactEvent(artifact_event)) => {
                            match &artifact_event.event {
                                Some(api::message::artifact_event::Event::Created(
                                    artifact_created,
                                )) => {
                                    match &artifact_created.artifact {
                                        Some(
                                            api::message::artifact_event::artifact_created::Artifact::PullRequest(pr),
                                        ) => {
                                            self.add_artifact(
                                                Artifact::from(pr.clone()),
                                                terminal_view_id,
                                                ctx,
                                            );
                                        }
                                        Some(
                                            api::message::artifact_event::artifact_created::Artifact::Screenshot(screenshot),
                                        ) => {
                                            self.add_artifact(
                                                Artifact::from(screenshot.clone()),
                                                terminal_view_id,
                                                ctx,
                                            );
                                        }
                                        Some(
                                            api::message::artifact_event::artifact_created::Artifact::File(file),
                                        ) => {
                                            self.add_artifact(
                                                Artifact::from(file.clone()),
                                                terminal_view_id,
                                                ctx,
                                            );
                                        }
                                        None => {}
                                    }
                                }
                                Some(api::message::artifact_event::Event::ForkArtifacts(
                                    fork_artifacts,
                                )) => {
                                    for proto_artifact in &fork_artifacts.artifacts {
                                        let Some(artifact) =
                                            artifact_from_fork_proto(proto_artifact)
                                        else {
                                            continue;
                                        };
                                        self.add_artifact(artifact, terminal_view_id, ctx);
                                    }
                                }
                                None => {}
                            }
                        }
                        Some(api::message::Message::OrchestrationConfigSnapshot(
                            snapshot,
                        )) => {
                            if !snapshot.plan_id.is_empty() {
                                if let Some(config) = snapshot
                                    .config
                                    .as_ref()
                                    .map(OrchestrationConfig::from_proto)
                                {
                                    let status = OrchestrationConfigStatus::from_proto(
                                        snapshot.status.as_ref(),
                                    );
                                    if self.set_orchestration_config_for_plan(
                                        snapshot.plan_id.clone(),
                                        config,
                                        status,
                                    ) {
                                        ctx.emit(
                                            BlocklistAIHistoryEvent::OrchestrationConfigUpdated {
                                                conversation_id: self.id,
                                                from_restore: false,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        Some(api::message::Message::ToolCallResult(tcr)) => {
                            // Clean up temp directories from conversation search subagents.
                            if let Some(api::message::tool_call_result::Result::Subagent(_)) =
                                &tcr.result
                            {
                                cleanup_conversation_search_temp_dir(
                                    &tcr.tool_call_id,
                                    &task_id,
                                    &self.task_store,
                                );
                            }
                        }
                        Some(api::message::Message::ModelUsed(model_used)) => {
                            let exchange_id = self
                                .added_exchanges_by_response
                                .get(response_stream_id)
                                .ok_or(UpdateConversationError::NoPendingRequest)?
                                .last()
                                .exchange_id;
                            let exchange = self.get_exchange_to_update(exchange_id)?;
                            if let Some(output) = exchange.output_status.output() {
                                let mut output = output.get_mut();
                                output.model_info = Some(OutputModelInfo {
                                    model_id: model_used.model_id.clone().into(),
                                    display_name: model_used.model_display_name.clone(),
                                    is_fallback: model_used.is_fallback,
                                });
                            }
                        }
                        _ => {}
                    }
                }

                let task_id = TaskId::new(task_id);
                self.checkpoint_task(&task_id);
                let current_todo_list = self.todo_lists.last().cloned();

                // Remove the task to relinquish mutable borrow on self, we add it back later.
                let mut task = self
                    .task_store
                    .remove(&task_id)
                    .ok_or(UpdateConversationError::TaskNotFound)?;
                let added_exchanges = self
                    .added_exchanges_by_response
                    .get(response_stream_id)
                    .ok_or(UpdateConversationError::NoPendingRequest)?;
                let exchange_id = if let Some(info) =
                    added_exchanges.iter().find(|info| info.task_id == task_id)
                {
                    info.exchange_id
                } else {
                    let existing_exchange = self
                        .get_task(&added_exchanges.last().task_id)
                        .ok_or(UpdateConversationError::TaskNotFound)?
                        .exchange(added_exchanges.last().exchange_id)
                        .ok_or(UpdateConversationError::ExchangeNotFound)?;
                    let new_exchange_id = task.append_new_exchange(existing_exchange);
                    if self.optimistic_cli_subagent_subtask_id.is_some() && task.is_root_task() {
                        // If we are lazily creating a new exchange at this point, this means we are updating
                        // a new task for the first time in this response stream.
                        //
                        // This is a bit of a hack, but if the optimistic CLI Subagent task is some and this is
                        // the root task, then this exchange corresponds to "setup" messages in the root task
                        // for bootstrapping the CLI subagent. In these cases, we don't care about
                        // surfacing the new root task messages in the UI (e.g. the blocklist) - there would basically
                        // be an empty AI Block corresponding to the CLI subagent tool call message added to the root
                        // task, with not user rendered output.
                        //
                        // The real fix here is to lazily create exchanges only when there are real messages to be
                        // rendered, or at the very least, lazily create AI blocks for an exchange only once the exchange
                        // actually has renderable content.
                        self.hidden_exchanges.insert(new_exchange_id);
                    }
                    new_exchange_id
                };

                let current_comment_state = self.code_review.as_ref().cloned();
                task.add_messages(
                    messages,
                    exchange_id,
                    TaskMessageContext {
                        current_todo_list: current_todo_list.as_ref(),
                        active_code_review: current_comment_state.as_ref(),
                        skill_path_origin,
                    },
                    // In shared-session viewers, we have to reconstruct what the original user input
                    // was using subsequent conversation messages (as the original input was not
                    // sent on this client). Once we reconstruct these inputs, we will insert them
                    // to mimic the normal conversation flow. (If this is not a shared session, the
                    // exchange inputs will already be populated).
                    self.is_viewing_shared_session,
                )?;

                self.task_store.insert(task);
                if !added_exchanges
                    .iter()
                    .any(|new_exchange_info| new_exchange_info.exchange_id == exchange_id)
                {
                    self.added_exchanges_by_response
                        .get_mut(response_stream_id)
                        .ok_or(UpdateConversationError::NoPendingRequest)?
                        .push(AddedExchange {
                            task_id: task_id.clone(),
                            exchange_id,
                        });
                    let is_hidden = self.hidden_exchanges.contains(&exchange_id);
                    ctx.emit(BlocklistAIHistoryEvent::AppendedExchange {
                        response_stream_id: Some(response_stream_id.clone()),
                        exchange_id,
                        task_id: task_id.clone(),
                        terminal_view_id,
                        conversation_id: self.id,
                        is_hidden,
                    });
                }
                ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                    exchange_id,
                    terminal_view_id,
                    conversation_id: self.id,
                    is_hidden: self.is_exchange_hidden(exchange_id),
                });
            }
            Action::UpdateTaskServerData(UpdateTaskServerData {
                task_id,
                server_data,
            }) => {
                let task_id = TaskId::new(task_id);
                self.task_store
                    .modify_task(&task_id, |task| task.update_task_server_data(server_data))
                    .ok_or(UpdateConversationError::TaskNotFound)?;
            }
            Action::UpdateTaskMessage(UpdateTaskMessage {
                task_id,
                message: Some(message),
                mask: Some(mask),
            }) => {
                // Process OrchestrationConfigSnapshot if the updated
                // message carries one (e.g. create_orchestration_config
                // tool call result updating a single message in place).
                if let Some(api::message::Message::OrchestrationConfigSnapshot(snapshot)) =
                    &message.message
                {
                    if !snapshot.plan_id.is_empty() {
                        if let Some(config) = snapshot
                            .config
                            .as_ref()
                            .map(OrchestrationConfig::from_proto)
                        {
                            let status =
                                OrchestrationConfigStatus::from_proto(snapshot.status.as_ref());
                            if self.set_orchestration_config_for_plan(
                                snapshot.plan_id.clone(),
                                config,
                                status,
                            ) {
                                ctx.emit(BlocklistAIHistoryEvent::OrchestrationConfigUpdated {
                                    conversation_id: self.id,
                                    from_restore: false,
                                });
                            }
                        }
                    }
                }

                let task_id = TaskId::new(task_id);
                let exchange_id = self
                    .added_exchanges_by_response
                    .get(response_stream_id)
                    .ok_or(UpdateConversationError::NoPendingRequest)?
                    .iter()
                    .find_map(|new_exchange| {
                        (new_exchange.task_id == task_id).then_some(new_exchange.exchange_id)
                    })
                    .ok_or(UpdateConversationError::ExchangeNotFound)?;

                let current_todo_list = self.todo_lists.last().cloned();
                let current_comment_state = self.code_review.as_ref().cloned();
                let is_viewing_shared_session = self.is_viewing_shared_session;
                // In shared-session viewers, we have to reconstruct what the original user input
                // was using subsequent conversation messages (as the original input was not
                // sent on this client). Once we reconstruct these inputs, we will insert them
                // to mimic the normal conversation flow. (If this is not a shared session, the
                // exchange inputs will already be populated).
                let todos_op = self
                    .task_store
                    .modify_task(&task_id, |task| {
                        task.upsert_message(
                            message,
                            exchange_id,
                            TaskMessageContext {
                                current_todo_list: current_todo_list.as_ref(),
                                active_code_review: current_comment_state.as_ref(),
                                skill_path_origin,
                            },
                            mask,
                            is_viewing_shared_session,
                        )
                        .map(|msg| msg.todos_op().cloned())
                    })
                    .ok_or(UpdateConversationError::TaskNotFound)??;
                // Update todo list if needed
                if let Some(todos_op) = todos_op {
                    update_todo_list_from_todo_op(&mut self.todo_lists, todos_op);
                    ctx.emit(BlocklistAIHistoryEvent::UpdatedTodoList { terminal_view_id });
                }
                ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                    exchange_id,
                    terminal_view_id,
                    conversation_id: self.id,
                    is_hidden: self.is_exchange_hidden(exchange_id),
                });
            }
            Action::AppendToMessageContent(AppendToMessageContent {
                task_id,
                message: Some(message),
                mask: Some(mask),
            }) => {
                let task_id = TaskId::new(task_id);
                let exchange_id = self
                    .added_exchanges_by_response
                    .get(response_stream_id)
                    .ok_or(UpdateConversationError::NoPendingRequest)?
                    .iter()
                    .find_map(|new_exchange| {
                        (new_exchange.task_id == task_id).then_some(new_exchange.exchange_id)
                    })
                    .ok_or(UpdateConversationError::ExchangeNotFound)?;

                let current_todo_list = self.todo_lists.last().cloned();
                let current_comment_state = self.code_review.as_ref().cloned();
                // Update the message and get the updated todos op, if any.
                let todos_op = self
                    .task_store
                    .modify_task(&task_id, |task| {
                        task.append_to_message_content(
                            message,
                            exchange_id,
                            TaskMessageContext {
                                current_todo_list: current_todo_list.as_ref(),
                                active_code_review: current_comment_state.as_ref(),
                                skill_path_origin,
                            },
                            mask,
                        )
                        .map(|msg| msg.todos_op().cloned())
                    })
                    .ok_or(UpdateConversationError::TaskNotFound)??;
                // Update todo list if needed
                if let Some(todos_op) = todos_op {
                    update_todo_list_from_todo_op(&mut self.todo_lists, todos_op);
                    ctx.emit(BlocklistAIHistoryEvent::UpdatedTodoList { terminal_view_id });
                }
                ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                    exchange_id,
                    terminal_view_id,
                    conversation_id: self.id,
                    is_hidden: self.is_exchange_hidden(exchange_id),
                });
            }
            Action::ShowSuggestions(suggestions) => {
                let exchange_id = self
                    .added_exchanges_by_response
                    .get(response_stream_id)
                    .ok_or(UpdateConversationError::NoPendingRequest)?
                    .last()
                    .exchange_id;
                let exchange_to_update = self.get_exchange_to_update(exchange_id)?;
                exchange_to_update.update_suggestions(suggestions);
                ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                    exchange_id,
                    terminal_view_id,
                    conversation_id: self.id,
                    is_hidden: self.is_exchange_hidden(exchange_id),
                });
            }
            Action::MoveMessagesToNewTask(MoveMessagesToNewTask {
                source_task_id,
                new_task: Some(mut new_task),
                first_message_id,
                last_message_id,
                expected_message_count,
                replacement_messages,
            }) => {
                let source_task_id = TaskId::new(source_task_id);
                self.checkpoint_task(&source_task_id);

                // Extract messages from the source task (this also inserts replacement messages).
                let mut extracted_messages = self
                    .task_store
                    .modify_task(&source_task_id, |task| {
                        task.splice_messages(
                            &first_message_id,
                            &last_message_id,
                            expected_message_count,
                            replacement_messages,
                        )
                    })
                    .ok_or(UpdateConversationError::TaskNotFound)??;

                // Update task_id on each extracted message to reference the new task.
                for msg in &mut extracted_messages {
                    msg.task_id = new_task.id.clone();
                }

                // Append extracted messages to the new task.
                new_task.messages.extend(extracted_messages);

                // Get the source task's api::Task to look up subagent_params.
                // At this point, the source task contains the replacement messages (including the
                // subagent call referencing the new task), so new_summary_subtask can find them.
                let source_api_task = self
                    .task_store
                    .get(&source_task_id)
                    .and_then(|t| t.source())
                    .cloned()
                    .ok_or(UpdateConversationError::TaskNotInitialized)?;

                // Create the subtask and add it to the task store.
                let subtask = Task::new_moved_messages_subtask(new_task, &source_api_task);
                self.task_store.insert(subtask);

                // Note: We do NOT emit any BlocklistAIHistoryEvent here because we
                // intentionally keep the UI unchanged during a live session. The
                // exchange's client representation (added_message_ids) remains
                // unmodified, pointing to message IDs that now exist in a subtask.
            }
            Action::StartNewConversation(_) => {
                // New conversations are handled at the BlocklistAIHistoryModel layer
            }
            _ => {
                log::warn!("Received unsupported client action: {action:?}");
            }
        }

        Ok(())
    }

    pub fn get_exchange_to_update(
        &mut self,
        exchange_id: AIAgentExchangeId,
    ) -> Result<&mut AIAgentExchange, UpdateConversationError> {
        self.task_store
            .exchange_mut(exchange_id)
            .ok_or(UpdateConversationError::ExchangeNotFound)
    }

    pub fn get_root_task(&self) -> Option<&Task> {
        self.task_store.root_task()
    }

    pub fn get_root_task_id(&self) -> &TaskId {
        self.task_store.root_task_id()
    }

    pub fn get_task(&self, task_id: &TaskId) -> Option<&Task> {
        self.task_store.get(task_id)
    }

    /// Optimistically creates a subtask for the CLISubagent task when a user query is sent while
    /// the a command is running but no subagent has been spawned yet.
    ///
    /// This is done in two scenarios:
    ///
    /// 1) The user enters agent mode while a user-executed command is running, and sends a query.
    /// 2) The agent has executed a long-running requested command, but before the response stream
    /// finishes (in which the CLI subagent would be spawned), the user pre-empts with a query.
    ///
    /// In both cases, we optimistically create a subtask for the query, and the next time we receive
    /// a `CreateTask` client action for a subtask, we upgrade this optimistic subtask to a
    /// server-backed task.
    pub fn create_optimistic_cli_subagent_task(
        &mut self,
        block_id: &BlockId,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> TaskId {
        if self.optimistic_cli_subagent_subtask_id.take().is_some() {
            log::error!(
                "Tried to optimistically create new subtask for CLI agent when one exists already."
            );
        }

        let new_task = Task::new_optimistic_cli_agent_subtask(block_id.clone());
        let new_task_id = new_task.id().clone();
        self.optimistic_cli_subagent_subtask_id = Some(new_task_id.clone());
        self.task_store.insert(new_task);
        ctx.emit(BlocklistAIHistoryEvent::CreatedSubtask {
            conversation_id: self.id,
            terminal_view_id,
            task_id: new_task_id.clone(),
        });
        new_task_id
    }

    pub fn is_subagent_task_finished(
        &self,
        subagent_task_id: &TaskId,
    ) -> Result<bool, SubagentTaskNotFound> {
        let subagent_task = self
            .task_store
            .get(subagent_task_id)
            .ok_or(SubagentTaskNotFound)?;
        let (Some(subagent_params), Some(parent_id)) =
            (subagent_task.subagent_params(), subagent_task.parent_id())
        else {
            return Err(SubagentTaskNotFound);
        };

        let parent_task = self
            .task_store
            .get(&parent_id)
            .ok_or(SubagentTaskNotFound)?;

        Ok(parent_task
            .source()
            .into_iter()
            .flat_map(|source| source.messages.iter())
            .any(|message| {
                message
                    .tool_call_result()
                    .is_some_and(|result| result.tool_call_id == subagent_params.tool_call_id)
            }))
    }

    /// Returns true if any subagent task is currently active (not yet finished).
    ///
    /// This covers both optimistic CLI subagent tasks (created before server
    /// confirmation) and server-backed subagent tasks. Used to prevent
    /// piggybacking orchestration events onto followup requests while a
    /// subagent is active, since subagents cannot interpret those events and
    /// inserting them breaks tool_use/tool_result ordering requirements.
    pub fn has_active_subagent(&self) -> bool {
        if self.optimistic_cli_subagent_subtask_id.is_some() {
            return true;
        }
        self.all_tasks().any(|task| {
            !task.is_root_task()
                && self
                    .is_subagent_task_finished(task.id())
                    .is_ok_and(|finished| !finished)
        })
    }

    pub fn todo_lists(&self) -> &Vec<AIAgentTodoList> {
        &self.todo_lists
    }

    pub fn active_todo_list(&self) -> Option<&AIAgentTodoList> {
        self.todo_lists.last()
    }

    pub fn active_todo(&self) -> Option<&AIAgentTodo> {
        self.active_todo_list()
            .and_then(|todo_list| todo_list.in_progress_item())
    }

    pub fn todo_status(&self, todo_id: &AIAgentTodoId) -> Option<TodoStatus> {
        for (i, list) in self.todo_lists.iter().rev().enumerate() {
            let is_active_list = i == 0;
            if let Some(pos) = list
                .pending_items()
                .iter()
                .position(|item| &item.id == todo_id)
            {
                if is_active_list {
                    if pos == 0 {
                        return if self.status.is_in_progress() {
                            Some(TodoStatus::InProgress)
                        } else {
                            Some(TodoStatus::Stopped)
                        };
                    } else {
                        return Some(TodoStatus::Pending);
                    }
                } else {
                    return Some(TodoStatus::Cancelled);
                }
            } else if list
                .completed_items()
                .iter()
                .any(|item| &item.id == todo_id)
            {
                return Some(TodoStatus::Completed);
            }
        }
        None
    }

    pub fn begin_transaction(&mut self) {
        if self.transaction.is_some() {
            log::error!("Transaction already in progress.");
            return;
        }
        self.transaction = Some(Transaction::new());
    }

    fn commit_transaction(&mut self) {
        // Clear the transaction if it exists.
        if self.transaction.take().is_none() {
            log::error!("No transaction in progress.");
        }
    }

    pub(crate) fn write_updated_conversation_state(
        &mut self,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) {
        // We should not persist non-local conversations (e.g. shared sessions).
        if self.is_viewing_shared_session {
            return;
        }

        // Check if session restoration is enabled before writing any state.
        if !*GeneralSettings::as_ref(ctx).restore_session
            || !AppExecutionMode::as_ref(ctx).can_save_session()
        {
            return;
        }

        let Some(sqlite_sender) = GlobalResourceHandlesProvider::as_ref(ctx)
            .get()
            .model_event_sender
            .clone()
        else {
            return;
        };

        let reverted_action_ids = if self.reverted_action_ids.is_empty() {
            None
        } else {
            Some(
                self.reverted_action_ids
                    .clone()
                    .into_iter()
                    .map_into()
                    .collect(),
            )
        };

        let artifacts_json = if self.artifacts.is_empty() {
            None
        } else {
            match serde_json::to_string(&self.artifacts) {
                Ok(json) => Some(json),
                Err(e) => {
                    log::error!(
                        "Failed to serialize artifacts when persisting conversation data: {e}"
                    );
                    None
                }
            }
        };

        let event = ModelEvent::UpdateMultiAgentConversation {
            conversation_id: self.id.to_string(),
            updated_tasks: self
                .all_tasks()
                .filter_map(|task| task.source_for_persistence())
                .collect(),
            conversation_data: AgentConversationData {
                server_conversation_token: self
                    .server_conversation_token
                    .clone()
                    .map(|token| token.into()),
                conversation_usage_metadata: Some(self.conversation_usage_metadata.clone()),
                reverted_action_ids,
                forked_from_server_conversation_token: self
                    .forked_from_server_conversation_token
                    .clone()
                    .map(|token| token.into()),
                artifacts_json,
                parent_agent_id: self.parent_agent_id.clone(),
                agent_name: self.agent_name.clone(),
                orchestration_harness_type: self.orchestration_harness_type.clone(),
                parent_conversation_id: self.parent_conversation_id.map(|id| id.to_string()),
                is_remote_child: self.is_remote_child,
                // Legacy field; retained for backward-compatible
                // deserialization but no longer written. The optimistic-root
                // case is now handled by `Task::source_for_persistence`
                // (returns `None`) and `new_restored_synthesizing_on_empty`.
                root_task_is_optimistic: None,
                run_id: self.task_id.map(|id| id.to_string()),
                autoexecute_override: Some(self.autoexecute_override.into()),
                last_event_sequence: self.last_event_sequence,
                pinned: self.pinned,
            },
        };
        ctx.spawn(
            async move {
                if let Err(e) = sqlite_sender.send(event) {
                    log::warn!("Failed to send updated AI tasks to sqlite writer thread: {e:?}");
                }
            },
            |_, _, _| {},
        );
    }

    pub fn rollback_transaction(&mut self, response_stream_id: &ResponseStreamId) {
        let Some(transaction) = self.transaction.take() else {
            log::error!("No transaction in progress.");
            return;
        };
        let mut deleted_tasks = Vec::new();
        let mut updated_tasks = Vec::new();

        // For each saved task in the transaction:
        for (_, saved_task) in transaction.saved_tasks() {
            match saved_task {
                SavedTask::New(id) => {
                    // The task was added during the transaction, so we need to delete it
                    deleted_tasks.push(id);
                }
                SavedTask::Existing(saved_task) => {
                    // The task was updated during the transaction, so we need to restore it
                    updated_tasks.push(*saved_task);
                }
            }
        }

        updated_tasks.into_iter().for_each(|task| {
            log::debug!("Rolling back existing task: {:?}", task.id());
            self.task_store.insert(task);
        });
        deleted_tasks.into_iter().for_each(|task_id| {
            log::debug!("Rolling back new task: {task_id:?}");
            self.task_store.remove(&task_id);
        });

        if let Some(added_exchanges) = self
            .added_exchanges_by_response
            .get(response_stream_id)
            .cloned()
        {
            let mut updated_added_exchanges: Option<Vec1<AddedExchange>> = None;
            for added_exchange in added_exchanges.into_iter() {
                let does_exchange_exist = self
                    .task_store
                    .get(&added_exchange.task_id)
                    .and_then(|task| {
                        task.exchanges()
                            .find(|exchange| exchange.id == added_exchange.exchange_id)
                    })
                    .is_some();
                if does_exchange_exist {
                    if let Some(updated_added_exchanges) = updated_added_exchanges.as_mut() {
                        updated_added_exchanges.push(added_exchange);
                    } else {
                        updated_added_exchanges = Some(Vec1::new(added_exchange));
                    }
                }
            }
            if let Some(updated_added_exchanges) = updated_added_exchanges {
                self.added_exchanges_by_response
                    .insert(response_stream_id.clone(), updated_added_exchanges);
            }
        }
    }

    pub fn checkpoint_task(&mut self, task_id: &TaskId) {
        if let Some(transaction) = &mut self.transaction {
            if let Some(task) = self.task_store.get(task_id) {
                transaction.checkpoint_task(task);
            } else {
                transaction.checkpoint_new_task(task_id);
            }
        }
    }

    pub fn toggle_autoexecute_override(&mut self) {
        self.autoexecute_override =
            if self.autoexecute_override == AIConversationAutoexecuteMode::RespectUserSettings {
                AIConversationAutoexecuteMode::RunToCompletion
            } else {
                AIConversationAutoexecuteMode::RespectUserSettings
            };
    }

    pub fn autoexecute_override(&self) -> AIConversationAutoexecuteMode {
        self.autoexecute_override
    }

    pub fn autoexecute_any_action(&self) -> bool {
        self.autoexecute_override.is_autoexecute_any_action()
    }

    pub fn initial_working_directory(&self) -> Option<String> {
        self.task_store
            .root_task()
            .and_then(Task::initial_working_directory)
    }

    /// Returns the current working directory from the most recent exchange that has one.
    /// Scans exchanges in reverse order and returns the first populated working directory.
    pub fn current_working_directory(&self) -> Option<String> {
        self.task_store
            .all_exchanges_rev()
            .find_map(|exchange| exchange.working_directory.clone())
    }

    #[allow(dead_code)]
    pub fn total_request_cost(&self) -> RequestCost {
        self.total_request_cost
    }

    #[allow(dead_code)]
    pub fn total_token_usage(&self) -> Vec<TokenUsage> {
        self.total_token_usage_by_model.values().cloned().collect()
    }

    /// Normalize all newlines to CRLF so restored blocks render lines starting at column 0,
    /// which is consistent with how we serialize real terminal blocks.
    fn to_stylized_bytes(s: &str) -> Vec<u8> {
        let s = s.replace("\r\n", "\n");
        s.replace('\n', "\r\n").into_bytes()
    }

    /// Extracts all shell command blocks, in order, from the conversation's API task
    /// messages.
    ///
    /// This includes:
    /// - RunShellCommand tool calls that completed
    /// - Attachments from UserQuery/SystemQuery messages
    /// - Context blocks from UserQuery/SystemQuery/ToolCallResult messages
    ///
    /// Returns CommandBlockInfo with command, output, exit_code, and optional ai_metadata.
    fn extract_command_blocks(&self) -> Vec<CommandBlockInfo> {
        let mut command_blocks = Vec::new();

        // Get the root task's API messages.
        let Some(root_task) = self.get_root_task() else {
            return command_blocks;
        };
        let Some(api_task) = root_task.source() else {
            return command_blocks;
        };

        // Build a map from message ID to exchange for timestamp lookups.
        // The exchange's start_time (derived from CurrentTime input context) is combined with
        // the result message proto ts to pick the earlier time as completed_ts for RunShellCommand blocks.
        let message_id_to_exchange: HashMap<&str, &AIAgentExchange> = self
            .all_exchanges()
            .into_iter()
            .flat_map(|exchange| {
                exchange
                    .added_message_ids
                    .iter()
                    .map(move |mid| (&**mid, exchange))
            })
            .collect();

        let mut seen_command_ids = HashSet::new();
        self.extract_command_blocks_from_messages(
            &api_task.messages,
            &message_id_to_exchange,
            &mut command_blocks,
            &mut seen_command_ids,
        );

        command_blocks
    }

    /// Extracts command blocks from a list of messages.
    ///
    /// This recurses when it encounters a summarization subagent call, producing the list
    /// of command blocks as it would have been had no summarization ever occurred.
    fn extract_command_blocks_from_messages(
        &self,
        messages: &[api::Message],
        message_id_to_exchange: &HashMap<&str, &AIAgentExchange>,
        command_blocks: &mut Vec<CommandBlockInfo>,
        seen_command_ids: &mut HashSet<String>,
    ) {
        // Build a map from tool_call_id to (RunShellCommandResult, result_message_id, result_proto_timestamp)
        // for efficient lookup within this message set.
        let tool_call_results: HashMap<
            &str,
            (&api::RunShellCommandResult, &str, Option<DateTime<Local>>),
        > = messages
            .iter()
            .filter_map(|msg| {
                let result = msg.tool_call_result()?;
                if let Some(api::message::tool_call_result::Result::RunShellCommand(cmd_result)) =
                    &result.result
                {
                    let ts = msg
                        .timestamp
                        .as_ref()
                        .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos));
                    Some((
                        result.tool_call_id.as_str(),
                        (cmd_result, msg.id.as_str(), ts),
                    ))
                } else {
                    None
                }
            })
            .collect();

        for message in messages {
            let message_id = message.id.clone();

            if let Some(tool_call) = message.tool_call() {
                // Check if this is a moved-messages subtask (summarization subagent).
                // If so, extract its command blocks here to maintain chronological order.
                if let Some(subagent) = tool_call.subagent() {
                    if subagent.is_summarization() {
                        let subtask_id = TaskId::new(subagent.task_id.clone());
                        if let Some(subtask) = self.task_store.get(&subtask_id) {
                            if let Some(subtask_source) = subtask.source() {
                                // Recursively extract from subtask (in case of nested summarization).
                                self.extract_command_blocks_from_messages(
                                    &subtask_source.messages,
                                    message_id_to_exchange,
                                    command_blocks,
                                    seen_command_ids,
                                );
                            }
                        }
                        // Don't process this message further - it's just a subagent call.
                        continue;
                    }
                }

                // Extract from RunShellCommand tool calls.
                if let Some(api::message::tool_call::Tool::RunShellCommand(run_cmd)) =
                    &tool_call.tool
                {
                    let tool_call_id = &tool_call.tool_call_id;
                    let command = &run_cmd.command;

                    // Find the corresponding tool call result in this message set.
                    if let Some((cmd_result, result_message_id, result_proto_ts)) =
                        tool_call_results.get(tool_call_id.as_str())
                    {
                        if let Some(api::run_shell_command_result::Result::CommandFinished(
                            api::ShellCommandFinished {
                                output: command_output,
                                exit_code,
                                command_id: finished_command_id,
                                start_ts: proto_start_ts,
                                finish_ts: proto_finish_ts,
                            },
                        )) = &cmd_result.result
                        {
                            // Track the command_id so attachment/context blocks for the
                            // same command are skipped (RunShellCommand blocks have
                            // better timestamps).
                            if !finished_command_id.is_empty() {
                                seen_command_ids.insert(finished_command_id.clone());
                            }

                            // start_ts: prefer the block timestamp stored on ShellCommandFinished,
                            // falling back to the tool call message's proto timestamp.
                            let start_ts = proto_start_ts
                                .as_ref()
                                .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos))
                                .or_else(|| {
                                    message.timestamp.as_ref().map(|ts| {
                                        proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)
                                    })
                                });
                            if start_ts.is_none() {
                                log::error!(
                                    "RunShellCommand tool call message has no timestamp (message_id: {message_id})"
                                );
                            }

                            // completed_ts: prefer the block timestamp stored on ShellCommandFinished.
                            // Fall back to the earlier of (1) the exchange start_time for the
                            // exchange containing the result message (from CurrentTime input
                            // context) and (2) the result message's proto timestamp.
                            let completed_ts = proto_finish_ts
                                .as_ref()
                                .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos))
                                .or_else(|| {
                                    let exchange_ts = message_id_to_exchange
                                        .get(*result_message_id)
                                        .map(|exchange| exchange.start_time);
                                    match (*result_proto_ts, exchange_ts) {
                                        (Some(proto_ts), Some(exchange_ts)) => {
                                            Some(proto_ts.min(exchange_ts))
                                        }
                                        (Some(proto_ts), None) => Some(proto_ts),
                                        (None, Some(exchange_ts)) => Some(exchange_ts),
                                        (None, None) => None,
                                    }
                                });

                            command_blocks.push(CommandBlockInfo {
                                command: command.clone(),
                                output: command_output.clone(),
                                exit_code: ExitCode::from(*exit_code),
                                ai_metadata: Some(
                                    serde_json::to_string(&Some(
                                        Into::<SerializedAIMetadata>::into(
                                            AgentInteractionMetadata::new_hidden(
                                                tool_call_id.clone().into(),
                                                self.id(),
                                            ),
                                        ),
                                    ))
                                    .unwrap_or_default(),
                                ),
                                // Use the tool call message ID (not the result message ID)
                                // so that to_serialized_blocklist_items looks up the exchange
                                // where the command was initiated — the right exchange for PWD
                                // and the start_ts fallback.
                                message_id: message_id.clone(),
                                start_ts,
                                completed_ts,
                            });
                        }
                    }
                }
            }

            // Extract from UserQuery/SystemQuery attachments.
            let attachments = match message.message.as_ref() {
                Some(api::message::Message::UserQuery(user_query)) => user_query
                    .referenced_attachments
                    .values()
                    .collect::<Vec<_>>(),
                Some(api::message::Message::SystemQuery(_)) => {
                    // SystemQuery doesn't have attachments currently.
                    vec![]
                }
                _ => vec![],
            };

            let msg_ts = message
                .timestamp
                .as_ref()
                .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos));

            for attachment in attachments {
                // Attachments have ExecutedShellCommand in their value oneof.
                if let Some(api::attachment::Value::ExecutedShellCommand(cmd)) = &attachment.value {
                    // Skip if we've already seen this command_id (e.g. from a
                    // RunShellCommand tool call or a duplicate attachment).
                    if !cmd.command_id.is_empty()
                        && !seen_command_ids.insert(cmd.command_id.clone())
                    {
                        continue;
                    }
                    let start_ts = cmd
                        .started_ts
                        .as_ref()
                        .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos))
                        .or(msg_ts);
                    let completed_ts = cmd
                        .finished_ts
                        .as_ref()
                        .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos))
                        .or(msg_ts);
                    command_blocks.push(CommandBlockInfo {
                        command: cmd.command.clone(),
                        output: cmd.output.clone(),
                        exit_code: ExitCode::from(cmd.exit_code),
                        ai_metadata: None,
                        message_id: message_id.clone(),
                        start_ts,
                        completed_ts,
                    });
                }
            }

            // Extract from UserQuery/SystemQuery context blocks.
            let context_blocks = match message.message.as_ref() {
                Some(api::message::Message::UserQuery(user_query)) => user_query.context.as_ref(),
                Some(api::message::Message::SystemQuery(system_query)) => {
                    system_query.context.as_ref()
                }
                _ => None,
            };

            if let Some(context) = context_blocks {
                #[allow(deprecated)]
                for executed_shell_command in &context.executed_shell_commands {
                    if !executed_shell_command.command.is_empty() {
                        // Skip if we've already seen this command_id.
                        if !executed_shell_command.command_id.is_empty()
                            && !seen_command_ids.insert(executed_shell_command.command_id.clone())
                        {
                            continue;
                        }
                        let start_ts = executed_shell_command
                            .started_ts
                            .as_ref()
                            .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos))
                            .or(msg_ts);
                        let completed_ts = executed_shell_command
                            .finished_ts
                            .as_ref()
                            .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos))
                            .or(msg_ts);
                        command_blocks.push(CommandBlockInfo {
                            command: executed_shell_command.command.clone(),
                            output: executed_shell_command.output.clone(),
                            exit_code: ExitCode::from(executed_shell_command.exit_code),
                            ai_metadata: None,
                            message_id: message_id.clone(),
                            start_ts,
                            completed_ts,
                        });
                    }
                }
            }
        }
    }

    /// Converts the conversation into a vector of serialized command blocks.
    /// When we open a new tab to restore a conversation in, we need to precompute this serialized list of blocks
    /// to pass into the TerminalModel constructor since command blocks must be created
    /// before the warp input block to not break bootstrapping.
    /// Only the command blocks are actually created in the terminal model. During restoration in the TerminalView,
    /// AI blocks are inserted relative to the command blocks based on timestamp.
    pub fn to_serialized_blocklist_items(&self) -> Vec<SerializedBlockListItem> {
        let mut serialized_blocks = Vec::new();

        // Extract all command blocks from the task messages
        let command_blocks = self.extract_command_blocks();
        log::info!(
            "Extracted {} command blocks for conversation {}",
            command_blocks.len(),
            self.id()
        );

        // Build a map from message ID to exchange for quick lookup
        let mut message_id_to_exchange: HashMap<&str, &AIAgentExchange> = HashMap::new();
        for exchange in self.root_task_exchanges() {
            for message_id in &exchange.added_message_ids {
                // MessageId derefs to str, so use &**message_id to get &str
                message_id_to_exchange.insert(&**message_id, exchange);
            }
        }

        // Get a fallback working directory from the first exchange (used if message ID not found)
        let fallback_pwd = self
            .root_task_exchanges()
            .next()
            .and_then(|e| e.working_directory.clone());

        // Create serialized blocks from the extracted command blocks
        for command_block in command_blocks {
            // Find the exchange that contains this command block's message ID for PWD and
            // a fallback timestamp. The exchange start time is used as a last-resort fallback
            // when proto-level timestamps are unavailable, because `restore_block` treats
            // `start_ts: None` as "block was never started" and skips `start()`/`finish()`,
            // which leaves the block in an unfinished state with zero height.
            let (pwd, exchange_time) = message_id_to_exchange
                .get(command_block.message_id.as_str())
                .map(|e| (e.working_directory.clone(), Some(e.start_time)))
                .unwrap_or((fallback_pwd.clone(), None));

            let serialized_block = SerializedBlock {
                id: BlockId::new(),
                stylized_command: Self::to_stylized_bytes(&command_block.command),
                stylized_output: Self::to_stylized_bytes(&command_block.output),
                pwd,
                git_head: None,
                git_branch_name: None,
                virtual_env: None,
                conda_env: None,
                node_version: None,
                exit_code: command_block.exit_code,
                did_execute: true,
                start_ts: command_block.start_ts.or(exchange_time),
                completed_ts: command_block.completed_ts.or(exchange_time),
                ps1: None,
                rprompt: None,
                honor_ps1: false,
                session_id: None,
                shell_host: None,
                is_background: false,
                prompt_snapshot: None,
                ai_metadata: command_block.ai_metadata,
                is_local: None,
                agent_view_visibility: Some(
                    AgentViewVisibility::new_from_conversation(self.id).into(),
                ),
            };
            serialized_blocks.push(SerializedBlockListItem::Command {
                block: Box::new(serialized_block),
            });
        }

        serialized_blocks
    }

    pub fn mark_action_as_reverted(
        &mut self,
        action_id: AIAgentActionId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) {
        self.reverted_action_ids.insert(action_id);
        self.write_updated_conversation_state(ctx);
    }

    pub fn is_action_reverted(&self, action_id: &AIAgentActionId) -> bool {
        self.reverted_action_ids.contains(action_id)
    }

    pub fn reverted_action_ids(&self) -> &HashSet<AIAgentActionId> {
        &self.reverted_action_ids
    }

    /// Truncates the conversation from the given exchange ID, removing all exchanges
    /// from that exchange onwards (inclusive). This is a lossy operation - the removed
    /// exchanges are permanently deleted from this conversation.
    ///
    /// Returns the set of exchange IDs that were removed.
    pub fn truncate_from_exchange(
        &mut self,
        from_exchange_id: AIAgentExchangeId,
        ctx: &mut ModelContext<BlocklistAIHistoryModel>,
    ) -> Result<HashSet<AIAgentExchangeId>, UpdateConversationError> {
        let all_exchanges: Vec<AIAgentExchangeId> =
            self.root_task_exchanges().map(|e| e.id).collect();

        let truncate_from_idx = all_exchanges
            .iter()
            .position(|id| *id == from_exchange_id)
            .ok_or(UpdateConversationError::ExchangeNotFound)?;

        let exchanges_to_remove: HashSet<AIAgentExchangeId> =
            all_exchanges[truncate_from_idx..].iter().copied().collect();

        if exchanges_to_remove.is_empty() {
            return Ok(exchanges_to_remove);
        }

        let message_ids_to_remove: HashSet<MessageId> = exchanges_to_remove
            .iter()
            .filter_map(|ex_id| self.exchange_with_id(*ex_id))
            .flat_map(|ex| ex.added_message_ids.iter().cloned())
            .collect();

        if let Some(new_todo_lists) = self.task_store.modify_root_task(|root_task| {
            root_task.truncate_exchanges_from(from_exchange_id);
            root_task.remove_messages(&message_ids_to_remove);

            // Return updated todo state
            derive_todo_lists_from_root_task(root_task)
        }) {
            self.todo_lists = new_todo_lists;
        }

        // Make sure we don't have stale code review comment state
        self.code_review = None;

        self.added_exchanges_by_response
            .retain(|_, added_exchanges| {
                if added_exchanges
                    .iter()
                    .all(|added| exchanges_to_remove.contains(&added.exchange_id))
                {
                    return false;
                }
                let _ = added_exchanges
                    .retain(|added| !exchanges_to_remove.contains(&added.exchange_id));
                true
            });

        self.hidden_exchanges
            .retain(|ex_id| !exchanges_to_remove.contains(ex_id));

        // Stale ones are harmless, but might as well remove stale reverted action IDs
        let mut new_reverted_action_ids = std::mem::take(&mut self.reverted_action_ids);
        new_reverted_action_ids.retain(|id| self.contains_action(id));
        self.reverted_action_ids = new_reverted_action_ids;

        let root_task_is_empty = self
            .task_store
            .root_task()
            .is_none_or(|task| task.exchanges_len() == 0);

        // If all exchanges were removed, reset the root task to optimistic state.
        // This allows the next message to go through the normal "first message" flow,
        // where the server will create a new task and we'll upgrade the optimistic task.
        if root_task_is_empty {
            let root_task_id = self.task_store.root_task_id().clone();
            self.task_store.remove(&root_task_id);
            let new_root_task = Task::new_optimistic_root();
            self.task_store.set_root_task(new_root_task);
            self.server_conversation_token = None;
        }

        self.write_updated_conversation_state(ctx);

        Ok(exchanges_to_remove)
    }
}

fn parse_orchestration_harness_type(value: &str) -> Harness {
    Harness::from_config_name(value)
        .or_else(|| Harness::parse_orchestration_harness(value))
        .unwrap_or(Harness::Unknown)
}

pub(super) fn update_todo_list_from_todo_op(
    todo_lists: &mut Vec<AIAgentTodoList>,
    op: api::message::update_todos::Operation,
) {
    use api::message::update_todos::Operation;

    match op {
        Operation::CreateTodoList(create_todo_list) => {
            todo_lists.push(
                AIAgentTodoList::default().with_pending_items(
                    create_todo_list
                        .initial_todos
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                ),
            );
        }
        Operation::UpdatePendingTodos(update_pending_todos) => {
            let updated_todo_list = todo_lists.pop().unwrap_or_default().with_pending_items(
                update_pending_todos
                    .updated_pending_todos
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            );
            todo_lists.push(updated_todo_list);
        }
        Operation::MarkTodosCompleted(completed_items) => {
            if let Some(todo_list) = todo_lists.last_mut() {
                todo_list.mark_todos_complete(completed_items.todo_ids);
            }
        }
    }
}

pub(super) fn update_comment_from_comment_operation(
    current_comment_state: &mut CodeReview,
    op: api::message::update_review_comments::Operation,
) -> usize {
    use api::message::update_review_comments::Operation;

    let mut resolved_count = 0usize;

    match op {
        Operation::AddressReviewComments(addressed_comments) => {
            for comment_id in addressed_comments.comment_ids {
                if let Some(item) = current_comment_state
                    .pending_comments
                    .iter()
                    .position(|item| item.id.to_string() == comment_id)
                    .map(|i| current_comment_state.pending_comments.remove(i))
                {
                    current_comment_state.addressed_comments.push(item);
                    resolved_count += 1;
                }
            }
        }
    }

    resolved_count
}

/// Cleans up temporary directories created by conversation search subagents.
///
/// When a SubagentResult comes back for a conversation_search subagent, the temp
/// directory containing materialized YAML files is no longer needed and should be removed.
fn cleanup_conversation_search_temp_dir(
    tool_call_id: &str,
    parent_task_id: &str,
    task_store: &TaskStore,
) {
    let parent_task_id = TaskId::new(parent_task_id.to_string());
    let Some(parent_task) = task_store.get(&parent_task_id) else {
        return;
    };

    // Find the Subagent tool call matching this tool_call_id.
    let subtask_id = parent_task.messages().find_map(|m| {
        let tc = m.tool_call()?;
        if tc.tool_call_id != tool_call_id {
            return None;
        }
        let sub = tc.subagent()?;
        sub.is_conversation_search().then(|| sub.task_id.clone())
    });

    let Some(subtask_id) = subtask_id else {
        return;
    };

    // Find the subtask and look for a FetchConversationResult with a directory_path.
    let subtask_id = TaskId::new(subtask_id);
    let Some(subtask) = task_store.get(&subtask_id) else {
        return;
    };

    let base_dir = super::conversation_yaml::base_dir();
    for msg in subtask.messages() {
        if let Some(api::message::Message::ToolCallResult(tcr)) = &msg.message {
            if let Some(api::message::tool_call_result::Result::FetchConversation(result)) =
                &tcr.result
            {
                if let Some(api::fetch_conversation_result::Result::Success(success)) =
                    &result.result
                {
                    let dir = std::path::Path::new(&success.directory_path);
                    if dir.starts_with(&base_dir) {
                        if let Err(e) = std::fs::remove_dir_all(dir) {
                            log::warn!(
                                "Failed to clean up conversation search temp dir {}: {e}",
                                dir.display(),
                            );
                        } else {
                            log::info!(
                                "Cleaned up conversation search temp dir: {}",
                                dir.display(),
                            );
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateConversationError {
    #[error("Exchange not found.")]
    ExchangeNotFound,
    #[error("Could not update task: {0:?}")]
    UpdateTask(#[from] UpdateTaskError),
    #[error("Could not update upgrade optimistic task for server task: {0:?}")]
    UpgradeOptimisticTask(#[from] UpgradeOptimisticTaskError),
    #[error("Could not extract messages: {0:?}")]
    ExtractMessages(#[from] ExtractMessagesError),
    #[error("Task not found.")]
    TaskNotFound,
    #[error("Task never initialized with CreateTask client action.")]
    TaskNotInitialized,
    #[error("Message not found.")]
    MessageNotFound,
    #[error("Attempted to update already-finished output.")]
    OutputAlreadyFinished,
    #[error("Attempted to update output that was never initialized.")]
    OutputNeverInitialized,
    #[error("Failed to convert API message to client type: {0}")]
    ConversionError(#[from] MessageToAIAgentOutputMessageError),
    #[error("No active task")]
    NoActiveTask,
    #[error("No pending request.")]
    NoPendingRequest,
}

/// A globally unique ID for a conversation with an AI agent.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AIConversationId(Uuid);

impl Display for AIConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AIConversationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AIConversationId {
    fn default() -> Self {
        Self::new()
    }
}

impl TryFrom<String> for AIConversationId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(Self(Uuid::try_parse(&value)?))
    }
}

/// The harness that produced an agent conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIAgentHarness {
    Oz,
    ClaudeCode,
    Gemini,
    Codex,
    Unknown,
}

/// Describes the format of the conversation transcript data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIAgentSerializedBlockFormat {
    JsonV1,
}

/// Describes the format capabilities of a conversation.
#[derive(Debug, Clone)]
pub struct AIAgentConversationFormat {
    /// Whether there is a Warp MAA task list available for this conversation.
    pub has_task_list: bool,
    /// The format of the TUI serialized block, if available.
    pub block_snapshot: Option<AIAgentSerializedBlockFormat>,
}

/// Metadata for an AI conversation, containing all information from the GraphQL API
/// except the full task list data.
#[derive(Debug, Clone)]
pub struct ServerAIConversationMetadata {
    /// The title of the conversation.
    pub title: String,

    /// The working directory where the conversation was started.
    pub working_directory: Option<String>,

    /// The harness that produced this conversation.
    pub harness: AIAgentHarness,

    /// Usage metadata including token counts, credits spent, etc.
    pub usage: ConversationUsageMetadata,

    /// Server metadata (revision, timestamps, creator info, etc.).
    pub metadata: crate::cloud_object::ServerMetadata,
    /// Public profile for the conversation's creator, when available.
    pub creator: Option<UserProfileWithUID>,

    /// Permissions for this conversation (space, guests, link sharing).
    pub permissions: crate::cloud_object::ServerPermissions,

    /// The ID of the associated ambient agent task, if any.
    pub ambient_agent_task_id: Option<crate::ai::ambient_agents::AmbientAgentTaskId>,

    /// The server conversation token used to identify this conversation on the server.
    pub server_conversation_token: ServerConversationToken,

    /// Artifacts (plans, PRs) created during this conversation.
    pub artifacts: Vec<Artifact>,
}

/// Returns an iterator over `AIAgentContext`s attached to inputs in the given `exchanges`, in the
/// same order in which they appeared.
pub(super) fn context_in_exchanges<'a>(
    exchanges: impl Iterator<Item = &'a AIAgentExchange> + 'a,
) -> impl Iterator<Item = &'a AIAgentContext> + 'a {
    exchanges.flat_map(|exchange| {
        exchange
            .input
            .iter()
            .filter_map(AIAgentInput::context)
            .flatten()
    })
}

impl AIAgentExchange {
    /// Returns an error if the output was already initialized.
    pub(super) fn init_output(
        &mut self,
        server_output_id: ServerOutputId,
    ) -> Result<(), UpdateTaskError> {
        match &mut self.output_status {
            AIAgentOutputStatus::Streaming { ref mut output } => {
                if let Some(shared_output) = output {
                    // We expect to initialize output that has already been initialized if we retry
                    // after receiving a StreamInit event but before receiving any ClientActions.
                    shared_output.get_mut().server_output_id = Some(server_output_id);
                } else {
                    *output = Some(Shared::new(AIAgentOutput {
                        messages: vec![],
                        citations: vec![],
                        server_output_id: Some(server_output_id),
                        api_metadata_bytes: None,
                        suggestions: None,
                        telemetry_events: vec![],
                        model_info: None,
                        request_cost: None,
                    }));
                }
                Ok(())
            }
            AIAgentOutputStatus::Finished { .. } => Err(UpdateTaskError::OutputAlreadyFinished),
        }
    }

    fn update_suggestions(&self, suggestions: api::Suggestions) {
        if let AIAgentOutputStatus::Streaming {
            output: Some(output),
        } = &self.output_status
        {
            let mut output = output.get_mut();
            output.suggestions = Some(suggestions.into());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AIConversationAutoexecuteMode {
    #[default]
    RespectUserSettings,
    RunToCompletion,
}

impl AIConversationAutoexecuteMode {
    pub fn is_autoexecute_any_action(&self) -> bool {
        matches!(self, AIConversationAutoexecuteMode::RunToCompletion)
    }
}

impl From<PersistedAutoexecuteMode> for AIConversationAutoexecuteMode {
    fn from(value: PersistedAutoexecuteMode) -> Self {
        match value {
            PersistedAutoexecuteMode::RespectUserSettings => Self::RespectUserSettings,
            PersistedAutoexecuteMode::RunToCompletion => Self::RunToCompletion,
        }
    }
}

impl From<AIConversationAutoexecuteMode> for PersistedAutoexecuteMode {
    fn from(value: AIConversationAutoexecuteMode) -> Self {
        match value {
            AIConversationAutoexecuteMode::RespectUserSettings => Self::RespectUserSettings,
            AIConversationAutoexecuteMode::RunToCompletion => Self::RunToCompletion,
        }
    }
}

#[derive(Clone, Copy)]
pub enum StatusColorStyle {
    /// Foreground-blend colors (`ansi_fg`) used by the regular status badge.
    Standard,
    /// Background-blend colors (`ansi_bg`) used by the cloud overlay badge.
    Cloud,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConversationStatus {
    /// Agent is running.
    InProgress,

    /// The last turn of the agent finished with success.
    Success,

    /// The last turn of the agent completed with error.
    Error,

    /// The last turn failed transiently and an automatic recovery (retry or resume)
    /// is pending. Non-terminal: returns to `InProgress` when the recovery request
    /// sends, or falls to `Error` if recovery is exhausted.
    TransientError,

    /// The last turn of the agent was cancelled by the user.
    Cancelled,

    /// The last turn of the agent resulted in an action whose execution is blocked by the user.
    Blocked { blocked_action: String },

    /// Agent yielded via wait_for_events and is listening for inbound
    /// input. Quiescent but not terminal.
    WaitingForEvents,
}

impl std::fmt::Display for ConversationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConversationStatus::InProgress => write!(f, "In progress"),
            ConversationStatus::Success => write!(f, "Done"),
            ConversationStatus::Error => write!(f, "Error"),
            ConversationStatus::TransientError => write!(f, "Reconnecting"),
            ConversationStatus::Cancelled => write!(f, "Cancelled"),
            ConversationStatus::Blocked { .. } => write!(f, "Blocked"),
            ConversationStatus::WaitingForEvents => write!(f, "Waiting"),
        }
    }
}

impl ConversationStatus {
    pub fn render_icon(&self, appearance: &Appearance) -> warpui::elements::Icon {
        match self {
            ConversationStatus::InProgress => in_progress_icon(appearance),
            ConversationStatus::Success => succeeded_icon(appearance),
            ConversationStatus::Blocked { .. } => yellow_stop_icon(appearance),
            ConversationStatus::Error => failed_icon(appearance),
            // Recovery pending: keep the in-progress treatment rather than an error one.
            ConversationStatus::TransientError => in_progress_icon(appearance),
            ConversationStatus::Cancelled => gray_stop_icon(appearance),
            ConversationStatus::WaitingForEvents => in_progress_icon(appearance),
        }
    }

    pub fn status_icon_and_color(
        &self,
        theme: &WarpTheme,
        color_style: StatusColorStyle,
    ) -> (Icon, ColorU) {
        match self {
            ConversationStatus::InProgress => (
                Icon::ClockLoader,
                match color_style {
                    StatusColorStyle::Standard => theme.ansi_fg_magenta(),
                    StatusColorStyle::Cloud => theme.ansi_bg_magenta(),
                },
            ),
            ConversationStatus::Success => (
                Icon::Check,
                match color_style {
                    StatusColorStyle::Standard => theme.ansi_fg_green(),
                    StatusColorStyle::Cloud => theme.ansi_bg_green(),
                },
            ),
            ConversationStatus::Error => (
                Icon::Triangle,
                match color_style {
                    StatusColorStyle::Standard => theme.ansi_fg_red(),
                    StatusColorStyle::Cloud => theme.ansi_bg_red(),
                },
            ),
            ConversationStatus::TransientError => (
                Icon::ClockLoader,
                match color_style {
                    StatusColorStyle::Standard => theme.ansi_fg_yellow(),
                    StatusColorStyle::Cloud => theme.ansi_bg_yellow(),
                },
            ),
            ConversationStatus::Cancelled => (Icon::StopFilled, internal_colors::neutral_5(theme)),
            ConversationStatus::Blocked { .. } => (
                Icon::StopFilled,
                match color_style {
                    StatusColorStyle::Standard => theme.ansi_fg_yellow(),
                    StatusColorStyle::Cloud => theme.ansi_bg_yellow(),
                },
            ),
            ConversationStatus::WaitingForEvents => (
                Icon::ClockLoader,
                match color_style {
                    StatusColorStyle::Standard => theme.ansi_fg_magenta(),
                    StatusColorStyle::Cloud => theme.ansi_bg_magenta(),
                },
            ),
        }
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(self, ConversationStatus::InProgress)
    }

    /// True while a transient failure is being automatically recovered.
    pub fn is_transient_error(&self) -> bool {
        matches!(self, ConversationStatus::TransientError)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, ConversationStatus::Blocked { .. })
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, ConversationStatus::Cancelled)
    }

    /// True iff the run is finished and cannot resume on its own.
    pub fn is_done(&self) -> bool {
        matches!(
            self,
            ConversationStatus::Success | ConversationStatus::Error | ConversationStatus::Cancelled
        )
    }

    /// True iff the agent has yielded via `wait_for_events` and is listening
    /// for inbound input.
    pub fn is_waiting_for_events(&self) -> bool {
        matches!(self, ConversationStatus::WaitingForEvents)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, ConversationStatus::Error)
    }
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
