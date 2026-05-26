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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum InvocationContextSpec {
    InsideWarpOnly,
    OutsideWarpOnly,
    Any,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ActionSpec {
    name: &'static str,
    implementation_status: ActionImplementationStatus,
    requires_authenticated_user: bool,
    invocation_contexts: InvocationContextSpec,
    state_data_category: StateDataCategory,
    target_scope: TargetScope,
    parameter_spec: ActionParameterSpec,
    result_spec: ActionResultSpec,
}

macro_rules! define_action_catalog {
    ($(
        $group:ident {
            $(
                $variant:ident => {
                    name: $name:literal,
                    status: $status:ident,
                    authenticated_user: $authenticated_user:literal,
                    contexts: $contexts:ident,
                    state: $state:ident,
                    target: $target:ident,
                    params: $params:ident,
                    result: $result:ident $(,)?
                }
            ),+ $(,)?
        }
    )+ $(,)?) => {
        /// Stable protocol name for every approved `warpctrl` action.
        #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum ActionKind {
            $(
                $(
                    #[serde(rename = $name)]
                    $variant,
                )+
            )+
        }

        impl ActionKind {
            pub const ALL: &[Self] = &[
                $(
                    $(Self::$variant,)+
                )+
            ];

            pub fn as_str(self) -> &'static str {
                self.spec().name
            }

            pub fn metadata(self) -> ActionMetadata {
                let spec = self.spec();
                ActionMetadata {
                    kind: self,
                    name: spec.name.to_owned(),
                    implementation_status: spec.implementation_status,
                    risk_tier: self.default_risk_tier(),
                    state_data_category: spec.state_data_category,
                    requires_authenticated_user: spec.requires_authenticated_user,
                    authenticated_user: AuthenticatedUserRequirement {
                        required: spec.requires_authenticated_user,
                    },
                    allowed_invocation_contexts: self.allowed_invocation_contexts(),
                    permission_category: self.default_permission_category(),
                    target_scope: spec.target_scope,
                    parameter_spec: spec.parameter_spec,
                    result_spec: spec.result_spec,
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
                self.spec().implementation_status == ActionImplementationStatus::Implemented
            }

            fn spec(self) -> ActionSpec {
                match self {
                    $(
                        $(Self::$variant => ActionSpec {
                            name: $name,
                            implementation_status: ActionImplementationStatus::$status,
                            requires_authenticated_user: $authenticated_user,
                            invocation_contexts: InvocationContextSpec::$contexts,
                            state_data_category: StateDataCategory::$state,
                            target_scope: TargetScope::$target,
                            parameter_spec: ActionParameterSpec::$params,
                            result_spec: ActionResultSpec::$result,
                        },)+
                    )+
                }
            }

            fn allowed_invocation_contexts(self) -> Vec<InvocationContext> {
                match self.spec().invocation_contexts {
                    InvocationContextSpec::InsideWarpOnly => vec![InvocationContext::InsideWarp],
                    InvocationContextSpec::OutsideWarpOnly => vec![InvocationContext::OutsideWarp],
                    InvocationContextSpec::Any => vec![
                        InvocationContext::InsideWarp,
                        InvocationContext::OutsideWarp,
                    ],
                }
            }

            fn default_risk_tier(self) -> RiskTier {
                match self.spec().state_data_category {
                    StateDataCategory::MetadataRead => RiskTier::ReadOnlyMetadata,
                    StateDataCategory::UnderlyingDataRead => RiskTier::ReadOnlyTerminalData,
                    StateDataCategory::UnderlyingDataMutation => RiskTier::MutatingDestructiveOrExecution,
                    StateDataCategory::AppStateMutation
                    | StateDataCategory::MetadataConfigurationMutation => RiskTier::MutatingNonDestructive,
                }
            }

            fn default_permission_category(self) -> PermissionCategory {
                match self.spec().state_data_category {
                    StateDataCategory::MetadataRead => PermissionCategory::ReadMetadata,
                    StateDataCategory::UnderlyingDataRead => PermissionCategory::ReadUnderlyingData,
                    StateDataCategory::AppStateMutation => PermissionCategory::MutateAppState,
                    StateDataCategory::MetadataConfigurationMutation => {
                        PermissionCategory::MutateMetadataConfiguration
                    }
                    StateDataCategory::UnderlyingDataMutation => PermissionCategory::MutateUnderlyingData,
                }
            }
        }
    };
}

define_action_catalog! {
    instance {
        InstanceList => { name: "instance.list", status: Implemented, authenticated_user: false, contexts: OutsideWarpOnly, state: MetadataRead, target: Instance, params: None, result: InstanceList },
        InstanceInspect => { name: "instance.inspect", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Instance, params: None, result: InstanceMetadata },
    }

    app {
        AppPing => { name: "app.ping", status: Implemented, authenticated_user: false, contexts: OutsideWarpOnly, state: MetadataRead, target: Instance, params: None, result: InstanceMetadata },
        AppVersion => { name: "app.version", status: Implemented, authenticated_user: false, contexts: OutsideWarpOnly, state: MetadataRead, target: Instance, params: None, result: InstanceMetadata },
        AppActive => { name: "app.active", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Instance, params: None, result: ActiveTarget },
        AppFocus => { name: "app.focus", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Instance, params: None, result: Acknowledgement },
    }

    auth {
        AuthStatus => { name: "auth.status", status: Stub, authenticated_user: false, contexts: Any, state: MetadataRead, target: Auth, params: None, result: AuthStatus },
        AuthLogin => { name: "auth.login", status: Stub, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Auth, params: None, result: Acknowledgement },
    }

    capability {
        CapabilityList => { name: "capability.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Capability, params: None, result: CapabilityList },
        CapabilityInspect => { name: "capability.inspect", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Capability, params: ActionName, result: CapabilityMetadata },
    }

    window {
        WindowList => { name: "window.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Window, params: None, result: TargetList },
        WindowInspect => { name: "window.inspect", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Window, params: None, result: TargetMetadata },
        WindowCreate => { name: "window.create", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Window, params: TabCreate, result: Acknowledgement },
        WindowFocus => { name: "window.focus", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Window, params: None, result: Acknowledgement },
        WindowClose => { name: "window.close", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Window, params: None, result: Acknowledgement },
    }

    tab {
        TabList => { name: "tab.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Tab, params: None, result: TargetList },
        TabInspect => { name: "tab.inspect", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Tab, params: None, result: TargetMetadata },
        TabCreate => { name: "tab.create", status: Implemented, authenticated_user: false, contexts: OutsideWarpOnly, state: AppStateMutation, target: Tab, params: TabCreate, result: Acknowledgement },
        TabActivate => { name: "tab.activate", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Tab, params: TabActivate, result: Acknowledgement },
        TabMove => { name: "tab.move", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Tab, params: Direction, result: Acknowledgement },
        TabClose => { name: "tab.close", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Tab, params: TabClose, result: Acknowledgement },
        TabRename => { name: "tab.rename", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Tab, params: Rename, result: Acknowledgement },
        TabResetName => { name: "tab.reset_name", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Tab, params: None, result: Acknowledgement },
        TabColorSet => { name: "tab.color.set", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Tab, params: ColorValue, result: Acknowledgement },
        TabColorClear => { name: "tab.color.clear", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Tab, params: None, result: Acknowledgement },
    }

    pane {
        PaneList => { name: "pane.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Pane, params: None, result: TargetList },
        PaneInspect => { name: "pane.inspect", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Pane, params: None, result: TargetMetadata },
        PaneSplit => { name: "pane.split", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Pane, params: Direction, result: Acknowledgement },
        PaneFocus => { name: "pane.focus", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Pane, params: None, result: Acknowledgement },
        PaneNavigate => { name: "pane.navigate", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Pane, params: Direction, result: Acknowledgement },
        PaneResize => { name: "pane.resize", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Pane, params: Resize, result: Acknowledgement },
        PaneMaximize => { name: "pane.maximize", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Pane, params: None, result: Acknowledgement },
        PaneUnmaximize => { name: "pane.unmaximize", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Pane, params: None, result: Acknowledgement },
        PaneClose => { name: "pane.close", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Pane, params: None, result: Acknowledgement },
        PaneRename => { name: "pane.rename", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Pane, params: Rename, result: Acknowledgement },
        PaneResetName => { name: "pane.reset_name", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Pane, params: None, result: Acknowledgement },
    }

    session {
        SessionList => { name: "session.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Session, params: None, result: TargetList },
        SessionInspect => { name: "session.inspect", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Session, params: None, result: TargetMetadata },
        SessionActivate => { name: "session.activate", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Session, params: None, result: Acknowledgement },
        SessionPrevious => { name: "session.previous", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Session, params: None, result: Acknowledgement },
        SessionNext => { name: "session.next", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Session, params: None, result: Acknowledgement },
        SessionReopenClosed => { name: "session.reopen_closed", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Session, params: None, result: Acknowledgement },
    }

    block {
        BlockList => { name: "block.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Block, params: Limit, result: TargetList },
        BlockInspect => { name: "block.inspect", status: Implemented, authenticated_user: false, contexts: Any, state: UnderlyingDataRead, target: Block, params: BlockId, result: Content },
        BlockOutput => { name: "block.output", status: Implemented, authenticated_user: false, contexts: Any, state: UnderlyingDataRead, target: Block, params: BlockId, result: Content },
    }

    input {
        InputGet => { name: "input.get", status: Implemented, authenticated_user: false, contexts: Any, state: UnderlyingDataRead, target: Input, params: None, result: Content },
        InputInsert => { name: "input.insert", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Input, params: Text, result: Acknowledgement },
        InputReplace => { name: "input.replace", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Input, params: Text, result: Acknowledgement },
        InputClear => { name: "input.clear", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Input, params: None, result: Acknowledgement },
        InputModeSet => { name: "input.mode.set", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Input, params: InputMode, result: Acknowledgement },
        InputRun => { name: "input.run", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, state: UnderlyingDataMutation, target: Input, params: Text, result: Acknowledgement },
    }

    history {
        HistoryList => { name: "history.list", status: Implemented, authenticated_user: false, contexts: Any, state: UnderlyingDataRead, target: History, params: Limit, result: Content },
    }

    theme {
        ThemeList => { name: "theme.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Appearance, params: None, result: ThemeList },
        ThemeGet => { name: "theme.get", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Appearance, params: None, result: ThemeState },
        ThemeSet => { name: "theme.set", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: ThemeName, result: Acknowledgement },
        ThemeSystemSet => { name: "theme.system.set", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: BooleanValue, result: Acknowledgement },
        ThemeLightSet => { name: "theme.light.set", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: ThemeName, result: Acknowledgement },
        ThemeDarkSet => { name: "theme.dark.set", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: ThemeName, result: Acknowledgement },
    }

    appearance {
        AppearanceGet => { name: "appearance.get", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Appearance, params: None, result: AppearanceState },
        AppearanceFontSizeIncrease => { name: "appearance.font_size.increase", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: None, result: Acknowledgement },
        AppearanceFontSizeDecrease => { name: "appearance.font_size.decrease", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: None, result: Acknowledgement },
        AppearanceFontSizeReset => { name: "appearance.font_size.reset", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: None, result: Acknowledgement },
        AppearanceZoomIncrease => { name: "appearance.zoom.increase", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: None, result: Acknowledgement },
        AppearanceZoomDecrease => { name: "appearance.zoom.decrease", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: None, result: Acknowledgement },
        AppearanceZoomReset => { name: "appearance.zoom.reset", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Appearance, params: None, result: Acknowledgement },
    }

    setting {
        SettingList => { name: "setting.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Settings, params: None, result: SettingList },
        SettingGet => { name: "setting.get", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Settings, params: Key, result: SettingValue },
        SettingSet => { name: "setting.set", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Settings, params: KeyValue, result: Acknowledgement },
        SettingToggle => { name: "setting.toggle", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataConfigurationMutation, target: Settings, params: Key, result: Acknowledgement },
    }

    keybinding {
        KeybindingList => { name: "keybinding.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Keybinding, params: None, result: KeybindingList },
        KeybindingGet => { name: "keybinding.get", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Keybinding, params: BindingName, result: KeybindingMetadata },
    }

    action {
        ActionList => { name: "action.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Action, params: None, result: CapabilityList },
        ActionInspect => { name: "action.inspect", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: Action, params: ActionName, result: CapabilityMetadata },
    }

    surface {
        SurfaceSettingsOpen => { name: "surface.settings.open", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: PageQuery, result: Acknowledgement },
        SurfaceCommandPaletteOpen => { name: "surface.command_palette.open", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: Query, result: Acknowledgement },
        SurfaceCommandSearchOpen => { name: "surface.command_search.open", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: Query, result: Acknowledgement },
        SurfaceWarpDriveOpen => { name: "surface.warp_drive.open", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: None, result: Acknowledgement },
        SurfaceWarpDriveToggle => { name: "surface.warp_drive.toggle", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: None, result: Acknowledgement },
        SurfaceResourceCenterToggle => { name: "surface.resource_center.toggle", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: None, result: Acknowledgement },
        SurfaceAiAssistantToggle => { name: "surface.ai_assistant.toggle", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: None, result: Acknowledgement },
        SurfaceCodeReviewToggle => { name: "surface.code_review.toggle", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: None, result: Acknowledgement },
        SurfaceLeftPanelToggle => { name: "surface.left_panel.toggle", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: None, result: Acknowledgement },
        SurfaceRightPanelToggle => { name: "surface.right_panel.toggle", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: None, result: Acknowledgement },
        SurfaceVerticalTabsToggle => { name: "surface.vertical_tabs.toggle", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: Surface, params: None, result: Acknowledgement },
    }

    file {
        FileList => { name: "file.list", status: Implemented, authenticated_user: false, contexts: Any, state: MetadataRead, target: File, params: None, result: FileList },
        FileOpen => { name: "file.open", status: Implemented, authenticated_user: false, contexts: Any, state: AppStateMutation, target: File, params: FileOpen, result: Acknowledgement },
    }

    drive {
        DriveList => { name: "drive.list", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: MetadataRead, target: DriveObject, params: DriveObjectList, result: DriveObjectList },
        DriveInspect => { name: "drive.inspect", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: UnderlyingDataRead, target: DriveObject, params: DriveObjectId, result: DriveObjectMetadata },
        DriveOpen => { name: "drive.open", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: AppStateMutation, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveNotebookOpen => { name: "drive.notebook.open", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: AppStateMutation, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveEnvVarCollectionOpen => { name: "drive.env_var_collection.open", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: AppStateMutation, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveObjectShareOpen => { name: "drive.object.share.open", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: AppStateMutation, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveObjectCreate => { name: "drive.object.create", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: UnderlyingDataMutation, target: DriveObject, params: DriveObjectCreate, result: Acknowledgement },
        DriveObjectUpdate => { name: "drive.object.update", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: UnderlyingDataMutation, target: DriveObject, params: DriveObjectUpdate, result: Acknowledgement },
        DriveObjectDelete => { name: "drive.object.delete", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: UnderlyingDataMutation, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveObjectInsert => { name: "drive.object.insert", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: UnderlyingDataMutation, target: DriveObject, params: DriveObjectInsert, result: Acknowledgement },
        DriveObjectShareToTeam => { name: "drive.object.share_to_team", status: Implemented, authenticated_user: true, contexts: InsideWarpOnly, state: UnderlyingDataMutation, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveWorkflowRun => { name: "drive.workflow.run", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, state: UnderlyingDataMutation, target: DriveObject, params: WorkflowRun, result: Acknowledgement },
    }
}
