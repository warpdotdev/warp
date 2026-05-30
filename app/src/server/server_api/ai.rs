use std::collections::{HashMap, HashSet};
use std::time::Duration;

use ai::index::full_source_code_embedding::store_client::{IntermediateNode, StoreClient};
use ai::index::full_source_code_embedding::{
    self, CodebaseContextConfig, ContentHash, EmbeddingConfig, NodeHash, RepoMetadata,
};
use anyhow::anyhow;
use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use cynic::{MutationBuilder, QueryBuilder};
use itertools::Itertools;
#[cfg(test)]
use mockall::automock;
use prost::Message;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_core::report_error;
use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};
use warp_graphql::client::Operation;
use warp_graphql::mutations::confirm_file_artifact_upload::{
    ConfirmFileArtifactUpload, ConfirmFileArtifactUploadInput, ConfirmFileArtifactUploadResult,
    ConfirmFileArtifactUploadVariables,
};
use warp_graphql::mutations::create_agent_task::{
    CreateAgentTask, CreateAgentTaskInput, CreateAgentTaskResult, CreateAgentTaskVariables,
};
use warp_graphql::mutations::create_file_artifact_upload_target::{
    CreateFileArtifactUploadTarget, CreateFileArtifactUploadTargetInput,
    CreateFileArtifactUploadTargetResult, CreateFileArtifactUploadTargetVariables,
};
use warp_graphql::mutations::delete_ai_conversation::{
    DeleteAIConversation, DeleteAIConversationVariables, DeleteConversationInput,
    DeleteConversationResult,
};
use warp_graphql::mutations::generate_code_embeddings::{
    GenerateCodeEmbeddings, GenerateCodeEmbeddingsInput, GenerateCodeEmbeddingsResult,
    GenerateCodeEmbeddingsVariables,
};
use warp_graphql::mutations::generate_commands::{
    GenerateCommands, GenerateCommandsInput, GenerateCommandsResult, GenerateCommandsStatus,
    GenerateCommandsVariables,
};
use warp_graphql::mutations::generate_dialogue::{
    GenerateDialogue, GenerateDialogueInput,
    GenerateDialogueResult as GenerateDialogueResultGraphql, GenerateDialogueStatus,
    GenerateDialogueVariables, TranscriptPart as TranscriptPartGraphql,
};
use warp_graphql::mutations::generate_metadata_for_command::{
    GenerateMetadataForCommand, GenerateMetadataForCommandInput, GenerateMetadataForCommandResult,
    GenerateMetadataForCommandStatus, GenerateMetadataForCommandVariables,
};
use warp_graphql::mutations::populate_merkle_tree_cache::{
    PopulateMerkleTreeCache, PopulateMerkleTreeCacheResult, PopulateMerkleTreeCacheVariables,
};
use warp_graphql::mutations::request_bonus::{
    ProvideNegativeFeedbackResponseForAiConversation,
    ProvideNegativeFeedbackResponseForAiConversationInput,
    ProvideNegativeFeedbackResponseForAiConversationVariables, RequestsRefundedResult,
};
use warp_graphql::mutations::update_agent_task::{
    AgentTaskStatusMessageInput, UpdateAgentTask, UpdateAgentTaskInput, UpdateAgentTaskResult,
    UpdateAgentTaskVariables,
};
use warp_graphql::mutations::update_merkle_tree::{
    MerkleTreeNode, UpdateMerkleTree, UpdateMerkleTreeInput, UpdateMerkleTreeResult,
    UpdateMerkleTreeVariables,
};
use warp_graphql::queries::codebase_context_config::{
    CodebaseContextConfigQuery, CodebaseContextConfigResult, CodebaseContextConfigVariables,
};
use warp_graphql::queries::free_available_models::{
    FreeAvailableModels, FreeAvailableModelsInput, FreeAvailableModelsResult,
    FreeAvailableModelsVariables,
};
use warp_graphql::queries::get_available_harnesses::{
    GetAvailableHarnesses, GetAvailableHarnessesVariables,
};
use warp_graphql::queries::get_feature_model_choices::{
    GetFeatureModelChoices, GetFeatureModelChoicesVariables,
};
use warp_graphql::queries::get_relevant_fragments::{
    GetRelevantFragmentsQuery, GetRelevantFragmentsResult, GetRelevantFragmentsVariables,
};
#[cfg(not(feature = "agent_mode_evals"))]
use warp_graphql::queries::get_request_limit_info::{
    GetRequestLimitInfo, GetRequestLimitInfoVariables,
};
use warp_graphql::queries::get_scheduled_agent_history::{
    GetScheduledAgentHistory, GetScheduledAgentHistoryVariables, ScheduledAgentHistory,
    ScheduledAgentHistoryInput, ScheduledAgentHistoryResult,
};
use warp_graphql::queries::rerank_fragments::{
    RerankFragments, RerankFragmentsResult, RerankFragmentsVariables,
};
use warp_graphql::queries::sync_merkle_tree::{
    SyncMerkleTree, SyncMerkleTreeInput, SyncMerkleTreeResult, SyncMerkleTreeVariables,
};
use warp_graphql::queries::task_attachments::{
    Task as TaskAttachmentsQuery, TaskInput, TaskResult, TaskVariables,
};
use warp_graphql::queries::task_git_credentials::{
    TaskGitCredentials, TaskGitCredentialsInput, TaskGitCredentialsResult,
    TaskGitCredentialsVariables,
};
use warp_multi_agent_api::ConversationData;

use super::auth::AuthClient;
use super::harness_support::{UploadField, UploadFieldValue, UploadTarget};
use super::ServerApi;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIAgentConversationFormat, AIAgentHarness, AIAgentSerializedBlockFormat,
    ServerAIConversationMetadata,
};
pub use crate::ai::agent::UserQueryMode;
use crate::ai::ambient_agents::AmbientAgentTaskId;
// Re-export ambient agent types for backwards compatibility
pub use crate::ai::ambient_agents::{
    task::{AttachmentInput, TaskAttachment},
    AgentConfigSnapshot, AgentSource, AmbientAgentTask, AmbientAgentTaskState, TaskStatusMessage,
};
use crate::ai::artifacts::Artifact;
use crate::ai::generate_code_review_content::api::{
    GenerateCodeReviewContentRequest, GenerateCodeReviewContentResponse,
};
use crate::ai::harness_availability::HarnessAvailability;
use crate::ai::llms::{
    AvailableLLMs, DisableReason, LLMContextWindow, LLMInfo, LLMModelHost, LLMProvider, LLMSpec,
    LLMUsageMetadata, ModelsByFeature, RoutingHostConfig,
};
#[cfg(feature = "agent_mode_evals")]
use crate::ai::request_usage_model::RequestLimitInfo;
#[cfg(not(feature = "agent_mode_evals"))]
use crate::ai::BonusGrant;
use crate::ai::RequestUsageInfo;
use crate::ai_assistant::execution_context::WarpAiExecutionContext;
use crate::ai_assistant::requests::GenerateDialogueResult;
use crate::ai_assistant::utils::TranscriptPart;
use crate::ai_assistant::{AIGeneratedCommand, GenerateCommandsFromNaturalLanguageError};
use crate::drive::workflows::ai_assist::{GeneratedCommandMetadata, GeneratedCommandMetadataError};
use crate::persistence::model::ConversationUsageMetadata;
use crate::server::graphql::{
    default_request_options, get_request_context, get_user_facing_error_message,
};
use crate::terminal::model::block::SerializedBlock;
#[cfg(not(feature = "agent_mode_evals"))]
use crate::{
    ai::request_usage_model::BonusGrantScope,
    server::ids::ServerId,
    workspaces::{gql_convert::PLACEHOLDER_WORKSPACE_UID, workspace::WorkspaceUid},
};

const AI_ASSISTANT_REQUEST_TIMEOUT_SECONDS: u64 = 30;

/// A status update for a task, optionally including a platform error code.
pub struct TaskStatusUpdate {
    pub message: String,
    pub error_code: Option<PlatformErrorCode>,
}
fn public_api_user_query_mode(mode: UserQueryMode) -> &'static str {
    match mode {
        UserQueryMode::Normal => "normal",
        UserQueryMode::Plan => "plan",
        UserQueryMode::Orchestrate => "orchestrate",
    }
}

fn serialize_user_query_mode_for_public_api<S>(
    mode: &UserQueryMode,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(public_api_user_query_mode(*mode))
}

impl TaskStatusUpdate {
    /// Create a status update with just a message (no error code).
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            error_code: None,
        }
    }

    /// Create a status update with a message and error code.
    pub fn with_error_code(message: impl Into<String>, error_code: PlatformErrorCode) -> Self {
        Self {
            message: message.into(),
            error_code: Some(error_code),
        }
    }
}

/// JSON payload sent to the public `POST /agent/run` API.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpawnAgentRequest {
    /// None for skill-only or conversation-only invocations; omitted on the wire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The public API accepts lowercase mode strings (`normal`, `plan`, or `orchestrate`).
    #[serde(serialize_with = "serialize_user_query_mode_for_public_api")]
    pub mode: UserQueryMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<AgentConfigSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<bool>,
    /// Agent identity UID to use as the execution principal for the run.
    #[serde(rename = "agent_identity_uid", skip_serializing_if = "Option::is_none")]
    pub agent_identity_uid: Option<String>,
    /// Use a Claude-compatible skill as the base prompt.
    /// Format: "repo:skill_name" or just "skill_name".
    /// The skill is resolved at runtime in the agent environment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<bool>,
    /// Populated when a cloud agent spawns a child run via the public API.
    /// Not yet wired through the local start_agent flow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Base64-encoded `warp.multi_agent.v1.Skill` payloads to restore as runtime skills.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runtime_skills: Vec<String>,
    /// Base64-encoded `warp.multi_agent.v1.Attachment` payloads to restore as referenced attachments.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub referenced_attachments: Vec<String>,
    /// Server-side conversation id to resume against (sets `task.AgentConversationID`).
    /// For local-to-cloud handoff this is the forked conversation id returned by
    /// `POST /agent/conversations/{conversation_id}/fork` at chip-click time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// References a batch of files previously uploaded to handoff/{token}/
    /// via `POST /agent/handoff/upload-snapshot`. The server stores the token on the new run's
    /// queued execution input and resolves the prefix in place at rehydration time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_snapshot_token: Option<InitialSnapshotToken>,
    /// When `Some(true)`, the cloud agent skips the end-of-run snapshot upload.
    /// Set by the client when cloud conversation storage is disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_disabled: Option<bool>,
}

/// Server-minted token returned by `POST /agent/handoff/upload-snapshot` that scopes a batch
/// of presigned upload URLs to `handoff/{token}/`. The client passes it
/// back via `SpawnAgentRequest.initial_snapshot_token`; the server stores it on the new run's
/// queued execution input so rehydration discovery can read the same prefix.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InitialSnapshotToken(String);

impl InitialSnapshotToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Request body for `POST /agent/handoff/upload-snapshot`. Used by the local-to-cloud
/// handoff flow to allocate a token and presigned upload URLs scoped to
/// `handoff/{token}/` before any task exists.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UploadLocalHandoffSnapshotRequest {
    pub files: Vec<SnapshotUploadFileInfo>,
}

/// Describes a single file the client wants to upload as part of a handoff snapshot.
/// Wire-compatible with the server's `SnapshotUploadFileInfo` schema (also used by the
/// existing harness-side `/harness-support/upload-snapshot` endpoint).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SnapshotUploadFileInfo {
    pub filename: String,
    pub mime_type: String,
}

/// Response body for `POST /agent/handoff/upload-snapshot`. The `uploads` array is aligned
/// by index with the request `files` array; the client matches each `UploadTarget` back
/// to the requested filename by index.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct UploadLocalHandoffSnapshotResponse {
    pub initial_snapshot_token: InitialSnapshotToken,
    pub expires_at: String,
    pub uploads: Vec<UploadTarget>,
}

/// Request body for `POST /agent/conversations/{conversation_id}/fork`.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ForkConversationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

/// Response body for `POST /agent/conversations/{conversation_id}/fork`. The returned id is sent
/// on the subsequent `POST /agent/runs` request under `conversation_id` (resume semantics).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ForkConversationResponse {
    pub forked_conversation_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunFollowupRequest {
    pub message: String,
}

// --- Orchestrations V2 messaging types ---

#[derive(Debug, Clone, serde::Serialize)]
pub struct SendAgentMessageRequest {
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
    pub sender_run_id: String,
}

#[derive(Debug, Clone)]
pub struct ListAgentMessagesRequest {
    pub unread_only: bool,
    pub since: Option<String>,
    pub limit: i32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SendAgentMessageResponse {
    pub message_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentMessageHeader {
    pub message_id: String,
    pub sender_run_id: String,
    pub subject: String,
    pub sent_at: String,
    pub delivered_at: Option<String>,
    pub read_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentRunEvent {
    pub event_type: String,
    pub run_id: String,
    pub ref_id: Option<String>,
    pub execution_id: Option<String>,
    pub occurred_at: String,
    pub sequence: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReportAgentEventRequest {
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReportAgentEventResponse {
    pub sequence: i64,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentRunClientEventRequest {
    pub event_uuid: String,
    pub event_name: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<AgentRunClientEventPayload>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum AgentRunClientEventPayload {
    SetupMetric(AgentRunClientSetupMetricPayload),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentRunClientSetupMetricPayload {
    pub start_ts: DateTime<Utc>,
    pub finish_ts: DateTime<Utc>,
    pub latency_ms: i64,
    pub is_error: bool,
}

impl AgentRunClientEventRequest {
    pub fn timeline_event(event_name: impl Into<String>, timestamp: DateTime<Utc>) -> Self {
        Self {
            event_uuid: uuid::Uuid::new_v4().to_string(),
            event_name: event_name.into(),
            timestamp,
            payload: None,
        }
    }

    pub fn setup_metric_event(
        event_name: impl Into<String>,
        start_timestamp: DateTime<Utc>,
        finish_timestamp: DateTime<Utc>,
        is_error: bool,
    ) -> Self {
        Self {
            event_uuid: uuid::Uuid::new_v4().to_string(),
            event_name: event_name.into(),
            timestamp: finish_timestamp,
            payload: Some(AgentRunClientEventPayload::SetupMetric(
                AgentRunClientSetupMetricPayload {
                    start_ts: start_timestamp,
                    finish_ts: finish_timestamp,
                    latency_ms: finish_timestamp
                        .signed_duration_since(start_timestamp)
                        .num_milliseconds()
                        .max(0),
                    is_error,
                },
            )),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReadAgentMessageResponse {
    pub message_id: String,
    pub sender_run_id: String,
    pub subject: String,
    pub body: String,
    pub sent_at: String,
    pub delivered_at: Option<String>,
    pub read_at: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct SpawnAgentResponse {
    pub task_id: AmbientAgentTaskId,
    pub run_id: String,
    #[serde(default)]
    pub at_capacity: bool,
}

/// Response from the artifact endpoint.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "artifact_type")]
pub enum ArtifactDownloadResponse {
    #[serde(rename = "SCREENSHOT")]
    Screenshot {
        #[serde(flatten)]
        common: ArtifactDownloadCommonFields,
        data: ScreenshotArtifactResponseData,
    },
    #[serde(rename = "FILE")]
    File {
        #[serde(flatten)]
        common: ArtifactDownloadCommonFields,
        data: FileArtifactResponseData,
    },
}

impl ArtifactDownloadResponse {
    fn common(&self) -> &ArtifactDownloadCommonFields {
        match self {
            ArtifactDownloadResponse::Screenshot { common, .. }
            | ArtifactDownloadResponse::File { common, .. } => common,
        }
    }

    pub fn artifact_uid(&self) -> &str {
        &self.common().artifact_uid
    }

    pub fn artifact_type(&self) -> &'static str {
        match self {
            ArtifactDownloadResponse::Screenshot { .. } => "SCREENSHOT",
            ArtifactDownloadResponse::File { .. } => "FILE",
        }
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.common().created_at
    }

    pub fn download_url(&self) -> &str {
        match self {
            ArtifactDownloadResponse::Screenshot { data, .. } => &data.download_url,
            ArtifactDownloadResponse::File { data, .. } => &data.download_url,
        }
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        match self {
            ArtifactDownloadResponse::Screenshot { data, .. } => data.expires_at,
            ArtifactDownloadResponse::File { data, .. } => data.expires_at,
        }
    }

    pub fn content_type(&self) -> &str {
        match self {
            ArtifactDownloadResponse::Screenshot { data, .. } => &data.content_type,
            ArtifactDownloadResponse::File { data, .. } => &data.content_type,
        }
    }

    pub fn filepath(&self) -> Option<&str> {
        match self {
            ArtifactDownloadResponse::Screenshot { .. } => None,
            ArtifactDownloadResponse::File { data, .. } => Some(&data.filepath),
        }
    }

    pub fn filename(&self) -> Option<&str> {
        match self {
            ArtifactDownloadResponse::Screenshot { .. } => None,
            ArtifactDownloadResponse::File { data, .. } => Some(&data.filename),
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            ArtifactDownloadResponse::Screenshot { data, .. } => data.description.as_deref(),
            ArtifactDownloadResponse::File { data, .. } => data.description.as_deref(),
        }
    }

    pub fn size_bytes(&self) -> Option<i64> {
        match self {
            ArtifactDownloadResponse::Screenshot { .. } => None,
            ArtifactDownloadResponse::File { data, .. } => data.size_bytes,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ArtifactDownloadCommonFields {
    pub artifact_uid: String,
    pub created_at: DateTime<Utc>,
}

/// Screenshot-specific data from the artifact endpoint.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ScreenshotArtifactResponseData {
    pub download_url: String,
    pub expires_at: DateTime<Utc>,
    pub content_type: String,
    pub description: Option<String>,
}

/// File-specific data from the artifact endpoint.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct FileArtifactResponseData {
    pub download_url: String,
    pub expires_at: DateTime<Utc>,
    pub content_type: String,
    pub filepath: String,
    pub filename: String,
    pub description: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AttachmentFileInfo {
    pub filename: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PrepareAttachmentUploadsRequest {
    pub files: Vec<AttachmentFileInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DownloadAttachmentsRequest {
    pub attachment_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AttachmentDownloadInfo {
    pub attachment_id: String,
    pub download_url: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DownloadAttachmentsResponse {
    pub attachments: Vec<AttachmentDownloadInfo>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct HandoffSnapshotAttachmentInfo {
    pub attachment_id: String,
    pub filename: String,
    pub download_url: String,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListHandoffSnapshotAttachmentsResponse {
    pub attachments: Vec<HandoffSnapshotAttachmentInfo>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AttachmentUploadInfo {
    pub attachment_id: String,
    pub upload_url: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PrepareAttachmentUploadsResponse {
    pub attachments: Vec<AttachmentUploadInfo>,
}

#[derive(Debug, Clone)]
pub struct CreateFileArtifactUploadRequest {
    pub conversation_id: Option<String>,
    pub run_id: Option<String>,
    pub filepath: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
    pub size_bytes: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct FileArtifactRecord {
    pub artifact_uid: String,
    pub filepath: String,
    pub description: Option<String>,
    pub mime_type: String,
    pub size_bytes: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct FileArtifactUploadHeaderInfo {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct FileArtifactUploadTargetInfo {
    pub url: String,
    pub method: String,
    pub headers: Vec<FileArtifactUploadHeaderInfo>,
    /// Ordered multipart form fields for presigned POST uploads.
    pub fields: Vec<UploadField>,
}

#[derive(Debug, Clone)]
pub struct CreateFileArtifactUploadResponse {
    pub artifact: FileArtifactRecord,
    pub upload_target: FileArtifactUploadTargetInfo,
}

/// A single git credential entry returned by `taskGitCredentials`.
#[derive(Clone)]
pub struct GitCredential {
    /// The GitHub token (OAuth user token or App installation token).
    pub token: String,
    /// The GitHub username. `None` for service-account (installation token) principals.
    pub username: Option<String>,
    /// The GitHub email. `None` for service-account principals.
    pub email: Option<String>,
    /// The host (always `"github.com"` in V1).
    pub host: String,
}

/// Filter parameters for listing ambient agent tasks.
#[derive(Clone, Debug, Default)]
pub struct TaskListFilter {
    pub creator_uid: Option<String>,
    pub updated_after: Option<DateTime<Utc>>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub states: Option<Vec<AmbientAgentTaskState>>,
    pub source: Option<AgentSource>,
    pub execution_location: Option<ExecutionLocation>,
    pub environment_id: Option<String>,
    pub skill_spec: Option<String>,
    pub schedule_id: Option<String>,
    pub ancestor_run_id: Option<String>,
    pub config_name: Option<String>,
    pub model_id: Option<String>,
    pub artifact_type: Option<ArtifactType>,
    pub search_query: Option<String>,
    pub sort_by: Option<RunSortBy>,
    pub sort_order: Option<RunSortOrder>,
    pub cursor: Option<String>,
}

/// Execution location filter values accepted by the public API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionLocation {
    Local,
    Remote,
}

impl ExecutionLocation {
    pub fn as_query_param(&self) -> &'static str {
        match self {
            ExecutionLocation::Local => "LOCAL",
            ExecutionLocation::Remote => "REMOTE",
        }
    }
}

/// Artifact type filter values accepted by the public API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactType {
    Plan,
    PullRequest,
    Screenshot,
    File,
}

impl ArtifactType {
    pub fn as_query_param(&self) -> &'static str {
        match self {
            ArtifactType::Plan => "PLAN",
            ArtifactType::PullRequest => "PULL_REQUEST",
            ArtifactType::Screenshot => "SCREENSHOT",
            ArtifactType::File => "FILE",
        }
    }
}

/// Sort-by values accepted by the public API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunSortBy {
    UpdatedAt,
    CreatedAt,
    Title,
    Agent,
}

impl RunSortBy {
    pub fn as_query_param(&self) -> &'static str {
        match self {
            RunSortBy::UpdatedAt => "updated_at",
            RunSortBy::CreatedAt => "created_at",
            RunSortBy::Title => "title",
            RunSortBy::Agent => "agent",
        }
    }
}

/// Sort-order values accepted by the public API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunSortOrder {
    Asc,
    Desc,
}

impl RunSortOrder {
    pub fn as_query_param(&self) -> &'static str {
        match self {
            RunSortOrder::Asc => "asc",
            RunSortOrder::Desc => "desc",
        }
    }
}

/// Build the path + query string for `GET /api/v1/agent/runs` from a filter.
pub(crate) fn build_list_agent_runs_url(limit: i32, filter: &TaskListFilter) -> String {
    let mut url = format!("agent/runs?limit={limit}");

    let mut push = |key: &str, value: &str| {
        url.push('&');
        url.push_str(key);
        url.push('=');
        url.push_str(urlencoding::encode(value).as_ref());
    };

    if let Some(creator_uid) = filter.creator_uid.as_deref() {
        push("creator", creator_uid);
    }
    if let Some(updated_after) = filter.updated_after {
        push("updated_after", &updated_after.to_rfc3339());
    }
    if let Some(created_after) = filter.created_after {
        push("created_after", &created_after.to_rfc3339());
    }
    if let Some(created_before) = filter.created_before {
        push("created_before", &created_before.to_rfc3339());
    }
    if let Some(states) = filter.states.as_ref() {
        for state in states {
            if let Some(value) = state.as_query_param() {
                push("state", value);
            }
        }
    }
    if let Some(source) = filter.source.as_ref() {
        push("source", source.as_str());
    }
    if let Some(execution_location) = filter.execution_location {
        push("execution_location", execution_location.as_query_param());
    }
    if let Some(environment_id) = filter.environment_id.as_deref() {
        push("environment_id", environment_id);
    }
    if let Some(skill_spec) = filter.skill_spec.as_deref() {
        push("skill_spec", skill_spec);
    }
    if let Some(schedule_id) = filter.schedule_id.as_deref() {
        push("schedule_id", schedule_id);
    }
    if let Some(ancestor_run_id) = filter.ancestor_run_id.as_deref() {
        push("ancestor_run_id", ancestor_run_id);
    }
    if let Some(config_name) = filter.config_name.as_deref() {
        push("name", config_name);
    }
    if let Some(model_id) = filter.model_id.as_deref() {
        push("model_id", model_id);
    }
    if let Some(artifact_type) = filter.artifact_type {
        push("artifact_type", artifact_type.as_query_param());
    }
    if let Some(search_query) = filter.search_query.as_deref() {
        push("q", search_query);
    }
    if let Some(sort_by) = filter.sort_by {
        push("sort_by", sort_by.as_query_param());
    }
    if let Some(sort_order) = filter.sort_order {
        push("sort_order", sort_order.as_query_param());
    }
    if let Some(cursor) = filter.cursor.as_deref() {
        push("cursor", cursor);
    }

    url
}

pub(crate) fn build_run_followup_url(run_id: &AmbientAgentTaskId) -> String {
    format!("agent/runs/{run_id}/followups")
}
pub(crate) fn build_fork_conversation_url(conversation_id: &str) -> String {
    format!(
        "agent/conversations/{}/fork",
        urlencoding::encode(conversation_id)
    )
}

struct ListRunsResponse {
    runs: Vec<AmbientAgentTask>,
}

impl<'de> serde::Deserialize<'de> for ListRunsResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct RawResponse {
            runs: Vec<serde_json::Value>,
        }

        let raw = RawResponse::deserialize(deserializer)?;
        let mut runs = Vec::with_capacity(raw.runs.len());

        for task_value in raw.runs.into_iter() {
            match serde_json::from_value::<AmbientAgentTask>(task_value) {
                Ok(task) => runs.push(task),
                Err(e) => {
                    // Log the error and skip this task instead of failing the entire request
                    report_error!(anyhow!("Failed to deserialize ambient agent task: {}", e));
                }
            }
        }

        Ok(ListRunsResponse { runs })
    }
}

/// Source information for an agent skill.
#[derive(Clone, serde::Deserialize, Debug, PartialEq)]
pub struct AgentSkillSource {
    pub owner: String,
    pub name: String,
    pub skill_path: String,
}

/// Environment information for an agent skill.
#[derive(Clone, serde::Deserialize, Debug, PartialEq)]
pub struct AgentSkillEnvironment {
    pub uid: String,
    pub name: String,
}

/// A variant of an agent skill.
#[derive(Clone, serde::Deserialize, Debug, PartialEq)]
pub struct AgentSkillVariant {
    pub id: String,
    pub description: String,
    pub base_prompt: String,
    pub source: AgentSkillSource,
    pub environments: Vec<AgentSkillEnvironment>,
}

/// An agent skill item with its variants.
#[derive(Clone, serde::Deserialize, Debug, PartialEq)]
pub struct AgentSkillItem {
    pub name: String,
    pub variants: Vec<AgentSkillVariant>,
}

#[derive(serde::Deserialize)]
struct ListSkillsResponse {
    agents: Vec<AgentSkillItem>,
}

/// Reference to a managed secret by name.
#[derive(Clone, serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
pub struct SecretRef {
    pub name: String,
}

/// JSON payload sent to `POST /agent/identities`.
#[derive(Clone, serde::Serialize, Debug, PartialEq, Eq)]
pub struct CreateAgentRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
}

/// JSON payload sent to `PUT /agent/identities/{uid}`.
#[derive(Clone, Default, serde::Serialize, Debug, PartialEq, Eq)]
pub struct UpdateAgentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<SecretRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
}

/// Public API representation of a named agent identity.
#[derive(Clone, serde::Deserialize, serde::Serialize, Debug, PartialEq, Eq)]
pub struct AgentResponse {
    pub uid: String,
    pub name: String,
    pub description: Option<String>,
    pub available: bool,
    pub created_at: DateTime<Utc>,
    pub secrets: Vec<SecretRef>,
    pub skills: Vec<String>,
    pub base_model: Option<String>,
    #[serde(default)]
    pub environment_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct ListAgentsResponse {
    agents: Vec<AgentResponse>,
}
fn build_agent_url(uid: &str) -> String {
    format!("agent/identities/{}", urlencoding::encode(uid))
}

#[derive(Clone, serde::Deserialize, Debug, PartialEq, Eq)]
pub struct ConnectedSelfHostedWorker {
    pub worker_host: String,
    pub connection_count: u32,
    pub connected_at: String,
    pub last_seen_at: String,
}

#[derive(Clone, serde::Deserialize, Debug, PartialEq, Eq)]
pub struct ListConnectedSelfHostedWorkersResponse {
    pub workers: Vec<ConnectedSelfHostedWorker>,
}

pub(crate) const CONNECTED_SELF_HOSTED_WORKERS_PATH: &str = "agent/connected-self-hosted-workers";

#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait AIClient: 'static + Send + Sync {
    async fn generate_commands_from_natural_language(
        &self,
        prompt: String,
        ai_execution_context: Option<WarpAiExecutionContext>,
    ) -> Result<Vec<AIGeneratedCommand>, GenerateCommandsFromNaturalLanguageError>;

    async fn generate_dialogue_answer(
        &self,
        transcript: Vec<TranscriptPart>,
        prompt: String,
        ai_execution_context: Option<WarpAiExecutionContext>,
    ) -> anyhow::Result<GenerateDialogueResult>;

    async fn generate_metadata_for_command(
        &self,
        command: String,
    ) -> Result<GeneratedCommandMetadata, GeneratedCommandMetadataError>;

    async fn get_request_limit_info(&self) -> Result<RequestUsageInfo, anyhow::Error>;

    async fn get_feature_model_choices(&self) -> Result<ModelsByFeature, anyhow::Error>;

    async fn get_available_harnesses(&self) -> Result<Vec<HarnessAvailability>, anyhow::Error>;
    async fn list_connected_self_hosted_workers(
        &self,
    ) -> Result<ListConnectedSelfHostedWorkersResponse, anyhow::Error>;

    /// Fetches the free-tier available models without requiring authentication.
    /// Used during pre-login onboarding so logged-out users see an accurate model list
    /// instead of the hard-coded `ModelsByFeature::default()` fallback.
    async fn get_free_available_models(
        &self,
        referrer: Option<String>,
    ) -> Result<ModelsByFeature, anyhow::Error>;

    async fn update_merkle_tree(
        &self,
        embedding_config: EmbeddingConfig,
        nodes: Vec<IntermediateNode>,
    ) -> anyhow::Result<HashMap<NodeHash, bool>>;

    async fn generate_code_embeddings(
        &self,
        embedding_config: EmbeddingConfig,
        fragments: Vec<full_source_code_embedding::Fragment>,
        root_hash: NodeHash,
        repo_metadata: RepoMetadata,
    ) -> anyhow::Result<HashMap<ContentHash, bool>>;

    async fn provide_negative_feedback_response_for_ai_conversation(
        &self,
        conversation_id: String,
        request_ids: Vec<String>,
    ) -> anyhow::Result<i32, anyhow::Error>;

    async fn create_agent_task(
        &self,
        prompt: String,
        environment_uid: Option<String>,
        parent_run_id: Option<String>,
        config: Option<AgentConfigSnapshot>,
    ) -> anyhow::Result<AmbientAgentTaskId, anyhow::Error>;

    async fn update_agent_task(
        &self,
        task_id: AmbientAgentTaskId,
        task_state: Option<AgentTaskState>,
        session_id: Option<session_sharing_protocol::common::SessionId>,
        conversation_id: Option<String>,
        status_message: Option<TaskStatusUpdate>,
    ) -> anyhow::Result<(), anyhow::Error>;

    async fn spawn_agent(
        &self,
        request: SpawnAgentRequest,
    ) -> anyhow::Result<SpawnAgentResponse, anyhow::Error>;

    /// Allocate an initial snapshot token and presigned upload URLs for staging local-to-cloud
    /// handoff snapshot files before the corresponding cloud task exists.
    async fn upload_local_handoff_snapshot(
        &self,
        request: UploadLocalHandoffSnapshotRequest,
    ) -> anyhow::Result<UploadLocalHandoffSnapshotResponse, anyhow::Error>;

    /// Materialize a server-side fork of a conversation.
    async fn fork_conversation(
        &self,
        conversation_id: String,
        title: Option<String>,
    ) -> anyhow::Result<ForkConversationResponse, anyhow::Error>;

    async fn list_ambient_agent_tasks(
        &self,
        limit: i32,
        filter: TaskListFilter,
    ) -> anyhow::Result<Vec<AmbientAgentTask>, anyhow::Error>;

    /// List agent runs and return the raw server JSON response.
    async fn list_agent_runs_raw(
        &self,
        limit: i32,
        filter: TaskListFilter,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error>;

    async fn get_ambient_agent_task(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> anyhow::Result<AmbientAgentTask, anyhow::Error>;

    /// Fetch a single agent run and return the raw server JSON response.
    async fn get_agent_run_raw(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error>;

    async fn submit_run_followup(
        &self,
        run_id: &AmbientAgentTaskId,
        request: RunFollowupRequest,
    ) -> anyhow::Result<(), anyhow::Error>;

    async fn get_scheduled_agent_history(
        &self,
        schedule_id: &str,
    ) -> anyhow::Result<ScheduledAgentHistory, anyhow::Error>;

    async fn get_ai_conversation(
        &self,
        server_conversation_token: ServerConversationToken,
    ) -> anyhow::Result<(ConversationData, ServerAIConversationMetadata), anyhow::Error>;

    async fn list_ai_conversation_metadata(
        &self,
        conversation_ids: Option<Vec<String>>,
    ) -> anyhow::Result<Vec<ServerAIConversationMetadata>>;

    async fn get_ai_conversation_format(
        &self,
        server_conversation_token: ServerConversationToken,
    ) -> anyhow::Result<AIAgentConversationFormat, anyhow::Error>;

    async fn get_block_snapshot(
        &self,
        server_conversation_token: ServerConversationToken,
    ) -> anyhow::Result<SerializedBlock, anyhow::Error>;

    async fn delete_ai_conversation(
        &self,
        server_conversation_token: String,
    ) -> anyhow::Result<(), anyhow::Error>;

    async fn list_skills(
        &self,
        repo: Option<String>,
    ) -> anyhow::Result<Vec<AgentSkillItem>, anyhow::Error>;

    async fn list_agents(&self) -> anyhow::Result<Vec<AgentResponse>, anyhow::Error>;

    async fn list_agents_raw(&self) -> anyhow::Result<serde_json::Value, anyhow::Error>;

    async fn get_agent(&self, uid: &str) -> anyhow::Result<AgentResponse, anyhow::Error>;

    async fn get_agent_raw(&self, uid: &str) -> anyhow::Result<serde_json::Value, anyhow::Error>;

    async fn create_agent(
        &self,
        request: CreateAgentRequest,
    ) -> anyhow::Result<AgentResponse, anyhow::Error>;

    async fn create_agent_raw(
        &self,
        request: CreateAgentRequest,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error>;

    async fn update_agent(
        &self,
        uid: &str,
        request: UpdateAgentRequest,
    ) -> anyhow::Result<AgentResponse, anyhow::Error>;

    async fn update_agent_raw(
        &self,
        uid: &str,
        request: UpdateAgentRequest,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error>;

    async fn delete_agent(&self, uid: &str) -> anyhow::Result<(), anyhow::Error>;

    async fn cancel_ambient_agent_task(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> anyhow::Result<(), anyhow::Error>;

    async fn get_task_git_credentials(
        &self,
        task_id: String,
        workload_token: String,
    ) -> anyhow::Result<Vec<GitCredential>, anyhow::Error>;

    async fn get_task_attachments(
        &self,
        task_id: String,
    ) -> anyhow::Result<Vec<TaskAttachment>, anyhow::Error>;

    async fn create_file_artifact_upload_target(
        &self,
        request: CreateFileArtifactUploadRequest,
    ) -> anyhow::Result<CreateFileArtifactUploadResponse, anyhow::Error>;

    async fn confirm_file_artifact_upload(
        &self,
        artifact_uid: String,
        checksum: String,
    ) -> anyhow::Result<FileArtifactRecord, anyhow::Error>;

    async fn get_artifact_download(
        &self,
        artifact_uid: &str,
    ) -> anyhow::Result<ArtifactDownloadResponse, anyhow::Error>;

    async fn prepare_attachments_for_upload(
        &self,
        task_id: &AmbientAgentTaskId,
        files: &[AttachmentFileInfo],
    ) -> anyhow::Result<PrepareAttachmentUploadsResponse, anyhow::Error>;

    async fn download_task_attachments(
        &self,
        task_id: &AmbientAgentTaskId,
        attachment_ids: &[String],
    ) -> anyhow::Result<DownloadAttachmentsResponse, anyhow::Error>;

    async fn get_handoff_snapshot_attachments(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> anyhow::Result<Vec<TaskAttachment>, anyhow::Error>;

    // --- Orchestrations V2 messaging ---

    async fn send_agent_message(
        &self,
        request: SendAgentMessageRequest,
    ) -> anyhow::Result<SendAgentMessageResponse, anyhow::Error>;

    async fn list_agent_messages(
        &self,
        run_id: &str,
        request: ListAgentMessagesRequest,
    ) -> anyhow::Result<Vec<AgentMessageHeader>, anyhow::Error>;

    /// Persists the latest observed event sequence number for a run on the
    /// server. Used to keep the server-side cursor in sync with the client so
    /// that driver/cloud restores can resume without replaying events the
    /// parent has already acted on.
    async fn update_event_sequence_on_server(
        &self,
        run_id: &str,
        sequence: i64,
    ) -> anyhow::Result<(), anyhow::Error>;

    async fn report_agent_event(
        &self,
        run_id: &str,
        request: ReportAgentEventRequest,
    ) -> anyhow::Result<ReportAgentEventResponse, anyhow::Error>;
    async fn post_agent_run_client_event(
        &self,
        run_id: &AmbientAgentTaskId,
        request: AgentRunClientEventRequest,
    ) -> anyhow::Result<(), anyhow::Error>;

    async fn mark_message_delivered(&self, message_id: &str) -> anyhow::Result<(), anyhow::Error>;

    async fn read_agent_message(
        &self,
        message_id: &str,
    ) -> anyhow::Result<ReadAgentMessageResponse, anyhow::Error>;

    /// Fetch a normalized conversation by conversation ID.
    async fn get_public_conversation(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error>;

    /// Fetch a normalized conversation by run ID.
    async fn get_run_conversation(
        &self,
        run_id: &str,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error>;

    /// Generates AI copy for code-review flows: commit messages at dialog-open
    /// time and PR titles / bodies at confirm time. `output_type` in the
    /// request picks which of the three the server returns.
    async fn generate_code_review_content(
        &self,
        request: GenerateCodeReviewContentRequest,
    ) -> Result<GenerateCodeReviewContentResponse, anyhow::Error>;
}

fn into_file_artifact_record(
    artifact: warp_graphql::mutations::create_file_artifact_upload_target::FileArtifact,
) -> FileArtifactRecord {
    FileArtifactRecord {
        artifact_uid: artifact.artifact_uid.into_inner(),
        filepath: artifact.filepath,
        description: artifact.description,
        mime_type: artifact.mime_type,
        size_bytes: artifact.size_bytes,
    }
}

impl ServerApi {
    pub(crate) async fn send_agent_message_for_task(
        &self,
        task_id: &AmbientAgentTaskId,
        request: SendAgentMessageRequest,
    ) -> anyhow::Result<SendAgentMessageResponse, anyhow::Error> {
        let response = self
            .post_public_api_response_for_task(task_id, "agent/messages", &request)
            .await?;
        let response = response.json::<SendAgentMessageResponse>().await?;
        Ok(response)
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) async fn list_agent_messages_for_task(
        &self,
        task_id: &AmbientAgentTaskId,
        run_id: &str,
        request: ListAgentMessagesRequest,
    ) -> anyhow::Result<Vec<AgentMessageHeader>, anyhow::Error> {
        let mut params = vec![format!("limit={}", request.limit)];
        if request.unread_only {
            params.push("unread=true".to_string());
        }
        if let Some(since) = request.since {
            params.push(format!("since={}", urlencoding::encode(&since)));
        }

        let path = format!("agent/messages/{run_id}?{}", params.join("&"));
        let response = self
            .get_public_api_response_for_task(task_id, &path)
            .await?;
        let response = response.json::<Vec<AgentMessageHeader>>().await?;
        Ok(response)
    }

    pub(crate) async fn mark_message_delivered_for_task(
        &self,
        task_id: &AmbientAgentTaskId,
        message_id: &str,
    ) -> anyhow::Result<(), anyhow::Error> {
        self.post_public_api_response_for_task(
            task_id,
            &format!("agent/messages/{message_id}/delivered"),
            &(),
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn read_agent_message_for_task(
        &self,
        task_id: &AmbientAgentTaskId,
        message_id: &str,
    ) -> anyhow::Result<ReadAgentMessageResponse, anyhow::Error> {
        let response = self
            .post_public_api_response_for_task(
                task_id,
                &format!("agent/messages/{message_id}/read"),
                &(),
            )
            .await?;
        let response = response.json::<ReadAgentMessageResponse>().await?;
        Ok(response)
    }
}

/// Convert a cynic `FileArtifactUploadField` into the shared [`UploadField`]
/// domain type. Unknown variants bubble as an error rather than being silently
/// dropped, because a server-provided field we can't represent will almost certainly
/// cause the upload to fail.
fn convert_upload_field(
    field: warp_graphql::mutations::create_file_artifact_upload_target::FileArtifactUploadField,
) -> anyhow::Result<UploadField> {
    use warp_graphql::mutations::create_file_artifact_upload_target::FileArtifactUploadFieldValue;

    let value = match field.value {
        FileArtifactUploadFieldValue::StaticUploadFieldValue(v) => {
            UploadFieldValue::Static { value: v.value }
        }
        FileArtifactUploadFieldValue::ContentCRC32CFieldValue(_) => UploadFieldValue::ContentCrc32C,
        FileArtifactUploadFieldValue::ContentDataFieldValue(_) => UploadFieldValue::ContentData,
        FileArtifactUploadFieldValue::Unknown => {
            return Err(anyhow!(
                "Unknown UploadFieldValue variant for field '{}'; update client GraphQL types",
                field.name
            ));
        }
    };
    Ok(UploadField {
        name: field.name,
        value,
    })
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl AIClient for ServerApi {
    async fn generate_commands_from_natural_language(
        &self,
        prompt: String,
        // TODO: use relevant context from RequestContext and deprecate usage of ai_execution_context
        _ai_execution_context: Option<WarpAiExecutionContext>,
    ) -> Result<Vec<AIGeneratedCommand>, GenerateCommandsFromNaturalLanguageError> {
        let default_err = GenerateCommandsFromNaturalLanguageError::Other;

        let variables = GenerateCommandsVariables {
            input: GenerateCommandsInput { prompt },
            request_context: get_request_context(),
        };

        let operation = GenerateCommands::build(variables);
        let response = self
            .send_graphql_request(
                operation,
                Some(Duration::from_secs(AI_ASSISTANT_REQUEST_TIMEOUT_SECONDS)),
            )
            .await
            .map_err(|_| default_err)?;

        match response.generate_commands {
            GenerateCommandsResult::GenerateCommandsOutput(output) => match output.status {
                GenerateCommandsStatus::GenerateCommandsSuccess(success) => {
                    Ok(success.commands.into_iter().map(Into::into).collect_vec())
                }
                GenerateCommandsStatus::GenerateCommandsFailure(failure) => {
                    Err(failure.type_.into())
                }
                GenerateCommandsStatus::Unknown => {
                    Err(GenerateCommandsFromNaturalLanguageError::Other)
                }
            },
            _ => Err(GenerateCommandsFromNaturalLanguageError::Other),
        }
    }

    async fn generate_dialogue_answer(
        &self,
        transcript: Vec<TranscriptPart>,
        prompt: String,
        // TODO: use relevant context from RequestContext and deprecate usage of ai_execution_context
        _ai_execution_context: Option<WarpAiExecutionContext>,
    ) -> anyhow::Result<GenerateDialogueResult> {
        let graphql_transcript: Vec<TranscriptPartGraphql> = transcript
            .into_iter()
            .map(|part| TranscriptPartGraphql {
                user: part.raw_user_prompt().to_string(),
                assistant: part.raw_assistant_answer().to_string(),
            })
            .collect();
        let variables = GenerateDialogueVariables {
            input: GenerateDialogueInput {
                transcript: graphql_transcript,
                prompt,
            },
            request_context: get_request_context(),
        };

        let operation = GenerateDialogue::build(variables);
        let response = self
            .send_graphql_request(
                operation,
                Some(Duration::from_secs(AI_ASSISTANT_REQUEST_TIMEOUT_SECONDS)),
            )
            .await?;
        match response.generate_dialogue {
            GenerateDialogueResultGraphql::GenerateDialogueOutput(output) => match output.status {
                GenerateDialogueStatus::GenerateDialogueSuccess(success) => {
                    Ok(GenerateDialogueResult::Success {
                        answer: success.answer,
                        truncated: success.truncated,
                        request_limit_info: success.request_limit_info.into(),
                        transcript_summarized: success.transcript_summarized,
                    })
                }
                GenerateDialogueStatus::GenerateDialogueFailure(failure) => {
                    Ok(GenerateDialogueResult::Failure {
                        request_limit_info: failure.request_limit_info.into(),
                    })
                }
                GenerateDialogueStatus::Unknown => Err(anyhow!("failed to generate AI dialogue")),
            },
            GenerateDialogueResultGraphql::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            GenerateDialogueResultGraphql::Unknown => {
                Err(anyhow!("failed to generate AI dialogue"))
            }
        }
    }

    async fn generate_metadata_for_command(
        &self,
        command: String,
    ) -> Result<GeneratedCommandMetadata, GeneratedCommandMetadataError> {
        let default_err = GeneratedCommandMetadataError::Other;
        let variables = GenerateMetadataForCommandVariables {
            input: GenerateMetadataForCommandInput { command },
            request_context: get_request_context(),
        };

        let operation = GenerateMetadataForCommand::build(variables);
        let response = self
            .send_graphql_request(
                operation,
                Some(Duration::from_secs(AI_ASSISTANT_REQUEST_TIMEOUT_SECONDS)),
            )
            .await
            .map_err(|_| default_err)?;

        match response.generate_metadata_for_command {
            GenerateMetadataForCommandResult::GenerateMetadataForCommandOutput(output) => {
                match output.status {
                    GenerateMetadataForCommandStatus::GenerateMetadataForCommandSuccess(
                        success,
                    ) => Ok(success.into()),
                    GenerateMetadataForCommandStatus::GenerateMetadataForCommandFailure(
                        failure,
                    ) => Err(failure.type_.into()),
                    GenerateMetadataForCommandStatus::Unknown => {
                        Err(GeneratedCommandMetadataError::Other)
                    }
                }
            }
            _ => Err(GeneratedCommandMetadataError::Other),
        }
    }

    #[cfg(feature = "agent_mode_evals")]
    async fn get_request_limit_info(&self) -> Result<RequestUsageInfo, anyhow::Error> {
        Ok(RequestUsageInfo {
            request_limit_info: RequestLimitInfo::new_for_evals(),
            bonus_grants: vec![],
        })
    }

    #[cfg(not(feature = "agent_mode_evals"))]
    async fn get_request_limit_info(&self) -> Result<RequestUsageInfo, anyhow::Error> {
        let variables = GetRequestLimitInfoVariables {
            request_context: get_request_context(),
        };
        let operation = GetRequestLimitInfo::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.user {
            warp_graphql::queries::get_request_limit_info::UserResult::UserOutput(user_output) => {
                let request_limit_info = user_output.user.request_limit_info.into();

                let workspace_bonus_grants = user_output
                    .user
                    .workspaces
                    .into_iter()
                    .filter(|workspace| workspace.uid != PLACEHOLDER_WORKSPACE_UID.into())
                    .flat_map(|workspace| {
                        let workspace_uid =
                            WorkspaceUid::from(ServerId::from_string_lossy(workspace.uid.inner()));
                        workspace
                            .bonus_grants_info
                            .grants
                            .into_iter()
                            .map(move |grant| {
                                BonusGrant::from_gql_bonus_grant(
                                    grant,
                                    BonusGrantScope::Workspace(workspace_uid),
                                )
                            })
                    });

                let bonus_grants: Vec<BonusGrant> = user_output
                    .user
                    .bonus_grants
                    .into_iter()
                    .map(|grant| BonusGrant::from_gql_bonus_grant(grant, BonusGrantScope::User))
                    .chain(workspace_bonus_grants)
                    .collect();

                Ok(RequestUsageInfo {
                    request_limit_info,
                    bonus_grants,
                })
            }
            warp_graphql::queries::get_request_limit_info::UserResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            warp_graphql::queries::get_request_limit_info::UserResult::Unknown => {
                Err(anyhow!("failed to get request limit info"))
            }
        }
    }

    async fn get_feature_model_choices(&self) -> Result<ModelsByFeature, anyhow::Error> {
        let variables = GetFeatureModelChoicesVariables {
            request_context: get_request_context(),
        };
        let operation = GetFeatureModelChoices::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.user {
            warp_graphql::queries::get_feature_model_choices::UserResult::UserOutput(
                warp_graphql::queries::get_feature_model_choices::UserOutput {
                    user: warp_graphql::queries::get_feature_model_choices::User { mut workspaces },
                },
            ) if !workspaces.is_empty() => {
                // This is safe (`remove()` can panic) because we ensure workspaces is non-empty
                // above.
                workspaces.remove(0).feature_model_choice.try_into()
            }
            _ => Err(anyhow!("Failed to get available feature model choices")),
        }
    }

    async fn get_available_harnesses(&self) -> Result<Vec<HarnessAvailability>, anyhow::Error> {
        let variables = GetAvailableHarnessesVariables {
            request_context: get_request_context(),
        };
        let operation = GetAvailableHarnesses::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.user {
            warp_graphql::queries::get_available_harnesses::UserResult::UserOutput(output) => {
                Ok(output
                    .user
                    .available_harnesses
                    .harnesses
                    .into_iter()
                    .map(|h| HarnessAvailability {
                        harness: convert_harness(h.harness).into(),
                        display_name: h.display_name,
                        enabled: h.enabled,
                        available_models: h
                            .available_models
                            .into_iter()
                            .map(|m| crate::ai::harness_availability::HarnessModelInfo {
                                id: m.id.into_inner(),
                                display_name: m.display_name,
                                reasoning_level: m.reasoning_level,
                            })
                            .collect(),
                    })
                    .collect())
            }
            warp_graphql::queries::get_available_harnesses::UserResult::Unknown => {
                Err(anyhow!("Failed to get available harnesses"))
            }
        }
    }

    async fn get_free_available_models(
        &self,
        referrer: Option<String>,
    ) -> Result<ModelsByFeature, anyhow::Error> {
        // This resolver is public; it does not require an auth token. We must NOT go through
        // `send_graphql_request`, which awaits `get_or_refresh_access_token()`
        let variables = FreeAvailableModelsVariables {
            input: FreeAvailableModelsInput { referrer },
            request_context: get_request_context(),
        };
        let operation = FreeAvailableModels::build(variables);

        // Best-effort: if the user has a valid token (e.g. anonymous Firebase), include it;
        // otherwise send unauthenticated. Either is acceptable for this resolver.
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .ok()
            .and_then(|token| token.bearer_token());

        let response = operation
            .send_request(
                self.client.clone(),
                warp_graphql::client::RequestOptions {
                    auth_token,
                    ..default_request_options()
                },
            )
            .await?
            .data
            .ok_or_else(|| anyhow!("Missing data in freeAvailableModels response"))?;

        match response.free_available_models {
            FreeAvailableModelsResult::FreeAvailableModelsOutput(output) => {
                output.feature_model_choice.try_into()
            }
            FreeAvailableModelsResult::Unknown => {
                Err(anyhow!("Unexpected freeAvailableModels response variant"))
            }
        }
    }

    async fn update_merkle_tree(
        &self,
        embedding_config: EmbeddingConfig,
        nodes: Vec<IntermediateNode>,
    ) -> anyhow::Result<HashMap<NodeHash, bool>> {
        let nodes = nodes
            .into_iter()
            .map(|node| MerkleTreeNode {
                hash: node.hash.into(),
                children: node.children.into_iter().map(Into::into).collect(),
            })
            .collect_vec();
        let variables = UpdateMerkleTreeVariables {
            input: UpdateMerkleTreeInput {
                embedding_config: embedding_config.into(),
                nodes,
            },
            request_context: get_request_context(),
        };
        let operation = UpdateMerkleTree::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.update_merkle_tree {
            UpdateMerkleTreeResult::UpdateMerkleTreeOutput(output) => {
                let mut node_results = HashMap::with_capacity(output.results.len());
                for result in output.results {
                    node_results.insert(result.hash.try_into()?, result.success);
                }
                Ok(node_results)
            }
            UpdateMerkleTreeResult::UpdateMerkleTreeError(e) => Err(anyhow!(e.error)),
            UpdateMerkleTreeResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpdateMerkleTreeResult::Unknown => Err(anyhow!("failed to update merkle tree")),
        }
    }

    async fn generate_code_embeddings(
        &self,
        embedding_config: EmbeddingConfig,
        fragments: Vec<full_source_code_embedding::Fragment>,
        root_hash: NodeHash,
        repo_metadata: RepoMetadata,
    ) -> anyhow::Result<HashMap<ContentHash, bool>> {
        let variables = GenerateCodeEmbeddingsVariables {
            input: GenerateCodeEmbeddingsInput {
                embedding_config: embedding_config.into(),
                fragments: fragments.into_iter().map(Into::into).collect(),
                repo_metadata: repo_metadata.into(),
                root_hash: root_hash.into(),
            },
            request_context: get_request_context(),
        };

        let operation = GenerateCodeEmbeddings::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.generate_code_embeddings {
            GenerateCodeEmbeddingsResult::GenerateCodeEmbeddingsOutput(output) => {
                let mut results = HashMap::with_capacity(output.embedding_results.len());
                for result in output.embedding_results {
                    results.insert(result.hash.try_into()?, result.success);
                }
                Ok(results)
            }
            GenerateCodeEmbeddingsResult::GenerateCodeEmbeddingsError(e) => Err(anyhow!(e.error)),
            GenerateCodeEmbeddingsResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            GenerateCodeEmbeddingsResult::Unknown => {
                Err(anyhow!("failed to generate code embeddings"))
            }
        }
    }

    async fn provide_negative_feedback_response_for_ai_conversation(
        &self,
        conversation_id: String,
        request_ids: Vec<String>,
    ) -> anyhow::Result<i32, anyhow::Error> {
        let variables = ProvideNegativeFeedbackResponseForAiConversationVariables {
            input: ProvideNegativeFeedbackResponseForAiConversationInput {
                conversation_id: conversation_id.into(),
                request_ids: request_ids.into_iter().map(Into::into).collect(),
            },
            request_context: get_request_context(),
        };

        let operation = ProvideNegativeFeedbackResponseForAiConversation::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.provide_negative_feedback_response_for_ai_conversation {
            RequestsRefundedResult::RequestsRefundedOutput(output) => Ok(output.requests_refunded),
            RequestsRefundedResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            RequestsRefundedResult::Unknown => Err(anyhow!(
                "failed to provide negative feedback response for ai conversation"
            )),
        }
    }

    async fn create_agent_task(
        &self,
        prompt: String,
        environment_uid: Option<String>,
        parent_run_id: Option<String>,
        config: Option<AgentConfigSnapshot>,
    ) -> anyhow::Result<AmbientAgentTaskId, anyhow::Error> {
        // Serialize the config to JSON if provided
        let agent_config_snapshot = config
            .map(|c| serde_json::to_string(&c))
            .transpose()
            .map_err(|e| anyhow!("Failed to serialize agent config: {e}"))?;

        let variables = CreateAgentTaskVariables {
            input: CreateAgentTaskInput {
                prompt,
                environment_uid: environment_uid.map(|uid| uid.into()),
                parent_run_id: parent_run_id.map(|run_id| run_id.into()),
                agent_config_snapshot,
            },
            request_context: get_request_context(),
        };

        let operation = CreateAgentTask::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.create_agent_task {
            CreateAgentTaskResult::CreateAgentTaskOutput(output) => output
                .task_id
                .into_inner()
                .parse()
                .map_err(|e| anyhow!("Failed to parse task ID from server: {e}")),
            CreateAgentTaskResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            CreateAgentTaskResult::Unknown => Err(anyhow!("failed to create agent task")),
        }
    }

    async fn update_agent_task(
        &self,
        task_id: AmbientAgentTaskId,
        task_state: Option<AgentTaskState>,
        session_id: Option<session_sharing_protocol::common::SessionId>,
        conversation_id: Option<String>,
        status_message: Option<TaskStatusUpdate>,
    ) -> anyhow::Result<(), anyhow::Error> {
        let variables = UpdateAgentTaskVariables {
            input: UpdateAgentTaskInput {
                task_id: task_id.into(),
                task_state,
                session_id: session_id.map(|id| id.to_string().into()),
                conversation_id: conversation_id.map(|id| id.into()),
                status_message: status_message.map(|update| AgentTaskStatusMessageInput {
                    message: update.message,
                    error_code: update.error_code,
                }),
            },
            request_context: get_request_context(),
        };

        let operation = UpdateAgentTask::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.update_agent_task {
            UpdateAgentTaskResult::UpdateAgentTaskOutput(_) => Ok(()),
            UpdateAgentTaskResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            UpdateAgentTaskResult::Unknown => Err(anyhow!("failed to update agent task")),
        }
    }

    async fn spawn_agent(
        &self,
        request: SpawnAgentRequest,
    ) -> anyhow::Result<SpawnAgentResponse, anyhow::Error> {
        let response: SpawnAgentResponse = self.post_public_api("agent/run", &request).await?;
        Ok(response)
    }

    async fn list_connected_self_hosted_workers(
        &self,
    ) -> anyhow::Result<ListConnectedSelfHostedWorkersResponse, anyhow::Error> {
        self.get_public_api(CONNECTED_SELF_HOSTED_WORKERS_PATH)
            .await
    }

    async fn upload_local_handoff_snapshot(
        &self,
        request: UploadLocalHandoffSnapshotRequest,
    ) -> anyhow::Result<UploadLocalHandoffSnapshotResponse, anyhow::Error> {
        let response: UploadLocalHandoffSnapshotResponse = self
            .post_public_api("agent/handoff/upload-snapshot", &request)
            .await?;
        Ok(response)
    }

    async fn fork_conversation(
        &self,
        conversation_id: String,
        title: Option<String>,
    ) -> anyhow::Result<ForkConversationResponse, anyhow::Error> {
        let request = ForkConversationRequest { title };
        let response: ForkConversationResponse = self
            .post_public_api(&build_fork_conversation_url(&conversation_id), &request)
            .await?;
        Ok(response)
    }

    async fn list_ambient_agent_tasks(
        &self,
        limit: i32,
        filter: TaskListFilter,
    ) -> anyhow::Result<Vec<AmbientAgentTask>, anyhow::Error> {
        let url = build_list_agent_runs_url(limit, &filter);
        let response: ListRunsResponse = self.get_public_api(&url).await?;
        Ok(response.runs)
    }

    async fn list_agent_runs_raw(
        &self,
        limit: i32,
        filter: TaskListFilter,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error> {
        let url = build_list_agent_runs_url(limit, &filter);
        let response: serde_json::Value = self.get_public_api(&url).await?;
        Ok(response)
    }

    async fn get_ambient_agent_task(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> anyhow::Result<AmbientAgentTask, anyhow::Error> {
        let response: AmbientAgentTask = self
            .get_public_api(&format!("agent/runs/{task_id}"))
            .await?;
        Ok(response)
    }

    async fn get_agent_run_raw(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error> {
        let response: serde_json::Value = self
            .get_public_api(&format!("agent/runs/{task_id}"))
            .await?;
        Ok(response)
    }

    async fn submit_run_followup(
        &self,
        run_id: &AmbientAgentTaskId,
        request: RunFollowupRequest,
    ) -> anyhow::Result<(), anyhow::Error> {
        self.post_public_api_unit(&build_run_followup_url(run_id), &request)
            .await
    }

    async fn get_scheduled_agent_history(
        &self,
        schedule_id: &str,
    ) -> anyhow::Result<ScheduledAgentHistory, anyhow::Error> {
        let variables = GetScheduledAgentHistoryVariables {
            request_context: get_request_context(),
            input: ScheduledAgentHistoryInput {
                schedule_id: schedule_id.to_string().into(),
            },
        };

        let operation = GetScheduledAgentHistory::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.scheduled_agent_history {
            ScheduledAgentHistoryResult::ScheduledAgentHistoryOutput(output) => Ok(output.history),
            ScheduledAgentHistoryResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            ScheduledAgentHistoryResult::Unknown => {
                Err(anyhow!("failed to get scheduled agent history"))
            }
        }
    }

    async fn get_ai_conversation(
        &self,
        server_conversation_token: ServerConversationToken,
    ) -> anyhow::Result<(ConversationData, ServerAIConversationMetadata), anyhow::Error> {
        use warp_graphql::queries::list_ai_conversations::{
            ListAIConversations, ListAIConversationsInput, ListAIConversationsResult,
            ListAIConversationsVariables,
        };

        let conversation_id = server_conversation_token.as_str().to_string();
        let operation = ListAIConversations::build(ListAIConversationsVariables {
            input: ListAIConversationsInput {
                conversation_ids: Some(vec![cynic::Id::new(conversation_id)]),
            },
            request_context: get_request_context(),
        });
        let response = self.send_graphql_request(operation, None).await?;

        let gql_conversation = match response.list_ai_conversations {
            ListAIConversationsResult::ListAIConversationsOutput(output) => output
                .conversations
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("Conversation not found"))?,
            ListAIConversationsResult::UserFacingError(e) => {
                return Err(anyhow!(get_user_facing_error_message(e)));
            }
            ListAIConversationsResult::Unknown => {
                return Err(anyhow!("Failed to get AI conversation"));
            }
        };

        let conversation_data_bytes = base64::engine::general_purpose::STANDARD
            .decode(&gql_conversation.final_task_list)
            .map_err(|e| anyhow!("Failed to decode base64 conversation data: {e}"))?;

        let conversation_data = ConversationData::decode(conversation_data_bytes.as_slice())
            .map_err(|e| anyhow!("Failed to decode proto ConversationData: {e}"))?;

        // Build AIConversationMetadata from GraphQL response
        let metadata = gql_conversation.try_into()?;

        Ok((conversation_data, metadata))
    }

    async fn list_ai_conversation_metadata(
        &self,
        conversation_ids: Option<Vec<String>>,
    ) -> anyhow::Result<Vec<ServerAIConversationMetadata>> {
        if !FeatureFlag::CloudConversations.is_enabled() {
            return Ok(vec![]);
        }
        use warp_graphql::queries::list_ai_conversations::{
            ListAIConversationMetadata, ListAIConversationMetadataResult,
            ListAIConversationMetadataVariables, ListAIConversationsInput,
        };

        let input = ListAIConversationsInput {
            conversation_ids: conversation_ids
                .map(|ids| ids.into_iter().map(cynic::Id::new).collect()),
        };

        let variables = ListAIConversationMetadataVariables {
            input,
            request_context: get_request_context(),
        };

        let operation = ListAIConversationMetadata::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.list_ai_conversations {
            ListAIConversationMetadataResult::ListAIConversationsOutput(output) => {
                let metadata_vec: Result<Vec<_>, _> = output
                    .conversations
                    .into_iter()
                    .map(|conv| conv.try_into())
                    .collect();
                metadata_vec
            }
            ListAIConversationMetadataResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            ListAIConversationMetadataResult::Unknown => {
                Err(anyhow!("Failed to list AI conversations metadata"))
            }
        }
    }

    async fn get_ai_conversation_format(
        &self,
        server_conversation_token: ServerConversationToken,
    ) -> anyhow::Result<AIAgentConversationFormat, anyhow::Error> {
        use warp_graphql::queries::get_ai_conversation_format::{
            GetAIConversationFormat, GetAIConversationFormatResult,
            GetAIConversationFormatVariables,
        };
        use warp_graphql::queries::list_ai_conversations::ListAIConversationsInput;

        let conversation_id = server_conversation_token.as_str().to_string();
        let operation = GetAIConversationFormat::build(GetAIConversationFormatVariables {
            input: ListAIConversationsInput {
                conversation_ids: Some(vec![cynic::Id::new(conversation_id)]),
            },
            request_context: get_request_context(),
        });
        let response = self.send_graphql_request(operation, None).await?;

        match response.list_ai_conversations {
            GetAIConversationFormatResult::ListAIConversationsOutput(output) => {
                let conversation = output
                    .conversations
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow!("Conversation not found"))?;
                Ok(convert_conversation_format(conversation.format))
            }
            GetAIConversationFormatResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            GetAIConversationFormatResult::Unknown => {
                Err(anyhow!("Failed to get AI conversation format"))
            }
        }
    }

    async fn get_block_snapshot(
        &self,
        server_conversation_token: ServerConversationToken,
    ) -> anyhow::Result<SerializedBlock, anyhow::Error> {
        let conversation_id = server_conversation_token.as_str();
        // Make sure to use `SerializedBlock::from_json` to correctly handle the serialized
        // command and output grid contents.
        let response = self
            .get_public_api_response(&format!(
                "agent/conversations/{conversation_id}/block-snapshot"
            ))
            .await?;
        let json_bytes = response
            .bytes()
            .await
            .map_err(|e| anyhow!("Failed to read block snapshot for {conversation_id}: {e}"))?;
        SerializedBlock::from_json(&json_bytes)
    }

    async fn delete_ai_conversation(
        &self,
        server_conversation_token: String,
    ) -> anyhow::Result<(), anyhow::Error> {
        let variables = DeleteAIConversationVariables {
            input: DeleteConversationInput {
                conversation_id: server_conversation_token.into(),
            },
            request_context: get_request_context(),
        };

        let operation = DeleteAIConversation::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.delete_conversation {
            DeleteConversationResult::DeleteConversationOutput(_) => Ok(()),
            DeleteConversationResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)))
            }
            DeleteConversationResult::Unknown => Err(anyhow!("Failed to delete AI conversation")),
        }
    }

    async fn list_skills(
        &self,
        repo: Option<String>,
    ) -> anyhow::Result<Vec<AgentSkillItem>, anyhow::Error> {
        let path = match repo {
            Some(repo) => format!("agent?repo={}", urlencoding::encode(&repo)),
            None => "agent".to_string(),
        };
        let response: ListSkillsResponse = self.get_public_api(&path).await?;
        Ok(response.agents)
    }

    async fn list_agents(&self) -> anyhow::Result<Vec<AgentResponse>, anyhow::Error> {
        let response: ListAgentsResponse = self.get_public_api("agent/identities").await?;
        Ok(response.agents)
    }

    async fn list_agents_raw(&self) -> anyhow::Result<serde_json::Value, anyhow::Error> {
        self.get_public_api("agent/identities").await
    }

    async fn get_agent(&self, uid: &str) -> anyhow::Result<AgentResponse, anyhow::Error> {
        self.get_public_api(&build_agent_url(uid)).await
    }

    async fn get_agent_raw(&self, uid: &str) -> anyhow::Result<serde_json::Value, anyhow::Error> {
        self.get_public_api(&build_agent_url(uid)).await
    }

    async fn create_agent(
        &self,
        request: CreateAgentRequest,
    ) -> anyhow::Result<AgentResponse, anyhow::Error> {
        self.post_public_api("agent/identities", &request).await
    }

    async fn create_agent_raw(
        &self,
        request: CreateAgentRequest,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error> {
        self.post_public_api("agent/identities", &request).await
    }

    async fn update_agent(
        &self,
        uid: &str,
        request: UpdateAgentRequest,
    ) -> anyhow::Result<AgentResponse, anyhow::Error> {
        self.put_public_api(&build_agent_url(uid), &request).await
    }

    async fn update_agent_raw(
        &self,
        uid: &str,
        request: UpdateAgentRequest,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error> {
        self.put_public_api(&build_agent_url(uid), &request).await
    }

    async fn delete_agent(&self, uid: &str) -> anyhow::Result<(), anyhow::Error> {
        self.delete_public_api_unit(&build_agent_url(uid)).await
    }

    async fn cancel_ambient_agent_task(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> anyhow::Result<(), anyhow::Error> {
        let _: String = self
            .post_public_api(&format!("agent/tasks/{task_id}/cancel"), &())
            .await?;
        Ok(())
    }

    async fn get_task_git_credentials(
        &self,
        task_id: String,
        workload_token: String,
    ) -> anyhow::Result<Vec<GitCredential>, anyhow::Error> {
        let variables = TaskGitCredentialsVariables {
            input: TaskGitCredentialsInput {
                task_id: cynic::Id::new(task_id),
                workload_token,
            },
            request_context: get_request_context(),
        };
        let operation = TaskGitCredentials::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.task_git_credentials {
            TaskGitCredentialsResult::TaskGitCredentialsOutput(output) => {
                let credentials = output
                    .credentials
                    .into_iter()
                    .map(|c| GitCredential {
                        token: c.token,
                        username: c.username,
                        email: c.email,
                        host: c.host,
                    })
                    .collect();
                Ok(credentials)
            }
            TaskGitCredentialsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            TaskGitCredentialsResult::Unknown => {
                Err(anyhow!("Failed to fetch task git credentials"))
            }
        }
    }

    async fn get_task_attachments(
        &self,
        task_id: String,
    ) -> anyhow::Result<Vec<TaskAttachment>, anyhow::Error> {
        let variables = TaskVariables {
            input: TaskInput {
                task_id: cynic::Id::new(task_id),
            },
            request_context: get_request_context(),
        };
        let operation = TaskAttachmentsQuery::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.task {
            TaskResult::TaskOutput(output) => {
                let attachments = output
                    .task
                    .attachments
                    .into_iter()
                    .map(|att| TaskAttachment {
                        file_id: att.file_id.into_inner(),
                        filename: att.filename,
                        download_url: att.download_url,
                        mime_type: att.mime_type,
                    })
                    .collect();
                Ok(attachments)
            }
            TaskResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            TaskResult::Unknown => Err(anyhow!("Failed to fetch task attachments")),
        }
    }

    async fn create_file_artifact_upload_target(
        &self,
        request: CreateFileArtifactUploadRequest,
    ) -> anyhow::Result<CreateFileArtifactUploadResponse, anyhow::Error> {
        let variables = CreateFileArtifactUploadTargetVariables {
            input: CreateFileArtifactUploadTargetInput {
                conversation_id: request.conversation_id.map(cynic::Id::new),
                run_id: request.run_id.map(cynic::Id::new),
                filepath: request.filepath,
                description: request.description,
                mime_type: request.mime_type,
                size_bytes: request.size_bytes,
            },
            request_context: get_request_context(),
        };
        let operation = CreateFileArtifactUploadTarget::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.create_file_artifact_upload_target {
            CreateFileArtifactUploadTargetResult::CreateFileArtifactUploadTargetOutput(output) => {
                let headers = output
                    .upload_target
                    .headers
                    .into_iter()
                    .map(|header| FileArtifactUploadHeaderInfo {
                        name: header.name,
                        value: header.value,
                    })
                    .collect();
                let fields = output
                    .upload_target
                    .fields
                    .into_iter()
                    .map(convert_upload_field)
                    .collect::<anyhow::Result<Vec<_>>>()?;
                Ok(CreateFileArtifactUploadResponse {
                    artifact: into_file_artifact_record(output.artifact),
                    upload_target: FileArtifactUploadTargetInfo {
                        url: output.upload_target.url,
                        method: output.upload_target.method,
                        headers,
                        fields,
                    },
                })
            }
            CreateFileArtifactUploadTargetResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            CreateFileArtifactUploadTargetResult::Unknown => {
                Err(anyhow!("Failed to create file artifact upload target"))
            }
        }
    }

    async fn confirm_file_artifact_upload(
        &self,
        artifact_uid: String,
        checksum: String,
    ) -> anyhow::Result<FileArtifactRecord, anyhow::Error> {
        let variables = ConfirmFileArtifactUploadVariables {
            input: ConfirmFileArtifactUploadInput {
                artifact_uid: cynic::Id::new(artifact_uid),
                checksum,
            },
            request_context: get_request_context(),
        };
        let operation = ConfirmFileArtifactUpload::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.confirm_file_artifact_upload {
            ConfirmFileArtifactUploadResult::ConfirmFileArtifactUploadOutput(output) => {
                Ok(into_file_artifact_record(output.artifact))
            }
            ConfirmFileArtifactUploadResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            ConfirmFileArtifactUploadResult::Unknown => {
                Err(anyhow!("Failed to confirm file artifact upload"))
            }
        }
    }

    async fn get_artifact_download(
        &self,
        artifact_uid: &str,
    ) -> anyhow::Result<ArtifactDownloadResponse, anyhow::Error> {
        let response: ArtifactDownloadResponse = self
            .get_public_api(&format!("agent/artifacts/{artifact_uid}"))
            .await?;
        Ok(response)
    }

    async fn prepare_attachments_for_upload(
        &self,
        task_id: &AmbientAgentTaskId,
        files: &[AttachmentFileInfo],
    ) -> anyhow::Result<PrepareAttachmentUploadsResponse, anyhow::Error> {
        let request = PrepareAttachmentUploadsRequest {
            files: files.to_vec(),
        };
        let response: PrepareAttachmentUploadsResponse = self
            .post_public_api(
                &format!("agent/runs/{task_id}/attachments/prepare"),
                &request,
            )
            .await?;
        Ok(response)
    }

    async fn download_task_attachments(
        &self,
        task_id: &AmbientAgentTaskId,
        attachment_ids: &[String],
    ) -> anyhow::Result<DownloadAttachmentsResponse, anyhow::Error> {
        let request = DownloadAttachmentsRequest {
            attachment_ids: attachment_ids.to_vec(),
        };
        let response: DownloadAttachmentsResponse = self
            .post_public_api(
                &format!("agent/runs/{task_id}/attachments/download"),
                &request,
            )
            .await?;
        Ok(response)
    }

    async fn get_handoff_snapshot_attachments(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> anyhow::Result<Vec<TaskAttachment>, anyhow::Error> {
        let response: ListHandoffSnapshotAttachmentsResponse = self
            .get_public_api(&format!("agent/runs/{task_id}/handoff/attachments"))
            .await?;

        Ok(response
            .attachments
            .into_iter()
            .map(|attachment| TaskAttachment {
                file_id: attachment.attachment_id,
                filename: attachment.filename,
                download_url: attachment.download_url,
                mime_type: attachment
                    .mime_type
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
            })
            .collect())
    }

    // --- Orchestrations V2 messaging ---

    async fn send_agent_message(
        &self,
        request: SendAgentMessageRequest,
    ) -> anyhow::Result<SendAgentMessageResponse, anyhow::Error> {
        let response: SendAgentMessageResponse =
            self.post_public_api("agent/messages", &request).await?;
        Ok(response)
    }

    async fn list_agent_messages(
        &self,
        run_id: &str,
        request: ListAgentMessagesRequest,
    ) -> anyhow::Result<Vec<AgentMessageHeader>, anyhow::Error> {
        let mut params = vec![format!("limit={}", request.limit)];
        if request.unread_only {
            params.push("unread=true".to_string());
        }
        if let Some(since) = request.since {
            params.push(format!("since={}", urlencoding::encode(&since)));
        }

        let path = format!("agent/messages/{run_id}?{}", params.join("&"));
        let response: Vec<AgentMessageHeader> = self.get_public_api(&path).await?;
        Ok(response)
    }

    async fn update_event_sequence_on_server(
        &self,
        run_id: &str,
        sequence: i64,
    ) -> anyhow::Result<(), anyhow::Error> {
        #[derive(serde::Serialize)]
        struct UpdateBody {
            sequence: i64,
        }

        self.patch_public_api_unit(
            &format!("agent/runs/{run_id}/event-sequence"),
            &UpdateBody { sequence },
        )
        .await
    }

    async fn report_agent_event(
        &self,
        run_id: &str,
        request: ReportAgentEventRequest,
    ) -> anyhow::Result<ReportAgentEventResponse, anyhow::Error> {
        let response: ReportAgentEventResponse = self
            .post_public_api(&format!("agent/events/{run_id}"), &request)
            .await?;
        Ok(response)
    }
    async fn post_agent_run_client_event(
        &self,
        run_id: &AmbientAgentTaskId,
        request: AgentRunClientEventRequest,
    ) -> anyhow::Result<(), anyhow::Error> {
        self.post_public_api_response_for_task(
            run_id,
            &format!("agent/runs/{run_id}/client-events"),
            &request,
        )
        .await?;
        Ok(())
    }

    async fn mark_message_delivered(&self, message_id: &str) -> anyhow::Result<(), anyhow::Error> {
        self.post_public_api_unit(&format!("agent/messages/{message_id}/delivered"), &())
            .await
    }

    async fn read_agent_message(
        &self,
        message_id: &str,
    ) -> anyhow::Result<ReadAgentMessageResponse, anyhow::Error> {
        let response: ReadAgentMessageResponse = self
            .post_public_api(&format!("agent/messages/{message_id}/read"), &())
            .await?;
        Ok(response)
    }

    async fn get_public_conversation(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error> {
        let response: serde_json::Value = self
            .get_public_api(&format!("agent/conversations/{conversation_id}"))
            .await?;
        Ok(response)
    }

    async fn get_run_conversation(
        &self,
        run_id: &str,
    ) -> anyhow::Result<serde_json::Value, anyhow::Error> {
        let response: serde_json::Value = self
            .get_public_api(&format!("agent/runs/{run_id}/conversation"))
            .await?;
        Ok(response)
    }

    async fn generate_code_review_content(
        &self,
        request: GenerateCodeReviewContentRequest,
    ) -> Result<GenerateCodeReviewContentResponse, anyhow::Error> {
        let auth_token = self.get_or_refresh_access_token().await?;
        let request_builder = self.client.post(format!(
            "{}/ai/generate_code_review_content",
            ChannelState::server_root_url()
        ));
        let response = if let Some(token) = auth_token.as_bearer_token() {
            request_builder.bearer_auth(token)
        } else {
            request_builder
        }
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
        Ok(response)
    }
}

impl TryFrom<warp_graphql::queries::get_feature_model_choices::FeatureModelChoice>
    for ModelsByFeature
{
    type Error = anyhow::Error;

    fn try_from(
        value: warp_graphql::queries::get_feature_model_choices::FeatureModelChoice,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            agent_mode: value.agent_mode.try_into()?,
            coding: value.coding.try_into()?,
            cli_agent: Some(value.cli_agent.try_into()?),
            computer_use: Some(value.computer_use_agent.try_into()?),
        })
    }
}

impl TryFrom<warp_graphql::workspace::FeatureModelChoice> for ModelsByFeature {
    type Error = anyhow::Error;

    fn try_from(value: warp_graphql::workspace::FeatureModelChoice) -> Result<Self, Self::Error> {
        Ok(Self {
            agent_mode: value.agent_mode.try_into()?,
            coding: value.coding.try_into()?,
            cli_agent: Some(value.cli_agent.try_into()?),
            computer_use: Some(value.computer_use_agent.try_into()?),
        })
    }
}

impl TryFrom<warp_graphql::queries::get_feature_model_choices::AvailableLlms> for AvailableLLMs {
    type Error = anyhow::Error;

    fn try_from(
        value: warp_graphql::queries::get_feature_model_choices::AvailableLlms,
    ) -> Result<Self, Self::Error> {
        Self::new(
            value.default_id.into(),
            value.choices.into_iter().map(LLMInfo::from),
            value.preferred_codex_model_id.map(Into::into),
        )
    }
}

impl TryFrom<warp_graphql::workspace::AvailableLlms> for AvailableLLMs {
    type Error = anyhow::Error;

    fn try_from(value: warp_graphql::workspace::AvailableLlms) -> Result<Self, Self::Error> {
        Self::new(
            value.default_id.into(),
            value.choices.into_iter().map(LLMInfo::from),
            value.preferred_codex_model_id.map(Into::into),
        )
    }
}

impl From<warp_graphql::queries::get_feature_model_choices::LlmInfo> for LLMInfo {
    fn from(value: warp_graphql::queries::get_feature_model_choices::LlmInfo) -> Self {
        let host_configs = {
            let mut map = std::collections::HashMap::new();
            for config in value.host_configs {
                let config: RoutingHostConfig = config.into();
                let host = config.model_routing_host.clone();
                if map.insert(host.clone(), config).is_some() {
                    log::warn!(
                        "Duplicate LlmModelHost entry for {:?}, using latest value",
                        host
                    );
                }
            }
            map
        };
        Self {
            id: value.id.into(),
            display_name: value.display_name,
            base_model_name: value.base_model_name,
            reasoning_level: value.reasoning_level,
            usage_metadata: value.usage_metadata.into(),
            description: value.description,
            disable_reason: value.disable_reason.map(DisableReason::from),
            vision_supported: value.vision_supported,
            spec: value.spec.map(Into::into),
            provider: value.provider.into(),
            host_configs,
            discount_percentage: value.pricing.discount_percentage.map(|v| v as f32),
            context_window: LLMContextWindow {
                is_configurable: value.context_window.is_configurable,
                min: value.context_window.min.into(),
                max: value.context_window.max.into(),
                default_max: value.context_window.default.into(),
            },
        }
    }
}

impl From<warp_graphql::workspace::LlmInfo> for LLMInfo {
    fn from(value: warp_graphql::workspace::LlmInfo) -> Self {
        let host_configs = {
            let mut map = std::collections::HashMap::new();
            for config in value.host_configs {
                let config: RoutingHostConfig = config.into();
                let host = config.model_routing_host.clone();
                if map.insert(host.clone(), config).is_some() {
                    log::warn!(
                        "Duplicate LlmModelHost entry for {:?}, using latest value",
                        host
                    );
                }
            }
            map
        };
        Self {
            id: value.id.into(),
            display_name: value.display_name,
            base_model_name: value.base_model_name,
            reasoning_level: value.reasoning_level,
            usage_metadata: value.usage_metadata.into(),
            description: value.description,
            disable_reason: value.disable_reason.map(DisableReason::from),
            vision_supported: value.vision_supported,
            spec: value.spec.map(Into::into),
            provider: value.provider.into(),
            host_configs,
            discount_percentage: value.pricing.discount_percentage.map(|v| v as f32),
            context_window: LLMContextWindow {
                is_configurable: value.context_window.is_configurable,
                min: value.context_window.min.into(),
                max: value.context_window.max.into(),
                default_max: value.context_window.default.into(),
            },
        }
    }
}

impl From<warp_graphql::queries::get_feature_model_choices::RoutingHostConfig>
    for RoutingHostConfig
{
    fn from(value: warp_graphql::queries::get_feature_model_choices::RoutingHostConfig) -> Self {
        Self {
            enabled: value.enabled,
            model_routing_host: value.model_routing_host.into(),
        }
    }
}

impl From<warp_graphql::workspace::RoutingHostConfig> for RoutingHostConfig {
    fn from(value: warp_graphql::workspace::RoutingHostConfig) -> Self {
        Self {
            enabled: value.enabled,
            model_routing_host: value.model_routing_host.into(),
        }
    }
}

impl From<warp_graphql::queries::get_feature_model_choices::LlmModelHost> for LLMModelHost {
    fn from(value: warp_graphql::queries::get_feature_model_choices::LlmModelHost) -> Self {
        match value {
            warp_graphql::queries::get_feature_model_choices::LlmModelHost::DirectApi => {
                LLMModelHost::DirectApi
            }
            warp_graphql::queries::get_feature_model_choices::LlmModelHost::AwsBedrock => {
                LLMModelHost::AwsBedrock
            }
            warp_graphql::queries::get_feature_model_choices::LlmModelHost::CustomEndpoint => {
                LLMModelHost::CustomEndpoint
            }
            warp_graphql::queries::get_feature_model_choices::LlmModelHost::Other(value) => {
                report_error!(anyhow!(
                    "Unknown LlmModelHost '{value}'. Make sure to update client GraphQL types!"
                ));
                LLMModelHost::Unknown
            }
        }
    }
}

impl From<warp_graphql::queries::get_feature_model_choices::LlmProvider> for LLMProvider {
    fn from(value: warp_graphql::queries::get_feature_model_choices::LlmProvider) -> Self {
        match value {
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Openai => {
                LLMProvider::OpenAI
            }
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Anthropic => {
                LLMProvider::Anthropic
            }
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Google => {
                LLMProvider::Google
            }
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Xai => LLMProvider::Xai,
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Unknown => {
                LLMProvider::Unknown
            }
            warp_graphql::queries::get_feature_model_choices::LlmProvider::Other(value) => {
                report_error!(anyhow!(
                    "Invalid LlmProvider '{value}'. Make sure to update client GraphQL types!"
                ));
                LLMProvider::Unknown
            }
        }
    }
}

impl From<warp_graphql::workspace::LlmProvider> for LLMProvider {
    fn from(value: warp_graphql::workspace::LlmProvider) -> Self {
        match value {
            warp_graphql::workspace::LlmProvider::Openai => LLMProvider::OpenAI,
            warp_graphql::workspace::LlmProvider::Anthropic => LLMProvider::Anthropic,
            warp_graphql::workspace::LlmProvider::Google => LLMProvider::Google,
            warp_graphql::workspace::LlmProvider::Xai => LLMProvider::Xai,
            warp_graphql::workspace::LlmProvider::Unknown => LLMProvider::Unknown,
            warp_graphql::workspace::LlmProvider::Other(value) => {
                report_error!(anyhow!(
                    "Invalid LlmProvider '{value}'. Make sure to update client GraphQL types!"
                ));
                LLMProvider::Unknown
            }
        }
    }
}

impl From<warp_graphql::queries::get_feature_model_choices::LlmSpec> for LLMSpec {
    fn from(value: warp_graphql::queries::get_feature_model_choices::LlmSpec) -> Self {
        Self {
            cost: value.cost as f32,
            quality: value.quality as f32,
            speed: value.speed as f32,
        }
    }
}

impl From<warp_graphql::workspace::LlmSpec> for LLMSpec {
    fn from(value: warp_graphql::workspace::LlmSpec) -> Self {
        Self {
            cost: value.cost as f32,
            quality: value.quality as f32,
            speed: value.speed as f32,
        }
    }
}

impl From<warp_graphql::queries::get_feature_model_choices::LlmUsageMetadata> for LLMUsageMetadata {
    fn from(value: warp_graphql::queries::get_feature_model_choices::LlmUsageMetadata) -> Self {
        Self {
            request_multiplier: value.request_multiplier.max(1) as usize,
            credit_multiplier: value.credit_multiplier.map(|v| v as f32),
        }
    }
}

impl From<warp_graphql::workspace::LlmUsageMetadata> for LLMUsageMetadata {
    fn from(value: warp_graphql::workspace::LlmUsageMetadata) -> Self {
        Self {
            request_multiplier: value.request_multiplier.max(1) as usize,
            credit_multiplier: value.credit_multiplier.map(|v| v as f32),
        }
    }
}

impl From<warp_graphql::queries::get_feature_model_choices::DisableReason> for DisableReason {
    fn from(value: warp_graphql::queries::get_feature_model_choices::DisableReason) -> Self {
        match value {
            warp_graphql::queries::get_feature_model_choices::DisableReason::AdminDisabled => {
                DisableReason::AdminDisabled
            }
            warp_graphql::queries::get_feature_model_choices::DisableReason::OutOfRequests => {
                DisableReason::OutOfRequests
            }
            warp_graphql::queries::get_feature_model_choices::DisableReason::ProviderOutage => {
                DisableReason::ProviderOutage
            }
            warp_graphql::queries::get_feature_model_choices::DisableReason::RequiresUpgrade => {
                DisableReason::RequiresUpgrade
            }
            warp_graphql::queries::get_feature_model_choices::DisableReason::Other(_) => {
                DisableReason::Unavailable
            }
        }
    }
}

impl From<warp_graphql::workspace::DisableReason> for DisableReason {
    fn from(value: warp_graphql::workspace::DisableReason) -> Self {
        match value {
            warp_graphql::workspace::DisableReason::AdminDisabled => DisableReason::AdminDisabled,
            warp_graphql::workspace::DisableReason::OutOfRequests => DisableReason::OutOfRequests,
            warp_graphql::workspace::DisableReason::ProviderOutage => DisableReason::ProviderOutage,
            warp_graphql::workspace::DisableReason::RequiresUpgrade => {
                DisableReason::RequiresUpgrade
            }
            warp_graphql::workspace::DisableReason::Other(_) => DisableReason::Unavailable,
        }
    }
}

// Conversions for AIConversationMetadata from GraphQL types

fn convert_harness(harness: warp_graphql::ai::AgentHarness) -> AIAgentHarness {
    match harness {
        warp_graphql::ai::AgentHarness::Oz => AIAgentHarness::Oz,
        warp_graphql::ai::AgentHarness::ClaudeCode => AIAgentHarness::ClaudeCode,
        warp_graphql::ai::AgentHarness::Gemini => AIAgentHarness::Gemini,
        warp_graphql::ai::AgentHarness::Codex => AIAgentHarness::Codex,
        warp_graphql::ai::AgentHarness::Other(value) => {
            report_error!(anyhow!(
                "Invalid AgentHarness '{value}'. Make sure to update client GraphQL types!"
            ));
            AIAgentHarness::Unknown
        }
    }
}

fn convert_block_snapshot_format(
    format: warp_graphql::ai::SerializedBlockFormat,
) -> AIAgentSerializedBlockFormat {
    match format {
        warp_graphql::ai::SerializedBlockFormat::JsonV1 => AIAgentSerializedBlockFormat::JsonV1,
    }
}

fn convert_conversation_format(
    format: warp_graphql::ai::AIConversationFormat,
) -> AIAgentConversationFormat {
    AIAgentConversationFormat {
        has_task_list: format.has_task_list,
        block_snapshot: format.block_snapshot.map(convert_block_snapshot_format),
    }
}

// Helper function
fn convert_usage_metadata(
    summarized: bool,
    context_window_usage: f64,
    credits_spent: f64,
) -> ConversationUsageMetadata {
    ConversationUsageMetadata {
        was_summarized: summarized,
        context_window_usage: context_window_usage as f32,
        credits_spent: credits_spent as f32,
        credits_spent_for_last_block: None,
        token_usage: vec![],
        tool_usage_metadata: Default::default(),
    }
}

impl TryFrom<warp_graphql::ai::AIConversation> for ServerAIConversationMetadata {
    type Error = anyhow::Error;

    fn try_from(value: warp_graphql::ai::AIConversation) -> Result<Self, Self::Error> {
        let usage = convert_usage_metadata(
            value.usage.usage_metadata.summarized,
            value.usage.usage_metadata.context_window_usage,
            value.usage.usage_metadata.credits_spent,
        );
        let metadata = value.metadata.try_into()?;
        let permissions = value.permissions.try_into()?;
        let ambient_agent_task_id = value
            .ambient_agent_task_id
            .map(|id| id.into_inner().parse())
            .transpose()?;
        let server_conversation_token =
            ServerConversationToken::new(value.conversation_id.into_inner());

        // If we fail to parse any artifacts, don't fail the entire conversion -- just don't include them in the list
        let artifacts = value
            .artifacts
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| Artifact::try_from(a).ok())
            .collect();

        Ok(Self {
            title: value.title,
            working_directory: value.working_directory,
            harness: convert_harness(value.harness),
            usage,
            metadata,
            permissions,
            ambient_agent_task_id,
            server_conversation_token,
            artifacts,
        })
    }
}

impl TryFrom<warp_graphql::queries::list_ai_conversations::AIConversationMetadata>
    for ServerAIConversationMetadata
{
    type Error = anyhow::Error;

    fn try_from(
        value: warp_graphql::queries::list_ai_conversations::AIConversationMetadata,
    ) -> Result<Self, Self::Error> {
        let usage = convert_usage_metadata(
            value.usage.usage_metadata.summarized,
            value.usage.usage_metadata.context_window_usage,
            value.usage.usage_metadata.credits_spent,
        );
        let metadata = value.metadata.try_into()?;
        let permissions = value.permissions.try_into()?;
        let ambient_agent_task_id = value
            .ambient_agent_task_id
            .map(|id| id.into_inner().parse())
            .transpose()?;
        let server_conversation_token =
            ServerConversationToken::new(value.conversation_id.into_inner());

        let artifacts = value
            .artifacts
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| Artifact::try_from(a).ok())
            .collect();

        Ok(Self {
            title: value.title,
            working_directory: value.working_directory,
            harness: convert_harness(value.harness),
            usage,
            metadata,
            permissions,
            ambient_agent_task_id,
            server_conversation_token,
            artifacts,
        })
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl StoreClient for ServerApi {
    async fn update_intermediate_nodes(
        &self,
        embedding_config: EmbeddingConfig,
        nodes: Vec<IntermediateNode>,
    ) -> Result<HashMap<NodeHash, bool>, full_source_code_embedding::Error> {
        let results = self.update_merkle_tree(embedding_config, nodes).await?;
        Ok(results)
    }

    async fn generate_embeddings(
        &self,
        embedding_config: EmbeddingConfig,
        fragments: Vec<full_source_code_embedding::Fragment>,
        root_hash: NodeHash,
        repo_metadata: RepoMetadata,
    ) -> Result<HashMap<ContentHash, bool>, full_source_code_embedding::Error> {
        let results = self
            .generate_code_embeddings(embedding_config, fragments, root_hash, repo_metadata)
            .await?;
        Ok(results)
    }

    async fn populate_merkle_tree_cache(
        &self,
        embedding_config: EmbeddingConfig,
        root_hash: NodeHash,
        repo_metadata: RepoMetadata,
    ) -> Result<bool, full_source_code_embedding::Error> {
        let variables = PopulateMerkleTreeCacheVariables {
            embedding_config: embedding_config.into(),
            root_hash: root_hash.into(),
            repo_metadata: repo_metadata.into(),
            request_context: get_request_context(),
        };
        let operation = PopulateMerkleTreeCache::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.populate_merkle_tree_cache {
            PopulateMerkleTreeCacheResult::PopulateMerkleTreeCacheOutput(output) => {
                Ok(output.success)
            }
            PopulateMerkleTreeCacheResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)).into())
            }
            PopulateMerkleTreeCacheResult::Unknown => {
                Err(anyhow!("failed to populate merkle tree cache").into())
            }
        }
    }

    async fn sync_merkle_tree(
        &self,
        nodes: Vec<NodeHash>,
        embedding_config: EmbeddingConfig,
    ) -> Result<HashSet<NodeHash>, full_source_code_embedding::Error> {
        let input = SyncMerkleTreeInput {
            hashed_nodes: nodes.into_iter().map(Into::into).collect(),
            embedding_config: embedding_config.into(),
        };

        let variables = SyncMerkleTreeVariables {
            input,
            request_context: get_request_context(),
        };

        let operation = SyncMerkleTree::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.sync_merkle_tree {
            SyncMerkleTreeResult::SyncMerkleTreeOutput(output) => {
                let mut node_results = HashSet::with_capacity(output.changed_nodes.len());
                for hash in output.changed_nodes {
                    node_results.insert(hash.try_into()?);
                }
                Ok(node_results)
            }
            SyncMerkleTreeResult::SyncMerkleTreeError(e) => Err(anyhow!(e.error).into()),
            SyncMerkleTreeResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)).into())
            }
            SyncMerkleTreeResult::Unknown => Err(anyhow!("failed to sync merkle tree").into()),
        }
    }

    async fn rerank_fragments(
        &self,
        query: String,
        fragments: Vec<full_source_code_embedding::Fragment>,
    ) -> Result<Vec<full_source_code_embedding::Fragment>, full_source_code_embedding::Error> {
        let variables = RerankFragmentsVariables {
            query,
            fragments: fragments.into_iter().map(Into::into).collect(),
            request_context: get_request_context(),
        };
        let operation = RerankFragments::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.rerank_fragments {
            RerankFragmentsResult::RerankFragmentsOutput(output) => Ok(output
                .ranked_fragments
                .into_iter()
                .map(|fragment| fragment.try_into())
                .collect::<Result<Vec<_>, _>>()?),
            RerankFragmentsResult::RerankFragmentsError(e) => Err(anyhow!(e.error).into()),
            RerankFragmentsResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)).into())
            }
            RerankFragmentsResult::Unknown => Err(anyhow!("failed to rerank fragments").into()),
        }
    }

    async fn get_relevant_fragments(
        &self,
        embedding_config: EmbeddingConfig,
        query: String,
        root_hash: NodeHash,
        repo_metadata: RepoMetadata,
    ) -> Result<Vec<ContentHash>, full_source_code_embedding::Error> {
        let variables = GetRelevantFragmentsVariables {
            query,
            root_hash: root_hash.into(),
            embedding_config: embedding_config.into(),
            request_context: get_request_context(),
            repo_metadata: repo_metadata.into(),
        };
        let operation = GetRelevantFragmentsQuery::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.get_relevant_fragments {
            GetRelevantFragmentsResult::GetRelevantFragmentsOutput(output) => Ok(output
                .candidate_hashes
                .into_iter()
                .map(|hash| hash.try_into())
                .collect::<Result<Vec<_>, _>>()?),
            GetRelevantFragmentsResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)).into())
            }
            GetRelevantFragmentsResult::GetRelevantFragmentsError(e) => {
                Err(anyhow!(e.error).into())
            }
            GetRelevantFragmentsResult::Unknown => {
                Err(anyhow!("failed to get relevant fragments").into())
            }
        }
    }

    async fn codebase_context_config(
        &self,
    ) -> Result<CodebaseContextConfig, full_source_code_embedding::Error> {
        let variables = CodebaseContextConfigVariables {
            request_context: get_request_context(),
        };
        let operation = CodebaseContextConfigQuery::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.codebase_context_config {
            CodebaseContextConfigResult::CodebaseContextConfigOutput(output) => {
                Ok(CodebaseContextConfig {
                    embedding_config: output.embedding_config.try_into()?,
                    embedding_cadence: Duration::from_secs(output.embedding_cadence as u64),
                })
            }
            CodebaseContextConfigResult::UserFacingError(e) => {
                Err(anyhow!(get_user_facing_error_message(e)).into())
            }
            CodebaseContextConfigResult::Unknown => {
                Err(anyhow!("failed to retrieve codebase context config").into())
            }
        }
    }
}

#[cfg(test)]
#[path = "ai_tests.rs"]
mod tests;
