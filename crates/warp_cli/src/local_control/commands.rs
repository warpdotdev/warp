//! Implementations for user-facing `warpctrl` command groups.
use local_control::discovery::InstanceRecord;
use local_control::protocol::{
    Action, ActionKind, ActionNameParams, BindingNameParams, BooleanValueParams, ColorValueParams,
    ControlError, DirectionParams, EmptyParams, ErrorCode, FileOpenParams, KeyParams,
    KeyValueParams, PageQueryParams, QueryParams, RenameParams, RequestEnvelope, ResizeParams,
    SettingListParams, TabActivateParams, TabActivationMode, TabCloseMode, TabCloseParams,
    TabCreateParams, TextParams, ThemeNameParams,
};
use local_control::selection::select_instance;
use serde::Serialize;
use warp_core::channel::ChannelState;

use crate::agent::OutputFormat;
use crate::local_control::output::{write_json, write_json_line};
use crate::local_control::selectors::{instance_selector, target_selector};
use crate::local_control::{
    ActionCatalogCommand, AppCommand, AppearanceCommand, CapabilityCommand, FileCommand,
    InputCommand, InstanceCommand, KeybindingCommand, PaneCommand, SessionCommand, SettingCommand,
    SurfaceCommand, SurfaceOpenCommand, SurfaceOpenToggleCommand, SurfaceQueryCommand,
    SurfaceSettingsCommand, SurfaceToggleCommand, TabActivateArgs, TabCloseArgs, TabColorCommand,
    TabCommand, TargetArgs, ThemeCommand, WindowCommand,
};

pub(super) fn run_surface_command(
    command: SurfaceCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SurfaceCommand::List(args) => {
            run_action_with_params(args, ActionKind::SurfaceList, EmptyParams {}, output_format)
        }
        SurfaceCommand::Settings(command) => match command {
            SurfaceSettingsCommand::Open(args) => run_action_with_params(
                args.target,
                ActionKind::SurfaceSettingsOpen,
                PageQueryParams {
                    page: args.page,
                    query: args.query,
                },
                output_format,
            ),
        },
        SurfaceCommand::CommandPalette(command) => run_surface_query_command(
            command,
            ActionKind::SurfaceCommandPaletteOpen,
            output_format,
        ),
        SurfaceCommand::CommandSearch(command) => {
            run_surface_query_command(command, ActionKind::SurfaceCommandSearchOpen, output_format)
        }
        SurfaceCommand::ThemePicker(command) => {
            run_surface_open_command(command, ActionKind::SurfaceThemePickerOpen, output_format)
        }
        SurfaceCommand::Keybindings(command) => {
            run_surface_open_command(command, ActionKind::SurfaceKeybindingsOpen, output_format)
        }
        SurfaceCommand::WarpDrive(command) => match command {
            SurfaceOpenToggleCommand::Open(args) => run_action_with_params(
                args,
                ActionKind::SurfaceWarpDriveOpen,
                EmptyParams {},
                output_format,
            ),
            SurfaceOpenToggleCommand::Toggle(args) => run_action_with_params(
                args,
                ActionKind::SurfaceWarpDriveToggle,
                EmptyParams {},
                output_format,
            ),
        },
        SurfaceCommand::ResourceCenter(command) => run_surface_toggle_command(
            command,
            ActionKind::SurfaceResourceCenterToggle,
            output_format,
        ),
        SurfaceCommand::AiAssistant(command) => {
            run_surface_toggle_command(command, ActionKind::SurfaceAiAssistantToggle, output_format)
        }
        SurfaceCommand::CodeReview(command) => match command {
            SurfaceOpenToggleCommand::Open(args) => run_action_with_params(
                args,
                ActionKind::SurfaceCodeReviewOpen,
                EmptyParams {},
                output_format,
            ),
            SurfaceOpenToggleCommand::Toggle(args) => run_action_with_params(
                args,
                ActionKind::SurfaceCodeReviewToggle,
                EmptyParams {},
                output_format,
            ),
        },
        SurfaceCommand::ProjectExplorer(command) => run_surface_open_command(
            command,
            ActionKind::SurfaceProjectExplorerOpen,
            output_format,
        ),
        SurfaceCommand::GlobalSearch(command) => {
            run_surface_open_command(command, ActionKind::SurfaceGlobalSearchOpen, output_format)
        }
        SurfaceCommand::ConversationList(command) => run_surface_open_command(
            command,
            ActionKind::SurfaceConversationListOpen,
            output_format,
        ),
        SurfaceCommand::LeftPanel(command) => {
            run_surface_toggle_command(command, ActionKind::SurfaceLeftPanelToggle, output_format)
        }
        SurfaceCommand::RightPanel(command) => {
            run_surface_toggle_command(command, ActionKind::SurfaceRightPanelToggle, output_format)
        }
        SurfaceCommand::VerticalTabs(command) => match command {
            SurfaceOpenToggleCommand::Open(args) => run_action_with_params(
                args,
                ActionKind::SurfaceVerticalTabsOpen,
                EmptyParams {},
                output_format,
            ),
            SurfaceOpenToggleCommand::Toggle(args) => run_action_with_params(
                args,
                ActionKind::SurfaceVerticalTabsToggle,
                EmptyParams {},
                output_format,
            ),
        },
        SurfaceCommand::AgentManagement(command) => run_surface_open_command(
            command,
            ActionKind::SurfaceAgentManagementOpen,
            output_format,
        ),
    }
}

fn render_human_readable(action: ActionKind, data: &serde_json::Value) -> String {
    match action {
        ActionKind::AppPing => format!(
            "Warp instance {} is reachable (protocol version {})",
            value_or_unknown(data, "instance_id"),
            value_or_unknown(data, "protocol_version")
        ),
        ActionKind::AppVersion => format!(
            "Warp instance {}\nchannel: {}\napp_id: {}\nprotocol_version: {}",
            value_or_unknown(data, "instance_id"),
            value_or_unknown(data, "channel"),
            value_or_unknown(data, "app_id"),
            value_or_unknown(data, "protocol_version")
        ),
        ActionKind::TabCreate => format!(
            "Created tab {} in window {} (active index {}, tab count {})",
            nested_value_or_unknown(data, &["tab", "id"]),
            nested_value_or_unknown(data, &["window", "id"]),
            nested_value_or_unknown(data, &["tab", "active_index"]),
            nested_value_or_unknown(data, &["tab", "count"])
        ),
        ActionKind::PaneSplit => format!(
            "Split created pane {}",
            nested_value_or_unknown(data, &["pane", "id"])
        ),
        _ => serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string()),
    }
}

fn value_or_unknown(data: &serde_json::Value, key: &str) -> String {
    nested_value_or_unknown(data, &[key])
}

fn nested_value_or_unknown(data: &serde_json::Value, path: &[&str]) -> String {
    let value = path
        .iter()
        .try_fold(data, |value, key| value.get(*key))
        .unwrap_or(&serde_json::Value::Null);
    match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Null => "<unknown>".to_owned(),
        value => value.to_string(),
    }
}

#[cfg(test)]
pub(crate) fn render_human_readable_for_test(
    action: ActionKind,
    data: &serde_json::Value,
) -> String {
    render_human_readable(action, data)
}

pub(super) fn run_instance_command(
    command: InstanceCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        InstanceCommand::List => render_instance_list(
            local_control::discovery::list_instances(&ChannelState::channel().to_string()),
            output_format,
        ),
        InstanceCommand::Inspect(args) => run_action_with_params(
            args,
            ActionKind::InstanceInspect,
            EmptyParams {},
            output_format,
        ),
    }
}

/// JSON payload for `warpctrl instance list`.
#[derive(Serialize)]
pub(super) struct InstanceListOutput {
    instances: Vec<InstanceSummary>,
}

#[derive(Serialize)]
struct InstanceSummary {
    instance_id: String,
    pid: u32,
    channel: String,
    app_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_version: Option<String>,
    protocol_version: u32,
}

/// Builds the list payload from probed discovery records.
pub(super) fn instance_list_output(records: Vec<InstanceRecord>) -> InstanceListOutput {
    InstanceListOutput {
        instances: records
            .into_iter()
            .map(|record| InstanceSummary {
                instance_id: record.instance_id.0,
                pid: record.pid,
                channel: record.channel,
                app_id: record.app_id,
                app_version: record.app_version,
                protocol_version: record.protocol_version,
            })
            .collect(),
    }
}

/// Lists every reachable instance without selecting one. Zero reachable
/// instances is a successful empty list, never an error.
fn render_instance_list(
    records: Vec<InstanceRecord>,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    let output = instance_list_output(records);
    match output_format {
        OutputFormat::Json => write_json(&output),
        OutputFormat::Ndjson => write_json_line(&output),
        OutputFormat::Pretty | OutputFormat::Text => {
            if output.instances.is_empty() {
                println!("No running Warp instances with local control were found.");
                return Ok(());
            }
            for instance in &output.instances {
                println!(
                    "{} (pid {}, channel {}, app {}, protocol {})",
                    instance.instance_id,
                    instance.pid,
                    instance.channel,
                    instance.app_id,
                    instance.protocol_version
                );
            }
            Ok(())
        }
    }
}

pub(super) fn run_app_command(
    command: AppCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        AppCommand::Ping(args) => run_action(args, ActionKind::AppPing, output_format),
        AppCommand::Version(args) => run_action(args, ActionKind::AppVersion, output_format),
        AppCommand::Active(args) => {
            run_action_with_params(args, ActionKind::AppActive, EmptyParams {}, output_format)
        }
        AppCommand::Focus(args) => run_action(args, ActionKind::AppFocus, output_format),
    }
}

pub(super) fn run_action_catalog_command(
    command: ActionCatalogCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        ActionCatalogCommand::List => run_action_with_params(
            TargetArgs::default(),
            ActionKind::ActionList,
            EmptyParams {},
            output_format,
        ),
        ActionCatalogCommand::Inspect { action } => run_action_with_params(
            TargetArgs::default(),
            ActionKind::ActionInspect,
            ActionNameParams { action },
            output_format,
        ),
    }
}

pub(super) fn run_capability_command(
    command: CapabilityCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        CapabilityCommand::List => run_action_with_params(
            TargetArgs::default(),
            ActionKind::CapabilityList,
            EmptyParams {},
            output_format,
        ),
        CapabilityCommand::Inspect { action } => run_action_with_params(
            TargetArgs::default(),
            ActionKind::CapabilityInspect,
            ActionNameParams { action },
            output_format,
        ),
    }
}

pub(super) fn run_window_command(
    command: WindowCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        WindowCommand::List(args) => {
            run_action_with_params(args, ActionKind::WindowList, EmptyParams {}, output_format)
        }
        WindowCommand::Inspect(args) => run_action_with_params(
            args,
            ActionKind::WindowInspect,
            EmptyParams {},
            output_format,
        ),
        WindowCommand::Create(args) => run_action_with_params(
            args.target,
            ActionKind::WindowCreate,
            TabCreateParams {
                tab_type: args.tab_type.map(Into::into),
                shell: args.shell,
            },
            output_format,
        ),
        WindowCommand::Focus(args) => {
            run_action_with_params(args, ActionKind::WindowFocus, EmptyParams {}, output_format)
        }
        WindowCommand::Close(args) => {
            run_action_with_params(args, ActionKind::WindowClose, EmptyParams {}, output_format)
        }
    }
}

pub(super) fn run_tab_command(
    command: TabCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        TabCommand::List(args) => {
            run_action_with_params(args, ActionKind::TabList, EmptyParams {}, output_format)
        }
        TabCommand::Inspect(args) => {
            run_action_with_params(args, ActionKind::TabInspect, EmptyParams {}, output_format)
        }
        TabCommand::Create(args) => run_action_with_params(
            args.target,
            ActionKind::TabCreate,
            TabCreateParams {
                tab_type: args.tab_type.map(Into::into),
                shell: args.shell,
            },
            output_format,
        ),
        TabCommand::Activate(args) => {
            let mode = tab_activation_mode(&args);
            run_action_with_params(
                args.target,
                ActionKind::TabActivate,
                TabActivateParams { mode },
                output_format,
            )
        }
        TabCommand::Move(args) => run_action_with_params(
            args.target,
            ActionKind::TabMove,
            DirectionParams {
                direction: args.direction.into(),
            },
            output_format,
        ),
        TabCommand::Close(args) => {
            let mode = tab_close_mode(&args);
            run_action_with_params(
                args.target,
                ActionKind::TabClose,
                TabCloseParams { mode },
                output_format,
            )
        }
        TabCommand::Rename(args) => run_action_with_params(
            args.target,
            ActionKind::TabRename,
            RenameParams { title: args.title },
            output_format,
        ),
        TabCommand::ResetName(args) => run_action(args, ActionKind::TabResetName, output_format),
        TabCommand::Color(command) => match command {
            TabColorCommand::Set(args) => run_action_with_params(
                args.target,
                ActionKind::TabColorSet,
                ColorValueParams { color: args.color },
                output_format,
            ),
            TabColorCommand::Clear(args) => {
                run_action(args, ActionKind::TabColorClear, output_format)
            }
        },
    }
}

pub(super) fn run_pane_command(
    command: PaneCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        PaneCommand::List(args) => {
            run_action_with_params(args, ActionKind::PaneList, EmptyParams {}, output_format)
        }
        PaneCommand::Inspect(args) => {
            run_action_with_params(args, ActionKind::PaneInspect, EmptyParams {}, output_format)
        }
        PaneCommand::Split(args) => run_action_with_params(
            args.target,
            ActionKind::PaneSplit,
            DirectionParams {
                direction: args.direction.into(),
            },
            output_format,
        ),
        PaneCommand::Focus(args) => {
            run_action_with_params(args, ActionKind::PaneFocus, EmptyParams {}, output_format)
        }
        PaneCommand::Navigate(args) => run_action_with_params(
            args.target,
            ActionKind::PaneNavigate,
            DirectionParams {
                direction: args.direction.into(),
            },
            output_format,
        ),
        PaneCommand::Resize(args) => run_action_with_params(
            args.target,
            ActionKind::PaneResize,
            ResizeParams {
                direction: args.direction.into(),
                amount: args.amount,
            },
            output_format,
        ),
        PaneCommand::Maximize(args) => run_action_with_params(
            args,
            ActionKind::PaneMaximize,
            EmptyParams {},
            output_format,
        ),
        PaneCommand::Unmaximize(args) => run_action_with_params(
            args,
            ActionKind::PaneUnmaximize,
            EmptyParams {},
            output_format,
        ),
        PaneCommand::Close(args) => {
            run_action_with_params(args, ActionKind::PaneClose, EmptyParams {}, output_format)
        }
        PaneCommand::Rename(args) => run_action_with_params(
            args.target,
            ActionKind::PaneRename,
            RenameParams { title: args.title },
            output_format,
        ),
        PaneCommand::ResetName(args) => run_action(args, ActionKind::PaneResetName, output_format),
    }
}

pub(super) fn run_session_command(
    command: SessionCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SessionCommand::List(args) => {
            run_action_with_params(args, ActionKind::SessionList, EmptyParams {}, output_format)
        }
        SessionCommand::Inspect(args) => run_action_with_params(
            args,
            ActionKind::SessionInspect,
            EmptyParams {},
            output_format,
        ),
        SessionCommand::Activate(args) => run_action_with_params(
            args,
            ActionKind::SessionActivate,
            EmptyParams {},
            output_format,
        ),
        SessionCommand::Previous(args) => run_action_with_params(
            args,
            ActionKind::SessionPrevious,
            EmptyParams {},
            output_format,
        ),
        SessionCommand::Next(args) => {
            run_action_with_params(args, ActionKind::SessionNext, EmptyParams {}, output_format)
        }
        SessionCommand::ReopenClosed(args) => run_action_with_params(
            args,
            ActionKind::SessionReopenClosed,
            EmptyParams {},
            output_format,
        ),
    }
}

pub(super) fn run_input_command(
    command: InputCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        InputCommand::Insert(args) => run_action_with_params(
            args.target,
            ActionKind::InputInsert,
            TextParams { text: args.text },
            output_format,
        ),
        InputCommand::Replace(args) => run_action_with_params(
            args.target,
            ActionKind::InputReplace,
            TextParams { text: args.text },
            output_format,
        ),
    }
}

pub(super) fn run_theme_command(
    command: ThemeCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        ThemeCommand::List(args) => {
            run_action_with_params(args, ActionKind::ThemeList, EmptyParams {}, output_format)
        }
        ThemeCommand::Get(args) => {
            run_action_with_params(args, ActionKind::ThemeGet, EmptyParams {}, output_format)
        }
        ThemeCommand::Set(args) => run_action_with_params(
            args.target,
            ActionKind::ThemeSet,
            ThemeNameParams {
                theme_name: args.name,
            },
            output_format,
        ),
        ThemeCommand::SystemSet(args) => run_action_with_params(
            args.target,
            ActionKind::ThemeSystemSet,
            BooleanValueParams {
                value: args.enabled,
            },
            output_format,
        ),
        ThemeCommand::LightSet(args) => run_action_with_params(
            args.target,
            ActionKind::ThemeLightSet,
            ThemeNameParams {
                theme_name: args.name,
            },
            output_format,
        ),
        ThemeCommand::DarkSet(args) => run_action_with_params(
            args.target,
            ActionKind::ThemeDarkSet,
            ThemeNameParams {
                theme_name: args.name,
            },
            output_format,
        ),
    }
}

pub(super) fn run_appearance_command(
    command: AppearanceCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        AppearanceCommand::Get(args) => run_action_with_params(
            args,
            ActionKind::AppearanceGet,
            EmptyParams {},
            output_format,
        ),
        AppearanceCommand::FontSizeIncrease(args) => {
            run_action(args, ActionKind::AppearanceFontSizeIncrease, output_format)
        }
        AppearanceCommand::FontSizeDecrease(args) => {
            run_action(args, ActionKind::AppearanceFontSizeDecrease, output_format)
        }
        AppearanceCommand::FontSizeReset(args) => {
            run_action(args, ActionKind::AppearanceFontSizeReset, output_format)
        }
        AppearanceCommand::ZoomIncrease(args) => {
            run_action(args, ActionKind::AppearanceZoomIncrease, output_format)
        }
        AppearanceCommand::ZoomDecrease(args) => {
            run_action(args, ActionKind::AppearanceZoomDecrease, output_format)
        }
        AppearanceCommand::ZoomReset(args) => {
            run_action(args, ActionKind::AppearanceZoomReset, output_format)
        }
    }
}

pub(super) fn run_setting_command(
    command: SettingCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SettingCommand::List(args) => run_action_with_params(
            args.target,
            ActionKind::SettingList,
            SettingListParams {
                namespace: args.namespace,
            },
            output_format,
        ),
        SettingCommand::Get(args) => run_action_with_params(
            args.target,
            ActionKind::SettingGet,
            KeyParams { key: args.key },
            output_format,
        ),
        SettingCommand::Set(args) => run_action_with_params(
            args.target,
            ActionKind::SettingSet,
            KeyValueParams {
                key: args.key,
                value: parse_json_value_or_string(args.value),
            },
            output_format,
        ),
        SettingCommand::Toggle(args) => run_action_with_params(
            args.target,
            ActionKind::SettingToggle,
            KeyParams { key: args.key },
            output_format,
        ),
    }
}

pub(super) fn run_keybinding_command(
    command: KeybindingCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        KeybindingCommand::List(args) => run_action_with_params(
            args,
            ActionKind::KeybindingList,
            EmptyParams {},
            output_format,
        ),
        KeybindingCommand::Get(args) => run_action_with_params(
            args.target,
            ActionKind::KeybindingGet,
            BindingNameParams {
                binding_name: args.name,
            },
            output_format,
        ),
    }
}

pub(super) fn run_file_command(
    command: FileCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        FileCommand::Open(args) => run_action_with_params(
            args.target,
            ActionKind::FileOpen,
            FileOpenParams {
                path: args.path,
                line: args.line,
                column: args.column,
                new_tab: args.new_tab,
            },
            output_format,
        ),
    }
}

fn tab_activation_mode(args: &TabActivateArgs) -> TabActivationMode {
    if args.previous {
        TabActivationMode::Previous
    } else if args.next {
        TabActivationMode::Next
    } else if args.last {
        TabActivationMode::Last
    } else {
        TabActivationMode::Target
    }
}

fn tab_close_mode(args: &TabCloseArgs) -> TabCloseMode {
    if args.others {
        TabCloseMode::Others
    } else if args.right_of {
        TabCloseMode::RightOf
    } else if args.active {
        TabCloseMode::Active
    } else {
        TabCloseMode::Target
    }
}

fn run_surface_query_command(
    command: SurfaceQueryCommand,
    action: ActionKind,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SurfaceQueryCommand::Open(args) => run_action_with_params(
            args.target,
            action,
            QueryParams { query: args.query },
            output_format,
        ),
    }
}

fn run_surface_open_command(
    command: SurfaceOpenCommand,
    action: ActionKind,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SurfaceOpenCommand::Open(args) => {
            run_action_with_params(args, action, EmptyParams {}, output_format)
        }
    }
}
fn run_surface_toggle_command(
    command: SurfaceToggleCommand,
    action: ActionKind,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SurfaceToggleCommand::Toggle(args) => {
            run_action_with_params(args, action, EmptyParams {}, output_format)
        }
    }
}

fn run_action(
    args: TargetArgs,
    action: ActionKind,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    run_action_with_params(args, action, EmptyParams {}, output_format)
}

fn run_action_with_params<T: Serialize>(
    args: TargetArgs,
    action: ActionKind,
    params: T,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    let selector = instance_selector(&args);
    let records = local_control::discovery::list_instances(&ChannelState::channel().to_string());
    let target = target_selector(&args)?;
    let instance = select_instance(&records, &selector)?;
    let mut request = RequestEnvelope::new(Action::with_params(action, params)?);
    request.target = target;
    let response = local_control::client::send_request(&instance, &request)?;
    let local_control::protocol::ControlResponse::Ok { data } = response.response else {
        return Err(ControlError::new(
            ErrorCode::Internal,
            "local-control request failed without an error payload",
        ));
    };
    match output_format {
        OutputFormat::Json => write_json(&data),
        OutputFormat::Ndjson => write_json_line(&data),
        OutputFormat::Pretty | OutputFormat::Text => {
            println!("{}", render_human_readable(action, &data));
            Ok(())
        }
    }
}

fn parse_json_value_or_string(value: String) -> serde_json::Value {
    serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
}
