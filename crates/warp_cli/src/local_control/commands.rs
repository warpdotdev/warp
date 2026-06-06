//! Implementations for user-facing `warpctrl` command groups.
use local_control::protocol::{
    Action, ActionImplementationStatus, ActionKind, ActionMetadata, ActionParams, ControlError,
    DriveObjectId, EmptyParams, ErrorCode, RequestEnvelope, WorkflowRunParams,
};
use local_control::selection::select_instance;
use serde::Serialize;

use crate::agent::OutputFormat;
use crate::local_control::output::{write_json, write_json_line};
use crate::local_control::selectors::{instance_selector, target_selector};
use crate::local_control::{
    ActionCatalogCommand, AppCommand, AppearanceCommand, BlockCommand, CapabilityCommand,
    CatalogFilterArgs, DriveCommand, DriveWorkflowCommand, FileCommand, HistoryCommand,
    InputCommand, InstanceCommand, KeybindingCommand, PaneCommand, SessionCommand, SettingCommand,
    TabColorCommand, TabCommand, TargetArgs, ThemeCommand, WindowCommand,
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

#[derive(Serialize)]
struct CatalogActionSummary {
    name: String,
    implementation_status: ActionImplementationStatus,
    requires_authenticated_user: bool,
    target_scope: local_control::protocol::TargetScope,
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

impl From<ActionMetadata> for CatalogActionSummary {
    fn from(metadata: ActionMetadata) -> Self {
        Self {
            name: metadata.name,
            implementation_status: metadata.implementation_status,
            requires_authenticated_user: metadata.requires_authenticated_user,
            target_scope: metadata.target_scope,
        }
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
        AppCommand::Focus(args) => run_action(args, ActionKind::AppFocus, output_format),
    }
}

pub(super) fn run_action_catalog_command(
    command: ActionCatalogCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        ActionCatalogCommand::List(args) => render_catalog_list(args, output_format),
        ActionCatalogCommand::Inspect { action } => {
            render_catalog_metadata(metadata_for_action_name(&action)?, output_format)
        }
    }
}

pub(super) fn run_capability_command(
    command: CapabilityCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        CapabilityCommand::List(args) => render_catalog_list(args, output_format),
        CapabilityCommand::Inspect { action } => {
            render_catalog_metadata(metadata_for_action_name(&action)?, output_format)
        }
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
        InputCommand::Run(args) => run_action_with_params(
            args.target,
            ActionKind::InputRun,
            ActionParams::Text { text: args.text },
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
        DriveCommand::Workflow(command) => match command {
            DriveWorkflowCommand::Run(args) => run_action_with_params(
                args.target,
                ActionKind::DriveWorkflowRun,
                ActionParams::WorkflowRun(WorkflowRunParams {
                    id: DriveObjectId(args.id),
                    args: args.args,
                }),
                output_format,
            ),
        },
    }
}

fn render_catalog_list(
    args: CatalogFilterArgs,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    let metadata = ActionKind::ALL
        .iter()
        .copied()
        .map(ActionKind::metadata)
        .filter(|metadata| {
            if args.implemented_only {
                metadata.implementation_status == ActionImplementationStatus::Implemented
            } else if args.stubs_only {
                metadata.implementation_status == ActionImplementationStatus::Stub
            } else {
                true
            }
        })
        .collect::<Vec<_>>();
    match output_format {
        OutputFormat::Json => write_json(&metadata),
        OutputFormat::Ndjson => {
            for metadata in metadata {
                write_json_line(&metadata)?;
            }
            Ok(())
        }
        OutputFormat::Pretty | OutputFormat::Text => {
            for summary in metadata.into_iter().map(CatalogActionSummary::from) {
                println!(
                    "{}\tstatus={:?}\tscope={:?}\tauthenticated_user={}",
                    summary.name,
                    summary.implementation_status,
                    summary.target_scope,
                    summary.requires_authenticated_user
                );
            }
            Ok(())
        }
    }
}

fn render_catalog_metadata(
    metadata: ActionMetadata,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match output_format {
        OutputFormat::Json => write_json(&metadata),
        OutputFormat::Ndjson => write_json_line(&metadata),
        OutputFormat::Pretty | OutputFormat::Text => write_json(&metadata),
    }
}

fn metadata_for_action_name(action: &str) -> Result<ActionMetadata, ControlError> {
    ActionKind::ALL
        .iter()
        .copied()
        .find(|kind| kind.as_str() == action)
        .map(ActionKind::metadata)
        .ok_or_else(|| {
            ControlError::with_details(
                ErrorCode::NotAllowlisted,
                format!("{action} is not in the public warpctrl action catalog"),
                "Use `warpctrl action list` to inspect allowlisted actions.",
            )
        })
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
        OutputFormat::Pretty | OutputFormat::Text => {
            println!("{}", render_human_readable(action, &data));
            Ok(())
        }
    }
}

fn parse_json_value_or_string(value: String) -> serde_json::Value {
    serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
}
