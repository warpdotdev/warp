use std::io::Write as _;

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
use local_control::protocol::{Action, ActionKind, ControlResponse, RequestEnvelope};
use local_control::selection::{InstanceSelector, select_instance};
use serde::Serialize;
use serde_json::json;

use crate::agent::OutputFormat;

#[derive(Debug, Parser)]
#[command(
    name = "warpctrl",
    display_name = "warpctrl",
    about = "Control a running local Warp app instance"
)]
pub struct ControlArgs {
    /// Set the output format.
    #[arg(
        long = "output-format",
        global = true,
        value_enum,
        default_value_t = OutputFormat::Pretty,
        env = "WARP_OUTPUT_FORMAT"
    )]
    pub output_format: OutputFormat,

    #[command(subcommand)]
    pub command: ControlCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ControlCommand {
    /// Inspect local Warp app instances.
    #[command(subcommand)]
    Instance(InstanceCommand),

    /// Control local Warp tabs.
    #[command(subcommand)]
    Tab(TabCommand),
}

#[derive(Debug, Clone, Subcommand)]
pub enum InstanceCommand {
    /// List locally discoverable Warp instances.
    List,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TabCommand {
    /// Create a new terminal tab in the active window.
    Create(TargetArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct TargetArgs {
    /// Target a specific local Warp instance id from `warp instance list`.
    #[arg(long = "instance")]
    pub instance: Option<String>,

    /// Target a specific local Warp process id.
    #[arg(long = "pid", conflicts_with = "instance")]
    pub pid: Option<u32>,
}

#[derive(Serialize)]
struct InstanceSummary {
    instance_id: String,
    pid: u32,
    channel: String,
    app_id: String,
    app_version: Option<String>,
    started_at: String,
    endpoint: local_control::discovery::ControlEndpoint,
    actions: Vec<String>,
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
            actions: record
                .actions
                .into_iter()
                .map(|metadata| metadata.name)
                .collect(),
        }
    }
}

pub fn run(args: ControlArgs) -> anyhow::Result<()> {
    let output_format = args.output_format;
    match args.command {
        ControlCommand::Instance(command) => run_instance_command(command, output_format),
        ControlCommand::Tab(command) => run_tab_command(command, output_format),
    }
}

fn run_instance_command(
    command: InstanceCommand,
    output_format: OutputFormat,
) -> anyhow::Result<()> {
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
                        println!(
                            "{}\tpid={}\t{}\t{}:{}",
                            summary.instance_id,
                            summary.pid,
                            summary.channel,
                            summary.endpoint.host,
                            summary.endpoint.port
                        );
                    }
                    Ok(())
                }
            }
        }
    }
}

fn run_tab_command(command: TabCommand, output_format: OutputFormat) -> anyhow::Result<()> {
    match command {
        TabCommand::Create(args) => {
            run_action(args, ActionKind::TabCreate, json!({}), output_format)
        }
    }
}

fn run_action(
    args: TargetArgs,
    action: ActionKind,
    params: serde_json::Value,
    output_format: OutputFormat,
) -> anyhow::Result<()> {
    let records = local_control::discovery::list_instances();
    let selector = instance_selector(args);
    let instance = select_instance(&records, &selector)?;
    let request = RequestEnvelope::new(Action {
        kind: action,
        params,
    });
    let response = local_control::client::send_request(&instance, &request)?;
    let ControlResponse::Ok { data } = response.response else {
        anyhow::bail!("local-control request failed without an error payload");
    };
    match output_format {
        OutputFormat::Json => write_json(&data),
        OutputFormat::Ndjson => write_json_line(&data),
        OutputFormat::Pretty | OutputFormat::Text => {
            write_json(&data).context("unable to print local-control data")
        }
    }
}

fn instance_selector(args: TargetArgs) -> InstanceSelector {
    if let Some(instance_id) = args.instance {
        return InstanceSelector::Id(local_control::discovery::InstanceId(instance_id));
    }
    if let Some(pid) = args.pid {
        return InstanceSelector::Pid(pid);
    }
    InstanceSelector::Active
}

fn write_json(value: &impl Serialize) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer_pretty(&mut lock, value)?;
    writeln!(&mut lock)?;
    Ok(())
}
fn write_json_line(value: &impl Serialize) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, value)?;
    writeln!(&mut lock)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;

    #[test]
    fn parses_first_slice_tab_create() {
        let args =
            ControlArgs::try_parse_from(["warpctrl", "tab", "create", "--instance", "inst_123"])
                .expect("tab create parses");
        let ControlCommand::Tab(TabCommand::Create(target)) = args.command else {
            panic!("expected tab create command");
        };
        assert_eq!(target.instance.as_deref(), Some("inst_123"));
    }

    #[test]
    fn parses_first_slice_instance_list() {
        let args = ControlArgs::try_parse_from(["warpctrl", "instance", "list"])
            .expect("instance list parses");
        assert!(matches!(
            args.command,
            ControlCommand::Instance(InstanceCommand::List)
        ));
    }

    #[test]
    fn rejects_future_catalog_commands_not_in_first_slice() {
        assert!(ControlArgs::try_parse_from(["warpctrl", "window", "list"]).is_err());
        assert!(ControlArgs::try_parse_from(["warpctrl", "app", "ping"]).is_err());
        assert!(ControlArgs::try_parse_from(["warpctrl", "tab", "list"]).is_err());
        assert!(ControlArgs::try_parse_from(["warpctrl", "setting", "list"]).is_err());
    }
}
