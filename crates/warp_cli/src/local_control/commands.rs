//! Implementations for user-facing `warpctrl` command groups.
use local_control::protocol::{
    Action, ActionKind, ActionMetadata, ControlError, ErrorCode, RequestEnvelope,
};
use local_control::selection::select_instance;
use serde::Serialize;

use crate::agent::OutputFormat;
use crate::local_control::output::{write_json, write_json_line};
use crate::local_control::selectors::instance_selector;
use crate::local_control::{AppCommand, InstanceCommand, TabCommand, TargetArgs};

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
    }
}

pub(super) fn run_app_command(
    command: AppCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        AppCommand::Ping(args) => run_action(args, ActionKind::AppPing, output_format),
        AppCommand::Version(args) => run_action(args, ActionKind::AppVersion, output_format),
    }
}
pub(super) fn run_tab_command(
    command: TabCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        TabCommand::Create(args) => run_action(args, ActionKind::TabCreate, output_format),
    }
}

fn run_action(
    args: TargetArgs,
    action: ActionKind,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    let records = local_control::discovery::list_instances();
    let selector = instance_selector(args);
    let instance = select_instance(&records, &selector)?;
    let request = RequestEnvelope::new(Action::new(action));
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
