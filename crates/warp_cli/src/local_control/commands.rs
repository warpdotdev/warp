//! Implementations for user-facing `warpctrl` command groups.
use local_control::discovery::InstanceRecord;
use local_control::protocol::{
    Action, ActionKind, ActionMetadata, ControlError, ErrorCode, RequestEnvelope,
};
use local_control::selection::select_instance;
use serde::Serialize;
use serde_json::json;

use crate::agent::OutputFormat;
use crate::local_control::output::{write_json, write_json_line};
use crate::local_control::selectors::{instance_selector, target_selector};
use crate::local_control::{
    ActionCommand, AppCommand, AppearanceCommand, BlockCommand, CapabilityCommand, DriveCommand,
    FileCommand, HistoryCommand, InputCommand, InstanceCommand, KeybindingCommand, PaneCommand,
    ProjectCommand, SessionCommand, SettingCommand, SurfaceCommand, TabCommand, TabCreateArgs,
    TabType, TargetArgs, ThemeCommand, WindowCommand,
};

/// Display-oriented projection of a discoverable Warp instance.
#[derive(Serialize)]
struct InstanceSummary {
    instance_id: String,
    pid: u32,
    channel: String,
    app_id: String,
    app_version: Option<String>,
    started_at: String,
    endpoint: Option<local_control::discovery::ControlEndpoint>,
    outside_warp_control_enabled: bool,
    actions: Vec<ActionMetadata>,
}

impl From<local_control::discovery::InstanceRecord> for InstanceSummary {
    fn from(record: local_control::discovery::InstanceRecord) -> Self {
        Self {
            instance_id: record.instance_id.0,
            pid: record.pid,
            channel: record.channel,
            app_id: record.app_id,
            app_version: record.app_version,
            started_at: record.started_at.to_rfc3339(),
            endpoint: record.endpoint,
            outside_warp_control_enabled: record.outside_warp_control_enabled,
            actions: record.actions,
        }
    }
}

#[derive(Serialize)]
struct StubSummary<'a> {
    ok: bool,
    action: &'a str,
    implemented: bool,
}

pub(super) fn run_instance_command(
    command: InstanceCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        InstanceCommand::List => {
            let summaries = local_control::discovery::list_instances()
                .into_iter()
                .map(InstanceSummary::from)
                .collect::<Vec<_>>();
            write_values(summaries, output_format)
        }
        InstanceCommand::Inspect(args) => {
            let records = local_control::discovery::list_instances();
            let selector = instance_selector(args);
            let instance = select_instance(&records, &selector)?;
            let summary = InstanceSummary::from(instance.clone());
            write_value(&summary, output_format)
        }
    }
}

pub(super) fn run_app_command(
    command: AppCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        AppCommand::Ping(args) => run_action(args, ActionKind::AppPing, json!({}), output_format),
        AppCommand::Version(args) => {
            run_action(args, ActionKind::AppVersion, json!({}), output_format)
        }
        AppCommand::Active(args) => {
            run_action(args, ActionKind::AppActive, json!({}), output_format)
        }
        AppCommand::Focus(args) => run_action(args, ActionKind::AppFocus, json!({}), output_format),
    }
}

pub(super) fn run_capability_command(
    command: CapabilityCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        CapabilityCommand::List(_) => write_action_metadata(output_format),
        CapabilityCommand::Inspect(args) => {
            write_named_action_metadata(&args.action, output_format)
        }
    }
}

pub(super) fn run_window_command(
    command: WindowCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        WindowCommand::List(args) => {
            run_action(args, ActionKind::WindowList, json!({}), output_format)
        }
        WindowCommand::Inspect(args) => {
            run_action(args, ActionKind::WindowInspect, json!({}), output_format)
        }
        WindowCommand::Create(args) => run_action(
            args.target,
            ActionKind::WindowCreate,
            json!({ "profile": args.shell }),
            output_format,
        ),
        WindowCommand::Focus(args) => {
            run_action(args, ActionKind::WindowFocus, json!({}), output_format)
        }
        WindowCommand::Close(args) => {
            run_action(args, ActionKind::WindowClose, json!({}), output_format)
        }
    }
}

pub(super) fn run_tab_command(
    command: TabCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        TabCommand::List(args) => run_action(args, ActionKind::TabList, json!({}), output_format),
        TabCommand::Inspect(args) => {
            run_action(args, ActionKind::TabInspect, json!({}), output_format)
        }
        TabCommand::Create(args) => run_tab_create(args, output_format),
        TabCommand::Activate(args) => run_action(
            args.target,
            ActionKind::TabActivate,
            json!({ "relative": tab_activation_target(args.previous, args.next, args.last) }),
            output_format,
        ),
        TabCommand::Move(args) => run_action(
            args.target,
            ActionKind::TabMove,
            json!({ "direction": horizontal_direction(args.direction) }),
            output_format,
        ),
        TabCommand::Rename(_) => unsupported_action("tab.rename"),
        TabCommand::ResetName(_) => unsupported_action("tab.reset-name"),
        TabCommand::Color(_) => unsupported_action("tab.color"),
        TabCommand::Close(args) => run_action(
            args.target,
            ActionKind::TabClose,
            json!({ "scope": tab_close_scope(args.others, args.right_of), "force": false }),
            output_format,
        ),
    }
}

pub(super) fn run_pane_command(
    command: PaneCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        PaneCommand::List(args) => run_action(args, ActionKind::PaneList, json!({}), output_format),
        PaneCommand::Inspect(args) => {
            run_action(args, ActionKind::PaneInspect, json!({}), output_format)
        }
        PaneCommand::Split(args) => run_action(
            args.target,
            ActionKind::PaneSplit,
            json!({ "direction": split_direction(args.direction), "profile": args.shell }),
            output_format,
        ),
        PaneCommand::Focus(args) => {
            run_action(args, ActionKind::PaneFocus, json!({}), output_format)
        }
        PaneCommand::Navigate(args) => run_action(
            args.target,
            ActionKind::PaneNavigate,
            json!({ "direction": navigation_direction(args.direction) }),
            output_format,
        ),
        PaneCommand::Resize(args) => run_action(
            args.target,
            ActionKind::PaneResize,
            json!({ "direction": split_direction(args.direction), "amount": args.amount }),
            output_format,
        ),
        PaneCommand::Maximize(args) => run_action(
            args,
            ActionKind::PaneMaximize,
            json!({ "enabled": true }),
            output_format,
        ),
        PaneCommand::Unmaximize(args) => run_action(
            args,
            ActionKind::PaneMaximize,
            json!({ "enabled": false }),
            output_format,
        ),
        PaneCommand::Close(args) => {
            run_action(args, ActionKind::PaneClose, json!({}), output_format)
        }
        PaneCommand::Rename(_) => unsupported_action("pane.rename"),
        PaneCommand::ResetName(_) => unsupported_action("pane.reset-name"),
    }
}

pub(super) fn run_session_command(
    command: SessionCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SessionCommand::List(args) => {
            run_action(args, ActionKind::SessionList, json!({}), output_format)
        }
        SessionCommand::Inspect(args) => {
            run_action(args, ActionKind::SessionInspect, json!({}), output_format)
        }
        SessionCommand::Activate(_) => unsupported_action("session.activate"),
        SessionCommand::Previous(args) => {
            run_action(args, ActionKind::SessionPrevious, json!({}), output_format)
        }
        SessionCommand::Next(args) => {
            run_action(args, ActionKind::SessionNext, json!({}), output_format)
        }
        SessionCommand::ReopenClosed(args) => run_action(
            args,
            ActionKind::SessionReopenClosed,
            json!({}),
            output_format,
        ),
    }
}

pub(super) fn run_block_command(
    command: BlockCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        BlockCommand::List(args) => run_action(
            args.target,
            ActionKind::BlockList,
            json!({ "limit": args.limit }),
            output_format,
        ),
        BlockCommand::Inspect(args) => {
            run_action(args, ActionKind::BlockInspect, json!({}), output_format)
        }
        BlockCommand::Output(args) => run_action(
            args.target,
            ActionKind::BlockOutput,
            json!({ "format": block_output_format(args.plain, args.ansi, args.json) }),
            output_format,
        ),
    }
}

pub(super) fn run_input_command(
    command: InputCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        InputCommand::Get(args) => run_action(args, ActionKind::InputGet, json!({}), output_format),
        InputCommand::Insert(args) => run_action(
            args.target,
            ActionKind::InputInsert,
            json!({ "text": args.text }),
            output_format,
        ),
        InputCommand::Replace(args) => run_action(
            args.target,
            ActionKind::InputReplace,
            json!({ "text": args.text }),
            output_format,
        ),
        InputCommand::Clear(args) => {
            run_action(args, ActionKind::InputClear, json!({}), output_format)
        }
        InputCommand::Mode(command) => match command {
            crate::local_control::InputModeCommand::Set(args) => run_action(
                args.target,
                ActionKind::InputModeSet,
                json!({ "mode": input_mode(args.mode) }),
                output_format,
            ),
        },
    }
}

pub(super) fn run_history_command(
    command: HistoryCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        HistoryCommand::List(args) => run_action(
            args.target,
            ActionKind::HistoryList,
            json!({ "limit": args.limit }),
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
            run_action(args, ActionKind::ThemeList, json!({}), output_format)
        }
        ThemeCommand::Get(_) => unsupported_action("theme.get"),
        ThemeCommand::Set(args) => run_action(
            args.target,
            ActionKind::ThemeSet,
            json!({ "theme_name": args.theme_name }),
            output_format,
        ),
        ThemeCommand::System(_) => unsupported_action("theme.system.set"),
        ThemeCommand::Light(_) => unsupported_action("theme.light.set"),
        ThemeCommand::Dark(_) => unsupported_action("theme.dark.set"),
    }
}

pub(super) fn run_appearance_command(
    command: AppearanceCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        AppearanceCommand::Get(args) => {
            run_action(args, ActionKind::AppearanceGet, json!({}), output_format)
        }
        AppearanceCommand::FontSize(_) => unsupported_action("appearance.font-size"),
        AppearanceCommand::Zoom(_) => unsupported_action("appearance.zoom"),
    }
}

pub(super) fn run_setting_command(
    command: SettingCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SettingCommand::List(args) => run_action(
            args.target,
            ActionKind::SettingList,
            json!({ "namespace": args.namespace }),
            output_format,
        ),
        SettingCommand::Get(args) => run_action(
            args.target,
            ActionKind::SettingGet,
            json!({ "key": args.key }),
            output_format,
        ),
        SettingCommand::Set(args) => run_action(
            args.target,
            ActionKind::SettingSet,
            json!({ "key": args.key, "value": args.value }),
            output_format,
        ),
        SettingCommand::Toggle(args) => run_action(
            args.target,
            ActionKind::SettingToggle,
            json!({ "key": args.key }),
            output_format,
        ),
    }
}

pub(super) fn run_keybinding_command(command: KeybindingCommand) -> Result<(), ControlError> {
    match command {
        KeybindingCommand::List(_) => unsupported_action("keybinding.list"),
        KeybindingCommand::Get(_) => unsupported_action("keybinding.get"),
    }
}

pub(super) fn run_action_command(
    command: ActionCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        ActionCommand::List(_) => write_action_metadata(output_format),
        ActionCommand::Inspect(args) => write_named_action_metadata(&args.action, output_format),
    }
}

pub(super) fn run_file_command(
    command: FileCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        FileCommand::List(args) => run_action(args, ActionKind::FileList, json!({}), output_format),
        FileCommand::Open(args) => run_action(
            args.target,
            ActionKind::FileOpen,
            json!({
                "path": args.path.to_string_lossy(),
                "line": args.line,
                "column": args.column,
                "new_tab": args.new_tab,
            }),
            output_format,
        ),
    }
}

pub(super) fn run_project_command(
    command: ProjectCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        ProjectCommand::Active(args) => {
            run_action(args, ActionKind::ProjectActive, json!({}), output_format)
        }
        ProjectCommand::List(args) => {
            run_action(args, ActionKind::ProjectList, json!({}), output_format)
        }
        ProjectCommand::Open(args) => run_action(
            args.target,
            ActionKind::ProjectOpen,
            json!({ "type": "path", "path": args.path.to_string_lossy() }),
            output_format,
        ),
    }
}

pub(super) fn run_drive_command(
    command: DriveCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        DriveCommand::List(args) => run_action(
            args.target,
            ActionKind::DriveList,
            json!({ "object_type": drive_object_type_name(args.object_type) }),
            output_format,
        ),
        DriveCommand::Inspect(args) => run_action(
            args.target,
            ActionKind::DriveInspect,
            json!({ "id": args.id }),
            output_format,
        ),
        DriveCommand::Open(_) => unsupported_action("drive.open"),
        DriveCommand::Notebook(_) => unsupported_action("drive.notebook.open"),
        DriveCommand::EnvVarCollection(_) => unsupported_action("drive.env-var-collection.open"),
        DriveCommand::Object(_) => unsupported_action("drive.object"),
        DriveCommand::Workflow(_) => unsupported_action("drive.workflow"),
    }
}

pub(super) fn run_surface_command(
    command: SurfaceCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SurfaceCommand::Settings(command) => match command {
            crate::local_control::SurfaceSettingsCommand::Open(args) => run_action(
                args.target,
                ActionKind::SurfaceSettingsOpen,
                json!({ "page": args.page, "query": args.query }),
                output_format,
            ),
        },
        SurfaceCommand::CommandPalette(command) => match command {
            crate::local_control::SurfaceQueryOpenCommand::Open(args) => run_action(
                args.target,
                ActionKind::SurfaceCommandPaletteOpen,
                json!({ "query": args.query }),
                output_format,
            ),
        },
        SurfaceCommand::CommandSearch(command) => match command {
            crate::local_control::SurfaceQueryOpenCommand::Open(args) => run_action(
                args.target,
                ActionKind::SurfaceCommandSearchOpen,
                json!({ "query": args.query }),
                output_format,
            ),
        },
        SurfaceCommand::WarpDrive(command) => match command {
            crate::local_control::SurfaceOpenToggleCommand::Open(args) => run_action(
                args,
                ActionKind::SurfaceWarpDriveOpen,
                json!({}),
                output_format,
            ),
            crate::local_control::SurfaceOpenToggleCommand::Toggle(args) => run_action(
                args,
                ActionKind::SurfaceWarpDriveToggle,
                json!({}),
                output_format,
            ),
        },
        SurfaceCommand::ResourceCenter(command) => match command {
            crate::local_control::SurfaceToggleCommand::Toggle(args) => run_action(
                args,
                ActionKind::SurfaceResourceCenterToggle,
                json!({}),
                output_format,
            ),
        },
        SurfaceCommand::AiAssistant(command) => match command {
            crate::local_control::SurfaceToggleCommand::Toggle(args) => run_action(
                args,
                ActionKind::SurfaceAiAssistantToggle,
                json!({}),
                output_format,
            ),
        },
        SurfaceCommand::CodeReview(command) => match command {
            crate::local_control::SurfaceToggleCommand::Toggle(args) => run_action(
                args,
                ActionKind::SurfaceCodeReviewToggle,
                json!({}),
                output_format,
            ),
        },
        SurfaceCommand::LeftPanel(command) => match command {
            crate::local_control::SurfaceToggleCommand::Toggle(args) => run_action(
                args,
                ActionKind::SurfaceLeftPanelToggle,
                json!({}),
                output_format,
            ),
        },
        SurfaceCommand::RightPanel(command) => match command {
            crate::local_control::SurfaceToggleCommand::Toggle(args) => run_action(
                args,
                ActionKind::SurfaceRightPanelToggle,
                json!({}),
                output_format,
            ),
        },
        SurfaceCommand::VerticalTabs(command) => match command {
            crate::local_control::SurfaceToggleCommand::Toggle(args) => run_action(
                args,
                ActionKind::SurfaceVerticalTabsToggle,
                json!({}),
                output_format,
            ),
        },
    }
}

fn tab_activation_target(previous: bool, next: bool, last: bool) -> Option<&'static str> {
    if previous {
        Some("previous")
    } else if next {
        Some("next")
    } else if last {
        Some("last")
    } else {
        None
    }
}

fn tab_close_scope(others: bool, right_of: bool) -> &'static str {
    if others {
        "others"
    } else if right_of {
        "right"
    } else {
        "target"
    }
}

fn horizontal_direction(direction: crate::local_control::HorizontalDirection) -> &'static str {
    match direction {
        crate::local_control::HorizontalDirection::Left => "left",
        crate::local_control::HorizontalDirection::Right => "right",
    }
}

fn split_direction(direction: crate::local_control::SplitDirection) -> &'static str {
    match direction {
        crate::local_control::SplitDirection::Left => "left",
        crate::local_control::SplitDirection::Right => "right",
        crate::local_control::SplitDirection::Up => "up",
        crate::local_control::SplitDirection::Down => "down",
    }
}

fn navigation_direction(direction: crate::local_control::NavigationDirection) -> &'static str {
    match direction {
        crate::local_control::NavigationDirection::Left => "left",
        crate::local_control::NavigationDirection::Right => "right",
        crate::local_control::NavigationDirection::Up => "up",
        crate::local_control::NavigationDirection::Down => "down",
        crate::local_control::NavigationDirection::Previous => "left",
        crate::local_control::NavigationDirection::Next => "right",
    }
}

fn input_mode(mode: crate::local_control::InputMode) -> &'static str {
    match mode {
        crate::local_control::InputMode::Terminal => "terminal",
        crate::local_control::InputMode::Agent => "agent",
    }
}

fn drive_object_type_name(object_type: crate::local_control::DriveObjectType) -> &'static str {
    match object_type {
        crate::local_control::DriveObjectType::Workflow => "workflow",
        crate::local_control::DriveObjectType::Notebook => "notebook",
        crate::local_control::DriveObjectType::EnvVarCollection => "env_var_collection",
        crate::local_control::DriveObjectType::Prompt => "prompt",
        crate::local_control::DriveObjectType::Folder => "folder",
        crate::local_control::DriveObjectType::AiFact => "ai_fact",
        crate::local_control::DriveObjectType::McpServer => "mcp_server",
        crate::local_control::DriveObjectType::Space => "space",
        crate::local_control::DriveObjectType::Trash => "trash",
    }
}

fn block_output_format(plain: bool, ansi: bool, json: bool) -> &'static str {
    if ansi {
        "ansi"
    } else if json {
        "json"
    } else if plain {
        "plain"
    } else {
        "plain"
    }
}

fn run_tab_create(args: TabCreateArgs, output_format: OutputFormat) -> Result<(), ControlError> {
    match (args.tab_type, args.shell.as_ref()) {
        (TabType::Terminal, None) => run_action(
            args.target,
            ActionKind::TabCreate,
            serde_json::Value::Object(Default::default()),
            output_format,
        ),
        (TabType::Terminal, Some(_))
        | (TabType::Agent, None | Some(_))
        | (TabType::CloudAgent, None | Some(_))
        | (TabType::Default, None | Some(_)) => unsupported_action("tab.create.with-options"),
    }
}

fn run_action(
    args: TargetArgs,
    action: ActionKind,
    params: serde_json::Value,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    if !action.is_implemented() {
        return unsupported_action(action.as_str());
    }
    let instance = select_target_instance(args.clone())?;
    let mut request = RequestEnvelope::new(Action {
        kind: action,
        params,
    });
    request.target = target_selector(args)?;
    let response = local_control::client::send_request(&instance, &request)?;
    let data = match response.response {
        local_control::protocol::ControlResponse::Ok { data } => data,
        local_control::protocol::ControlResponse::Error { error } => return Err(error),
    };
    write_value(&data, output_format)
}

fn write_action_metadata(output_format: OutputFormat) -> Result<(), ControlError> {
    let metadata = ActionKind::ALL
        .iter()
        .copied()
        .map(ActionKind::metadata)
        .collect::<Vec<_>>();
    write_values(metadata, output_format)
}

fn write_named_action_metadata(
    action: &str,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    let metadata = ActionKind::ALL
        .iter()
        .copied()
        .map(ActionKind::metadata)
        .find(|metadata| metadata.name == action)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnsupportedAction,
                format!("{action} is not in the local-control action catalog"),
            )
        })?;
    write_value(&metadata, output_format)
}

fn write_values<T: Serialize>(
    values: Vec<T>,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match output_format {
        OutputFormat::Json => write_json(&values),
        OutputFormat::Ndjson => {
            for value in values {
                write_json_line(&value)?;
            }
            Ok(())
        }
        OutputFormat::Pretty | OutputFormat::Text => write_json(&values),
    }
}

fn write_value(value: &impl Serialize, output_format: OutputFormat) -> Result<(), ControlError> {
    match output_format {
        OutputFormat::Json | OutputFormat::Pretty => write_json(value),
        OutputFormat::Ndjson | OutputFormat::Text => write_json_line(value),
    }
}

pub(super) fn select_target_instance(args: TargetArgs) -> Result<InstanceRecord, ControlError> {
    let records = local_control::discovery::list_instances();
    let selector = instance_selector(args);
    select_instance(&records, &selector)
}

fn unsupported_action(action: &str) -> Result<(), ControlError> {
    let summary = StubSummary {
        ok: false,
        action,
        implemented: false,
    };
    let details = serde_json::to_string(&summary).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to render unsupported local-control action details",
            err.to_string(),
        )
    })?;
    Err(ControlError::with_details(
        ErrorCode::UnsupportedAction,
        format!(
            "{action} is part of the warpctrl command surface but is not implemented by this shard"
        ),
        details,
    ))
}
