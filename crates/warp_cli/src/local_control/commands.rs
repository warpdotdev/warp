//! Implementations for user-facing `warpctrl` command groups.
use local_control::protocol::{
    Action, ActionKind, ActionMetadata, ActionNameParams, ActionParams, ControlError,
    DriveObjectId, EmptyParams, ErrorCode, RequestEnvelope,
};
use local_control::selection::select_instance;
use serde::Serialize;

use crate::agent::OutputFormat;
use crate::local_control::output::{write_json, write_json_line};
use crate::local_control::selectors::{instance_selector, target_selector};
use crate::local_control::{
    ActionCommand, AppCommand, AppearanceCommand, BlockCommand, CapabilityCommand, DriveCommand,
    FileCommand, HistoryCommand, InputCommand, InstanceCommand, KeybindingCommand, PaneCommand,
    SessionCommand, SettingCommand, TabColorCommand, TabCommand, TargetArgs, ThemeCommand,
    WindowCommand,
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
            match output_format {
                OutputFormat::Json => write_json(&summaries),
                OutputFormat::Ndjson => {
                    for summary in summaries {
                        write_json_line(&summary)?;
                    }
                    Ok(())
                }
                OutputFormat::Pretty | OutputFormat::Text => {
                    for summary in summaries {
                        let endpoint = summary
                            .endpoint
                            .as_ref()
                            .map(|endpoint| format!("{}:{}", endpoint.host, endpoint.port))
                            .unwrap_or_else(|| "outside_warp_disabled".to_owned());
                        println!(
                            "{}\tpid={}\t{}\t{}",
                            summary.instance_id, summary.pid, summary.channel, endpoint
                        );
                    }
                    Ok(())
                }
            }
        }
        InstanceCommand::Inspect(args) => run_action_with_params(
            args,
            ActionKind::InstanceInspect,
            local_control::EmptyParams {},
            output_format,
        ),
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
    }
}

pub(super) fn run_capability_command(
    command: CapabilityCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        CapabilityCommand::List(args) => run_action_with_params(
            args,
            ActionKind::CapabilityList,
            EmptyParams {},
            output_format,
        ),
        CapabilityCommand::Inspect(args) => run_action_with_params(
            args.target,
            ActionKind::CapabilityInspect,
            ActionNameParams {
                action: args.action,
            },
            output_format,
        ),
    }
}

pub(super) fn run_action_command(
    command: ActionCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        ActionCommand::List(args) => {
            run_action_with_params(args, ActionKind::ActionList, EmptyParams {}, output_format)
        }
        ActionCommand::Inspect(args) => run_action_with_params(
            args.target,
            ActionKind::ActionInspect,
            ActionNameParams {
                action: args.action,
            },
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
        TabCommand::Create(args) => run_action(args, ActionKind::TabCreate, output_format),
        TabCommand::Rename(args) => run_action_with_params(
            args.target,
            ActionKind::TabRename,
            ActionParams::Rename { title: args.title },
            output_format,
        ),
        TabCommand::ResetName(args) => run_action(args, ActionKind::TabResetName, output_format),
        TabCommand::Color(command) => match command {
            TabColorCommand::Set(args) => run_action_with_params(
                args.target,
                ActionKind::TabColorSet,
                ActionParams::ColorValue { color: args.color },
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
        PaneCommand::Rename(args) => run_action_with_params(
            args.target,
            ActionKind::PaneRename,
            ActionParams::Rename { title: args.title },
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
    }
}

pub(super) fn run_block_command(
    command: BlockCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        BlockCommand::List(args) => run_action_with_params(
            args.target,
            ActionKind::BlockList,
            local_control::BlockListParams { limit: args.limit },
            output_format,
        ),
        BlockCommand::Inspect(args) => run_action_with_params(
            args.target,
            ActionKind::BlockInspect,
            local_control::BlockIdParams {
                block_id: args.block_id,
            },
            output_format,
        ),
        BlockCommand::Output(args) => run_action_with_params(
            args.target,
            ActionKind::BlockOutput,
            local_control::BlockIdParams {
                block_id: args.block_id,
            },
            output_format,
        ),
    }
}

pub(super) fn run_input_command(
    command: InputCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        InputCommand::Get(args) => run_action_with_params(
            args,
            ActionKind::InputGet,
            local_control::EmptyParams {},
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
            ActionParams::ThemeName {
                theme_name: args.name,
            },
            output_format,
        ),
        ThemeCommand::SystemSet(args) => run_action_with_params(
            args.target,
            ActionKind::ThemeSystemSet,
            ActionParams::BooleanValue {
                value: args.enabled,
            },
            output_format,
        ),
        ThemeCommand::LightSet(args) => run_action_with_params(
            args.target,
            ActionKind::ThemeLightSet,
            ActionParams::ThemeName {
                theme_name: args.name,
            },
            output_format,
        ),
        ThemeCommand::DarkSet(args) => run_action_with_params(
            args.target,
            ActionKind::ThemeDarkSet,
            ActionParams::ThemeName {
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

pub(super) fn run_history_command(
    command: HistoryCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        HistoryCommand::List(args) => run_action_with_params(
            args.target,
            ActionKind::HistoryList,
            local_control::HistoryListParams { limit: args.limit },
            output_format,
        ),
    }
}
pub(super) fn run_setting_command(
    command: SettingCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        SettingCommand::List(args) => run_action_with_params(
            args,
            ActionKind::SettingList,
            local_control::EmptyParams {},
            output_format,
        ),
        SettingCommand::Get(args) => run_action_with_params(
            args.target,
            ActionKind::SettingGet,
            local_control::SettingGetParams { key: args.key },
            output_format,
        ),
        SettingCommand::Set(args) => run_action_with_params(
            args.target,
            ActionKind::SettingSet,
            ActionParams::KeyValue {
                key: args.key,
                value: parse_json_value_or_string(args.value),
            },
            output_format,
        ),
        SettingCommand::Toggle(args) => run_action_with_params(
            args.target,
            ActionKind::SettingToggle,
            ActionParams::Key { key: args.key },
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
            local_control::EmptyParams {},
            output_format,
        ),
        KeybindingCommand::Get(args) => run_action_with_params(
            args.target,
            ActionKind::KeybindingGet,
            local_control::BindingNameParams {
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
        FileCommand::List(args) => run_action_with_params(
            args,
            ActionKind::FileList,
            local_control::EmptyParams {},
            output_format,
        ),
    }
}

pub(super) fn run_drive_command(
    command: DriveCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        DriveCommand::List(args) => run_action_with_params(
            args.target,
            ActionKind::DriveList,
            local_control::DriveObjectListParams {
                object_type: args.object_type.map(Into::into),
            },
            output_format,
        ),
        DriveCommand::Inspect(args) => run_action_with_params(
            args.target,
            ActionKind::DriveInspect,
            local_control::DriveInspectParams {
                id: DriveObjectId(args.id),
            },
            output_format,
        ),
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
    let records = local_control::discovery::list_instances();
    let selector = instance_selector(&args);
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
        OutputFormat::Pretty | OutputFormat::Text => write_json(&data),
    }
}

fn parse_json_value_or_string(value: String) -> serde_json::Value {
    serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
}
