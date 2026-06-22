//! Wire protocol envelopes and error types for Warp local control.
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::catalog::{
    ActionImplementationStatus, ActionKind, ActionMetadata, ActionParameterSpec, ActionResultSpec,
    PROTOCOL_VERSION, TargetScope,
};
pub use crate::selectors::{
    PaneSelector, PaneTarget, SessionSelector, SessionTarget, TabSelector, TabTarget,
    TargetSelector, WindowSelector, WindowTarget,
};

/// Common layout direction values accepted by pane and tab mutations.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
    Previous,
    Next,
}

/// Tab type accepted by `tab.create` and `window.create`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabType {
    Terminal,
    Agent,
    CloudAgent,
    Default,
}

/// Mode accepted by `tab.activate`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabActivationMode {
    Target,
    Previous,
    Next,
    Last,
}

/// Mode accepted by `tab.close`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabCloseMode {
    Target,
    Active,
    Others,
    RightOf,
}

/// Empty parameters for actions whose catalog parameter spec is `none`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionNameParams {
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingNameParams {
    pub binding_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BooleanValueParams {
    pub value: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorValueParams {
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionParams {
    pub direction: Direction,
}

/// Parameters for opening a file in Warp's app/editor state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileOpenParams {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(default)]
    pub new_tab: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyParams {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyValueParams {
    pub key: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQueryParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameParams {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResizeParams {
    pub direction: Direction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabActivateParams {
    pub mode: TabActivationMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabCloseParams {
    pub mode: TabCloseMode,
}

/// Parameters for `tab.create` and `window.create` shell/profile options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabCreateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_type: Option<TabType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextParams {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeNameParams {
    pub theme_name: String,
}

pub type KeybindingGetParams = BindingNameParams;
pub type KeybindingListParams = EmptyParams;
pub type SettingGetParams = KeyParams;
pub type SettingListParams = NamespaceParams;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionListResult {
    pub actions: Vec<ActionMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionInspectResult {
    pub action: ActionMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveTargetChain {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeSummary {
    pub name: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeListResult {
    pub themes: Vec<ThemeSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeStateResult {
    pub name: String,
    pub follow_system_theme: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark_theme: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceStateResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    pub follow_system_theme: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_zoom_percent: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingSummary {
    pub key: String,
    pub value: serde_json::Value,
    pub value_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingListResult {
    pub settings: Vec<SettingSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingGetResult {
    pub setting: SettingSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindingSummary {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keystroke: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_keystroke: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindingListResult {
    pub keybindings: Vec<KeybindingSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindingGetResult {
    pub keybinding: KeybindingSummary,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceSummary {
    pub name: String,
    pub is_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceListResult {
    pub surfaces: Vec<SurfaceSummary>,
}

/// Typed success payloads for catalog actions that need stable structured data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlResult {
    Acknowledgement { action: ActionKind },
    Metadata { data: serde_json::Value },
    Content { data: serde_json::Value },
}

/// Top-level request sent by a local-control client to a Warp instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub protocol_version: u32,
    pub request_id: Uuid,
    #[serde(default)]
    pub target: TargetSelector,
    pub action: Action,
}

impl RequestEnvelope {
    pub fn new(action: Action) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            target: TargetSelector::default(),
            action,
        }
    }
}

/// Requested action and action-specific JSON parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub kind: ActionKind,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Action {
    pub fn new(kind: ActionKind) -> Self {
        Self {
            kind,
            params: serde_json::Value::Object(Default::default()),
        }
    }

    pub fn with_params<T: Serialize>(kind: ActionKind, params: T) -> Result<Self, ControlError> {
        Ok(Self {
            kind,
            params: serde_json::to_value(params).map_err(|err| {
                ControlError::with_details(
                    ErrorCode::InvalidParams,
                    format!("failed to serialize {} parameters", kind.as_str()),
                    err.to_string(),
                )
            })?,
        })
    }

    pub fn params_as<T: DeserializeOwned>(&self) -> Result<T, ControlError> {
        serde_json::from_value(self.params.clone()).map_err(|err| {
            ControlError::with_details(
                ErrorCode::InvalidParams,
                format!("failed to decode {} parameters", self.kind.as_str()),
                err.to_string(),
            )
        })
    }
}

/// Top-level response returned by a Warp instance for a control request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub protocol_version: u32,
    pub request_id: Uuid,
    pub response: ControlResponse,
}

impl ResponseEnvelope {
    pub fn ok(request_id: Uuid, data: serde_json::Value) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            response: ControlResponse::Ok { data },
        }
    }

    pub fn error(request_id: Uuid, error: ControlError) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            response: ControlResponse::Error { error },
        }
    }
}

/// Success or error payload for a control response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ControlResponse {
    Ok { data: serde_json::Value },
    Error { error: ControlError },
}

/// Error envelope used when a request cannot be decoded into a full request envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponseEnvelope {
    pub protocol_version: u32,
    pub error: ControlError,
}

impl ErrorResponseEnvelope {
    pub fn new(error: ControlError) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            error,
        }
    }
}

/// Structured error returned by local-control protocol and transport layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ControlError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl ControlError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(
        code: ErrorCode,
        message: impl Into<String>,
        details: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            details: Some(details.into()),
        }
    }
}

/// Stable error code surfaced to CLI clients and automation.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    LocalControlDisabled,
    UnauthorizedLocalClient,
    InsufficientPermissions,
    ProtocolVersionUnsupported,
    InvalidRequest,
    InvalidSelector,
    InvalidParams,
    NoInstance,
    AmbiguousInstance,
    AmbiguousTarget,
    StaleTarget,
    TargetStateConflict,
    MissingTarget,
    TransportUnavailable,
    BridgeUnavailable,
    UnsupportedAction,
    NotAllowlisted,
    Internal,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = serde_json::to_value(self).map_err(|_| std::fmt::Error)?;
        let Some(value) = value.as_str() else {
            return Err(std::fmt::Error);
        };
        f.write_str(value)
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
