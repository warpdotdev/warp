#[allow(dead_code)]
pub mod entry;

pub use entry::{
    AgentConversationEntry, AgentConversationEntryId, AgentConversationNavigationSubject,
    AgentConversationProvenance,
};

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::{
    AgentSource, AmbientAgentLiveSessionState, AmbientAgentTask, AmbientAgentTaskState,
};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, ConversationStatusUpdate,
};
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::conversation_navigation::ConversationNavigationData;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::AuthStateProvider;
use crate::network::{NetworkStatus, NetworkStatusEvent, NetworkStatusKind};
use crate::server::cloud_objects::update_manager::{UpdateManager, UpdateManagerEvent};
use crate::server::ids::{ServerId, SyncId};
use crate::server::retry_strategies::{
    is_transient_http_error, OUT_OF_BAND_REQUEST_RETRY_STRATEGY, PERIODIC_POLL_RETRY_STRATEGY,
};
use crate::server::server_api::{ai::TaskListFilter, ServerApiProvider};
use crate::settings::AISettings;
use crate::ui_components::icons::Icon;
use crate::workspace::{RestoreConversationLayout, WorkspaceAction};
use chrono::{DateTime, Utc};
use clap::ValueEnum;
use futures::stream::AbortHandle;
use instant::Instant;
use itertools::Itertools;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use warp_cli::agent::Harness;
use warp_core::execution_mode::AppExecutionMode;
use warp_core::features::FeatureFlag;
use warp_core::report_error;
use warp_core::ui::theme::{color::internal_colors, WarpTheme};
use warpui::color::ColorU;
use warpui::r#async::Timer;
use warpui::windowing::{StateEvent, WindowManager};
use warpui::{
    duration_with_jitter, AppContext, Entity, EntityId, ModelContext, RequestState,
    SingletonEntity, WindowId,
};

const POLLING_INTERVAL: Duration = Duration::from_secs(30);
const INITIAL_TASK_AMOUNT: i32 = 100;

/// How long to skip refetching a task that just failed with a transient error
/// (5xx / 408 / 429 / network). Short cooldown — `spawn_with_retry_on_error_when` already
/// runs fast exponential retries before bubbling up the failure, so this is just enough to
/// absorb streaming-driven re-entries from `update_transcript_details_panel_data`.
const TRANSIENT_FETCH_FAILURE_COOLDOWN: Duration = Duration::from_secs(2);

/// How long to skip refetching a task that just failed with a permanent (non-transient) HTTP
/// error such as 401/403/404. We don't refuse forever — permissions can change mid-session
/// (e.g. an ACL grant) — but we wait long enough that streaming bursts and rapid re-entries
/// can't cause a flood.
const PERMANENT_FETCH_FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

/// Per-task fetch state for `get_or_async_fetch_task_data`. The three variants are mutually
/// exclusive: a task id is either being fetched right now, in a short cooldown after a
/// transient failure, or in a longer cooldown after a permanent (non-transient) failure.
#[derive(Debug)]
enum TaskFetchState {
    /// A retry chain is currently outstanding for this task id. Used to dedupe re-entries
    /// (e.g. from streaming-driven panel refreshes) so we don't spawn overlapping retry
    /// chains for the same task id.
    InFlight,
    /// The fetch returned a permanent (non-transient) HTTP error such as 401/403/404; remember
    /// when it failed so we can back off for [`PERMANENT_FETCH_FAILURE_COOLDOWN`] before
    /// retrying. We don't refuse forever in case permissions change mid-session.
    /// The `String` carries a human-readable description of the failure for display in the UI.
    PermanentlyFailed { at: Instant, message: String },
    /// The retry chain just exhausted on a transient error; remember when it failed so we
    /// can back off for [`TRANSIENT_FETCH_FAILURE_COOLDOWN`] before retrying.
    /// The `String` carries a human-readable description of the failure for display in the UI.
    TransientlyFailed { at: Instant, message: String },
}

/// Protected eviction: we'll always keep at least 200 personal tasks in the model.
/// This is so that whenever we evict stale tasks, we do not evict relevant, recent personal tasks
/// (e.g. if I load in 500 team Slack tasks from today, we should _not_ evict my personal conversation
/// from yesterday).
const MAX_PERSONAL_TASKS: usize = 200;
const MAX_TEAM_TASKS: usize = 300;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    Available,
    Expired,
    Unavailable,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum StatusFilter {
    #[default]
    All,
    Working,
    Done,
    Failed,
}

impl StatusFilter {
    /// Returns `true` if a status transition from `prev_bucket` to `new_bucket` flips
    /// whether an item is included by this filter. `All` matches every bucket so it
    /// is never crossed; the other variants are crossed when exactly one of the buckets
    /// equals this filter.
    pub(crate) fn is_membership_crossed(
        self,
        prev_bucket: StatusFilter,
        new_bucket: StatusFilter,
    ) -> bool {
        match self {
            StatusFilter::All => false,
            StatusFilter::Working | StatusFilter::Done | StatusFilter::Failed => {
                (prev_bucket == self) != (new_bucket == self)
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SourceFilter {
    #[default]
    All,
    Specific(AgentSource),
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum CreatorFilter {
    #[default]
    All,
    Specific {
        name: String,
        uid: String,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ArtifactFilter {
    #[default]
    All,
    PullRequest,
    Plan,
    Screenshot,
    File,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum CreatedOnFilter {
    #[default]
    All,
    Last24Hours,
    Past3Days,
    LastWeek,
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum EnvironmentFilter {
    #[default]
    All,
    NoEnvironment,
    Specific(String),
}

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnerFilter {
    All,
    #[default]
    PersonalOnly,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum HarnessFilter {
    #[default]
    All,
    Specific(Harness),
}

impl Serialize for HarnessFilter {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            HarnessFilter::All => serializer.serialize_str("all"),
            HarnessFilter::Specific(harness) => serializer.collect_str(harness),
        }
    }
}

impl<'de> Deserialize<'de> for HarnessFilter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Harness::from_str(&raw, false)
            .ok()
            .map(HarnessFilter::Specific)
            .unwrap_or(HarnessFilter::All))
    }
}

#[derive(Default, PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub struct AgentManagementFilters {
    pub owners: OwnerFilter,
    pub status: StatusFilter,
    pub source: SourceFilter,
    pub created_on: CreatedOnFilter,
    pub creator: CreatorFilter,
    pub artifact: ArtifactFilter,
    #[serde(default)]
    pub environment: EnvironmentFilter,
    #[serde(default)]
    pub harness: HarnessFilter,
}

impl AgentManagementFilters {
    pub fn reset_all_but_owner(&mut self) {
        self.status = StatusFilter::default();
        self.source = SourceFilter::default();
        self.created_on = CreatedOnFilter::default();
        self.creator = CreatorFilter::default();
        self.artifact = ArtifactFilter::default();
        self.environment = EnvironmentFilter::default();
        self.harness = HarnessFilter::default();
    }

    pub fn is_filtering(&self) -> bool {
        self.status != StatusFilter::default()
            || self.source != SourceFilter::default()
            || self.created_on != CreatedOnFilter::default()
            || self.creator != CreatorFilter::default() && self.owners != OwnerFilter::PersonalOnly
            || self.artifact != ArtifactFilter::default()
            || self.environment != EnvironmentFilter::default()
            || self.harness != HarnessFilter::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRunDisplayStatus {
    /// Raw task-service lifecycle states. `from_task` only returns `TaskInProgress` while the
    /// task still has an active execution, or when there is no shadowed local conversation to
    /// provide a more granular status.
    TaskQueued,
    TaskPending,
    TaskClaimed,
    TaskInProgress,
    TaskSucceeded,
    TaskFailed,
    TaskError,
    TaskBlocked {
        blocked_action: String,
    },
    TaskCancelled,
    TaskUnknown,
    /// Conversation-derived lifecycle states, used for interactive conversations and for
    /// in-progress ambient tasks after they can be resolved to their shadowed local conversation.
    ConversationInProgress,
    ConversationSucceeded,
    ConversationError,
    ConversationBlocked {
        blocked_action: String,
    },
    ConversationCancelled,
}

impl AgentRunDisplayStatus {
    pub fn from_task(task: &AmbientAgentTask, app: &AppContext) -> Self {
        match &task.state {
            AmbientAgentTaskState::Queued
            | AmbientAgentTaskState::Pending
            | AmbientAgentTaskState::Claimed => Self::from_task_state(task),
            AmbientAgentTaskState::InProgress => {
                if task.has_active_execution() {
                    return Self::from_task_state(task);
                }
                let history_model = BlocklistAIHistoryModel::as_ref(app);
                entry::conversation_id_shadowed_by_task(task, history_model)
                    .and_then(|conversation_id| history_model.conversation(&conversation_id))
                    .map(|conversation| Self::from_conversation_status(conversation.status()))
                    .unwrap_or_else(|| Self::from_task_state(task))
            }
            AmbientAgentTaskState::Succeeded
            | AmbientAgentTaskState::Failed
            | AmbientAgentTaskState::Error
            | AmbientAgentTaskState::Blocked
            | AmbientAgentTaskState::Cancelled
            | AmbientAgentTaskState::Unknown => Self::from_task_state(task),
        }
    }

    pub fn from_conversation_status(status: &ConversationStatus) -> Self {
        match status {
            ConversationStatus::InProgress => Self::ConversationInProgress,
            ConversationStatus::Success => Self::ConversationSucceeded,
            ConversationStatus::Error => Self::ConversationError,
            ConversationStatus::Cancelled => Self::ConversationCancelled,
            ConversationStatus::Blocked { blocked_action } => Self::ConversationBlocked {
                blocked_action: blocked_action.clone(),
            },
        }
    }

    fn from_task_state(task: &AmbientAgentTask) -> Self {
        match &task.state {
            AmbientAgentTaskState::Queued => Self::TaskQueued,
            AmbientAgentTaskState::Pending => Self::TaskPending,
            AmbientAgentTaskState::Claimed => Self::TaskClaimed,
            AmbientAgentTaskState::InProgress => Self::TaskInProgress,
            AmbientAgentTaskState::Succeeded => Self::TaskSucceeded,
            AmbientAgentTaskState::Failed => Self::TaskFailed,
            AmbientAgentTaskState::Error => Self::TaskError,
            AmbientAgentTaskState::Blocked => Self::TaskBlocked {
                blocked_action: task
                    .status_message
                    .as_ref()
                    .map(|m| m.message.clone())
                    .unwrap_or_else(|| "Task blocked".to_string()),
            },
            AmbientAgentTaskState::Cancelled => Self::TaskCancelled,
            AmbientAgentTaskState::Unknown => Self::TaskUnknown,
        }
    }

    pub fn status_filter(&self) -> StatusFilter {
        match self {
            AgentRunDisplayStatus::TaskQueued
            | AgentRunDisplayStatus::TaskPending
            | AgentRunDisplayStatus::TaskClaimed
            | AgentRunDisplayStatus::TaskInProgress
            | AgentRunDisplayStatus::ConversationInProgress => StatusFilter::Working,
            AgentRunDisplayStatus::TaskSucceeded | AgentRunDisplayStatus::ConversationSucceeded => {
                StatusFilter::Done
            }
            AgentRunDisplayStatus::TaskFailed
            | AgentRunDisplayStatus::TaskError
            | AgentRunDisplayStatus::TaskBlocked { .. }
            | AgentRunDisplayStatus::TaskCancelled
            | AgentRunDisplayStatus::TaskUnknown
            | AgentRunDisplayStatus::ConversationError
            | AgentRunDisplayStatus::ConversationBlocked { .. }
            | AgentRunDisplayStatus::ConversationCancelled => StatusFilter::Failed,
        }
    }

    pub fn to_conversation_status(&self) -> ConversationStatus {
        match self {
            AgentRunDisplayStatus::TaskQueued
            | AgentRunDisplayStatus::TaskPending
            | AgentRunDisplayStatus::TaskClaimed
            | AgentRunDisplayStatus::TaskInProgress
            | AgentRunDisplayStatus::ConversationInProgress => ConversationStatus::InProgress,
            AgentRunDisplayStatus::TaskSucceeded | AgentRunDisplayStatus::ConversationSucceeded => {
                ConversationStatus::Success
            }
            AgentRunDisplayStatus::TaskFailed
            | AgentRunDisplayStatus::TaskError
            | AgentRunDisplayStatus::TaskUnknown
            | AgentRunDisplayStatus::ConversationError => ConversationStatus::Error,
            AgentRunDisplayStatus::TaskBlocked { blocked_action }
            | AgentRunDisplayStatus::ConversationBlocked { blocked_action } => {
                ConversationStatus::Blocked {
                    blocked_action: blocked_action.clone(),
                }
            }
            AgentRunDisplayStatus::TaskCancelled | AgentRunDisplayStatus::ConversationCancelled => {
                ConversationStatus::Cancelled
            }
        }
    }

    pub fn is_cancellable(&self) -> bool {
        self.is_working()
    }

    pub fn is_working(&self) -> bool {
        matches!(
            self,
            AgentRunDisplayStatus::TaskQueued
                | AgentRunDisplayStatus::TaskPending
                | AgentRunDisplayStatus::TaskClaimed
                | AgentRunDisplayStatus::TaskInProgress
                | AgentRunDisplayStatus::ConversationInProgress
        )
    }

    pub fn status_icon_and_color(&self, theme: &WarpTheme) -> (Icon, ColorU) {
        match self {
            AgentRunDisplayStatus::TaskQueued
            | AgentRunDisplayStatus::TaskPending
            | AgentRunDisplayStatus::TaskClaimed
            | AgentRunDisplayStatus::TaskInProgress
            | AgentRunDisplayStatus::ConversationInProgress => {
                (Icon::ClockLoader, theme.ansi_fg_magenta())
            }
            AgentRunDisplayStatus::TaskSucceeded | AgentRunDisplayStatus::ConversationSucceeded => {
                (Icon::Check, theme.ansi_fg_green())
            }
            AgentRunDisplayStatus::TaskFailed
            | AgentRunDisplayStatus::TaskError
            | AgentRunDisplayStatus::TaskUnknown
            | AgentRunDisplayStatus::ConversationError => (Icon::Triangle, theme.ansi_fg_red()),
            AgentRunDisplayStatus::TaskBlocked { .. }
            | AgentRunDisplayStatus::ConversationBlocked { .. } => {
                (Icon::StopFilled, theme.ansi_fg_yellow())
            }
            AgentRunDisplayStatus::TaskCancelled => (
                Icon::Cancelled,
                theme.disabled_text_color(theme.background()).into_solid(),
            ),
            AgentRunDisplayStatus::ConversationCancelled => {
                (Icon::StopFilled, internal_colors::neutral_5(theme))
            }
        }
    }
}

impl std::fmt::Display for AgentRunDisplayStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentRunDisplayStatus::TaskQueued => write!(f, "Queued"),
            AgentRunDisplayStatus::TaskPending => write!(f, "Pending"),
            AgentRunDisplayStatus::TaskClaimed => write!(f, "Claimed"),
            AgentRunDisplayStatus::TaskInProgress
            | AgentRunDisplayStatus::ConversationInProgress => write!(f, "In progress"),
            AgentRunDisplayStatus::TaskSucceeded | AgentRunDisplayStatus::ConversationSucceeded => {
                write!(f, "Done")
            }
            AgentRunDisplayStatus::TaskFailed => write!(f, "Failed"),
            AgentRunDisplayStatus::TaskError | AgentRunDisplayStatus::ConversationError => {
                write!(f, "Error")
            }
            AgentRunDisplayStatus::TaskBlocked { .. }
            | AgentRunDisplayStatus::ConversationBlocked { .. } => write!(f, "Blocked"),
            AgentRunDisplayStatus::TaskCancelled | AgentRunDisplayStatus::ConversationCancelled => {
                write!(f, "Cancelled")
            }
            AgentRunDisplayStatus::TaskUnknown => write!(f, "Failed"),
        }
    }
}

/// Stores conversation metadata needed for display in conversation/task views.
pub struct ConversationMetadata {
    pub nav_data: ConversationNavigationData,
}

pub(crate) fn artifacts_match_filter(
    artifacts: &[Artifact],
    artifact_filter: &ArtifactFilter,
) -> bool {
    match artifact_filter {
        ArtifactFilter::All => true,
        ArtifactFilter::PullRequest => artifacts
            .iter()
            .any(|artifact| matches!(artifact, Artifact::PullRequest { .. })),
        ArtifactFilter::Plan => artifacts
            .iter()
            .any(|artifact| matches!(artifact, Artifact::Plan { .. })),
        ArtifactFilter::Screenshot => artifacts
            .iter()
            .any(|artifact| matches!(artifact, Artifact::Screenshot { .. })),
        ArtifactFilter::File => artifacts
            .iter()
            .any(|artifact| matches!(artifact, Artifact::File { .. })),
    }
}

/// This model serves as a unified interface for reading both local and ambient agent conversations
/// (i.e. conversations & tasks). The model is responsible for polling for new tasks and updating
/// its local state accordingly.
///
/// This model backs both the agent management view and the conversation list view.
pub struct AgentConversationsModel {
    /// A map of task IDs to agent tasks.
    tasks: HashMap<AmbientAgentTaskId, AmbientAgentTask>,
    /// A map of conversation IDs to local conversations.
    conversations: HashMap<AIConversationId, ConversationMetadata>,
    /// Handle to abort the in-flight polling request.
    in_flight_poll_abort_handle: Option<AbortHandle>,
    /// Handle to abort the timer for initiating the next poll.
    next_poll_abort_handle: Option<AbortHandle>,
    /// Set of view IDs actively consuming this model's data per window.
    /// When a window has at least one consumer, we poll for new tasks while that window is active.
    active_data_consumers_per_window: HashMap<WindowId, HashSet<EntityId>>,
    /// Whether we have finished the initial task load
    has_finished_initial_load: bool,
    /// Per-task fetch state for `get_or_async_fetch_task_data`. See [`TaskFetchState`] for
    /// the meaning of each variant. Tasks that have been successfully fetched live in `tasks`
    /// and are absent from this map.
    task_fetch_state: HashMap<AmbientAgentTaskId, TaskFetchState>,
}

pub enum AgentConversationsModelEvent {
    /// Initial load of tasks completed.
    ConversationsLoaded,
    /// New tasks were received during polling (view should diff against its local state).
    NewTasksReceived,
    /// Existing task data may have been updated (e.g., state changes).
    TasksUpdated,
    /// Conversation status data was updated
    ConversationUpdated { kind: ConversationUpdateKind },
    /// Conversation artifacts were updated (plans, PRs, etc.)
    ConversationArtifactsUpdated { conversation_id: AIConversationId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationUpdateKind {
    /// The conversation was re-loaded into a terminal view.
    Restored,
    /// The conversation's status was set.
    StatusSet {
        prev_filter: StatusFilter,
        new_filter: StatusFilter,
    },
    /// Conversation metadata or capabilities changed.
    MetadataChanged,
}

impl Entity for AgentConversationsModel {
    type Event = AgentConversationsModelEvent;
}

impl SingletonEntity for AgentConversationsModel {}

impl AgentConversationsModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // If FF not enabled, return an empty model and don't sync any tasks.
        if !FeatureFlag::AgentManagementView.is_enabled() {
            return Self {
                tasks: HashMap::new(),
                conversations: HashMap::new(),
                in_flight_poll_abort_handle: None,
                next_poll_abort_handle: None,
                active_data_consumers_per_window: HashMap::new(),
                has_finished_initial_load: true,
                task_fetch_state: HashMap::new(),
            };
        }

        // Subscribe to network status and window manager to inform whether we should poll for new task data
        let network_status = NetworkStatus::handle(ctx);
        ctx.subscribe_to_model(&network_status, Self::handle_network_status_changed);
        let window_manager = WindowManager::handle(ctx);
        ctx.subscribe_to_model(&window_manager, Self::handle_window_state_changed);

        // Subscribe to auth events to retry initial sync when user becomes available
        let auth_manager = AuthManager::handle(ctx);
        ctx.subscribe_to_model(&auth_manager, Self::handle_auth_manager_event);

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, move |me, event, ctx| {
            me.handle_history_event(event, ctx);
        });

        let active_views_model = ActiveAgentViewsModel::handle(ctx);
        ctx.subscribe_to_model(&active_views_model, |me, _event, ctx| {
            me.sync_conversations(ctx);
        });

        // Subscribe to UpdateManager for RTC task updates
        if FeatureFlag::AmbientAgentsRTC.is_enabled() {
            let update_manager = UpdateManager::handle(ctx);
            ctx.subscribe_to_model(&update_manager, Self::handle_update_manager_event);
        }

        let mut model = Self {
            tasks: HashMap::new(),
            conversations: HashMap::new(),
            in_flight_poll_abort_handle: None,
            next_poll_abort_handle: None,
            active_data_consumers_per_window: HashMap::new(),
            has_finished_initial_load: false,
            task_fetch_state: HashMap::new(),
        };

        // Only sync local conversations if we're not in CLI mode. Server-side data
        // (tasks and cloud conversation metadata) is fetched on AuthComplete instead of
        // here to avoid duplicate requests at startup.
        if AppExecutionMode::as_ref(ctx).can_fetch_agent_runs_for_management() {
            model.sync_conversations(ctx);
        } else {
            model.has_finished_initial_load = true;
        }
        model
    }

    pub fn is_loading(&self) -> bool {
        !self.has_finished_initial_load
    }

    fn handle_network_status_changed(
        &mut self,
        event: &NetworkStatusEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            NetworkStatusEvent::NetworkStatusChanged { new_status } => match new_status {
                NetworkStatusKind::Online => {
                    self.update_polling_state(ctx);
                }
                NetworkStatusKind::Offline => {
                    self.abort_existing_poll();
                }
            },
        }
    }

    fn handle_window_state_changed(&mut self, event: &StateEvent, ctx: &mut ModelContext<Self>) {
        match event {
            StateEvent::ValueChanged { current, previous } => {
                // If the active window changed, check if we need to start/stop polling
                if current.active_window != previous.active_window {
                    self.update_polling_state(ctx);
                }
            }
        }
    }

    fn handle_auth_manager_event(
        &mut self,
        event: &AuthManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // When auth completes, retry the initial task sync if we haven't loaded tasks yet
        // Only sync if we're not in CLI mode
        if matches!(event, AuthManagerEvent::AuthComplete)
            && !self.has_finished_initial_load
            && AppExecutionMode::as_ref(ctx).can_fetch_agent_runs_for_management()
        {
            self.fetch_ambient_agent_tasks_and_cloud_convo_metadata(ctx);
        }
    }

    fn handle_update_manager_event(
        &mut self,
        event: &UpdateManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if let UpdateManagerEvent::AmbientTaskUpdated { timestamp } = event {
            self.fetch_tasks_updated_after(*timestamp, ctx);
        }
    }

    /// Fetch tasks updated after the given timestamp (minus 1 second buffer since server uses `>` not `>=`).
    fn fetch_tasks_updated_after(
        &mut self,
        timestamp: DateTime<Utc>,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        // Subtract 1 second to give buffer for clock differences with server
        let updated_after = timestamp - chrono::Duration::seconds(1);

        ctx.spawn_with_retry_on_error(
            move || {
                let ai_client = ai_client.clone();
                async move {
                    ai_client
                        .list_ambient_agent_tasks(
                            INITIAL_TASK_AMOUNT,
                            TaskListFilter {
                                updated_after: Some(updated_after),
                                ..Default::default()
                            },
                        )
                        .await
                }
            },
            OUT_OF_BAND_REQUEST_RETRY_STRATEGY,
            |model, result, ctx| {
                if let RequestState::RequestSucceeded(tasks) = result {
                    model.update_model_with_new_tasks(tasks, ctx);
                } else if let RequestState::RequestFailed(e) = result {
                    report_error!(e);
                }
            },
        );
    }

    /// Sync all conversations to the AgentConversationsModel.
    ///
    /// This function will loop through all active panes, recently closed panes, and historical
    /// conversations to construct a complete snapshot of conversations.
    pub fn sync_conversations(&mut self, ctx: &mut ModelContext<Self>) {
        if !FeatureFlag::InteractiveConversationManagementView.is_enabled() {
            return;
        }

        let nav_data_list = ConversationNavigationData::all_conversations(ctx);

        self.conversations.clear();
        for nav_data in nav_data_list {
            let conversation_id = nav_data.id;
            let metadata = ConversationMetadata { nav_data };
            self.conversations.insert(conversation_id, metadata);
        }

        ctx.emit(AgentConversationsModelEvent::ConversationsLoaded);
    }

    /// Fetches tasks and cloud conversation metadata async. Cloud conversation metadata is merged with
    /// metadata stored in local db in the BlocklistAIHistoryModel
    fn fetch_ambient_agent_tasks_and_cloud_convo_metadata(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(creator_uid) = AuthStateProvider::as_ref(ctx)
            .get()
            .user_id()
            .map(|uid| uid.as_string())
        else {
            // If we don't have a user ID, don't pull tasks
            return;
        };

        let ai_settings = AISettings::as_ref(ctx);
        if !ai_settings.is_any_ai_enabled(ctx) {
            // If we don't have AI enabled, don't pull tasks
            return;
        }

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        ctx.spawn_with_retry_on_error(
            move || {
                let ai_client = ai_client.clone();
                let creator_uid = creator_uid.clone();
                async move {
                    // Fetch personal tasks only on initialization; team tasks fetched by the view model when filters applied
                    let personal_future = ai_client.list_ambient_agent_tasks(
                        INITIAL_TASK_AMOUNT,
                        TaskListFilter {
                            creator_uid: Some(creator_uid),
                            ..Default::default()
                        },
                    );
                    let conversation_metadata_future =
                        ai_client.list_ai_conversation_metadata(None);

                    let (personal_result, conversation_metadata_result) =
                        futures::future::join(personal_future, conversation_metadata_future).await;

                    // Handle tasks result
                    let tasks = match personal_result {
                        Ok(tasks) => tasks,
                        Err(e) => {
                            log::warn!("Failed to fetch ambient agent tasks: {e:?}");
                            vec![]
                        }
                    };

                    // Handle conversation metadata result
                    let mut conversation_metadata = match conversation_metadata_result {
                        Ok(metadata) => metadata,
                        Err(e) => {
                            log::warn!("Failed to fetch conversation metadata: {e:?}");
                            vec![]
                        }
                    };

                    // Collect all conversation IDs from tasks
                    let task_conversation_ids: HashSet<String> = tasks
                        .iter()
                        .filter_map(|task| task.conversation_id().map(str::to_string))
                        .collect();

                    // Build a set of conversation IDs we already have
                    let fetched_conversation_ids: HashSet<String> = conversation_metadata
                        .iter()
                        .map(|meta| meta.server_conversation_token.as_str().to_string())
                        .collect();

                    // Find conversation IDs that are in tasks but not in the initial metadata fetch
                    let missing_conversation_ids: Vec<String> = task_conversation_ids
                        .difference(&fetched_conversation_ids)
                        .cloned()
                        .collect();

                    // If there are missing conversation IDs, fetch their metadata
                    if !missing_conversation_ids.is_empty() {
                        log::info!(
                            "Fetching {} missing conversation metadata entries for ambient agent tasks",
                            missing_conversation_ids.len()
                        );
                        match ai_client
                            .list_ai_conversation_metadata(Some(missing_conversation_ids))
                            .await
                        {
                            Ok(additional_metadata) => {
                                log::info!(
                                    "Fetched {} additional conversation metadata entries",
                                    additional_metadata.len()
                                );
                                conversation_metadata.extend(additional_metadata);
                            }
                            Err(e) => {
                                log::warn!("Failed to fetch additional conversation metadata: {e:?}");
                            }
                        }
                    }

                    // Always return success - we handle failures individually above
                    Ok((tasks, conversation_metadata))
                }
            },
            OUT_OF_BAND_REQUEST_RETRY_STRATEGY,
            |model, result, ctx| {
                if let RequestState::RequestSucceeded((tasks, conversation_metadata)) = result {
                    model.has_finished_initial_load = true;

                    // Update tasks if we got any
                    if !tasks.is_empty() {
                        log::info!("Updating model with {} tasks", tasks.len());
                        for task in tasks {
                            model.tasks.insert(task.task_id, task);
                        }
                    }

                    // Update BlocklistAIHistoryModel with cloud conversation metadata if we got any
                    if !conversation_metadata.is_empty() {
                        log::info!(
                            "Fetched {} cloud conversation metadata entries total",
                            conversation_metadata.len()
                        );
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, _| {
                            history_model.merge_cloud_conversation_metadata(conversation_metadata);
                        });
                    }

                    // Sync conversations to refresh local cache
                    model.sync_conversations(ctx);

                    model.update_polling_state(ctx);
                    ctx.emit(AgentConversationsModelEvent::ConversationsLoaded);
                } else if let RequestState::RequestFailed(e) = result {
                    model.has_finished_initial_load = true;
                    model.update_polling_state(ctx);
                    report_error!(e);
                }
            },
        );
    }

    /// Called when a view that consumes this model's data becomes visible.
    /// Uses view_id to make registration idempotent.
    pub fn register_view_open(
        &mut self,
        window_id: WindowId,
        view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.active_data_consumers_per_window
            .entry(window_id)
            .or_default()
            .insert(view_id);
        self.update_polling_state(ctx);
    }

    /// Called when a view that consumes this model's data becomes hidden.
    /// Uses view_id to make unregistration idempotent.
    pub fn register_view_closed(
        &mut self,
        window_id: WindowId,
        view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(views) = self.active_data_consumers_per_window.get_mut(&window_id) {
            views.remove(&view_id);
            if views.is_empty() {
                self.active_data_consumers_per_window.remove(&window_id);
            }
        }
        self.update_polling_state(ctx);
    }

    /// Updates the polling state based on whether the active window has the view open.
    fn update_polling_state(&mut self, ctx: &mut ModelContext<Self>) {
        let should_poll = self.should_be_polling(ctx);

        if should_poll && self.next_poll_abort_handle.is_none() {
            self.poll_for_tasks(ctx);
        } else if !should_poll {
            self.abort_existing_poll();
        }
    }

    /// Returns true if we should be polling: online, not loading, and active window has the view open.
    fn should_be_polling(&self, ctx: &ModelContext<Self>) -> bool {
        if !self.has_finished_initial_load {
            return false;
        }

        // Don't poll if we're using RTC
        if FeatureFlag::AmbientAgentsRTC.is_enabled() {
            return false;
        }

        let is_online = NetworkStatus::as_ref(ctx).is_online();

        if !is_online {
            return false;
        }

        let active_window = WindowManager::as_ref(ctx).active_window();

        match active_window {
            Some(window_id) => self
                .active_data_consumers_per_window
                .get(&window_id)
                .is_some_and(|views| !views.is_empty()),
            None => false,
        }
    }

    /// Abort the current in-flight poll (does NOT abort initial sync)
    fn abort_existing_poll(&mut self) {
        if let Some(handle) = self.next_poll_abort_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.in_flight_poll_abort_handle.take() {
            handle.abort();
        }
    }

    fn schedule_next_poll(&mut self, ctx: &mut ModelContext<Self>) {
        let future_handle = ctx.spawn(
            async move {
                Timer::after(duration_with_jitter(POLLING_INTERVAL, 0.2)).await;
            },
            |model, _, ctx| {
                model.poll_for_tasks(ctx);
            },
        );
        self.next_poll_abort_handle = Some(future_handle.abort_handle());
    }

    fn poll_for_tasks(&mut self, ctx: &mut ModelContext<Self>) {
        self.abort_existing_poll();
        if !self.should_be_polling(ctx) {
            return;
        }

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        let future = ctx.spawn_with_retry_on_error(
            move || {
                let ai_client = ai_client.clone();
                async move {
                    ai_client
                        .list_ambient_agent_tasks(100, TaskListFilter::default())
                        .await
                }
            },
            PERIODIC_POLL_RETRY_STRATEGY,
            |model, result, ctx| {
                let should_poll_again = !result.has_pending_retries();

                if let RequestState::RequestSucceeded(tasks) = result {
                    model.update_model_with_new_tasks(tasks, ctx);
                }

                if should_poll_again {
                    model.schedule_next_poll(ctx);
                }
            },
        );

        self.in_flight_poll_abort_handle = Some(future.abort_handle());
    }

    // Update the model with new tasks retrieved from the server.
    fn update_model_with_new_tasks(
        &mut self,
        tasks: Vec<AmbientAgentTask>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut has_new_tasks = false;
        let mut has_updated_tasks = false;

        for task in tasks {
            let task_id = task.task_id;
            match self.tasks.get(&task_id) {
                Some(existing_task) => {
                    if existing_task != &task {
                        has_updated_tasks = true
                    }
                }
                None => has_new_tasks = true,
            };
            self.tasks.insert(task_id, task);
        }

        if has_new_tasks {
            ctx.emit(AgentConversationsModelEvent::NewTasksReceived);
        } else if has_updated_tasks {
            ctx.emit(AgentConversationsModelEvent::TasksUpdated);
        }
    }

    /// Returns true if we have tasks or local conversations in this view
    pub fn has_items(&self) -> bool {
        !self.tasks.is_empty() || !self.conversations.is_empty()
    }

    /// Returns an iterator over all ambient agent tasks.
    pub fn tasks_iter(&self) -> impl Iterator<Item = &AmbientAgentTask> {
        self.tasks.values()
    }

    #[cfg(test)]
    pub(crate) fn insert_task_for_test(&mut self, task: AmbientAgentTask) {
        self.tasks.insert(task.task_id, task);
    }

    pub(crate) fn mark_task_execution_ended(
        &mut self,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(task) = self.tasks.get_mut(&task_id) else {
            return;
        };
        let was_active = task.has_active_execution();
        task.is_sandbox_running = false;
        if was_active {
            ctx.emit(AgentConversationsModelEvent::TasksUpdated);
        }
    }

    /// Returns normalized, owned entries for agent management/navigation surfaces.
    pub fn get_entries(
        &self,
        filters: &AgentManagementFilters,
        app: &AppContext,
    ) -> Vec<AgentConversationEntry> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let mut entries = Vec::new();
        let mut attached_conversation_ids = HashSet::new();
        let mut emitted_conversation_ids = HashSet::new();

        for task in self.tasks.values() {
            let entry = entry::entry_for_task(task, history_model, app);
            if let Some(conversation_id) = entry.identity.local_conversation_id {
                attached_conversation_ids.insert(conversation_id);
            }
            entries.push(entry);
        }

        for metadata in self.conversations.values() {
            let conversation_id = metadata.nav_data.id;
            if attached_conversation_ids.contains(&conversation_id) {
                continue;
            }
            let entry = entry::entry_for_conversation(metadata, history_model, app);
            emitted_conversation_ids.insert(conversation_id);
            entries.push(entry);
        }

        for metadata in history_model.get_local_conversations_metadata() {
            if attached_conversation_ids.contains(&metadata.id)
                || emitted_conversation_ids.contains(&metadata.id)
            {
                continue;
            }
            let nav_data =
                ConversationNavigationData::from_historical_conversation_metadata(metadata);
            entries.push(entry::entry_for_historical_metadata(
                metadata,
                nav_data,
                history_model,
                app,
            ));
        }

        entries
            .into_iter()
            .filter(|entry| entry.matches_filters(filters, app))
            .sorted_by(|a, b| b.display.last_updated.cmp(&a.display.last_updated))
            .collect()
    }

    pub fn get_entry_by_id(
        &self,
        id: &AgentConversationEntryId,
        app: &AppContext,
    ) -> Option<AgentConversationEntry> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        match id {
            AgentConversationEntryId::AmbientRun(task_id) => self
                .tasks
                .get(task_id)
                .map(|task| entry::entry_for_task(task, history_model, app)),
            AgentConversationEntryId::Conversation(conversation_id) => self
                .conversations
                .get(conversation_id)
                .map(|metadata| entry::entry_for_conversation(metadata, history_model, app))
                .or_else(|| {
                    history_model
                        .get_conversation_metadata(conversation_id)
                        .map(|metadata| {
                            let nav_data =
                                ConversationNavigationData::from_historical_conversation_metadata(
                                    metadata,
                                );
                            entry::entry_for_historical_metadata(
                                metadata,
                                nav_data,
                                history_model,
                                app,
                            )
                        })
                }),
        }
    }

    pub fn resolve_open_action(
        subject: AgentConversationNavigationSubject,
        restore_layout: Option<RestoreConversationLayout>,
        app: &AppContext,
    ) -> Option<WorkspaceAction> {
        let model = Self::as_ref(app);
        match subject {
            AgentConversationNavigationSubject::Entry(id) => model
                .get_entry_by_id(&id, app)
                .and_then(|entry| model.resolve_entry_open_action(&entry, restore_layout, app)),
            AgentConversationNavigationSubject::ServerToken(server_token) => model
                .entry_for_server_token(&server_token, app)
                .and_then(|entry| model.resolve_entry_open_action(&entry, restore_layout, app))
                .or_else(|| {
                    Some(WorkspaceAction::OpenConversationTranscriptViewer {
                        ambient_agent_task_id: model.task_id_for_server_token(&server_token),
                        conversation_id: server_token,
                    })
                }),
        }
    }

    pub fn resolve_copy_link(
        subject: AgentConversationNavigationSubject,
        app: &AppContext,
    ) -> Option<String> {
        let model = Self::as_ref(app);
        match subject {
            AgentConversationNavigationSubject::Entry(id) => model
                .get_entry_by_id(&id, app)
                .and_then(|entry| model.resolve_entry_copy_link(&entry)),
            AgentConversationNavigationSubject::ServerToken(server_token) => model
                .entry_for_server_token(&server_token, app)
                .and_then(|entry| model.resolve_entry_copy_link(&entry))
                .or_else(|| Some(server_token.conversation_link())),
        }
    }

    fn resolve_entry_open_action(
        &self,
        entry: &AgentConversationEntry,
        restore_layout: Option<RestoreConversationLayout>,
        app: &AppContext,
    ) -> Option<WorkspaceAction> {
        let active_views_model = ActiveAgentViewsModel::as_ref(app);

        if let Some(task_id) = entry.identity.ambient_agent_task_id {
            match self
                .tasks
                .get(&task_id)
                .map(AmbientAgentTask::active_live_session_state)
            {
                Some(AmbientAgentLiveSessionState::Attachable { session_id }) => {
                    return Some(WorkspaceAction::OpenOrAttachAmbientAgentConversation {
                        session_id,
                        task_id,
                    });
                }
                Some(AmbientAgentLiveSessionState::ActiveUnattachable) => {
                    return active_views_model
                        .get_terminal_view_id_for_ambient_task(task_id)
                        .map(
                            |terminal_view_id| WorkspaceAction::FocusTerminalViewInWorkspace {
                                terminal_view_id,
                            },
                        );
                }
                Some(AmbientAgentLiveSessionState::Inactive) | None => {}
            }

            if let Some(terminal_view_id) =
                active_views_model.get_terminal_view_id_for_ambient_task(task_id)
            {
                return Some(WorkspaceAction::FocusTerminalViewInWorkspace { terminal_view_id });
            }
        }

        if let Some(conversation_id) = entry.identity.local_conversation_id {
            if active_views_model.is_conversation_open(conversation_id, app) {
                if let Some(nav_data) = self
                    .conversations
                    .get(&conversation_id)
                    .map(|metadata| &metadata.nav_data)
                {
                    return Some(WorkspaceAction::RestoreOrNavigateToConversation {
                        conversation_id,
                        window_id: nav_data.window_id,
                        pane_view_locator: nav_data.pane_view_locator,
                        terminal_view_id: nav_data.terminal_view_id,
                        restore_layout,
                    });
                }

                if let Some(terminal_view_id) =
                    active_views_model.get_terminal_view_id_for_conversation(conversation_id, app)
                {
                    return Some(WorkspaceAction::FocusTerminalViewInWorkspace {
                        terminal_view_id,
                    });
                }
            }
        }

        if let Some(conversation_id) = entry.identity.local_conversation_id {
            let nav_data = self
                .conversations
                .get(&conversation_id)
                .map(|metadata| &metadata.nav_data);
            if !entry.backing.has_cloud_data
                || entry.backing.has_local_persisted_data
                || entry.backing.has_loaded_conversation
                || nav_data.is_some()
            {
                return Some(WorkspaceAction::RestoreOrNavigateToConversation {
                    conversation_id,
                    window_id: nav_data.and_then(|nav_data| nav_data.window_id),
                    pane_view_locator: None,
                    terminal_view_id: nav_data.and_then(|nav_data| nav_data.terminal_view_id),
                    restore_layout,
                });
            }
        }

        entry
            .identity
            .server_conversation_token
            .as_ref()
            .map(|token| WorkspaceAction::OpenConversationTranscriptViewer {
                conversation_id: token.clone(),
                ambient_agent_task_id: entry.identity.ambient_agent_task_id,
            })
    }

    fn resolve_entry_copy_link(&self, entry: &AgentConversationEntry) -> Option<String> {
        if let Some(task_id) = entry.identity.ambient_agent_task_id {
            if let Some(session_link) = self.tasks.get(&task_id).and_then(|task| {
                task.has_active_execution()
                    .then(|| {
                        task.active_run_execution()
                            .session_link
                            .map(ToString::to_string)
                    })
                    .flatten()
            }) {
                return Some(session_link);
            }
        }

        entry
            .identity
            .server_conversation_token
            .as_ref()
            .map(ServerConversationToken::conversation_link)
    }

    fn entry_for_server_token(
        &self,
        server_token: &ServerConversationToken,
        app: &AppContext,
    ) -> Option<AgentConversationEntry> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        if let Some(task) = self.tasks.values().find(|task| {
            task.conversation_id()
                .is_some_and(|conversation_id| conversation_id == server_token.as_str())
        }) {
            return Some(entry::entry_for_task(task, history_model, app));
        }

        let conversation_id = history_model.find_conversation_id_by_server_token(server_token)?;
        if let Some(task) = self.tasks.values().find(|task| {
            entry::conversation_id_shadowed_by_task(task, history_model) == Some(conversation_id)
        }) {
            return Some(entry::entry_for_task(task, history_model, app));
        }

        self.get_entry_by_id(
            &AgentConversationEntryId::Conversation(conversation_id),
            app,
        )
    }

    fn task_id_for_server_token(
        &self,
        server_token: &ServerConversationToken,
    ) -> Option<AmbientAgentTaskId> {
        self.tasks.values().find_map(|task| {
            task.conversation_id()
                .is_some_and(|conversation_id| conversation_id == server_token.as_str())
                .then_some(task.task_id)
        })
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::InteractiveConversationManagementView.is_enabled() {
            return;
        }
        match event {
            // Events that affect conversation navigation data - need full sync
            BlocklistAIHistoryEvent::StartedNewConversation { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::SplitConversation { .. }
            | BlocklistAIHistoryEvent::RestoredConversations { .. }
            | BlocklistAIHistoryEvent::RemoveConversation { .. }
            | BlocklistAIHistoryEvent::DeletedConversation { .. }
            | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
            | BlocklistAIHistoryEvent::ClearedActiveConversation { .. } => {
                self.sync_conversations(ctx);
            }

            // Status changes - just trigger re-render since status is looked up at render time
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                update, new_status, ..
            } => {
                let kind = match update {
                    ConversationStatusUpdate::Restored => ConversationUpdateKind::Restored,
                    ConversationStatusUpdate::Changed { prev_status } => {
                        ConversationUpdateKind::StatusSet {
                            prev_filter: AgentRunDisplayStatus::from_conversation_status(
                                prev_status,
                            )
                            .status_filter(),
                            new_filter: AgentRunDisplayStatus::from_conversation_status(new_status)
                                .status_filter(),
                        }
                    }
                };
                ctx.emit(AgentConversationsModelEvent::ConversationUpdated { kind });
            }

            // Artifact changes - sync live artifacts into the cached task and notify.
            BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                conversation_id, ..
            } => {
                let conversation = BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id);
                let Some(conversation) = conversation else {
                    return;
                };

                let task_id = conversation
                    .server_metadata()
                    .and_then(|metadata| metadata.ambient_agent_task_id);
                if let Some(task_id) = task_id {
                    // If the conversation is associated with a task, update the saved task
                    // with live artifacts.
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        task.artifacts = conversation.artifacts().to_vec();
                        ctx.emit(AgentConversationsModelEvent::TasksUpdated);
                    }
                }
                ctx.emit(AgentConversationsModelEvent::ConversationArtifactsUpdated {
                    conversation_id: *conversation_id,
                });
            }

            // Task/exchange-level changes that don't affect conversation navigation.
            BlocklistAIHistoryEvent::CreatedSubtask { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. }
            | BlocklistAIHistoryEvent::ReassignedExchange { .. }
            | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
            // UpdatedStreamingExchange covers streaming and other exchange-level updates but
            // doesn't change any ConversationNavigationData fields (title comes from
            // UpdateTaskDescription, last_updated uses exchange.start_time which is set at append time).
            | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
            | BlocklistAIHistoryEvent::ConversationOwnershipTransferred { .. }
            | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
            | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. } => {}

            BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. } => {
                ctx.emit(AgentConversationsModelEvent::ConversationUpdated {
                    kind: ConversationUpdateKind::MetadataChanged,
                });
            }
        }
    }

    /// Get raw task data by task ID
    pub fn get_task_data(&self, task_id: &AmbientAgentTaskId) -> Option<AmbientAgentTask> {
        self.tasks.get(task_id).cloned()
    }

    /// Returns the error message when the most recent fetch for `task_id` ended in a
    /// permanent or transient failure and the cooldown has not yet elapsed. The caller
    /// can use this to display an error state in the details panel.
    pub fn task_fetch_error(&self, task_id: &AmbientAgentTaskId) -> Option<&str> {
        match self.task_fetch_state.get(task_id) {
            Some(
                TaskFetchState::PermanentlyFailed { message, .. }
                | TaskFetchState::TransientlyFailed { message, .. },
            ) => Some(message),
            _ => None,
        }
    }

    /// Get raw task data by task ID, fetching from server if not in memory.
    /// If the task is already in memory, returns it immediately.
    /// If not, spawns an async task to fetch it from the server, stores it in memory,
    /// and emits a TasksUpdated event when ready.
    ///
    /// Multiple unrelated callers (the WASM transcript details panel, the cloud-mode details
    /// panel, and pane-group restoration) can all hit this method, sometimes many times per
    /// second while an agent is streaming. To avoid spamming `GET /api/v1/agent/runs/{id}` we:
    /// * dedupe in-flight fetches per task id,
    /// * back off for [`TRANSIENT_FETCH_FAILURE_COOLDOWN`] after a transient retry chain
    ///   exhausts (5xx/408/429/network), and
    /// * back off for [`PERMANENT_FETCH_FAILURE_COOLDOWN`] after a non-transient failure
    ///   (e.g. 401/403/404). Permanent failures still get retried periodically so we recover
    ///   if permissions change mid-session.
    pub fn get_or_async_fetch_task_data(
        &mut self,
        task_id: &AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AmbientAgentTask> {
        // If we already have it, return it
        if let Some(task) = self.tasks.get(task_id) {
            return Some(task.clone());
        }

        // Consult the per-task fetch state. The three variants are mutually exclusive: at most
        // one applies to a given id.
        match self.task_fetch_state.get(task_id) {
            Some(TaskFetchState::InFlight) => return None,
            Some(TaskFetchState::PermanentlyFailed { at, .. }) => {
                if at.elapsed() < PERMANENT_FETCH_FAILURE_COOLDOWN {
                    return None;
                }
                // Cooldown has elapsed; clear the entry and fall through to fetch again.
                self.task_fetch_state.remove(task_id);
            }
            Some(TaskFetchState::TransientlyFailed { at, .. }) => {
                if at.elapsed() < TRANSIENT_FETCH_FAILURE_COOLDOWN {
                    return None;
                }
                self.task_fetch_state.remove(task_id);
            }
            None => {}
        }

        // Opportunistically purge other expired entries so the map doesn't grow unbounded.
        self.task_fetch_state.retain(|_, state| match state {
            TaskFetchState::TransientlyFailed { at, .. } => {
                at.elapsed() < TRANSIENT_FETCH_FAILURE_COOLDOWN
            }
            TaskFetchState::PermanentlyFailed { at, .. } => {
                at.elapsed() < PERMANENT_FETCH_FAILURE_COOLDOWN
            }
            TaskFetchState::InFlight => true,
        });

        // Otherwise, spawn a task to fetch it. Use the `_when` variant so non-transient errors
        // (e.g. 401/403/404) bail after the first attempt instead of issuing all 4 requests in
        // the retry chain before being cached.
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let task_id_clone = *task_id;

        self.task_fetch_state
            .insert(task_id_clone, TaskFetchState::InFlight);

        ctx.spawn_with_retry_on_error_when(
            move || {
                let ai_client = ai_client.clone();
                async move { ai_client.get_ambient_agent_task(&task_id_clone).await }
            },
            OUT_OF_BAND_REQUEST_RETRY_STRATEGY,
            is_transient_http_error,
            move |model, result, ctx| match result {
                RequestState::RequestSucceeded(task) => {
                    let fetched_id = task.task_id;
                    model.tasks.insert(fetched_id, task);
                    model.task_fetch_state.remove(&fetched_id);
                    ctx.emit(AgentConversationsModelEvent::TasksUpdated);
                }
                RequestState::RequestFailed(e) => {
                    let now = Instant::now();
                    let message = format!("{e}");
                    let new_state = if is_transient_http_error(&e) {
                        TaskFetchState::TransientlyFailed { at: now, message }
                    } else {
                        TaskFetchState::PermanentlyFailed { at: now, message }
                    };
                    model.task_fetch_state.insert(task_id_clone, new_state);
                    report_error!(e);

                    // On failure, this still emits an update event so that the details panel can re-render with the error message.
                    ctx.emit(AgentConversationsModelEvent::TasksUpdated);
                }
                RequestState::RequestFailedRetryPending(_) => {
                    // Wait for a terminal outcome before updating dedup/backoff state.
                }
            },
        );

        None
    }

    /// Returns all (name, uid) pairs for creators of tasks in the model.
    ///
    /// We use this function to populate the available creator filter list
    /// based on the tasks we have.
    pub fn get_all_creators(&self, app: &AppContext) -> Vec<(String, String)> {
        let mut creators: Vec<(String, String)> = self
            .tasks
            .values()
            .filter_map(|task| {
                let name = entry::task_creator_name(task, app)?;
                let uid = entry::task_creator_uid(task)?;
                Some((name, uid))
            })
            .collect();

        // Include the current user since they may have local conversations
        let auth_state = AuthStateProvider::as_ref(app).get();
        if let (Some(name), Some(uid)) = (auth_state.display_name(), auth_state.user_id()) {
            creators.push((name, uid.to_string()));
        }

        creators.sort_by(|a, b| a.0.cmp(&b.0));
        creators.dedup_by(|a, b| a.0 == b.0);

        creators
    }

    /// Returns a mapping of environment IDs to display names.
    ///
    /// When multiple environments share the same name, each is disambiguated
    /// as "<name> (<id>)".
    pub fn get_all_environment_ids_and_names(&self, ctx: &AppContext) -> HashMap<String, String> {
        let mut envs = HashMap::<String, String>::new();

        for task in self.tasks.values() {
            let Some(environment_id) = task
                .agent_config_snapshot
                .as_ref()
                .and_then(|s| s.environment_id.as_deref())
            else {
                continue;
            };

            let Some(server_id) = ServerId::try_from(environment_id).ok() else {
                continue;
            };
            let sync_id = SyncId::ServerId(server_id);
            let Some(env) = CloudAmbientAgentEnvironment::get_by_id(&sync_id, ctx) else {
                continue;
            };
            let env_model = &env.model().string_model;
            envs.insert(environment_id.to_string(), env_model.name.clone());
        }

        // Disambiguate duplicate names by appending the environment ID.
        let mut name_counts = HashMap::<String, usize>::new();
        for name in envs.values() {
            *name_counts.entry(name.clone()).or_default() += 1;
        }
        for (id, name) in &mut envs {
            if name_counts.get(name.as_str()).copied().unwrap_or(0) > 1 {
                *name = format!("{name} ({id})");
            }
        }

        envs
    }

    /// Converts AgentManagementFilters to TaskListFilter for server API calls.
    pub fn build_task_list_filter(
        &self,
        filters: &AgentManagementFilters,
        current_user_uid: &str,
    ) -> TaskListFilter {
        let states = match filters.status {
            StatusFilter::All => None,
            StatusFilter::Working => Some(vec![
                AmbientAgentTaskState::Queued,
                AmbientAgentTaskState::Pending,
                AmbientAgentTaskState::Claimed,
                AmbientAgentTaskState::InProgress,
            ]),
            StatusFilter::Done => Some(vec![
                AmbientAgentTaskState::Succeeded,
                AmbientAgentTaskState::InProgress,
            ]),
            StatusFilter::Failed => Some(vec![
                AmbientAgentTaskState::InProgress,
                AmbientAgentTaskState::Failed,
                AmbientAgentTaskState::Error,
                AmbientAgentTaskState::Blocked,
                AmbientAgentTaskState::Cancelled,
                AmbientAgentTaskState::Unknown,
            ]),
        };

        let source = match &filters.source {
            SourceFilter::All => None,
            SourceFilter::Specific(s) => Some(s.clone()),
        };

        let now = Utc::now();
        let created_after = match filters.created_on {
            CreatedOnFilter::All => None,
            CreatedOnFilter::Last24Hours => Some(now - chrono::Duration::hours(24)),
            CreatedOnFilter::Past3Days => Some(now - chrono::Duration::days(3)),
            CreatedOnFilter::LastWeek => Some(now - chrono::Duration::days(7)),
        };

        let creator_uid = match filters.owners {
            OwnerFilter::PersonalOnly => Some(current_user_uid.to_string()),
            OwnerFilter::All => match &filters.creator {
                CreatorFilter::All => None,
                CreatorFilter::Specific { uid, .. } => Some(uid.clone()),
            },
        };

        let environment_id = match &filters.environment {
            EnvironmentFilter::All | EnvironmentFilter::NoEnvironment => None,
            EnvironmentFilter::Specific(id) => Some(id.clone()),
        };

        TaskListFilter {
            creator_uid,
            states,
            source,
            created_after,
            environment_id,
            ..TaskListFilter::default()
        }
    }

    /// Fetches tasks matching the given filters from the server, merges them into the model,
    /// and enforces the task cap. Called when user changes filters in AgentManagementView.
    pub fn fetch_tasks_for_filters(
        &mut self,
        filters: &AgentManagementFilters,
        current_user_uid: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();
        let task_filter = self.build_task_list_filter(filters, current_user_uid);
        let current_user_uid = current_user_uid.to_string();

        ctx.spawn_with_retry_on_error(
            move || {
                let ai_client = ai_client.clone();
                let task_filter = task_filter.clone();
                async move {
                    ai_client
                        .list_ambient_agent_tasks(INITIAL_TASK_AMOUNT, task_filter)
                        .await
                }
            },
            OUT_OF_BAND_REQUEST_RETRY_STRATEGY,
            move |model, result, ctx| {
                if let RequestState::RequestSucceeded(tasks) = result {
                    // Merge results into model
                    let mut has_new_tasks = false;
                    let mut has_updated_tasks = false;

                    for task in tasks {
                        let task_id = task.task_id;
                        match model.tasks.get(&task_id) {
                            Some(existing_task) => {
                                if existing_task != &task {
                                    has_updated_tasks = true;
                                }
                            }
                            None => has_new_tasks = true,
                        };
                        model.tasks.insert(task_id, task);
                    }

                    // Enforce task cap
                    model.enforce_task_cap(&current_user_uid);

                    // Emit appropriate event
                    if has_new_tasks {
                        ctx.emit(AgentConversationsModelEvent::NewTasksReceived);
                    } else if has_updated_tasks {
                        ctx.emit(AgentConversationsModelEvent::TasksUpdated);
                    }
                } else if let RequestState::RequestFailed(e) = result {
                    report_error!(e);
                }
            },
        );
    }

    /// Enforces cap on tasks stored in the model so it doesn't grow without bound.
    /// We always keep at least 200 personal tasks around so an influx of team tasks
    /// doesn't result in evicting personal task data.
    fn enforce_task_cap(&mut self, current_user_uid: &str) {
        let total_cap = MAX_PERSONAL_TASKS + MAX_TEAM_TASKS;
        if self.tasks.len() <= total_cap {
            return;
        }

        let (mut personal, mut team): (Vec<_>, Vec<_>) =
            self.tasks.drain().partition(|(_, task)| {
                task.creator
                    .as_ref()
                    .is_some_and(|c| c.uid == current_user_uid)
            });

        // Sort each by updated_at (newest first), truncate
        personal.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
        team.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
        personal.truncate(MAX_PERSONAL_TASKS);
        team.truncate(MAX_TEAM_TASKS);

        self.tasks = personal.into_iter().chain(team).collect();
    }

    /// Clears all stored conversation and task data in memory.
    /// This is used when logging out to ensure no conversation history persists across users.
    pub(crate) fn reset(&mut self) {
        self.tasks.clear();
        self.conversations.clear();
        self.abort_existing_poll();
        self.active_data_consumers_per_window.clear();
        self.task_fetch_state.clear();
        // Reset the initial load flag so that we can retry the initial sync with the new logged in user
        self.has_finished_initial_load = false;
    }
}

#[cfg(test)]
#[path = "agent_conversations_model_tests.rs"]
mod tests;
