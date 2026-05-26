//! Action catalog and metadata used for discovery, permissions, and CLI support.
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

pub const EXCLUDED_LOCAL_FILE_MUTATION_ACTION_NAMES: &[&str] = &[
    "file.read",
    "file.write",
    "file.append",
    "file.delete",
    "file.copy",
    "file.move",
    "file.mkdir",
];

pub const EXCLUDED_STANDALONE_SECRET_AUTH_ACTION_NAMES: &[&str] = &[
    "auth.api_key.set",
    "auth.api_key.status",
    "auth.api_key.revoke",
];

/// Runtime context from which a control request was initiated.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationContext {
    InsideWarp,
    OutsideWarp,
}

/// Execution proof supplied with a credential request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionContextProof {
    VerifiedWarpTerminal {
        proof_id: String,
        terminal_session_id: String,
        proof_secret: String,
    },
    ExternalClient,
}

/// User-facing risk tier for an action.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    ReadOnlyMetadata,
    ReadOnlyTerminalData,
    MutatingNonDestructive,
    MutatingDestructiveOrExecution,
}

/// Category of Warp state or data an action reads or mutates.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateDataCategory {
    MetadataRead,
    UnderlyingDataRead,
    AppStateMutation,
    MetadataConfigurationMutation,
    UnderlyingDataMutation,
}

/// Settings permission bucket required before an action may execute.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionCategory {
    ReadMetadata,
    ReadUnderlyingData,
    MutateAppState,
    MutateMetadataConfiguration,
    MutateUnderlyingData,
}

/// Whether an action requires an authenticated Warp user context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedUserRequirement {
    pub required: bool,
}

/// Level of Warp hierarchy or orthogonal product noun an action targets.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetScope {
    Instance,
    Window,
    Tab,
    Pane,
    Session,
    Block,
    Input,
    History,
    Settings,
    Appearance,
    Surface,
    File,
    Project,
    DriveObject,
    Auth,
    Keybinding,
    Action,
    Capability,
}

/// Whether an action has an app-side implementation in this stack layer.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionImplementationStatus {
    Implemented,
    Stub,
}

/// Typed parameter contract for a catalog action.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionParameterSpec {
    None,
    ActionName,
    BlockId,
    BindingName,
    BooleanValue,
    ColorValue,
    Direction,
    DriveObjectCreate,
    DriveObjectId,
    DriveObjectInsert,
    DriveObjectList,
    DriveObjectUpdate,
    FileOpen,
    InputMode,
    Key,
    KeyValue,
    Limit,
    Namespace,
    PageQuery,
    Path,
    Query,
    Rename,
    Resize,
    TabActivate,
    TabClose,
    TabCreate,
    Text,
    ThemeName,
    WorkflowRun,
}

/// Typed result contract for a catalog action.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionResultSpec {
    Acknowledgement,
    ActiveTarget,
    AppearanceState,
    AuthStatus,
    CapabilityList,
    CapabilityMetadata,
    Content,
    DriveObjectList,
    DriveObjectMetadata,
    FileList,
    InstanceList,
    InstanceMetadata,
    KeybindingList,
    KeybindingMetadata,
    ProjectList,
    SettingList,
    SettingValue,
    TargetList,
    TargetMetadata,
    ThemeList,
    ThemeState,
}

/// Discoverable metadata describing one local-control action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionMetadata {
    pub kind: ActionKind,
    pub name: String,
    pub implementation_status: ActionImplementationStatus,
    pub risk_tier: RiskTier,
    pub state_data_category: StateDataCategory,
    pub requires_authenticated_user: bool,
    pub authenticated_user: AuthenticatedUserRequirement,
    pub allowed_invocation_contexts: Vec<InvocationContext>,
    pub permission_category: PermissionCategory,
    pub target_scope: TargetScope,
    pub parameter_spec: ActionParameterSpec,
    pub result_spec: ActionResultSpec,
}

/// Stable protocol name for every approved `warpctrl` action.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionKind {
    #[serde(rename = "instance.list")]
    InstanceList,
    #[serde(rename = "instance.inspect")]
    InstanceInspect,
    #[serde(rename = "app.ping")]
    AppPing,
    #[serde(rename = "app.version")]
    AppVersion,
    #[serde(rename = "app.active")]
    AppActive,
    #[serde(rename = "app.focus")]
    AppFocus,
    #[serde(rename = "auth.status")]
    AuthStatus,
    #[serde(rename = "auth.login")]
    AuthLogin,
    #[serde(rename = "capability.list")]
    CapabilityList,
    #[serde(rename = "capability.inspect")]
    CapabilityInspect,
    #[serde(rename = "window.list")]
    WindowList,
    #[serde(rename = "window.inspect")]
    WindowInspect,
    #[serde(rename = "window.create")]
    WindowCreate,
    #[serde(rename = "window.focus")]
    WindowFocus,
    #[serde(rename = "window.close")]
    WindowClose,
    #[serde(rename = "tab.list")]
    TabList,
    #[serde(rename = "tab.inspect")]
    TabInspect,
    #[serde(rename = "tab.create")]
    TabCreate,
    #[serde(rename = "tab.activate")]
    TabActivate,
    #[serde(rename = "tab.move")]
    TabMove,
    #[serde(rename = "tab.close")]
    TabClose,
    #[serde(rename = "tab.rename")]
    TabRename,
    #[serde(rename = "tab.reset_name")]
    TabResetName,
    #[serde(rename = "tab.color.set")]
    TabColorSet,
    #[serde(rename = "tab.color.clear")]
    TabColorClear,
    #[serde(rename = "pane.list")]
    PaneList,
    #[serde(rename = "pane.inspect")]
    PaneInspect,
    #[serde(rename = "pane.split")]
    PaneSplit,
    #[serde(rename = "pane.focus")]
    PaneFocus,
    #[serde(rename = "pane.navigate")]
    PaneNavigate,
    #[serde(rename = "pane.resize")]
    PaneResize,
    #[serde(rename = "pane.maximize")]
    PaneMaximize,
    #[serde(rename = "pane.unmaximize")]
    PaneUnmaximize,
    #[serde(rename = "pane.close")]
    PaneClose,
    #[serde(rename = "pane.rename")]
    PaneRename,
    #[serde(rename = "pane.reset_name")]
    PaneResetName,
    #[serde(rename = "session.list")]
    SessionList,
    #[serde(rename = "session.inspect")]
    SessionInspect,
    #[serde(rename = "session.activate")]
    SessionActivate,
    #[serde(rename = "session.previous")]
    SessionPrevious,
    #[serde(rename = "session.next")]
    SessionNext,
    #[serde(rename = "session.reopen_closed")]
    SessionReopenClosed,
    #[serde(rename = "block.list")]
    BlockList,
    #[serde(rename = "block.inspect")]
    BlockInspect,
    #[serde(rename = "block.output")]
    BlockOutput,
    #[serde(rename = "input.get")]
    InputGet,
    #[serde(rename = "input.insert")]
    InputInsert,
    #[serde(rename = "input.replace")]
    InputReplace,
    #[serde(rename = "input.clear")]
    InputClear,
    #[serde(rename = "input.mode.set")]
    InputModeSet,
    #[serde(rename = "input.run")]
    InputRun,
    #[serde(rename = "history.list")]
    HistoryList,
    #[serde(rename = "theme.list")]
    ThemeList,
    #[serde(rename = "theme.get")]
    ThemeGet,
    #[serde(rename = "theme.set")]
    ThemeSet,
    #[serde(rename = "theme.system.set")]
    ThemeSystemSet,
    #[serde(rename = "theme.light.set")]
    ThemeLightSet,
    #[serde(rename = "theme.dark.set")]
    ThemeDarkSet,
    #[serde(rename = "appearance.get")]
    AppearanceGet,
    #[serde(rename = "appearance.font_size.increase")]
    AppearanceFontSizeIncrease,
    #[serde(rename = "appearance.font_size.decrease")]
    AppearanceFontSizeDecrease,
    #[serde(rename = "appearance.font_size.reset")]
    AppearanceFontSizeReset,
    #[serde(rename = "appearance.zoom.increase")]
    AppearanceZoomIncrease,
    #[serde(rename = "appearance.zoom.decrease")]
    AppearanceZoomDecrease,
    #[serde(rename = "appearance.zoom.reset")]
    AppearanceZoomReset,
    #[serde(rename = "setting.list")]
    SettingList,
    #[serde(rename = "setting.get")]
    SettingGet,
    #[serde(rename = "setting.set")]
    SettingSet,
    #[serde(rename = "setting.toggle")]
    SettingToggle,
    #[serde(rename = "keybinding.list")]
    KeybindingList,
    #[serde(rename = "keybinding.get")]
    KeybindingGet,
    #[serde(rename = "action.list")]
    ActionList,
    #[serde(rename = "action.inspect")]
    ActionInspect,
    #[serde(rename = "surface.settings.open")]
    SurfaceSettingsOpen,
    #[serde(rename = "surface.command_palette.open")]
    SurfaceCommandPaletteOpen,
    #[serde(rename = "surface.command_search.open")]
    SurfaceCommandSearchOpen,
    #[serde(rename = "surface.warp_drive.open")]
    SurfaceWarpDriveOpen,
    #[serde(rename = "surface.warp_drive.toggle")]
    SurfaceWarpDriveToggle,
    #[serde(rename = "surface.resource_center.toggle")]
    SurfaceResourceCenterToggle,
    #[serde(rename = "surface.ai_assistant.toggle")]
    SurfaceAiAssistantToggle,
    #[serde(rename = "surface.code_review.toggle")]
    SurfaceCodeReviewToggle,
    #[serde(rename = "surface.left_panel.toggle")]
    SurfaceLeftPanelToggle,
    #[serde(rename = "surface.right_panel.toggle")]
    SurfaceRightPanelToggle,
    #[serde(rename = "surface.vertical_tabs.toggle")]
    SurfaceVerticalTabsToggle,
    #[serde(rename = "file.list")]
    FileList,
    #[serde(rename = "file.open")]
    FileOpen,
    #[serde(rename = "project.active")]
    ProjectActive,
    #[serde(rename = "project.list")]
    ProjectList,
    #[serde(rename = "project.open")]
    ProjectOpen,
    #[serde(rename = "drive.list")]
    DriveList,
    #[serde(rename = "drive.inspect")]
    DriveInspect,
    #[serde(rename = "drive.open")]
    DriveOpen,
    #[serde(rename = "drive.notebook.open")]
    DriveNotebookOpen,
    #[serde(rename = "drive.env_var_collection.open")]
    DriveEnvVarCollectionOpen,
    #[serde(rename = "drive.object.share.open")]
    DriveObjectShareOpen,
    #[serde(rename = "drive.object.create")]
    DriveObjectCreate,
    #[serde(rename = "drive.object.update")]
    DriveObjectUpdate,
    #[serde(rename = "drive.object.delete")]
    DriveObjectDelete,
    #[serde(rename = "drive.object.insert")]
    DriveObjectInsert,
    #[serde(rename = "drive.object.share_to_team")]
    DriveObjectShareToTeam,
    #[serde(rename = "drive.workflow.run")]
    DriveWorkflowRun,
}

impl ActionKind {
    pub const ALL: &[Self] = &[
        Self::InstanceList,
        Self::InstanceInspect,
        Self::AppPing,
        Self::AppVersion,
        Self::AppActive,
        Self::AppFocus,
        Self::AuthStatus,
        Self::AuthLogin,
        Self::CapabilityList,
        Self::CapabilityInspect,
        Self::WindowList,
        Self::WindowInspect,
        Self::WindowCreate,
        Self::WindowFocus,
        Self::WindowClose,
        Self::TabList,
        Self::TabInspect,
        Self::TabCreate,
        Self::TabActivate,
        Self::TabMove,
        Self::TabClose,
        Self::TabRename,
        Self::TabResetName,
        Self::TabColorSet,
        Self::TabColorClear,
        Self::PaneList,
        Self::PaneInspect,
        Self::PaneSplit,
        Self::PaneFocus,
        Self::PaneNavigate,
        Self::PaneResize,
        Self::PaneMaximize,
        Self::PaneUnmaximize,
        Self::PaneClose,
        Self::PaneRename,
        Self::PaneResetName,
        Self::SessionList,
        Self::SessionInspect,
        Self::SessionActivate,
        Self::SessionPrevious,
        Self::SessionNext,
        Self::SessionReopenClosed,
        Self::BlockList,
        Self::BlockInspect,
        Self::BlockOutput,
        Self::InputGet,
        Self::InputInsert,
        Self::InputReplace,
        Self::InputClear,
        Self::InputModeSet,
        Self::InputRun,
        Self::HistoryList,
        Self::ThemeList,
        Self::ThemeGet,
        Self::ThemeSet,
        Self::ThemeSystemSet,
        Self::ThemeLightSet,
        Self::ThemeDarkSet,
        Self::AppearanceGet,
        Self::AppearanceFontSizeIncrease,
        Self::AppearanceFontSizeDecrease,
        Self::AppearanceFontSizeReset,
        Self::AppearanceZoomIncrease,
        Self::AppearanceZoomDecrease,
        Self::AppearanceZoomReset,
        Self::SettingList,
        Self::SettingGet,
        Self::SettingSet,
        Self::SettingToggle,
        Self::KeybindingList,
        Self::KeybindingGet,
        Self::ActionList,
        Self::ActionInspect,
        Self::SurfaceSettingsOpen,
        Self::SurfaceCommandPaletteOpen,
        Self::SurfaceCommandSearchOpen,
        Self::SurfaceWarpDriveOpen,
        Self::SurfaceWarpDriveToggle,
        Self::SurfaceResourceCenterToggle,
        Self::SurfaceAiAssistantToggle,
        Self::SurfaceCodeReviewToggle,
        Self::SurfaceLeftPanelToggle,
        Self::SurfaceRightPanelToggle,
        Self::SurfaceVerticalTabsToggle,
        Self::FileList,
        Self::FileOpen,
        Self::ProjectActive,
        Self::ProjectList,
        Self::ProjectOpen,
        Self::DriveList,
        Self::DriveInspect,
        Self::DriveOpen,
        Self::DriveNotebookOpen,
        Self::DriveEnvVarCollectionOpen,
        Self::DriveObjectShareOpen,
        Self::DriveObjectCreate,
        Self::DriveObjectUpdate,
        Self::DriveObjectDelete,
        Self::DriveObjectInsert,
        Self::DriveObjectShareToTeam,
        Self::DriveWorkflowRun,
    ];

    pub fn as_str(self) -> &'static str {
        serde_names::action_name(self)
    }

    pub fn metadata(self) -> ActionMetadata {
        let implementation_status = self.implementation_status();
        let requires_authenticated_user = self.requires_authenticated_user();
        ActionMetadata {
            kind: self,
            name: self.as_str().to_owned(),
            implementation_status,
            risk_tier: self.default_risk_tier(),
            state_data_category: self.default_state_data_category(),
            requires_authenticated_user,
            authenticated_user: AuthenticatedUserRequirement {
                required: requires_authenticated_user,
            },
            allowed_invocation_contexts: self.allowed_invocation_contexts(),
            permission_category: self.default_permission_category(),
            target_scope: self.default_target_scope(),
            parameter_spec: self.parameter_spec(),
            result_spec: self.result_spec(),
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
        self.implementation_status() == ActionImplementationStatus::Implemented
    }

    fn implementation_status(self) -> ActionImplementationStatus {
        match self {
            Self::InstanceList
            | Self::InstanceInspect
            | Self::AppPing
            | Self::AppVersion
            | Self::AppActive
            | Self::CapabilityList
            | Self::CapabilityInspect
            | Self::WindowList
            | Self::WindowInspect
            | Self::TabList
            | Self::TabInspect
            | Self::TabCreate
            | Self::PaneList
            | Self::PaneInspect
            | Self::SessionList
            | Self::SessionInspect
            | Self::BlockList
            | Self::BlockInspect
            | Self::BlockOutput
            | Self::InputGet
            | Self::HistoryList
            | Self::ThemeList
            | Self::ThemeGet
            | Self::AppearanceGet
            | Self::SettingList
            | Self::SettingGet
            | Self::KeybindingList
            | Self::KeybindingGet
            | Self::ActionList
            | Self::ActionInspect
            | Self::FileList
            | Self::ProjectActive
            | Self::ProjectList
            | Self::DriveList
            | Self::DriveInspect => ActionImplementationStatus::Implemented,
            Self::AppFocus
            | Self::AuthStatus
            | Self::AuthLogin
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::WindowClose
            | Self::TabActivate
            | Self::TabMove
            | Self::TabClose
            | Self::TabRename
            | Self::TabResetName
            | Self::TabColorSet
            | Self::TabColorClear
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneResize
            | Self::PaneMaximize
            | Self::PaneUnmaximize
            | Self::PaneClose
            | Self::PaneRename
            | Self::PaneResetName
            | Self::SessionActivate
            | Self::SessionPrevious
            | Self::SessionNext
            | Self::SessionReopenClosed
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun
            | Self::ThemeSet
            | Self::ThemeSystemSet
            | Self::ThemeLightSet
            | Self::ThemeDarkSet
            | Self::AppearanceFontSizeIncrease
            | Self::AppearanceFontSizeDecrease
            | Self::AppearanceFontSizeReset
            | Self::AppearanceZoomIncrease
            | Self::AppearanceZoomDecrease
            | Self::AppearanceZoomReset
            | Self::SettingSet
            | Self::SettingToggle
            | Self::SurfaceSettingsOpen
            | Self::SurfaceCommandPaletteOpen
            | Self::SurfaceCommandSearchOpen
            | Self::SurfaceWarpDriveOpen
            | Self::SurfaceWarpDriveToggle
            | Self::SurfaceResourceCenterToggle
            | Self::SurfaceAiAssistantToggle
            | Self::SurfaceCodeReviewToggle
            | Self::SurfaceLeftPanelToggle
            | Self::SurfaceRightPanelToggle
            | Self::SurfaceVerticalTabsToggle
            | Self::FileOpen
            | Self::ProjectOpen
            | Self::DriveOpen
            | Self::DriveNotebookOpen
            | Self::DriveEnvVarCollectionOpen
            | Self::DriveObjectShareOpen
            | Self::DriveObjectCreate
            | Self::DriveObjectUpdate
            | Self::DriveObjectDelete
            | Self::DriveObjectInsert
            | Self::DriveObjectShareToTeam
            | Self::DriveWorkflowRun => ActionImplementationStatus::Stub,
        }
    }

    fn allowed_invocation_contexts(self) -> Vec<InvocationContext> {
        match self {
            Self::InstanceList | Self::AppPing | Self::AppVersion | Self::TabCreate => {
                vec![InvocationContext::OutsideWarp]
            }
            _ if self.requires_authenticated_user() => vec![InvocationContext::InsideWarp],
            _ => vec![
                InvocationContext::InsideWarp,
                InvocationContext::OutsideWarp,
            ],
        }
    }

    fn requires_authenticated_user(self) -> bool {
        match self {
            Self::DriveList
            | Self::DriveInspect
            | Self::DriveOpen
            | Self::DriveNotebookOpen
            | Self::DriveEnvVarCollectionOpen
            | Self::DriveObjectShareOpen
            | Self::DriveObjectCreate
            | Self::DriveObjectUpdate
            | Self::DriveObjectDelete
            | Self::DriveObjectInsert
            | Self::DriveObjectShareToTeam
            | Self::DriveWorkflowRun
            | Self::InputRun => true,
            Self::InstanceList
            | Self::InstanceInspect
            | Self::AppPing
            | Self::AppVersion
            | Self::AppActive
            | Self::AppFocus
            | Self::AuthStatus
            | Self::AuthLogin
            | Self::CapabilityList
            | Self::CapabilityInspect
            | Self::WindowList
            | Self::WindowInspect
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::WindowClose
            | Self::TabList
            | Self::TabInspect
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabClose
            | Self::TabRename
            | Self::TabResetName
            | Self::TabColorSet
            | Self::TabColorClear
            | Self::PaneList
            | Self::PaneInspect
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneResize
            | Self::PaneMaximize
            | Self::PaneUnmaximize
            | Self::PaneClose
            | Self::PaneRename
            | Self::PaneResetName
            | Self::SessionList
            | Self::SessionInspect
            | Self::SessionActivate
            | Self::SessionPrevious
            | Self::SessionNext
            | Self::SessionReopenClosed
            | Self::BlockList
            | Self::BlockInspect
            | Self::BlockOutput
            | Self::InputGet
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::HistoryList
            | Self::ThemeList
            | Self::ThemeGet
            | Self::ThemeSet
            | Self::ThemeSystemSet
            | Self::ThemeLightSet
            | Self::ThemeDarkSet
            | Self::AppearanceFontSizeIncrease
            | Self::AppearanceFontSizeDecrease
            | Self::AppearanceFontSizeReset
            | Self::AppearanceZoomIncrease
            | Self::AppearanceZoomDecrease
            | Self::AppearanceZoomReset
            | Self::SettingSet
            | Self::SettingToggle
            | Self::KeybindingList
            | Self::KeybindingGet
            | Self::ActionList
            | Self::ActionInspect
            | Self::SurfaceSettingsOpen
            | Self::SurfaceCommandPaletteOpen
            | Self::SurfaceCommandSearchOpen
            | Self::SurfaceWarpDriveOpen
            | Self::SurfaceWarpDriveToggle
            | Self::SurfaceResourceCenterToggle
            | Self::SurfaceAiAssistantToggle
            | Self::SurfaceCodeReviewToggle
            | Self::SurfaceLeftPanelToggle
            | Self::SurfaceRightPanelToggle
            | Self::SurfaceVerticalTabsToggle
            | Self::FileList
            | Self::FileOpen
            | Self::ProjectActive
            | Self::ProjectList
            | Self::ProjectOpen => false,
        }
    }

    fn default_risk_tier(self) -> RiskTier {
        match self.default_state_data_category() {
            StateDataCategory::MetadataRead => RiskTier::ReadOnlyMetadata,
            StateDataCategory::UnderlyingDataRead => RiskTier::ReadOnlyTerminalData,
            StateDataCategory::UnderlyingDataMutation => RiskTier::MutatingDestructiveOrExecution,
            StateDataCategory::AppStateMutation
            | StateDataCategory::MetadataConfigurationMutation => RiskTier::MutatingNonDestructive,
        }
    }

    fn default_state_data_category(self) -> StateDataCategory {
        match self {
            Self::InstanceList
            | Self::InstanceInspect
            | Self::AppPing
            | Self::AppVersion
            | Self::AppActive
            | Self::AuthStatus
            | Self::CapabilityList
            | Self::CapabilityInspect
            | Self::WindowList
            | Self::WindowInspect
            | Self::TabList
            | Self::TabInspect
            | Self::PaneList
            | Self::PaneInspect
            | Self::SessionList
            | Self::SessionInspect
            | Self::BlockList
            | Self::ThemeList
            | Self::ThemeGet
            | Self::AppearanceGet
            | Self::SettingList
            | Self::SettingGet
            | Self::KeybindingList
            | Self::KeybindingGet
            | Self::ActionList
            | Self::ActionInspect
            | Self::FileList
            | Self::ProjectActive
            | Self::ProjectList
            | Self::DriveList => StateDataCategory::MetadataRead,
            Self::BlockInspect
            | Self::BlockOutput
            | Self::InputGet
            | Self::HistoryList
            | Self::DriveInspect => StateDataCategory::UnderlyingDataRead,
            Self::TabRename
            | Self::TabResetName
            | Self::TabColorSet
            | Self::TabColorClear
            | Self::PaneRename
            | Self::PaneResetName
            | Self::ThemeSet
            | Self::ThemeSystemSet
            | Self::ThemeLightSet
            | Self::ThemeDarkSet
            | Self::AppearanceFontSizeIncrease
            | Self::AppearanceFontSizeDecrease
            | Self::AppearanceFontSizeReset
            | Self::AppearanceZoomIncrease
            | Self::AppearanceZoomDecrease
            | Self::AppearanceZoomReset
            | Self::SettingSet
            | Self::SettingToggle => StateDataCategory::MetadataConfigurationMutation,
            Self::DriveObjectCreate
            | Self::DriveObjectUpdate
            | Self::DriveObjectDelete
            | Self::DriveObjectInsert
            | Self::DriveObjectShareToTeam
            | Self::DriveWorkflowRun
            | Self::InputRun => StateDataCategory::UnderlyingDataMutation,
            Self::AppFocus
            | Self::AuthLogin
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::WindowClose
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabClose
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneResize
            | Self::PaneMaximize
            | Self::PaneUnmaximize
            | Self::PaneClose
            | Self::SessionActivate
            | Self::SessionPrevious
            | Self::SessionNext
            | Self::SessionReopenClosed
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::SurfaceSettingsOpen
            | Self::SurfaceCommandPaletteOpen
            | Self::SurfaceCommandSearchOpen
            | Self::SurfaceWarpDriveOpen
            | Self::SurfaceWarpDriveToggle
            | Self::SurfaceResourceCenterToggle
            | Self::SurfaceAiAssistantToggle
            | Self::SurfaceCodeReviewToggle
            | Self::SurfaceLeftPanelToggle
            | Self::SurfaceRightPanelToggle
            | Self::SurfaceVerticalTabsToggle
            | Self::FileOpen
            | Self::ProjectOpen
            | Self::DriveOpen
            | Self::DriveNotebookOpen
            | Self::DriveEnvVarCollectionOpen
            | Self::DriveObjectShareOpen => StateDataCategory::AppStateMutation,
        }
    }

    fn default_permission_category(self) -> PermissionCategory {
        match self.default_state_data_category() {
            StateDataCategory::MetadataRead => PermissionCategory::ReadMetadata,
            StateDataCategory::UnderlyingDataRead => PermissionCategory::ReadUnderlyingData,
            StateDataCategory::AppStateMutation => PermissionCategory::MutateAppState,
            StateDataCategory::MetadataConfigurationMutation => {
                PermissionCategory::MutateMetadataConfiguration
            }
            StateDataCategory::UnderlyingDataMutation => PermissionCategory::MutateUnderlyingData,
        }
    }

    fn default_target_scope(self) -> TargetScope {
        match self {
            Self::InstanceList
            | Self::InstanceInspect
            | Self::AppPing
            | Self::AppVersion
            | Self::AppActive
            | Self::AppFocus => TargetScope::Instance,
            Self::AuthStatus | Self::AuthLogin => TargetScope::Auth,
            Self::CapabilityList | Self::CapabilityInspect => TargetScope::Capability,
            Self::WindowList
            | Self::WindowInspect
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::WindowClose => TargetScope::Window,
            Self::TabList
            | Self::TabInspect
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabClose
            | Self::TabRename
            | Self::TabResetName
            | Self::TabColorSet
            | Self::TabColorClear => TargetScope::Tab,
            Self::PaneList
            | Self::PaneInspect
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneResize
            | Self::PaneMaximize
            | Self::PaneUnmaximize
            | Self::PaneClose
            | Self::PaneRename
            | Self::PaneResetName => TargetScope::Pane,
            Self::SessionList
            | Self::SessionInspect
            | Self::SessionActivate
            | Self::SessionPrevious
            | Self::SessionNext
            | Self::SessionReopenClosed => TargetScope::Session,
            Self::BlockList | Self::BlockInspect | Self::BlockOutput => TargetScope::Block,
            Self::InputGet
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun => TargetScope::Input,
            Self::HistoryList => TargetScope::History,
            Self::ThemeList
            | Self::ThemeGet
            | Self::ThemeSet
            | Self::ThemeSystemSet
            | Self::ThemeLightSet
            | Self::ThemeDarkSet
            | Self::AppearanceFontSizeIncrease
            | Self::AppearanceFontSizeDecrease
            | Self::AppearanceFontSizeReset
            | Self::AppearanceZoomIncrease
            | Self::AppearanceZoomDecrease
            | Self::AppearanceZoomReset => TargetScope::Appearance,
            Self::SettingList | Self::SettingGet | Self::SettingSet | Self::SettingToggle => {
                TargetScope::Settings
            }
            Self::KeybindingList | Self::KeybindingGet => TargetScope::Keybinding,
            Self::ActionList | Self::ActionInspect => TargetScope::Action,
            Self::SurfaceSettingsOpen
            | Self::SurfaceCommandPaletteOpen
            | Self::SurfaceCommandSearchOpen
            | Self::SurfaceWarpDriveOpen
            | Self::SurfaceWarpDriveToggle
            | Self::SurfaceResourceCenterToggle
            | Self::SurfaceAiAssistantToggle
            | Self::SurfaceCodeReviewToggle
            | Self::SurfaceLeftPanelToggle
            | Self::SurfaceRightPanelToggle
            | Self::SurfaceVerticalTabsToggle => TargetScope::Surface,
            Self::FileList | Self::FileOpen => TargetScope::File,
            Self::ProjectActive | Self::ProjectList | Self::ProjectOpen => TargetScope::Project,
            Self::DriveList
            | Self::DriveInspect
            | Self::DriveOpen
            | Self::DriveNotebookOpen
            | Self::DriveEnvVarCollectionOpen
            | Self::DriveObjectShareOpen
            | Self::DriveObjectCreate
            | Self::DriveObjectUpdate
            | Self::DriveObjectDelete
            | Self::DriveObjectInsert
            | Self::DriveObjectShareToTeam
            | Self::DriveWorkflowRun => TargetScope::DriveObject,
        }
    }

    fn parameter_spec(self) -> ActionParameterSpec {
        match self {
            Self::InstanceList
            | Self::InstanceInspect
            | Self::AppPing
            | Self::AppVersion
            | Self::AppActive
            | Self::AppFocus
            | Self::AuthStatus
            | Self::AuthLogin
            | Self::CapabilityList
            | Self::WindowList
            | Self::WindowInspect
            | Self::TabList
            | Self::TabInspect
            | Self::PaneList
            | Self::PaneInspect
            | Self::SessionList
            | Self::SessionInspect
            | Self::ThemeList
            | Self::ThemeGet
            | Self::AppearanceGet
            | Self::KeybindingList
            | Self::ActionList
            | Self::FileList
            | Self::ProjectActive
            | Self::ProjectList => ActionParameterSpec::None,
            Self::CapabilityInspect | Self::ActionInspect => ActionParameterSpec::ActionName,
            Self::WindowCreate | Self::TabCreate => ActionParameterSpec::TabCreate,
            Self::WindowFocus
            | Self::WindowClose
            | Self::TabResetName
            | Self::TabColorClear
            | Self::PaneFocus
            | Self::PaneMaximize
            | Self::PaneUnmaximize
            | Self::PaneClose
            | Self::PaneResetName
            | Self::SessionActivate => ActionParameterSpec::None,
            Self::TabActivate => ActionParameterSpec::TabActivate,
            Self::TabMove | Self::PaneSplit | Self::PaneNavigate => ActionParameterSpec::Direction,
            Self::TabClose => ActionParameterSpec::TabClose,
            Self::TabRename | Self::PaneRename => ActionParameterSpec::Rename,
            Self::TabColorSet => ActionParameterSpec::ColorValue,
            Self::PaneResize => ActionParameterSpec::Resize,
            Self::SessionPrevious | Self::SessionNext | Self::SessionReopenClosed => {
                ActionParameterSpec::None
            }
            Self::BlockList | Self::HistoryList => ActionParameterSpec::Limit,
            Self::BlockInspect | Self::BlockOutput => ActionParameterSpec::BlockId,
            Self::InputGet => ActionParameterSpec::None,
            Self::InputInsert | Self::InputReplace | Self::InputRun => ActionParameterSpec::Text,
            Self::InputClear => ActionParameterSpec::None,
            Self::InputModeSet => ActionParameterSpec::InputMode,
            Self::ThemeSet | Self::ThemeLightSet | Self::ThemeDarkSet => {
                ActionParameterSpec::ThemeName
            }
            Self::ThemeSystemSet => ActionParameterSpec::BooleanValue,
            Self::AppearanceFontSizeIncrease
            | Self::AppearanceFontSizeDecrease
            | Self::AppearanceFontSizeReset
            | Self::AppearanceZoomIncrease
            | Self::AppearanceZoomDecrease
            | Self::AppearanceZoomReset => ActionParameterSpec::None,
            Self::SettingList => ActionParameterSpec::None,
            Self::SettingGet => ActionParameterSpec::Key,
            Self::SettingSet => ActionParameterSpec::KeyValue,
            Self::SettingToggle => ActionParameterSpec::Key,
            Self::KeybindingGet => ActionParameterSpec::BindingName,
            Self::SurfaceSettingsOpen => ActionParameterSpec::PageQuery,
            Self::SurfaceCommandPaletteOpen | Self::SurfaceCommandSearchOpen => {
                ActionParameterSpec::Query
            }
            Self::SurfaceWarpDriveOpen
            | Self::SurfaceWarpDriveToggle
            | Self::SurfaceResourceCenterToggle
            | Self::SurfaceAiAssistantToggle
            | Self::SurfaceCodeReviewToggle
            | Self::SurfaceLeftPanelToggle
            | Self::SurfaceRightPanelToggle
            | Self::SurfaceVerticalTabsToggle => ActionParameterSpec::None,
            Self::FileOpen => ActionParameterSpec::FileOpen,
            Self::ProjectOpen => ActionParameterSpec::Path,
            Self::DriveList => ActionParameterSpec::DriveObjectList,
            Self::DriveInspect
            | Self::DriveOpen
            | Self::DriveNotebookOpen
            | Self::DriveEnvVarCollectionOpen
            | Self::DriveObjectShareOpen
            | Self::DriveObjectDelete
            | Self::DriveObjectShareToTeam => ActionParameterSpec::DriveObjectId,
            Self::DriveObjectCreate => ActionParameterSpec::DriveObjectCreate,
            Self::DriveObjectUpdate => ActionParameterSpec::DriveObjectUpdate,
            Self::DriveObjectInsert => ActionParameterSpec::DriveObjectInsert,
            Self::DriveWorkflowRun => ActionParameterSpec::WorkflowRun,
        }
    }

    fn result_spec(self) -> ActionResultSpec {
        match self {
            Self::InstanceList => ActionResultSpec::InstanceList,
            Self::InstanceInspect | Self::AppPing | Self::AppVersion => {
                ActionResultSpec::InstanceMetadata
            }
            Self::AppActive => ActionResultSpec::ActiveTarget,
            Self::AuthStatus => ActionResultSpec::AuthStatus,
            Self::CapabilityList => ActionResultSpec::CapabilityList,
            Self::CapabilityInspect => ActionResultSpec::CapabilityMetadata,
            Self::WindowList
            | Self::TabList
            | Self::PaneList
            | Self::SessionList
            | Self::BlockList => ActionResultSpec::TargetList,
            Self::WindowInspect | Self::TabInspect | Self::PaneInspect | Self::SessionInspect => {
                ActionResultSpec::TargetMetadata
            }
            Self::BlockInspect | Self::BlockOutput | Self::InputGet | Self::HistoryList => {
                ActionResultSpec::Content
            }
            Self::ThemeList => ActionResultSpec::ThemeList,
            Self::ThemeGet => ActionResultSpec::ThemeState,
            Self::AppearanceGet => ActionResultSpec::AppearanceState,
            Self::SettingList => ActionResultSpec::SettingList,
            Self::SettingGet => ActionResultSpec::SettingValue,
            Self::KeybindingList => ActionResultSpec::KeybindingList,
            Self::KeybindingGet => ActionResultSpec::KeybindingMetadata,
            Self::ActionList => ActionResultSpec::CapabilityList,
            Self::ActionInspect => ActionResultSpec::CapabilityMetadata,
            Self::FileList => ActionResultSpec::FileList,
            Self::ProjectActive | Self::ProjectList => ActionResultSpec::ProjectList,
            Self::DriveList => ActionResultSpec::DriveObjectList,
            Self::DriveInspect => ActionResultSpec::DriveObjectMetadata,
            Self::AppFocus
            | Self::AuthLogin
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::WindowClose
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabClose
            | Self::TabRename
            | Self::TabResetName
            | Self::TabColorSet
            | Self::TabColorClear
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneResize
            | Self::PaneMaximize
            | Self::PaneUnmaximize
            | Self::PaneClose
            | Self::PaneRename
            | Self::PaneResetName
            | Self::SessionActivate
            | Self::SessionPrevious
            | Self::SessionNext
            | Self::SessionReopenClosed
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun
            | Self::ThemeSet
            | Self::ThemeSystemSet
            | Self::ThemeLightSet
            | Self::ThemeDarkSet
            | Self::AppearanceFontSizeIncrease
            | Self::AppearanceFontSizeDecrease
            | Self::AppearanceFontSizeReset
            | Self::AppearanceZoomIncrease
            | Self::AppearanceZoomDecrease
            | Self::AppearanceZoomReset
            | Self::SettingSet
            | Self::SettingToggle
            | Self::SurfaceSettingsOpen
            | Self::SurfaceCommandPaletteOpen
            | Self::SurfaceCommandSearchOpen
            | Self::SurfaceWarpDriveOpen
            | Self::SurfaceWarpDriveToggle
            | Self::SurfaceResourceCenterToggle
            | Self::SurfaceAiAssistantToggle
            | Self::SurfaceCodeReviewToggle
            | Self::SurfaceLeftPanelToggle
            | Self::SurfaceRightPanelToggle
            | Self::SurfaceVerticalTabsToggle
            | Self::FileOpen
            | Self::ProjectOpen
            | Self::DriveOpen
            | Self::DriveNotebookOpen
            | Self::DriveEnvVarCollectionOpen
            | Self::DriveObjectShareOpen
            | Self::DriveObjectCreate
            | Self::DriveObjectUpdate
            | Self::DriveObjectDelete
            | Self::DriveObjectInsert
            | Self::DriveObjectShareToTeam
            | Self::DriveWorkflowRun => ActionResultSpec::Acknowledgement,
        }
    }
}

mod serde_names {
    use super::ActionKind;

    pub(super) fn action_name(action: ActionKind) -> &'static str {
        match action {
            ActionKind::InstanceList => "instance.list",
            ActionKind::InstanceInspect => "instance.inspect",
            ActionKind::AppPing => "app.ping",
            ActionKind::AppVersion => "app.version",
            ActionKind::AppActive => "app.active",
            ActionKind::AppFocus => "app.focus",
            ActionKind::AuthStatus => "auth.status",
            ActionKind::AuthLogin => "auth.login",
            ActionKind::CapabilityList => "capability.list",
            ActionKind::CapabilityInspect => "capability.inspect",
            ActionKind::WindowList => "window.list",
            ActionKind::WindowInspect => "window.inspect",
            ActionKind::WindowCreate => "window.create",
            ActionKind::WindowFocus => "window.focus",
            ActionKind::WindowClose => "window.close",
            ActionKind::TabList => "tab.list",
            ActionKind::TabInspect => "tab.inspect",
            ActionKind::TabCreate => "tab.create",
            ActionKind::TabActivate => "tab.activate",
            ActionKind::TabMove => "tab.move",
            ActionKind::TabClose => "tab.close",
            ActionKind::TabRename => "tab.rename",
            ActionKind::TabResetName => "tab.reset_name",
            ActionKind::TabColorSet => "tab.color.set",
            ActionKind::TabColorClear => "tab.color.clear",
            ActionKind::PaneList => "pane.list",
            ActionKind::PaneInspect => "pane.inspect",
            ActionKind::PaneSplit => "pane.split",
            ActionKind::PaneFocus => "pane.focus",
            ActionKind::PaneNavigate => "pane.navigate",
            ActionKind::PaneResize => "pane.resize",
            ActionKind::PaneMaximize => "pane.maximize",
            ActionKind::PaneUnmaximize => "pane.unmaximize",
            ActionKind::PaneClose => "pane.close",
            ActionKind::PaneRename => "pane.rename",
            ActionKind::PaneResetName => "pane.reset_name",
            ActionKind::SessionList => "session.list",
            ActionKind::SessionInspect => "session.inspect",
            ActionKind::SessionActivate => "session.activate",
            ActionKind::SessionPrevious => "session.previous",
            ActionKind::SessionNext => "session.next",
            ActionKind::SessionReopenClosed => "session.reopen_closed",
            ActionKind::BlockList => "block.list",
            ActionKind::BlockInspect => "block.inspect",
            ActionKind::BlockOutput => "block.output",
            ActionKind::InputGet => "input.get",
            ActionKind::InputInsert => "input.insert",
            ActionKind::InputReplace => "input.replace",
            ActionKind::InputClear => "input.clear",
            ActionKind::InputModeSet => "input.mode.set",
            ActionKind::InputRun => "input.run",
            ActionKind::HistoryList => "history.list",
            ActionKind::ThemeList => "theme.list",
            ActionKind::ThemeGet => "theme.get",
            ActionKind::ThemeSet => "theme.set",
            ActionKind::ThemeSystemSet => "theme.system.set",
            ActionKind::ThemeLightSet => "theme.light.set",
            ActionKind::ThemeDarkSet => "theme.dark.set",
            ActionKind::AppearanceGet => "appearance.get",
            ActionKind::AppearanceFontSizeIncrease => "appearance.font_size.increase",
            ActionKind::AppearanceFontSizeDecrease => "appearance.font_size.decrease",
            ActionKind::AppearanceFontSizeReset => "appearance.font_size.reset",
            ActionKind::AppearanceZoomIncrease => "appearance.zoom.increase",
            ActionKind::AppearanceZoomDecrease => "appearance.zoom.decrease",
            ActionKind::AppearanceZoomReset => "appearance.zoom.reset",
            ActionKind::SettingList => "setting.list",
            ActionKind::SettingGet => "setting.get",
            ActionKind::SettingSet => "setting.set",
            ActionKind::SettingToggle => "setting.toggle",
            ActionKind::KeybindingList => "keybinding.list",
            ActionKind::KeybindingGet => "keybinding.get",
            ActionKind::ActionList => "action.list",
            ActionKind::ActionInspect => "action.inspect",
            ActionKind::SurfaceSettingsOpen => "surface.settings.open",
            ActionKind::SurfaceCommandPaletteOpen => "surface.command_palette.open",
            ActionKind::SurfaceCommandSearchOpen => "surface.command_search.open",
            ActionKind::SurfaceWarpDriveOpen => "surface.warp_drive.open",
            ActionKind::SurfaceWarpDriveToggle => "surface.warp_drive.toggle",
            ActionKind::SurfaceResourceCenterToggle => "surface.resource_center.toggle",
            ActionKind::SurfaceAiAssistantToggle => "surface.ai_assistant.toggle",
            ActionKind::SurfaceCodeReviewToggle => "surface.code_review.toggle",
            ActionKind::SurfaceLeftPanelToggle => "surface.left_panel.toggle",
            ActionKind::SurfaceRightPanelToggle => "surface.right_panel.toggle",
            ActionKind::SurfaceVerticalTabsToggle => "surface.vertical_tabs.toggle",
            ActionKind::FileList => "file.list",
            ActionKind::FileOpen => "file.open",
            ActionKind::ProjectActive => "project.active",
            ActionKind::ProjectList => "project.list",
            ActionKind::ProjectOpen => "project.open",
            ActionKind::DriveList => "drive.list",
            ActionKind::DriveInspect => "drive.inspect",
            ActionKind::DriveOpen => "drive.open",
            ActionKind::DriveNotebookOpen => "drive.notebook.open",
            ActionKind::DriveEnvVarCollectionOpen => "drive.env_var_collection.open",
            ActionKind::DriveObjectShareOpen => "drive.object.share.open",
            ActionKind::DriveObjectCreate => "drive.object.create",
            ActionKind::DriveObjectUpdate => "drive.object.update",
            ActionKind::DriveObjectDelete => "drive.object.delete",
            ActionKind::DriveObjectInsert => "drive.object.insert",
            ActionKind::DriveObjectShareToTeam => "drive.object.share_to_team",
            ActionKind::DriveWorkflowRun => "drive.workflow.run",
        }
    }
}
