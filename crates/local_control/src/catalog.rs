//! Action catalog and metadata used for discovery, permissions, and CLI support.
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

/// Runtime context from which a control request was initiated.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationContext {
    InsideWarp,
    OutsideWarp,
}

/// Future proof shape for distinguishing verified Warp terminals from external clients.
///
/// `VerifiedWarpTerminal` is currently a protocol placeholder only. The
/// foundation implementation rejects inside-Warp credential requests until the
/// app-issued terminal-session proof broker is implemented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionContextProof {
    VerifiedWarpTerminal { proof_id: String },
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

/// Level of Warp hierarchy an action targets.
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
    Action,
    File,
    Project,
    Drive,
}

/// Whether an action has an app-side implementation in this stack layer.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionImplementationStatus {
    Implemented,
    Stub,
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
    pub requires_authenticated_scripting: bool,
    pub allowed_invocation_contexts: Vec<InvocationContext>,
    pub permission_category: PermissionCategory,
    pub target_scope: TargetScope,
}

/// Stable protocol name for every planned `warpctrl` action.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionKind {
    #[serde(rename = "instance.list")]
    InstanceList,
    #[serde(rename = "app.ping")]
    AppPing,
    #[serde(rename = "app.inspect")]
    AppInspect,
    #[serde(rename = "app.version")]
    AppVersion,
    #[serde(rename = "app.active")]
    AppActive,
    #[serde(rename = "action.list")]
    ActionList,
    #[serde(rename = "action.get")]
    ActionGet,
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
    #[serde(rename = "pane.session.reopen")]
    PaneSessionReopen,
    #[serde(rename = "session.list")]
    SessionList,
    #[serde(rename = "block.list")]
    BlockList,
    #[serde(rename = "block.get")]
    BlockGet,
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
    #[serde(rename = "history.list")]
    HistoryList,
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
    #[serde(rename = "input.run")]
    InputRun,
    #[serde(rename = "file.list")]
    FileList,
    #[serde(rename = "file.open")]
    FileOpen,
    #[serde(rename = "file.write")]
    FileWrite,
    #[serde(rename = "file.delete")]
    FileDelete,
    #[serde(rename = "project.active")]
    ProjectActive,
    #[serde(rename = "project.list")]
    ProjectList,
    #[serde(rename = "drive.list")]
    DriveList,
    #[serde(rename = "drive.get")]
    DriveGet,
    #[serde(rename = "drive.create")]
    DriveCreate,
    #[serde(rename = "drive.update")]
    DriveUpdate,
    #[serde(rename = "drive.delete")]
    DriveDelete,
    #[serde(rename = "drive.run")]
    DriveRun,
    #[serde(rename = "drive.insert")]
    DriveInsert,
}

impl ActionKind {
    pub const ALL: &[Self] = &[
        Self::InstanceList,
        Self::AppPing,
        Self::AppInspect,
        Self::AppVersion,
        Self::AppActive,
        Self::ActionList,
        Self::ActionGet,
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
        Self::PaneSessionReopen,
        Self::SessionList,
        Self::BlockList,
        Self::BlockGet,
        Self::InputGet,
        Self::InputInsert,
        Self::InputReplace,
        Self::InputClear,
        Self::InputModeSet,
        Self::InputRun,
        Self::HistoryList,
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
        Self::FileList,
        Self::FileOpen,
        Self::FileWrite,
        Self::FileDelete,
        Self::ProjectActive,
        Self::ProjectList,
        Self::DriveList,
        Self::DriveGet,
        Self::DriveCreate,
        Self::DriveUpdate,
        Self::DriveDelete,
        Self::DriveRun,
        Self::DriveInsert,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InstanceList => "instance.list",
            Self::AppPing => "app.ping",
            Self::AppInspect => "app.inspect",
            Self::AppVersion => "app.version",
            Self::AppActive => "app.active",
            Self::ActionList => "action.list",
            Self::ActionGet => "action.get",
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
            Self::PaneSessionReopen => "pane.session.reopen",
            Self::SessionList => "session.list",
            Self::BlockList => "block.list",
            Self::BlockGet => "block.get",
            Self::InputGet => "input.get",
            Self::InputInsert => "input.insert",
            Self::InputReplace => "input.replace",
            Self::InputClear => "input.clear",
            Self::InputModeSet => "input.mode.set",
            Self::InputRun => "input.run",
            Self::HistoryList => "history.list",
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
            Self::FileList => "file.list",
            Self::FileOpen => "file.open",
            Self::FileWrite => "file.write",
            Self::FileDelete => "file.delete",
            Self::ProjectActive => "project.active",
            Self::ProjectList => "project.list",
            Self::DriveList => "drive.list",
            Self::DriveGet => "drive.get",
            Self::DriveCreate => "drive.create",
            Self::DriveUpdate => "drive.update",
            Self::DriveDelete => "drive.delete",
            Self::DriveRun => "drive.run",
            Self::DriveInsert => "drive.insert",
        }
    }

    pub fn metadata(self) -> ActionMetadata {
        let implementation_status = match self {
            Self::InstanceList
            | Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::ActionList
            | Self::ActionGet
            | Self::WindowList
            | Self::TabList
            | Self::TabCreate
            | Self::PaneList
            | Self::SessionList
            | Self::BlockList
            | Self::BlockGet
            | Self::InputGet
            | Self::HistoryList
            | Self::ThemeList
            | Self::AppearanceGet
            | Self::SettingGet
            | Self::SettingList
            | Self::AppFocus
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::WindowClose
            | Self::TabActivate
            | Self::TabMove
            | Self::TabClose
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneClose
            | Self::PaneMaximize
            | Self::PaneResize
            | Self::PaneSessionPrevious
            | Self::PaneSessionNext
            | Self::PaneSessionReopen
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::ThemeSet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom
            | Self::SettingSet
            | Self::SettingToggle
            | Self::AppSettingsOpen
            | Self::AppCommandPaletteOpen
            | Self::AppCommandSearchOpen
            | Self::AppWarpDriveOpen
            | Self::AppWarpDriveToggle
            | Self::AppResourceCenterToggle
            | Self::AppAiAssistantToggle
            | Self::AppCodeReviewToggle
            | Self::AppVerticalTabsToggle
            | Self::FileWrite
            | Self::FileDelete
            | Self::DriveCreate
            | Self::DriveUpdate
            | Self::DriveDelete
            | Self::DriveRun
            | Self::DriveInsert => ActionImplementationStatus::Implemented,
            _ => ActionImplementationStatus::Stub,
        };
        let requires_authenticated_user = self.default_requires_authenticated_user();
        let requires_authenticated_scripting = self.default_requires_authenticated_scripting();
        let allowed_invocation_contexts = self.default_allowed_invocation_contexts();
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
            requires_authenticated_scripting,
            allowed_invocation_contexts,
            permission_category: self.default_permission_category(),
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
            Self::InstanceList
            | Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::ActionList
            | Self::ActionGet
            | Self::WindowList
            | Self::TabList
            | Self::PaneList
            | Self::SessionList
            | Self::ThemeList
            | Self::AppearanceGet
            | Self::SettingGet
            | Self::SettingList
            | Self::FileList
            | Self::ProjectActive
            | Self::ProjectList
            | Self::DriveList => RiskTier::ReadOnlyMetadata,
            Self::BlockList
            | Self::BlockGet
            | Self::InputGet
            | Self::HistoryList
            | Self::DriveGet => RiskTier::ReadOnlyTerminalData,
            Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun
            | Self::FileWrite
            | Self::FileDelete
            | Self::DriveCreate
            | Self::DriveUpdate
            | Self::DriveDelete
            | Self::DriveRun
            | Self::DriveInsert
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
            | Self::PaneSessionReopen
            | Self::ThemeSet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom
            | Self::SettingSet
            | Self::SettingToggle
            | Self::FileOpen => RiskTier::MutatingNonDestructive,
        }
    }

    fn default_state_data_category(self) -> StateDataCategory {
        match self {
            Self::InstanceList
            | Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::ActionList
            | Self::ActionGet
            | Self::WindowList
            | Self::TabList
            | Self::PaneList
            | Self::SessionList
            | Self::ThemeList
            | Self::AppearanceGet
            | Self::SettingGet
            | Self::SettingList
            | Self::FileList
            | Self::ProjectActive
            | Self::ProjectList
            | Self::DriveList => StateDataCategory::MetadataRead,
            Self::BlockList
            | Self::BlockGet
            | Self::InputGet
            | Self::HistoryList
            | Self::DriveGet => StateDataCategory::UnderlyingDataRead,
            Self::SettingSet
            | Self::SettingToggle
            | Self::ThemeSet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom
            | Self::TabRename => StateDataCategory::MetadataConfigurationMutation,
            Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun
            | Self::FileWrite
            | Self::FileDelete
            | Self::DriveCreate
            | Self::DriveUpdate
            | Self::DriveDelete
            | Self::DriveRun
            | Self::DriveInsert => StateDataCategory::UnderlyingDataMutation,
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
            | Self::WindowClose
            | Self::TabCreate
            | Self::TabActivate
            | Self::TabMove
            | Self::TabClose
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneClose
            | Self::PaneMaximize
            | Self::PaneResize
            | Self::PaneSessionPrevious
            | Self::PaneSessionNext
            | Self::PaneSessionReopen
            | Self::FileOpen => StateDataCategory::AppStateMutation,
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

    fn default_requires_authenticated_user(self) -> bool {
        matches!(
            self,
            Self::BlockList
                | Self::BlockGet
                | Self::InputGet
                | Self::HistoryList
                | Self::DriveList
                | Self::DriveGet
                | Self::AppFocus
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
                | Self::WindowClose
                | Self::TabActivate
                | Self::TabMove
                | Self::TabRename
                | Self::TabClose
                | Self::PaneSplit
                | Self::PaneFocus
                | Self::PaneNavigate
                | Self::PaneClose
                | Self::PaneMaximize
                | Self::PaneResize
                | Self::PaneSessionPrevious
                | Self::PaneSessionNext
                | Self::PaneSessionReopen
                | Self::InputInsert
                | Self::InputReplace
                | Self::InputClear
                | Self::InputModeSet
                | Self::InputRun
                | Self::ThemeSet
                | Self::AppearanceSet
                | Self::AppearanceFontSize
                | Self::AppearanceZoom
                | Self::SettingSet
                | Self::SettingToggle
                | Self::FileOpen
                | Self::FileWrite
                | Self::FileDelete
                | Self::DriveCreate
                | Self::DriveUpdate
                | Self::DriveDelete
                | Self::DriveRun
                | Self::DriveInsert
        )
    }

    fn default_allowed_invocation_contexts(self) -> Vec<InvocationContext> {
        let metadata_status = match self {
            Self::InstanceList
            | Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::ActionList
            | Self::ActionGet
            | Self::WindowList
            | Self::TabList
            | Self::TabCreate
            | Self::PaneList
            | Self::SessionList
            | Self::BlockList
            | Self::BlockGet
            | Self::InputGet
            | Self::HistoryList
            | Self::ThemeList
            | Self::AppearanceGet
            | Self::SettingGet
            | Self::SettingList
            | Self::AppFocus
            | Self::WindowCreate
            | Self::WindowFocus
            | Self::WindowClose
            | Self::TabActivate
            | Self::TabMove
            | Self::TabClose
            | Self::PaneSplit
            | Self::PaneFocus
            | Self::PaneNavigate
            | Self::PaneClose
            | Self::PaneMaximize
            | Self::PaneResize
            | Self::ThemeSet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom
            | Self::SettingSet
            | Self::SettingToggle
            | Self::AppSettingsOpen
            | Self::AppCommandPaletteOpen
            | Self::AppCommandSearchOpen
            | Self::AppWarpDriveOpen
            | Self::AppWarpDriveToggle
            | Self::AppResourceCenterToggle
            | Self::AppAiAssistantToggle
            | Self::AppCodeReviewToggle
            | Self::AppVerticalTabsToggle => {
                return vec![InvocationContext::OutsideWarp];
            }
            Self::PaneSessionPrevious
            | Self::PaneSessionNext
            | Self::PaneSessionReopen
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet => {
                return vec![InvocationContext::InsideWarp];
            }
            Self::FileWrite
            | Self::FileDelete
            | Self::DriveCreate
            | Self::DriveUpdate
            | Self::DriveDelete
            | Self::DriveRun
            | Self::DriveInsert => {
                return vec![InvocationContext::OutsideWarp];
            }
            _ => ActionImplementationStatus::Stub,
        };
        if metadata_status == ActionImplementationStatus::Implemented {
            vec![InvocationContext::OutsideWarp]
        } else {
            Vec::new()
        }
    }

    fn default_requires_authenticated_scripting(self) -> bool {
        match self.default_state_data_category() {
            StateDataCategory::UnderlyingDataMutation => true,
            StateDataCategory::MetadataRead
            | StateDataCategory::UnderlyingDataRead
            | StateDataCategory::AppStateMutation
            | StateDataCategory::MetadataConfigurationMutation => false,
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
            | Self::PaneSessionNext
            | Self::PaneSessionReopen => TargetScope::Pane,
            Self::SessionList
            | Self::InputGet
            | Self::InputInsert
            | Self::InputReplace
            | Self::InputClear
            | Self::InputModeSet
            | Self::InputRun => TargetScope::Session,
            Self::BlockList | Self::BlockGet => TargetScope::Block,
            Self::HistoryList => TargetScope::History,
            Self::ThemeList
            | Self::ThemeSet
            | Self::AppearanceGet
            | Self::AppearanceSet
            | Self::AppearanceFontSize
            | Self::AppearanceZoom => TargetScope::Appearance,
            Self::SettingGet | Self::SettingList | Self::SettingSet | Self::SettingToggle => {
                TargetScope::Settings
            }
            Self::ActionList | Self::ActionGet => TargetScope::Action,
            Self::FileList | Self::FileOpen | Self::FileWrite | Self::FileDelete => {
                TargetScope::File
            }
            Self::ProjectActive | Self::ProjectList => TargetScope::Project,
            Self::DriveList
            | Self::DriveGet
            | Self::DriveCreate
            | Self::DriveUpdate
            | Self::DriveDelete
            | Self::DriveRun
            | Self::DriveInsert => TargetScope::Drive,
            Self::AppSettingsOpen
            | Self::AppCommandPaletteOpen
            | Self::AppCommandSearchOpen
            | Self::AppWarpDriveOpen
            | Self::AppWarpDriveToggle
            | Self::AppResourceCenterToggle
            | Self::AppAiAssistantToggle
            | Self::AppCodeReviewToggle
            | Self::AppVerticalTabsToggle => TargetScope::Surface,
            Self::InstanceList
            | Self::AppPing
            | Self::AppInspect
            | Self::AppVersion
            | Self::AppActive
            | Self::AppFocus => TargetScope::Instance,
        }
    }
}
