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
    /// Stable public action identifier exposed through discovery, help, and wire
    /// payloads, such as `tab.create`.
    pub name: String,
    pub implementation_status: ActionImplementationStatus,
    pub requires_authenticated_user: bool,
    pub authenticated_user: AuthenticatedUserRequirement,
    pub allowed_invocation_contexts: Vec<InvocationContext>,
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
                    target: $target:ident,
                    params: $params:ident,
                    result: $result:ident $(,)?
                }
            ),+ $(,)?
        }
    )+ $(,)?) => {
        /// Stable protocol name for every approved `warpctrl` action.
        ///
        /// These names are user-visible as CLI/API action identifiers, so they
        /// should be treated as stable public contract strings.
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
                    requires_authenticated_user: spec.requires_authenticated_user,
                    authenticated_user: AuthenticatedUserRequirement {
                        required: spec.requires_authenticated_user,
                    },
                    allowed_invocation_contexts: self.allowed_invocation_contexts(),
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

        }
    };
}

define_action_catalog! {
    instance {
        InstanceList => { name: "instance.list", status: Implemented, authenticated_user: false, contexts: OutsideWarpOnly, target: Instance, params: None, result: InstanceList },
        InstanceInspect => { name: "instance.inspect", status: Stub, authenticated_user: false, contexts: Any, target: Instance, params: None, result: InstanceMetadata },
    }

    app {
        AppPing => { name: "app.ping", status: Implemented, authenticated_user: false, contexts: OutsideWarpOnly, target: Instance, params: None, result: InstanceMetadata },
        AppVersion => { name: "app.version", status: Implemented, authenticated_user: false, contexts: OutsideWarpOnly, target: Instance, params: None, result: InstanceMetadata },
        AppActive => { name: "app.active", status: Stub, authenticated_user: false, contexts: Any, target: Instance, params: None, result: ActiveTarget },
        AppFocus => { name: "app.focus", status: Stub, authenticated_user: false, contexts: Any, target: Instance, params: None, result: Acknowledgement },
    }

    auth {
        AuthStatus => { name: "auth.status", status: Stub, authenticated_user: false, contexts: Any, target: Auth, params: None, result: AuthStatus },
        AuthLogin => { name: "auth.login", status: Stub, authenticated_user: false, contexts: Any, target: Auth, params: None, result: Acknowledgement },
    }

    capability {
        CapabilityList => { name: "capability.list", status: Stub, authenticated_user: false, contexts: Any, target: Capability, params: None, result: CapabilityList },
        CapabilityInspect => { name: "capability.inspect", status: Stub, authenticated_user: false, contexts: Any, target: Capability, params: ActionName, result: CapabilityMetadata },
    }

    window {
        WindowList => { name: "window.list", status: Stub, authenticated_user: false, contexts: Any, target: Window, params: None, result: TargetList },
        WindowInspect => { name: "window.inspect", status: Stub, authenticated_user: false, contexts: Any, target: Window, params: None, result: TargetMetadata },
        WindowCreate => { name: "window.create", status: Stub, authenticated_user: false, contexts: Any, target: Window, params: TabCreate, result: Acknowledgement },
        WindowFocus => { name: "window.focus", status: Stub, authenticated_user: false, contexts: Any, target: Window, params: None, result: Acknowledgement },
        WindowClose => { name: "window.close", status: Stub, authenticated_user: false, contexts: Any, target: Window, params: None, result: Acknowledgement },
    }

    tab {
        TabList => { name: "tab.list", status: Stub, authenticated_user: false, contexts: Any, target: Tab, params: None, result: TargetList },
        TabInspect => { name: "tab.inspect", status: Stub, authenticated_user: false, contexts: Any, target: Tab, params: None, result: TargetMetadata },
        TabCreate => { name: "tab.create", status: Implemented, authenticated_user: false, contexts: OutsideWarpOnly, target: Tab, params: None, result: Acknowledgement },
        TabActivate => { name: "tab.activate", status: Stub, authenticated_user: false, contexts: Any, target: Tab, params: TabActivate, result: Acknowledgement },
        TabMove => { name: "tab.move", status: Stub, authenticated_user: false, contexts: Any, target: Tab, params: Direction, result: Acknowledgement },
        TabClose => { name: "tab.close", status: Stub, authenticated_user: false, contexts: Any, target: Tab, params: TabClose, result: Acknowledgement },
        TabRename => { name: "tab.rename", status: Stub, authenticated_user: false, contexts: Any, target: Tab, params: Rename, result: Acknowledgement },
        TabResetName => { name: "tab.reset_name", status: Stub, authenticated_user: false, contexts: Any, target: Tab, params: None, result: Acknowledgement },
        TabColorSet => { name: "tab.color.set", status: Stub, authenticated_user: false, contexts: Any, target: Tab, params: ColorValue, result: Acknowledgement },
        TabColorClear => { name: "tab.color.clear", status: Stub, authenticated_user: false, contexts: Any, target: Tab, params: None, result: Acknowledgement },
    }

    pane {
        PaneList => { name: "pane.list", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: None, result: TargetList },
        PaneInspect => { name: "pane.inspect", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: None, result: TargetMetadata },
        PaneSplit => { name: "pane.split", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: Direction, result: Acknowledgement },
        PaneFocus => { name: "pane.focus", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: None, result: Acknowledgement },
        PaneNavigate => { name: "pane.navigate", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: Direction, result: Acknowledgement },
        PaneResize => { name: "pane.resize", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: Resize, result: Acknowledgement },
        PaneMaximize => { name: "pane.maximize", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: None, result: Acknowledgement },
        PaneUnmaximize => { name: "pane.unmaximize", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: None, result: Acknowledgement },
        PaneClose => { name: "pane.close", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: None, result: Acknowledgement },
        PaneRename => { name: "pane.rename", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: Rename, result: Acknowledgement },
        PaneResetName => { name: "pane.reset_name", status: Stub, authenticated_user: false, contexts: Any, target: Pane, params: None, result: Acknowledgement },
    }

    session {
        SessionList => { name: "session.list", status: Stub, authenticated_user: false, contexts: Any, target: Session, params: None, result: TargetList },
        SessionInspect => { name: "session.inspect", status: Stub, authenticated_user: false, contexts: Any, target: Session, params: None, result: TargetMetadata },
        SessionActivate => { name: "session.activate", status: Stub, authenticated_user: false, contexts: Any, target: Session, params: None, result: Acknowledgement },
        SessionPrevious => { name: "session.previous", status: Stub, authenticated_user: false, contexts: Any, target: Session, params: None, result: Acknowledgement },
        SessionNext => { name: "session.next", status: Stub, authenticated_user: false, contexts: Any, target: Session, params: None, result: Acknowledgement },
        SessionReopenClosed => { name: "session.reopen_closed", status: Stub, authenticated_user: false, contexts: Any, target: Session, params: None, result: Acknowledgement },
    }

    block {
        BlockList => { name: "block.list", status: Stub, authenticated_user: false, contexts: Any, target: Block, params: Limit, result: TargetList },
        BlockInspect => { name: "block.inspect", status: Stub, authenticated_user: false, contexts: Any, target: Block, params: None, result: Content },
        BlockOutput => { name: "block.output", status: Stub, authenticated_user: false, contexts: Any, target: Block, params: None, result: Content },
    }

    input {
        InputGet => { name: "input.get", status: Stub, authenticated_user: false, contexts: Any, target: Input, params: None, result: Content },
        InputInsert => { name: "input.insert", status: Stub, authenticated_user: false, contexts: Any, target: Input, params: Text, result: Acknowledgement },
        InputReplace => { name: "input.replace", status: Stub, authenticated_user: false, contexts: Any, target: Input, params: Text, result: Acknowledgement },
        InputClear => { name: "input.clear", status: Stub, authenticated_user: false, contexts: Any, target: Input, params: None, result: Acknowledgement },
        InputModeSet => { name: "input.mode.set", status: Stub, authenticated_user: false, contexts: Any, target: Input, params: InputMode, result: Acknowledgement },
        InputRun => { name: "input.run", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: Input, params: Text, result: Acknowledgement },
    }

    history {
        HistoryList => { name: "history.list", status: Stub, authenticated_user: false, contexts: Any, target: History, params: Limit, result: Content },
    }

    theme {
        ThemeList => { name: "theme.list", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: None, result: ThemeList },
        ThemeGet => { name: "theme.get", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: None, result: ThemeState },
        ThemeSet => { name: "theme.set", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: ThemeName, result: Acknowledgement },
        ThemeSystemSet => { name: "theme.system.set", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: BooleanValue, result: Acknowledgement },
        ThemeLightSet => { name: "theme.light.set", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: ThemeName, result: Acknowledgement },
        ThemeDarkSet => { name: "theme.dark.set", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: ThemeName, result: Acknowledgement },
    }

    appearance {
        AppearanceGet => { name: "appearance.get", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: None, result: AppearanceState },
        AppearanceFontSizeIncrease => { name: "appearance.font_size.increase", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: None, result: Acknowledgement },
        AppearanceFontSizeDecrease => { name: "appearance.font_size.decrease", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: None, result: Acknowledgement },
        AppearanceFontSizeReset => { name: "appearance.font_size.reset", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: None, result: Acknowledgement },
        AppearanceZoomIncrease => { name: "appearance.zoom.increase", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: None, result: Acknowledgement },
        AppearanceZoomDecrease => { name: "appearance.zoom.decrease", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: None, result: Acknowledgement },
        AppearanceZoomReset => { name: "appearance.zoom.reset", status: Stub, authenticated_user: false, contexts: Any, target: Appearance, params: None, result: Acknowledgement },
    }

    setting {
        SettingList => { name: "setting.list", status: Stub, authenticated_user: false, contexts: Any, target: Settings, params: Namespace, result: SettingList },
        SettingGet => { name: "setting.get", status: Stub, authenticated_user: false, contexts: Any, target: Settings, params: Key, result: SettingValue },
        SettingSet => { name: "setting.set", status: Stub, authenticated_user: false, contexts: Any, target: Settings, params: KeyValue, result: Acknowledgement },
        SettingToggle => { name: "setting.toggle", status: Stub, authenticated_user: false, contexts: Any, target: Settings, params: Key, result: Acknowledgement },
    }

    keybinding {
        KeybindingList => { name: "keybinding.list", status: Stub, authenticated_user: false, contexts: Any, target: Keybinding, params: None, result: KeybindingList },
        KeybindingGet => { name: "keybinding.get", status: Stub, authenticated_user: false, contexts: Any, target: Keybinding, params: BindingName, result: KeybindingMetadata },
    }

    action {
        ActionList => { name: "action.list", status: Stub, authenticated_user: false, contexts: Any, target: Action, params: None, result: CapabilityList },
        ActionInspect => { name: "action.inspect", status: Stub, authenticated_user: false, contexts: Any, target: Action, params: ActionName, result: CapabilityMetadata },
    }

    surface {
        SurfaceSettingsOpen => { name: "surface.settings.open", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: PageQuery, result: Acknowledgement },
        SurfaceCommandPaletteOpen => { name: "surface.command_palette.open", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: Query, result: Acknowledgement },
        SurfaceCommandSearchOpen => { name: "surface.command_search.open", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: Query, result: Acknowledgement },
        SurfaceWarpDriveOpen => { name: "surface.warp_drive.open", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: None, result: Acknowledgement },
        SurfaceWarpDriveToggle => { name: "surface.warp_drive.toggle", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: None, result: Acknowledgement },
        SurfaceResourceCenterToggle => { name: "surface.resource_center.toggle", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: None, result: Acknowledgement },
        SurfaceAiAssistantToggle => { name: "surface.ai_assistant.toggle", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: None, result: Acknowledgement },
        SurfaceCodeReviewToggle => { name: "surface.code_review.toggle", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: None, result: Acknowledgement },
        SurfaceLeftPanelToggle => { name: "surface.left_panel.toggle", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: None, result: Acknowledgement },
        SurfaceRightPanelToggle => { name: "surface.right_panel.toggle", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: None, result: Acknowledgement },
        SurfaceVerticalTabsToggle => { name: "surface.vertical_tabs.toggle", status: Stub, authenticated_user: false, contexts: Any, target: Surface, params: None, result: Acknowledgement },
    }

    file {
        FileList => { name: "file.list", status: Stub, authenticated_user: false, contexts: Any, target: File, params: None, result: FileList },
        FileOpen => { name: "file.open", status: Stub, authenticated_user: false, contexts: Any, target: File, params: FileOpen, result: Acknowledgement },
    }

    drive {
        DriveList => { name: "drive.list", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectList, result: DriveObjectList },
        DriveInspect => { name: "drive.inspect", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectId, result: DriveObjectMetadata },
        DriveOpen => { name: "drive.open", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveNotebookOpen => { name: "drive.notebook.open", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveEnvVarCollectionOpen => { name: "drive.env_var_collection.open", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveObjectShareOpen => { name: "drive.object.share.open", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveObjectCreate => { name: "drive.object.create", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectCreate, result: Acknowledgement },
        DriveObjectUpdate => { name: "drive.object.update", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectUpdate, result: Acknowledgement },
        DriveObjectDelete => { name: "drive.object.delete", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveObjectInsert => { name: "drive.object.insert", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectInsert, result: Acknowledgement },
        DriveObjectShareToTeam => { name: "drive.object.share_to_team", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: DriveObjectId, result: Acknowledgement },
        DriveWorkflowRun => { name: "drive.workflow.run", status: Stub, authenticated_user: true, contexts: InsideWarpOnly, target: DriveObject, params: WorkflowRun, result: Acknowledgement },
    }
}
