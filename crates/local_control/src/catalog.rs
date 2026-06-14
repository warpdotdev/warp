//! Action catalog and metadata used for discovery, permissions, and CLI support.
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

/// Level of Warp hierarchy or orthogonal product noun an action targets.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetScope {
    Instance,
    Window,
    Tab,
    Pane,
    Session,
    Input,
    Settings,
    Appearance,
    Surface,
    File,
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
    FileOpen,
    Key,
    KeyValue,
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
}

/// Typed result contract for a catalog action.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionResultSpec {
    Acknowledgement,
    ActiveTarget,
    AppearanceState,
    CapabilityList,
    CapabilityMetadata,
    InstanceList,
    InstanceMetadata,
    KeybindingList,
    KeybindingMetadata,
    SettingList,
    SettingValue,
    SurfaceList,
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
    pub target_scope: TargetScope,
    pub parameter_spec: ActionParameterSpec,
    pub result_spec: ActionResultSpec,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ActionSpec {
    name: &'static str,
    implementation_status: ActionImplementationStatus,
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
            $($(#[serde(rename = $name)] $variant,)+)+
        }

        impl ActionKind {
            pub const ALL: &[Self] = &[$($(Self::$variant,)+)+];

            pub fn as_str(self) -> &'static str {
                self.spec().name
            }

            pub fn metadata(self) -> ActionMetadata {
                let spec = self.spec();
                ActionMetadata {
                    kind: self,
                    name: spec.name.to_owned(),
                    implementation_status: spec.implementation_status,
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
                    .filter(|metadata| metadata.implementation_status == ActionImplementationStatus::Implemented)
                    .collect()
            }

            pub fn is_implemented(self) -> bool {
                self.spec().implementation_status == ActionImplementationStatus::Implemented
            }

            fn spec(self) -> ActionSpec {
                match self {
                    $($(Self::$variant => ActionSpec {
                        name: $name,
                        implementation_status: ActionImplementationStatus::$status,
                        target_scope: TargetScope::$target,
                        parameter_spec: ActionParameterSpec::$params,
                        result_spec: ActionResultSpec::$result,
                    },)+)+
                }
            }
        }
    };
}

define_action_catalog! {
    instance {
        InstanceList => { name: "instance.list", status: Implemented, target: Instance, params: None, result: InstanceList },
        InstanceInspect => { name: "instance.inspect", status: Implemented, target: Instance, params: None, result: InstanceMetadata },
    }

    app {
        AppPing => { name: "app.ping", status: Implemented, target: Instance, params: None, result: InstanceMetadata },
        AppVersion => { name: "app.version", status: Implemented, target: Instance, params: None, result: InstanceMetadata },
        AppActive => { name: "app.active", status: Implemented, target: Instance, params: None, result: ActiveTarget },
        AppFocus => { name: "app.focus", status: Implemented, target: Instance, params: None, result: Acknowledgement },
    }

    capability {
        CapabilityList => { name: "capability.list", status: Implemented, target: Capability, params: None, result: CapabilityList },
        CapabilityInspect => { name: "capability.inspect", status: Implemented, target: Capability, params: ActionName, result: CapabilityMetadata },
    }

    window {
        WindowList => { name: "window.list", status: Implemented, target: Window, params: None, result: TargetList },
        WindowInspect => { name: "window.inspect", status: Implemented, target: Window, params: None, result: TargetMetadata },
        WindowCreate => { name: "window.create", status: Implemented, target: Window, params: TabCreate, result: Acknowledgement },
        WindowFocus => { name: "window.focus", status: Implemented, target: Window, params: None, result: Acknowledgement },
        WindowClose => { name: "window.close", status: Implemented, target: Window, params: None, result: Acknowledgement },
    }

    tab {
        TabList => { name: "tab.list", status: Implemented, target: Tab, params: None, result: TargetList },
        TabInspect => { name: "tab.inspect", status: Implemented, target: Tab, params: None, result: TargetMetadata },
        TabCreate => { name: "tab.create", status: Implemented, target: Tab, params: TabCreate, result: Acknowledgement },
        TabActivate => { name: "tab.activate", status: Implemented, target: Tab, params: TabActivate, result: Acknowledgement },
        TabMove => { name: "tab.move", status: Implemented, target: Tab, params: Direction, result: Acknowledgement },
        TabClose => { name: "tab.close", status: Implemented, target: Tab, params: TabClose, result: Acknowledgement },
        TabRename => { name: "tab.rename", status: Implemented, target: Tab, params: Rename, result: Acknowledgement },
        TabResetName => { name: "tab.reset_name", status: Implemented, target: Tab, params: None, result: Acknowledgement },
        TabColorSet => { name: "tab.color.set", status: Implemented, target: Tab, params: ColorValue, result: Acknowledgement },
        TabColorClear => { name: "tab.color.clear", status: Implemented, target: Tab, params: None, result: Acknowledgement },
    }

    pane {
        PaneList => { name: "pane.list", status: Implemented, target: Pane, params: None, result: TargetList },
        PaneInspect => { name: "pane.inspect", status: Implemented, target: Pane, params: None, result: TargetMetadata },
        PaneSplit => { name: "pane.split", status: Implemented, target: Pane, params: Direction, result: Acknowledgement },
        PaneFocus => { name: "pane.focus", status: Implemented, target: Pane, params: None, result: Acknowledgement },
        PaneNavigate => { name: "pane.navigate", status: Implemented, target: Pane, params: Direction, result: Acknowledgement },
        PaneResize => { name: "pane.resize", status: Implemented, target: Pane, params: Resize, result: Acknowledgement },
        PaneMaximize => { name: "pane.maximize", status: Implemented, target: Pane, params: None, result: Acknowledgement },
        PaneUnmaximize => { name: "pane.unmaximize", status: Implemented, target: Pane, params: None, result: Acknowledgement },
        PaneClose => { name: "pane.close", status: Implemented, target: Pane, params: None, result: Acknowledgement },
        PaneRename => { name: "pane.rename", status: Implemented, target: Pane, params: Rename, result: Acknowledgement },
        PaneResetName => { name: "pane.reset_name", status: Implemented, target: Pane, params: None, result: Acknowledgement },
    }

    session {
        SessionList => { name: "session.list", status: Implemented, target: Session, params: None, result: TargetList },
        SessionInspect => { name: "session.inspect", status: Implemented, target: Session, params: None, result: TargetMetadata },
        SessionActivate => { name: "session.activate", status: Implemented, target: Session, params: None, result: Acknowledgement },
        SessionPrevious => { name: "session.previous", status: Implemented, target: Session, params: None, result: Acknowledgement },
        SessionNext => { name: "session.next", status: Implemented, target: Session, params: None, result: Acknowledgement },
        SessionReopenClosed => { name: "session.reopen_closed", status: Implemented, target: Session, params: None, result: Acknowledgement },
    }

    input {
        InputInsert => { name: "input.insert", status: Implemented, target: Input, params: Text, result: Acknowledgement },
        InputReplace => { name: "input.replace", status: Implemented, target: Input, params: Text, result: Acknowledgement },
    }

    theme {
        ThemeList => { name: "theme.list", status: Implemented, target: Appearance, params: None, result: ThemeList },
        ThemeGet => { name: "theme.get", status: Implemented, target: Appearance, params: None, result: ThemeState },
        ThemeSet => { name: "theme.set", status: Implemented, target: Appearance, params: ThemeName, result: Acknowledgement },
        ThemeSystemSet => { name: "theme.system.set", status: Implemented, target: Appearance, params: BooleanValue, result: Acknowledgement },
        ThemeLightSet => { name: "theme.light.set", status: Implemented, target: Appearance, params: ThemeName, result: Acknowledgement },
        ThemeDarkSet => { name: "theme.dark.set", status: Implemented, target: Appearance, params: ThemeName, result: Acknowledgement },
    }

    appearance {
        AppearanceGet => { name: "appearance.get", status: Implemented, target: Appearance, params: None, result: AppearanceState },
        AppearanceFontSizeIncrease => { name: "appearance.font_size.increase", status: Implemented, target: Appearance, params: None, result: Acknowledgement },
        AppearanceFontSizeDecrease => { name: "appearance.font_size.decrease", status: Implemented, target: Appearance, params: None, result: Acknowledgement },
        AppearanceFontSizeReset => { name: "appearance.font_size.reset", status: Implemented, target: Appearance, params: None, result: Acknowledgement },
        AppearanceZoomIncrease => { name: "appearance.zoom.increase", status: Implemented, target: Appearance, params: None, result: Acknowledgement },
        AppearanceZoomDecrease => { name: "appearance.zoom.decrease", status: Implemented, target: Appearance, params: None, result: Acknowledgement },
        AppearanceZoomReset => { name: "appearance.zoom.reset", status: Implemented, target: Appearance, params: None, result: Acknowledgement },
    }

    setting {
        SettingList => { name: "setting.list", status: Implemented, target: Settings, params: Namespace, result: SettingList },
        SettingGet => { name: "setting.get", status: Implemented, target: Settings, params: Key, result: SettingValue },
        SettingSet => { name: "setting.set", status: Implemented, target: Settings, params: KeyValue, result: Acknowledgement },
        SettingToggle => { name: "setting.toggle", status: Implemented, target: Settings, params: Key, result: Acknowledgement },
    }

    keybinding {
        KeybindingList => { name: "keybinding.list", status: Implemented, target: Keybinding, params: None, result: KeybindingList },
        KeybindingGet => { name: "keybinding.get", status: Implemented, target: Keybinding, params: BindingName, result: KeybindingMetadata },
    }

    action {
        ActionList => { name: "action.list", status: Implemented, target: Action, params: None, result: CapabilityList },
        ActionInspect => { name: "action.inspect", status: Implemented, target: Action, params: ActionName, result: CapabilityMetadata },
    }

    surface {
        SurfaceList => { name: "surface.list", status: Implemented, target: Instance, params: None, result: SurfaceList },
        SurfaceSettingsOpen => { name: "surface.settings.open", status: Implemented, target: Surface, params: PageQuery, result: Acknowledgement },
        SurfaceCommandPaletteOpen => { name: "surface.command_palette.open", status: Implemented, target: Surface, params: Query, result: Acknowledgement },
        SurfaceCommandSearchOpen => { name: "surface.command_search.open", status: Implemented, target: Surface, params: Query, result: Acknowledgement },
        SurfaceThemePickerOpen => { name: "surface.theme_picker.open", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceKeybindingsOpen => { name: "surface.keybindings.open", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceWarpDriveOpen => { name: "surface.warp_drive.open", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceWarpDriveToggle => { name: "surface.warp_drive.toggle", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceResourceCenterToggle => { name: "surface.resource_center.toggle", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceAiAssistantToggle => { name: "surface.ai_assistant.toggle", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceCodeReviewOpen => { name: "surface.code_review.open", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceCodeReviewToggle => { name: "surface.code_review.toggle", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceProjectExplorerOpen => { name: "surface.project_explorer.open", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceGlobalSearchOpen => { name: "surface.global_search.open", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceConversationListOpen => { name: "surface.conversation_list.open", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceLeftPanelToggle => { name: "surface.left_panel.toggle", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceRightPanelToggle => { name: "surface.right_panel.toggle", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceVerticalTabsOpen => { name: "surface.vertical_tabs.open", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceVerticalTabsToggle => { name: "surface.vertical_tabs.toggle", status: Implemented, target: Surface, params: None, result: Acknowledgement },
        SurfaceAgentManagementOpen => { name: "surface.agent_management.open", status: Implemented, target: Surface, params: None, result: Acknowledgement },
    }

    file {
        FileOpen => { name: "file.open", status: Implemented, target: File, params: FileOpen, result: Acknowledgement },
    }
}
