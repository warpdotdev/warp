use std::collections::BTreeMap;

use crate::ai::PlatformErrorCode;
use crate::schema;

/// Canonical client representation of structured platform-error information.
///
/// GraphQL exposes separate output and input object types for this data. This
/// type provides the stable representation used between those wire boundaries.
#[derive(Clone, Debug, PartialEq)]
pub struct PlatformErrorInfo {
    pub code: PlatformErrorCode,
    pub retryable: bool,
    pub metadata: BTreeMap<String, String>,
    pub debug: Option<String>,
}

impl PlatformErrorInfo {
    pub fn new(code: PlatformErrorCode, retryable: bool) -> Self {
        Self {
            code,
            retryable,
            metadata: BTreeMap::new(),
            debug: None,
        }
    }

    /// Remove dogfood-only debug details before forwarding an error to a
    /// persisted or user-visible status.
    pub fn without_debug(mut self) -> Self {
        self.debug = None;
        self
    }
}

/// GraphQL output-side representation of [`PlatformErrorInfo`].
#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(graphql_type = "PlatformErrorInfo")]
pub struct PlatformErrorInfoResponse {
    pub code: PlatformErrorCode,
    pub retryable: bool,
    pub metadata: Vec<PlatformErrorMetadataResponse>,
    pub debug: Option<String>,
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
            code: response.code,
            retryable: response.retryable,
            metadata: response
                .metadata
                .into_iter()
                .map(|entry| (entry.key, entry.value))
                .collect(),
            debug: response.debug,
        }
    }
}

/// GraphQL input-side representation of [`PlatformErrorInfo`].
#[derive(cynic::InputObject, Clone, Debug)]
pub struct PlatformErrorInput {
    pub code: PlatformErrorCode,
    pub retryable: bool,
    pub metadata: Vec<PlatformErrorMetadataInput>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub debug: Option<String>,
}

#[derive(cynic::InputObject, Clone, Debug)]
pub struct PlatformErrorMetadataInput {
    pub key: String,
    pub value: String,
}

impl From<PlatformErrorInfo> for PlatformErrorInput {
    fn from(info: PlatformErrorInfo) -> Self {
        Self {
            code: info.code,
            retryable: info.retryable,
            metadata: info
                .metadata
                .into_iter()
                .map(|(key, value)| PlatformErrorMetadataInput { key, value })
                .collect(),
            debug: info.debug,
        }
    }
}

#[cfg(test)]
#[path = "platform_error_tests.rs"]
mod tests;
