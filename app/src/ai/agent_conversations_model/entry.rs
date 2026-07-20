use chrono::{DateTime, Utc};
use session_sharing_protocol::common::SessionId;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity};

use super::{
    AgentManagementFilters, AgentRunDisplayStatus, ArtifactFilter, ConversationMetadata,
    CreatedOnFilter, CreatorFilter, EnvironmentFilter, HarnessFilter, OwnerFilter, SessionStatus,
    SourceFilter, StatusFilter, artifacts_match_filter,
};
use crate::ai::active_agent_views_model::{ActiveAgentViewsModel, ConversationOrTaskId};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::ambient_agents::{AgentSource, AmbientAgentTask, AmbientAgentTaskId};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::history_model::{AIConversationMetadata, BlocklistAIHistoryModel};
use crate::ai::conversation_navigation::ConversationNavigationData;
use crate::auth::{AuthStateProvider, UserUid};
use crate::util::time_format::human_readable_precise_duration;
use crate::workspace::RestoreConversationLayout;
use crate::workspaces::user_profiles::{UserProfileWithUID, UserProfiles};

const SESSION_EXPIRATION_TIME: chrono::Duration = chrono::Duration::weeks(1);

/// Stable projection identity used by list and navigation surfaces.
///
/// Task-backed rows use the ambient run ID even when they are attached to a local
/// conversation, so task-specific affordances do not disappear when local data is present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentConversationEntryId {
    AmbientRun(AmbientAgentTaskId),
    Conversation(AIConversationId),
}

impl AgentConversationEntryId {
    pub fn as_key(&self) -> String {
        match self {
            AgentConversationEntryId::AmbientRun(id) => format!("task_{id}"),
            AgentConversationEntryId::Conversation(id) => format!("conv_{id}"),
        }
    }
}

impl From<ConversationOrTaskId> for AgentConversationEntryId {
    fn from(id: ConversationOrTaskId) -> Self {
        match id {
            ConversationOrTaskId::ConversationId(conversation_id) => {
                AgentConversationEntryId::Conversation(conversation_id)
            }
            ConversationOrTaskId::TaskId(task_id) => AgentConversationEntryId::AmbientRun(task_id),
        }
    }
}

/// Navigation request input for resolving an entry or server-token handle at action time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentConversationNavigationSubject {
    Entry(AgentConversationEntryId),
    #[allow(dead_code)]
    ServerToken(ServerConversationToken),
}

/// Normalized row data for agent conversation list, management, and navigation surfaces.
///
/// The entry keeps local conversation identity, ambient run identity, cloud token identity,
/// display fields, and available actions together so callers do not recompute navigation
/// policy from stale partial sources.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentConversationEntry {
    pub id: AgentConversationEntryId,
    pub identity: AgentConversationIdentity,
    pub provenance: AgentConversationProvenance,
    pub display: AgentConversationDisplayData,
    pub backing: AgentConversationBackingData,
    pub capabilities: AgentConversationCapabilities,
}

/// Cross-system identifiers that may refer to the same underlying conversation/run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConversationIdentity {
    pub local_conversation_id: Option<AIConversationId>,
    pub ambient_agent_task_id: Option<AmbientAgentTaskId>,
    pub server_conversation_token: Option<ServerConversationToken>,
    pub session_id: Option<SessionId>,
}

/// Display-only fields for rendering a conversation entry without consulting source models.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentConversationDisplayData {
    pub title: String,
    pub initial_query: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    pub status: AgentRunDisplayStatus,
    pub creator: AgentConversationPrincipal,
    pub executor: Option<AgentConversationPrincipal>,
    pub request_usage: Option<f32>,
    pub run_time: Option<String>,
    pub session_status: Option<SessionStatus>,
    pub source: Option<AgentSource>,
    pub working_directory: Option<String>,
    pub environment_id: Option<String>,
    pub harness: Option<Harness>,
    pub artifacts: Vec<Artifact>,
}

/// Type of principal that created or executed a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrincipalType {
    User,
    ServiceAccount,
}

impl PrincipalType {
    /// Parse from the wire-format string sent by the server.
    pub fn parse(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("user") {
            Some(PrincipalType::User)
        } else if s.eq_ignore_ascii_case("service_account") || s.eq_ignore_ascii_case("agent") {
            Some(PrincipalType::ServiceAccount)
        } else {
            None
        }
    }

    pub fn is_service_account(self) -> bool {
        self == PrincipalType::ServiceAccount
    }
}

/// Principal information normalized across local conversations and ambient runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentConversationPrincipal {
    pub name: Option<String>,
    pub uid: Option<String>,
    pub principal_type: Option<PrincipalType>,
}

/// Source category that explains why an entry exists and which backing systems can refresh it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentConversationProvenance {
    LocalInteractive,
    AmbientRun,
    CloudSyncedConversation,
}

/// Availability flags for the source data that contributed to an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConversationBackingData {
    pub has_loaded_conversation: bool,
    pub has_local_persisted_data: bool,
    pub has_cloud_data: bool,
    pub has_ambient_run: bool,
}

/// Actions that should be exposed for an entry after applying current navigation policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConversationCapabilities {
    pub can_open: bool,
    pub can_copy_link: bool,
    pub can_share: bool,
    pub can_delete: bool,
    pub can_fork_locally: bool,
    pub can_cancel: bool,
}

impl AgentConversationEntry {
    pub(super) fn matches_filters(
        &self,
        filters: &AgentManagementFilters,
        app: &AppContext,
    ) -> bool {
        self.matches_owner_and_creator(&filters.owners, &filters.creator, app)
            && self.matches_status(&filters.status)
            && self.matches_source(&filters.source)
            && self.matches_created_on(&filters.created_on)
            && self.matches_artifact(&filters.artifact)
            && self.matches_environment(&filters.environment)
            && self.matches_harness(&filters.harness)
    }

    fn matches_owner_and_creator(
        &self,
        owner_filter: &OwnerFilter,
        creator_filter: &CreatorFilter,
        app: &AppContext,
    ) -> bool {
        let current_user_id = AuthStateProvider::as_ref(app)
            .get()
            .user_id()
            .map(|uid| uid.as_string());

        let passes_owner = match owner_filter {
            OwnerFilter::All => true,
            OwnerFilter::PersonalOnly => {
                if self.backing.has_ambient_run {
                    self.display.creator.uid == current_user_id
                } else {
                    true
                }
            }
        };

        if !passes_owner || matches!(owner_filter, OwnerFilter::PersonalOnly) {
            return passes_owner;
        }

        match creator_filter {
            CreatorFilter::All => true,
            CreatorFilter::Specific { name, .. } => {
                self.display.creator.name.as_ref() == Some(name)
            }
        }
    }

    fn matches_status(&self, status_filter: &StatusFilter) -> bool {
        match status_filter {
            StatusFilter::All => true,
            StatusFilter::Working | StatusFilter::Done | StatusFilter::Failed => {
                self.display.status.status_filter() == *status_filter
            }
        }
    }

    fn matches_source(&self, source_filter: &SourceFilter) -> bool {
        match source_filter {
            SourceFilter::All => true,
            SourceFilter::Specific(source) => self.display.source.as_ref() == Some(source),
        }
    }

    fn matches_created_on(&self, created_on_filter: &CreatedOnFilter) -> bool {
        let now = Utc::now();
        let created_cutoff = match created_on_filter {
            CreatedOnFilter::All => None,
            CreatedOnFilter::Last24Hours => Some(now - chrono::Duration::hours(24)),
            CreatedOnFilter::Past3Days => Some(now - chrono::Duration::days(3)),
            CreatedOnFilter::LastWeek => Some(now - chrono::Duration::days(7)),
        };
        match created_cutoff {
            Some(cutoff) => self.display.created_at >= cutoff,
            None => true,
        }
    }

    fn matches_artifact(&self, artifact_filter: &ArtifactFilter) -> bool {
        artifacts_match_filter(&self.display.artifacts, artifact_filter)
    }

    fn matches_environment(&self, environment_filter: &EnvironmentFilter) -> bool {
        match environment_filter {
            EnvironmentFilter::All => true,
            EnvironmentFilter::NoEnvironment => self.display.environment_id.is_none(),
            EnvironmentFilter::Specific(id) => self.display.environment_id.as_ref() == Some(id),
        }
    }

    fn matches_harness(&self, harness_filter: &HarnessFilter) -> bool {
        match harness_filter {
            HarnessFilter::All => true,
            HarnessFilter::Specific(harness) => self.display.harness == Some(*harness),
        }
    }

    pub fn has_open_action(
        &self,
        restore_layout: Option<RestoreConversationLayout>,
        app: &AppContext,
    ) -> bool {
        super::AgentConversationsModel::resolve_open_action(
            AgentConversationNavigationSubject::Entry(self.id),
            restore_layout,
            app,
        )
        .is_some()
    }
}

/// Returns the local conversation ID represented by the given task, if this task and a
/// conversation entry both point at the same underlying local run.
///
/// We first match using the orchestration agent ID (task ID / run ID under v2), and fall back
/// to the server conversation token for cases where the task only carries conversation identity
/// through `conversation_id`.
pub(super) fn conversation_id_shadowed_by_task(
    task: &AmbientAgentTask,
    history_model: &BlocklistAIHistoryModel,
) -> Option<AIConversationId> {
    history_model
        .conversation_id_for_agent_id(&task.run_id().to_string())
        .or_else(|| {
            task.conversation_id().and_then(|conversation_id| {
                history_model.find_conversation_id_by_server_token(&ServerConversationToken::new(
                    conversation_id.to_string(),
                ))
            })
        })
}

pub(super) fn task_creator_name(task: &AmbientAgentTask, app: &AppContext) -> Option<String> {
    task.creator_display_name().or_else(|| {
        let uid = task.creator.as_ref().map(|creator| &creator.uid)?;
        UserProfiles::as_ref(app).displayable_identifier_for_uid(UserUid::new(uid))
    })
}

pub(super) fn task_creator_uid(task: &AmbientAgentTask) -> Option<String> {
    task.creator.as_ref().map(|creator| creator.uid.clone())
}

fn current_user_name(app: &AppContext) -> Option<String> {
    AuthStateProvider::as_ref(app).get().username_for_display()
}

fn current_user_uid(app: &AppContext) -> Option<String> {
    AuthStateProvider::as_ref(app)
        .get()
        .user_id()
        .map(|uid| uid.to_string())
}

fn task_session_id(task: &AmbientAgentTask) -> Option<SessionId> {
    task.session_id.as_deref().and_then(parse_session_id)
}

fn task_session_status(task: &AmbientAgentTask) -> SessionStatus {
    if FeatureFlag::CloudConversations.is_enabled() {
        return if task.active_run_execution().session_link.is_some() {
            SessionStatus::Available
        } else {
            SessionStatus::Unavailable
        };
    }

    if task.active_run_execution().session_id.is_some() {
        SessionStatus::Available
    } else if (Utc::now() - task.created_at) > SESSION_EXPIRATION_TIME {
        SessionStatus::Expired
    } else {
        SessionStatus::Unavailable
    }
}

fn task_run_time(task: &AmbientAgentTask) -> Option<String> {
    task.run_time().map(human_readable_precise_duration)
}

fn task_harness(task: &AmbientAgentTask) -> Option<Harness> {
    task.agent_config_snapshot.as_ref().and_then(|config| {
        config
            .harness
            .as_ref()
            .map(|harness| harness.harness_type)
            .or(Some(Harness::Oz))
    })
}

fn conversation_title(
    metadata: &ConversationMetadata,
    history_model: &BlocklistAIHistoryModel,
) -> String {
    history_model
        .conversation(&metadata.nav_data.id)
        .and_then(|conversation| conversation.title().clone())
        .unwrap_or(metadata.nav_data.title.clone())
}

fn conversation_display_status(
    metadata: &ConversationMetadata,
    history_model: &BlocklistAIHistoryModel,
) -> AgentRunDisplayStatus {
    history_model
        .conversation(&metadata.nav_data.id)
        .map(|conversation| AgentRunDisplayStatus::from_conversation_status(conversation.status()))
        .unwrap_or(AgentRunDisplayStatus::ConversationSucceeded)
}

fn conversation_request_usage(
    metadata: &ConversationMetadata,
    history_model: &BlocklistAIHistoryModel,
) -> Option<f32> {
    history_model
        .conversation(&metadata.nav_data.id)
        .map(|conversation| conversation.credits_spent())
        .or_else(|| {
            history_model
                .get_conversation_metadata(&metadata.nav_data.id)
                .and_then(|metadata| metadata.credits_spent)
        })
}

fn conversation_artifacts(
    metadata: &ConversationMetadata,
    history_model: &BlocklistAIHistoryModel,
) -> Vec<Artifact> {
    history_model
        .conversation(&metadata.nav_data.id)
        .map(|conversation| conversation.artifacts().to_vec())
        .or_else(|| {
            history_model
                .get_conversation_metadata(&metadata.nav_data.id)
                .map(|metadata| metadata.artifacts.clone())
        })
        .unwrap_or_default()
}

fn principal_from_user_profile(profile: &UserProfileWithUID) -> AgentConversationPrincipal {
    let name = profile
        .display_name
        .as_ref()
        .filter(|name| !name.is_empty())
        .or_else(|| (!profile.email.is_empty()).then_some(&profile.email))
        .cloned()
        .or_else(|| Some(profile.firebase_uid.to_string()));

    AgentConversationPrincipal {
        name,
        uid: Some(profile.firebase_uid.to_string()),
        principal_type: Some(PrincipalType::User),
    }
}

fn conversation_creator(
    metadata: &ConversationMetadata,
    history_model: &BlocklistAIHistoryModel,
    app: &AppContext,
) -> AgentConversationPrincipal {
    let server_metadata = history_model.get_server_conversation_metadata(&metadata.nav_data.id);
    if let Some(profile) = server_metadata.and_then(|metadata| metadata.creator.as_ref()) {
        return principal_from_user_profile(profile);
    }

    if let Some(uid) = server_metadata.and_then(|metadata| metadata.metadata.creator_uid.as_ref()) {
        return AgentConversationPrincipal {
            name: UserProfiles::as_ref(app).displayable_identifier_for_uid(UserUid::new(uid)),
            uid: Some(uid.clone()),
            principal_type: Some(PrincipalType::User),
        };
    }

    AgentConversationPrincipal {
        name: current_user_name(app),
        uid: current_user_uid(app),
        principal_type: Some(PrincipalType::User),
    }
}

pub(super) fn entry_for_task(
    task: &AmbientAgentTask,
    history_model: &BlocklistAIHistoryModel,
    app: &AppContext,
) -> AgentConversationEntry {
    let local_conversation_id = conversation_id_shadowed_by_task(task, history_model);
    let conversation_metadata =
        local_conversation_id.and_then(|id| history_model.get_conversation_metadata(&id));
    let server_conversation_token = task
        .conversation_id()
        .map(|id| ServerConversationToken::new(id.to_string()))
        .or_else(|| {
            local_conversation_id.and_then(|conversation_id| {
                server_conversation_token_for_conversation(conversation_id, None, history_model)
            })
        });
    let status = AgentRunDisplayStatus::from_task(task, app);
    let has_active_session_id = task
        .active_execution_session_id()
        .and_then(parse_session_id)
        .is_some();
    let has_open_ambient_session = ActiveAgentViewsModel::as_ref(app)
        .get_terminal_view_id_for_ambient_task(task.task_id)
        .is_some();
    let can_open = has_open_ambient_session
        || has_active_session_id
        || local_conversation_id.is_some()
        || server_conversation_token.is_some();
    let can_copy_link = task.has_active_execution()
        && task.active_run_execution().session_link.is_some()
        || server_conversation_token.is_some();

    AgentConversationEntry {
        id: AgentConversationEntryId::AmbientRun(task.task_id),
        identity: AgentConversationIdentity {
            local_conversation_id,
            ambient_agent_task_id: Some(task.task_id),
            server_conversation_token,
            session_id: task_session_id(task),
        },
        provenance: AgentConversationProvenance::AmbientRun,
        display: AgentConversationDisplayData {
            title: task.title.clone(),
            initial_query: Some(task.prompt.clone()),
            created_at: task.created_at,
            last_updated: task.updated_at,
            status: status.clone(),
            creator: AgentConversationPrincipal {
                name: task_creator_name(task, app),
                uid: task_creator_uid(task),
                principal_type: task
                    .creator
                    .as_ref()
                    .and_then(|c| PrincipalType::parse(&c.creator_type)),
            },
            executor: task
                .executor
                .as_ref()
                .map(|executor| AgentConversationPrincipal {
                    name: executor.display_name.clone(),
                    uid: Some(executor.uid.clone()),
                    principal_type: PrincipalType::parse(&executor.creator_type),
                }),
            request_usage: task.credits_used(),
            run_time: task_run_time(task),
            session_status: Some(task_session_status(task)),
            source: task.source.clone(),
            working_directory: conversation_metadata
                .and_then(|metadata| metadata.initial_working_directory.clone()),
            environment_id: task
                .agent_config_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.environment_id.clone()),
            harness: task_harness(task),
            artifacts: task.artifacts.clone(),
        },
        backing: AgentConversationBackingData {
            has_loaded_conversation: local_conversation_id
                .is_some_and(|id| history_model.conversation(&id).is_some()),
            has_local_persisted_data: conversation_metadata
                .is_some_and(|metadata| metadata.has_local_data),
            has_cloud_data: conversation_metadata.is_some_and(|metadata| metadata.has_cloud_data)
                || task.conversation_id().is_some(),
            has_ambient_run: true,
        },
        capabilities: AgentConversationCapabilities {
            can_open,
            can_copy_link,
            can_share: task.conversation_id().is_some()
                || local_conversation_id
                    .is_some_and(|id| history_model.can_conversation_be_shared(&id)),
            can_delete: false,
            can_fork_locally: local_conversation_id.is_some(),
            can_cancel: status.is_cancellable(),
        },
    }
}

pub(super) fn entry_for_conversation(
    metadata: &ConversationMetadata,
    history_model: &BlocklistAIHistoryModel,
    app: &AppContext,
) -> AgentConversationEntry {
    let conversation_metadata = history_model.get_conversation_metadata(&metadata.nav_data.id);
    entry_for_conversation_parts(
        metadata.nav_data.clone(),
        conversation_metadata,
        history_model,
        app,
    )
}

pub(super) fn entry_for_historical_metadata(
    metadata: &AIConversationMetadata,
    nav_data: ConversationNavigationData,
    history_model: &BlocklistAIHistoryModel,
    app: &AppContext,
) -> AgentConversationEntry {
    entry_for_conversation_parts(nav_data, Some(metadata), history_model, app)
}

fn entry_for_conversation_parts(
    nav_data: ConversationNavigationData,
    conversation_metadata: Option<&AIConversationMetadata>,
    history_model: &BlocklistAIHistoryModel,
    app: &AppContext,
) -> AgentConversationEntry {
    let metadata = ConversationMetadata { nav_data };
    let conversation_id = metadata.nav_data.id;
    let status = conversation_display_status(&metadata, history_model);
    let has_loaded_conversation = history_model.conversation(&conversation_id).is_some();
    let has_local_persisted_data = conversation_metadata
        .is_some_and(|metadata| metadata.has_local_data)
        || has_loaded_conversation;
    let has_cloud_data = conversation_metadata.is_some_and(|metadata| metadata.has_cloud_data)
        || server_conversation_token_for_conversation(
            conversation_id,
            Some(&metadata.nav_data),
            history_model,
        )
        .is_some();
    let provenance = if has_cloud_data {
        AgentConversationProvenance::CloudSyncedConversation
    } else {
        AgentConversationProvenance::LocalInteractive
    };

    AgentConversationEntry {
        id: AgentConversationEntryId::Conversation(conversation_id),
        identity: AgentConversationIdentity {
            local_conversation_id: Some(conversation_id),
            ambient_agent_task_id: conversation_metadata
                .and_then(|metadata| metadata.server_conversation_metadata.as_ref())
                .and_then(|metadata| metadata.ambient_agent_task_id),
            server_conversation_token: server_conversation_token_for_conversation(
                conversation_id,
                Some(&metadata.nav_data),
                history_model,
            ),
            session_id: None,
        },
        provenance,
        display: AgentConversationDisplayData {
            title: conversation_title(&metadata, history_model),
            initial_query: metadata.nav_data.initial_query.clone(),
            created_at: metadata.nav_data.last_updated.into(),
            last_updated: metadata.nav_data.last_updated.into(),
            status: status.clone(),
            creator: conversation_creator(&metadata, history_model, app),
            executor: None,
            request_usage: conversation_request_usage(&metadata, history_model),
            run_time: None,
            session_status: None,
            source: Some(AgentSource::Interactive),
            working_directory: metadata
                .nav_data
                .latest_working_directory
                .clone()
                .or_else(|| metadata.nav_data.initial_working_directory.clone()),
            environment_id: None,
            harness: conversation_metadata
                .and_then(|metadata| metadata.server_conversation_metadata.as_ref())
                .map(|metadata| Harness::from(metadata.harness))
                .or(Some(Harness::Oz)),
            artifacts: conversation_artifacts(&metadata, history_model),
        },
        backing: AgentConversationBackingData {
            has_loaded_conversation,
            has_local_persisted_data,
            has_cloud_data,
            has_ambient_run: conversation_metadata
                .is_some_and(AIConversationMetadata::is_ambient_agent_conversation),
        },
        capabilities: AgentConversationCapabilities {
            can_open: has_local_persisted_data || has_cloud_data,
            can_copy_link: server_conversation_token_for_conversation(
                conversation_id,
                Some(&metadata.nav_data),
                history_model,
            )
            .is_some(),
            can_share: history_model.can_conversation_be_shared(&conversation_id),
            can_delete: has_local_persisted_data,
            can_fork_locally: has_local_persisted_data,
            can_cancel: status.is_cancellable(),
        },
    }
}

fn server_conversation_token_for_conversation(
    conversation_id: AIConversationId,
    nav_data: Option<&ConversationNavigationData>,
    history_model: &BlocklistAIHistoryModel,
) -> Option<ServerConversationToken> {
    history_model
        .conversation(&conversation_id)
        .and_then(|conversation| conversation.server_conversation_token())
        .cloned()
        .or_else(|| {
            history_model
                .get_conversation_metadata(&conversation_id)
                .and_then(|metadata| metadata.server_conversation_token.clone())
        })
        .or_else(|| nav_data.and_then(|nav_data| nav_data.server_conversation_token.clone()))
}

pub(super) fn parse_session_id(session_id: &str) -> Option<SessionId> {
    match session_id.parse::<SessionId>() {
        Ok(session_id) => Some(session_id),
        Err(e) => {
            log::warn!("Failed to parse shared session ID: {e}");
            None
        }
    }
}
