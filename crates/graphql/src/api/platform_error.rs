use std::collections::BTreeMap;

use crate::ai::PlatformErrorCode;
use crate::schema;

/// Canonical client representation of structured platform-error information.
///
/// GraphQL exposes separate output and input object types for this data. This
/// type provides the stable representation used between those wire boundaries.
#[derive(Clone, Debug, PartialEq)]
pub struct PlatformErrorInfo {
    pub error_message: Option<String>,
    pub code: PlatformErrorCode,
    pub http_status: Option<i32>,
    pub user_facing_messages: BTreeMap<PlatformErrorMessageFormat, String>,
    pub detail: Option<String>,
    pub retryable: bool,
    pub is_user_error: Option<bool>,
    pub metadata: BTreeMap<String, String>,
    pub debug: Option<String>,
    pub metrics_category: Option<String>,
    pub trace_id: Option<String>,
}

impl PlatformErrorInfo {
    pub fn new(code: PlatformErrorCode, retryable: bool) -> Self {
        Self {
            error_message: None,
            code,
            http_status: None,
            user_facing_messages: BTreeMap::new(),
            detail: None,
            retryable,
            is_user_error: None,
            metadata: BTreeMap::new(),
            debug: None,
            metrics_category: None,
            trace_id: None,
        }
    }
}
#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlatformErrorMessageFormat {
    #[cynic(rename = "PLAIN_TEXT")]
    PlainText,
    #[cynic(rename = "SLACK_MRKDWN")]
    SlackMrkdwn,
    #[cynic(rename = "MARKDOWN")]
    Markdown,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(graphql_type = "PlatformErrorMessage")]
pub struct PlatformErrorMessage {
    pub format: PlatformErrorMessageFormat,
    pub message: String,
}

pub fn platform_error_code_from_snake_case(value: &str) -> Option<PlatformErrorCode> {
    match value {
        "authentication_required" => Some(PlatformErrorCode::AuthenticationRequired),
        "budget_exceeded" => Some(PlatformErrorCode::BudgetExceeded),
        "content_policy_violation" => Some(PlatformErrorCode::ContentPolicyViolation),
        "environment_setup_failed" => Some(PlatformErrorCode::EnvironmentSetupFailed),
        "external_authentication_required" => {
            Some(PlatformErrorCode::ExternalAuthenticationRequired)
        }
        "feature_not_available" => Some(PlatformErrorCode::FeatureNotAvailable),
        "insufficient_credits" => Some(PlatformErrorCode::InsufficientCredits),
        "integration_disabled" => Some(PlatformErrorCode::IntegrationDisabled),
        "integration_not_configured" => Some(PlatformErrorCode::IntegrationNotConfigured),
        "internal_error" => Some(PlatformErrorCode::InternalError),
        "invalid_request" => Some(PlatformErrorCode::InvalidRequest),
        "not_authorized" => Some(PlatformErrorCode::NotAuthorized),
        "resource_unavailable" => Some(PlatformErrorCode::ResourceUnavailable),
        "resource_not_found" => Some(PlatformErrorCode::ResourceNotFound),
        _ => None,
    }
}

/// GraphQL output-side representation of [`PlatformErrorInfo`].
#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(graphql_type = "PlatformErrorInfo")]
pub struct PlatformErrorInfoResponse {
    pub error_message: String,
    pub code: PlatformErrorCode,
    pub http_status: i32,
    pub user_facing_messages: Vec<PlatformErrorMessage>,
    pub detail: Option<String>,
    pub retryable: bool,
    pub is_user_error: bool,
    pub metadata: Vec<PlatformErrorMetadataResponse>,
    pub debug: Option<String>,
    pub metrics_category: String,
    pub trace_id: Option<String>,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(graphql_type = "PlatformErrorMetadata")]
pub struct PlatformErrorMetadataResponse {
    pub key: String,
    pub value: String,
}

impl From<PlatformErrorInfoResponse> for PlatformErrorInfo {
    fn from(response: PlatformErrorInfoResponse) -> Self {
        Self {
            error_message: Some(response.error_message),
            code: response.code,
            http_status: Some(response.http_status),
            user_facing_messages: response
                .user_facing_messages
                .into_iter()
                .map(|entry| (entry.format, entry.message))
                .collect(),
            detail: response.detail,
            retryable: response.retryable,
            is_user_error: Some(response.is_user_error),
            metadata: response
                .metadata
                .into_iter()
                .map(|entry| (entry.key, entry.value))
                .collect(),
            debug: response.debug,
            metrics_category: Some(response.metrics_category),
            trace_id: response.trace_id,
        }
    }
}

/// GraphQL input-side representation of [`PlatformErrorInfo`].
#[derive(cynic::InputObject, Clone, Debug)]
pub struct PlatformErrorInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub code: PlatformErrorCode,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<i32>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub user_facing_messages: Option<Vec<PlatformErrorMessageInput>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub retryable: bool,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub is_user_error: Option<bool>,
    pub metadata: Vec<PlatformErrorMetadataInput>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub debug: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub metrics_category: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

#[derive(cynic::InputObject, Clone, Debug)]
pub struct PlatformErrorMessageInput {
    pub format: PlatformErrorMessageFormat,
    pub message: String,
}

#[derive(cynic::InputObject, Clone, Debug)]
pub struct PlatformErrorMetadataInput {
    pub key: String,
    pub value: String,
}

impl From<PlatformErrorInfo> for PlatformErrorInput {
    fn from(info: PlatformErrorInfo) -> Self {
        Self {
            error_message: info.error_message,
            code: info.code,
            http_status: info.http_status,
            user_facing_messages: Some(
                info.user_facing_messages
                    .into_iter()
                    .map(|(format, message)| PlatformErrorMessageInput { format, message })
                    .collect(),
            ),
            detail: info.detail,
            retryable: info.retryable,
            is_user_error: info.is_user_error,
            metadata: info
                .metadata
                .into_iter()
                .map(|(key, value)| PlatformErrorMetadataInput { key, value })
                .collect(),
            debug: info.debug,
            metrics_category: info.metrics_category,
            trace_id: info.trace_id,
        }
    }
}

#[cfg(test)]
#[path = "platform_error_tests.rs"]
mod tests;
