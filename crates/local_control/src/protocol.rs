use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationContext {
    InsideWarp,
    OutsideWarp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionContextProof {
    VerifiedWarpTerminal { proof_id: String },
    ExternalClient,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    ReadOnlyMetadata,
    ReadOnlyTerminalData,
    MutatingNonDestructive,
    MutatingDestructiveOrExecution,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalControlPermission {
    MetadataRead,
    UnderlyingDataRead,
    AppStateMutation,
    MetadataConfigurationMutation,
    UnderlyingDataMutation,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetScope {
    Instance,
    Window,
    Tab,
    Pane,
    Session,
    Settings,
    Appearance,
    Surface,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionImplementationStatus {
    Implemented,
    Stub,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionMetadata {
    pub kind: ActionKind,
    pub name: String,
    pub implementation_status: ActionImplementationStatus,
    pub risk_tier: RiskTier,
    pub requires_authenticated_user: bool,
    pub allowed_invocation_contexts: Vec<InvocationContext>,
    pub permission: LocalControlPermission,
    pub target_scope: TargetScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TabSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaneSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TargetSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PaneTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WindowTarget {
    Active,
    Id { id: WindowSelector },
    Index { index: u32 },
    Title { title: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabTarget {
    Active,
    Id { id: TabSelector },
    Index { index: u32 },
    Title { title: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneTarget {
    Active,
    Id { id: PaneSelector },
    Index { index: u32 },
}

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
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionKind {
    #[serde(rename = "app.ping")]
    AppPing,
    #[serde(rename = "app.inspect")]
    AppInspect,
    #[serde(rename = "app.version")]
    AppVersion,
    #[serde(rename = "app.active")]
    AppActive,
    #[serde(rename = "app.focus")]
    AppFocus,
    #[serde(rename = "app.settings.open")]
    AppSettingsOpen,
    #[serde(rename = "app.command_palette.open")]
    AppCommandPaletteOpen,
    #[serde(rename = "app.command_search.open")]
    AppCommandSearchOpen,
    #[serde(rename = "app.warp_drive.open")]
    AppWarpDriveOpen,
    #[serde(rename = "app.warp_drive.toggle")]
    AppWarpDriveToggle,
    #[serde(rename = "app.resource_center.toggle")]
    AppResourceCenterToggle,
    #[serde(rename = "app.ai_assistant.toggle")]
    AppAiAssistantToggle,
    #[serde(rename = "app.code_review.toggle")]
    AppCodeReviewToggle,
    #[serde(rename = "app.vertical_tabs.toggle")]
    AppVerticalTabsToggle,
    #[serde(rename = "window.list")]
    WindowList,
    #[serde(rename = "window.create")]
    WindowCreate,
    #[serde(rename = "window.focus")]
    WindowFocus,
    #[serde(rename = "window.close")]
    WindowClose,
    #[serde(rename = "tab.list")]
    TabList,
    #[serde(rename = "tab.create")]
    TabCreate,
    #[serde(rename = "tab.activate")]
    TabActivate,
    #[serde(rename = "tab.move")]
    TabMove,
    #[serde(rename = "tab.rename")]
    TabRename,
    #[serde(rename = "tab.close")]
    TabClose,
    #[serde(rename = "pane.list")]
    PaneList,
    #[serde(rename = "pane.split")]
    PaneSplit,
    #[serde(rename = "pane.focus")]
    PaneFocus,
    #[serde(rename = "pane.navigate")]
    PaneNavigate,
    #[serde(rename = "pane.close")]
    PaneClose,
    #[serde(rename = "pane.maximize")]
    PaneMaximize,
    #[serde(rename = "pane.resize")]
    PaneResize,
    #[serde(rename = "pane.session.previous")]
    PaneSessionPrevious,
    #[serde(rename = "pane.session.next")]
    PaneSessionNext,
    #[serde(rename = "session.list")]
    SessionList,
    #[serde(rename = "input.insert")]
    InputInsert,
    #[serde(rename = "input.replace")]
    InputReplace,
    #[serde(rename = "input.clear")]
    InputClear,
    #[serde(rename = "input.mode.set")]
    InputModeSet,
    #[serde(rename = "theme.list")]
    ThemeList,
    #[serde(rename = "theme.set")]
    ThemeSet,
    #[serde(rename = "appearance.get")]
    AppearanceGet,
    #[serde(rename = "appearance.set")]
    AppearanceSet,
    #[serde(rename = "appearance.font_size")]
    AppearanceFontSize,
    #[serde(rename = "appearance.zoom")]
    AppearanceZoom,
    #[serde(rename = "setting.get")]
    SettingGet,
    #[serde(rename = "setting.list")]
    SettingList,
    #[serde(rename = "setting.set")]
    SettingSet,
    #[serde(rename = "setting.toggle")]
    SettingToggle,
}

impl ActionKind {
    pub const ALL: &[Self] = &[
        Self::AppPing,
        Self::AppInspect,
        Self::AppVersion,
        Self::AppActive,
        Self::AppFocus,
        Self::AppSettingsOpen,
        Self::AppCommandPaletteOpen,
        Self::AppCommandSearchOpen,
        Self::AppWarpDriveOpen,
        Self::AppWarpDriveToggle,
        Self::AppResourceCenterToggle,
        Self::AppAiAssistantToggle,
        Self::AppCodeReviewToggle,
        Self::AppVerticalTabsToggle,
        Self::WindowList,
        Self::WindowCreate,
        Self::WindowFocus,
        Self::WindowClose,
        Self::TabList,
        Self::TabCreate,
        Self::TabActivate,
        Self::TabMove,
        Self::TabRename,
        Self::TabClose,
        Self::PaneList,
        Self::PaneSplit,
        Self::PaneFocus,
        Self::PaneNavigate,
        Self::PaneClose,
        Self::PaneMaximize,
        Self::PaneResize,
        Self::PaneSessionPrevious,
        Self::PaneSessionNext,
        Self::SessionList,
        Self::InputInsert,
        Self::InputReplace,
        Self::InputClear,
        Self::InputModeSet,
        Self::ThemeList,
        Self::ThemeSet,
        Self::AppearanceGet,
        Self::AppearanceSet,
        Self::AppearanceFontSize,
        Self::AppearanceZoom,
        Self::SettingGet,
        Self::SettingList,
        Self::SettingSet,
        Self::SettingToggle,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AppPing => "app.ping",
            Self::AppInspect => "app.inspect",
            Self::AppVersion => "app.version",
            Self::AppActive => "app.active",
            Self::AppFocus => "app.focus",
            Self::AppSettingsOpen => "app.settings.open",
            Self::AppCommandPaletteOpen => "app.command_palette.open",
            Self::AppCommandSearchOpen => "app.command_search.open",
            Self::AppWarpDriveOpen => "app.warp_drive.open",
            Self::AppWarpDriveToggle => "app.warp_drive.toggle",
            Self::AppResourceCenterToggle => "app.resource_center.toggle",
            Self::AppAiAssistantToggle => "app.ai_assistant.toggle",
            Self::AppCodeReviewToggle => "app.code_review.toggle",
            Self::AppVerticalTabsToggle => "app.vertical_tabs.toggle",
            Self::WindowList => "window.list",
            Self::WindowCreate => "window.create",
            Self::WindowFocus => "window.focus",
            Self::WindowClose => "window.close",
            Self::TabList => "tab.list",
            Self::TabCreate => "tab.create",
            Self::TabActivate => "tab.activate",
            Self::TabMove => "tab.move",
            Self::TabRename => "tab.rename",
            Self::TabClose => "tab.close",
            Self::PaneList => "pane.list",
            Self::PaneSplit => "pane.split",
            Self::PaneFocus => "pane.focus",
            Self::PaneNavigate => "pane.navigate",
            Self::PaneClose => "pane.close",
            Self::PaneMaximize => "pane.maximize",
            Self::PaneResize => "pane.resize",
            Self::PaneSessionPrevious => "pane.session.previous",
            Self::PaneSessionNext => "pane.session.next",
            Self::SessionList => "session.list",
            Self::InputInsert => "input.insert",
            Self::InputReplace => "input.replace",
            Self::InputClear => "input.clear",
            Self::InputModeSet => "input.mode.set",
            Self::ThemeList => "theme.list",
            Self::ThemeSet => "theme.set",
            Self::AppearanceGet => "appearance.get",
            Self::AppearanceSet => "appearance.set",
            Self::AppearanceFontSize => "appearance.font_size",
            Self::AppearanceZoom => "appearance.zoom",
            Self::SettingGet => "setting.get",
            Self::SettingList => "setting.list",
            Self::SettingSet => "setting.set",
            Self::SettingToggle => "setting.toggle",
        }
    }

    pub fn metadata(self) -> ActionMetadata {
        if self == Self::TabCreate {
            return ActionMetadata {
                kind: self,
                name: self.as_str().to_owned(),
                implementation_status: ActionImplementationStatus::Implemented,
                risk_tier: RiskTier::MutatingNonDestructive,
                requires_authenticated_user: false,
                allowed_invocation_contexts: vec![
                    InvocationContext::InsideWarp,
                    InvocationContext::OutsideWarp,
                ],
                permission: LocalControlPermission::AppStateMutation,
                target_scope: TargetScope::Window,
            };
        }
        ActionMetadata {
            kind: self,
            name: self.as_str().to_owned(),
            implementation_status: ActionImplementationStatus::Stub,
            risk_tier: self.default_risk_tier(),
            requires_authenticated_user: true,
            allowed_invocation_contexts: Vec::new(),
            permission: self.default_permission(),
            target_scope: self.default_target_scope(),
        }
    }

    pub fn implemented_metadata() -> Vec<ActionMetadata> {
        Self::ALL
            .iter()
            .copied()
            .map(Self::metadata)
            .filter(|metadata| {
                metadata.implementation_status == ActionImplementationStatus::Implemented
            })
            .collect()
    }

    pub fn is_implemented(self) -> bool {
        self.metadata().implementation_status == ActionImplementationStatus::Implemented
    }

    fn default_risk_tier(self) -> RiskTier {
        match self {
            Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::WindowList
            | Self::TabList
            | Self::PaneList
            | Self::SessionList
            | Self::ThemeList
            | Self::AppearanceGet
            | Self::SettingGet
            | Self::SettingList => RiskTier::ReadOnlyMetadata,
            Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::WindowClose
            | Self::TabClose
            | Self::PaneClose => RiskTier::MutatingDestructiveOrExecution,
            Self::AppFocus
            | Self::AppSettingsOpen
            | Self::AppCommandPaletteOpen
            | Self::AppCommandSearchOpen
            | Self::AppWarpDriveOpen
            | Self::AppWarpDriveToggle
            | Self::AppResourceCenterToggle
            | Self::AppAiAssistantToggle
            | Self::AppCodeReviewToggle
            | Self::AppVerticalTabsToggle
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabRename
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneMaximize
            | Self::PaneResize
            | Self::PaneSessionPrevious
            | Self::PaneSessionNext
            | Self::ThemeSet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom
            | Self::SettingSet
            | Self::SettingToggle => RiskTier::MutatingNonDestructive,
        }
    }

    fn default_permission(self) -> LocalControlPermission {
        match self.default_risk_tier() {
            RiskTier::ReadOnlyMetadata | RiskTier::ReadOnlyTerminalData => {
                if matches!(self.default_risk_tier(), RiskTier::ReadOnlyTerminalData) {
                    LocalControlPermission::UnderlyingDataRead
                } else {
                    LocalControlPermission::MetadataRead
                }
            }
            RiskTier::MutatingNonDestructive => match self.default_target_scope() {
                TargetScope::Settings | TargetScope::Appearance => {
                    LocalControlPermission::MetadataConfigurationMutation
                }
                TargetScope::Instance
                | TargetScope::Window
                | TargetScope::Tab
                | TargetScope::Pane
                | TargetScope::Session
                | TargetScope::Surface => LocalControlPermission::AppStateMutation,
            },
            RiskTier::MutatingDestructiveOrExecution => {
                LocalControlPermission::UnderlyingDataMutation
            }
        }
    }

    fn default_target_scope(self) -> TargetScope {
        match self {
            Self::WindowList | Self::WindowCreate | Self::WindowFocus | Self::WindowClose => {
                TargetScope::Window
            }
            Self::TabList
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabRename
            | Self::TabClose => TargetScope::Tab,
            Self::PaneList
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneClose
            | Self::PaneMaximize
            | Self::PaneResize
            | Self::PaneSessionPrevious
            | Self::PaneSessionNext => TargetScope::Pane,
            Self::SessionList
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet => TargetScope::Session,
            Self::ThemeList
            | Self::ThemeSet
            | Self::AppearanceGet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom => TargetScope::Appearance,
            Self::SettingGet | Self::SettingList | Self::SettingSet | Self::SettingToggle => {
                TargetScope::Settings
            }
            Self::AppSettingsOpen
            | Self::AppCommandPaletteOpen
            | Self::AppCommandSearchOpen
            | Self::AppWarpDriveOpen
            | Self::AppWarpDriveToggle
            | Self::AppResourceCenterToggle
            | Self::AppAiAssistantToggle
            | Self::AppCodeReviewToggle
            | Self::AppVerticalTabsToggle => TargetScope::Surface,
            Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::AppFocus => TargetScope::Instance,
        }
    }
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ControlResponse {
    Ok { data: serde_json::Value },
    Error { error: ControlError },
}

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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    LocalControlDisabled,
    UnauthorizedLocalClient,
    InsufficientPermissions,
    AuthenticatedUserRequired,
    AuthenticatedUserUnavailable,
    ExecutionContextNotAllowed,
    ProtocolVersionUnsupported,
    InvalidRequest,
    InvalidSelector,
    InvalidParams,
    NoInstance,
    AmbiguousInstance,
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
mod tests {
    use super::*;

    #[test]
    fn request_envelope_serializes_stable_action_names() {
        let request = RequestEnvelope::new(Action::new(ActionKind::WindowFocus));
        let value = serde_json::to_value(&request).expect("request serializes");
        assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(value["action"]["kind"], "window.focus");
    }

    #[test]
    fn response_error_serializes_machine_code() {
        let response = ResponseEnvelope::error(
            Uuid::nil(),
            ControlError::new(ErrorCode::UnauthorizedLocalClient, "bad token"),
        );
        let value = serde_json::to_value(&response).expect("response serializes");
        assert_eq!(value["response"]["status"], "error");
        assert_eq!(
            value["response"]["error"]["code"],
            "unauthorized_local_client"
        );
    }

    #[test]
    fn input_run_is_not_in_the_allowlisted_catalog() {
        let action = serde_json::from_value::<ActionKind>(serde_json::json!("input.run"));
        assert!(action.is_err());
    }

    #[test]
    fn tab_create_metadata_is_first_slice_logged_out_safe_mutation() {
        let metadata = ActionKind::TabCreate.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
        assert_eq!(metadata.risk_tier, RiskTier::MutatingNonDestructive);
        assert!(!metadata.requires_authenticated_user);
        assert_eq!(
            metadata.permission,
            LocalControlPermission::AppStateMutation
        );
        assert_eq!(
            metadata.allowed_invocation_contexts,
            vec![
                InvocationContext::InsideWarp,
                InvocationContext::OutsideWarp
            ]
        );
    }

    #[test]
    fn non_first_slice_actions_are_catalog_stubs() {
        let metadata = ActionKind::WindowCreate.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Stub
        );
        assert!(
            !metadata
                .allowed_invocation_contexts
                .contains(&InvocationContext::OutsideWarp)
        );
    }
}
