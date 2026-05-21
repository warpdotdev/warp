use crate::object::ObjectMetadata;
use crate::object_permissions::ObjectPermissions;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum RequestLimitRefreshDuration {
    Monthly,
    Weekly,
    EveryTwoWeeks,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RequestLimitInfo {
    pub is_unlimited: bool,
    pub next_refresh_time: Time,
    pub request_limit: i32,
    pub requests_used_since_last_refresh: i32,
    pub request_limit_refresh_duration: RequestLimitRefreshDuration,
    pub is_unlimited_voice: bool,
    pub voice_request_limit: i32,
    pub voice_requests_used_since_last_refresh: i32,
    pub is_unlimited_codebase_indices: bool,
    pub max_codebase_indices: i32,
    pub max_files_per_repo: i32,
    pub embedding_generation_batch_size: i32,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum AgentTaskState {
    #[cynic(rename = "BLOCKED")]
    Blocked,
    #[cynic(rename = "CANCELLED")]
    Cancelled,
    #[cynic(rename = "CLAIMED")]
    Claimed,
    #[cynic(rename = "ERROR")]
    Error,
    #[cynic(rename = "IN_PROGRESS")]
    InProgress,
    #[cynic(rename = "SUCCEEDED")]
    Succeeded,
    #[cynic(rename = "FAILED")]
    Failed,
}

/// Machine-readable error code from the platform error catalog.
/// Used in task status messages to identify the class of error.
/// See platformerrors package for the canonical definitions.
#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum PlatformErrorCode {
    #[cynic(rename = "AUTHENTICATION_REQUIRED")]
    AuthenticationRequired,
    #[cynic(rename = "BUDGET_EXCEEDED")]
    BudgetExceeded,
    #[cynic(rename = "CONTENT_POLICY_VIOLATION")]
    ContentPolicyViolation,
    #[cynic(rename = "ENVIRONMENT_SETUP_FAILED")]
    EnvironmentSetupFailed,
    #[cynic(rename = "EXTERNAL_AUTHENTICATION_REQUIRED")]
    ExternalAuthenticationRequired,
    #[cynic(rename = "FEATURE_NOT_AVAILABLE")]
    FeatureNotAvailable,
    #[cynic(rename = "INSUFFICIENT_CREDITS")]
    InsufficientCredits,
    #[cynic(rename = "INTEGRATION_DISABLED")]
    IntegrationDisabled,
    #[cynic(rename = "INTEGRATION_NOT_CONFIGURED")]
    IntegrationNotConfigured,
    #[cynic(rename = "INTERNAL_ERROR")]
    InternalError,
    #[cynic(rename = "INVALID_REQUEST")]
    InvalidRequest,
    #[cynic(rename = "NOT_AUTHORIZED")]
    NotAuthorized,
    #[cynic(rename = "RESOURCE_UNAVAILABLE")]
    ResourceUnavailable,
    #[cynic(rename = "RESOURCE_NOT_FOUND")]
    ResourceNotFound,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct PlanArtifact {
    pub document_uid: cynic::Id,
    pub notebook_uid: Option<cynic::Id>,
    pub title: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct PullRequestArtifact {
    pub url: String,
    pub branch: String,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ScreenshotArtifact {
    pub artifact_uid: cynic::Id,
    pub mime_type: String,
    pub description: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct FileArtifact {
    pub artifact_uid: cynic::Id,
    pub filepath: String,
    pub mime_type: String,
    pub description: Option<String>,
    pub size_bytes: Option<i32>,
}

#[derive(cynic::InlineFragments, Debug, Clone)]
pub enum AIConversationArtifact {
    PlanArtifact(PlanArtifact),
    PullRequestArtifact(PullRequestArtifact),
    ScreenshotArtifact(ScreenshotArtifact),
    FileArtifact(FileArtifact),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Debug, PartialEq)]
pub enum AgentHarness {
    Oz,
    ClaudeCode,
    Gemini,
    Codex,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum SerializedBlockFormat {
    JsonV1,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AIConversationFormat {
    pub has_task_list: bool,
    pub block_snapshot: Option<SerializedBlockFormat>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(graphql_type = "AIConversation")]
pub struct AIConversation {
    pub conversation_id: cynic::Id,
    pub final_task_list: String,
    pub harness: AgentHarness,
    pub title: String,
    pub working_directory: Option<String>,
    pub usage: ConversationUsage,
    pub metadata: ObjectMetadata,
    pub permissions: ObjectPermissions,
    pub ambient_agent_task_id: Option<cynic::Id>,
    pub artifacts: Option<Vec<AIConversationArtifact>>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ConversationUsage {
    pub conversation_id: String,
    pub last_updated: Time,
    pub title: String,
    pub usage_metadata: ConversationUsageMetadata,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ConversationUsageMetadata {
    pub context_window_usage: f64,
    pub credits_spent: f64,
    pub summarized: bool,
}
