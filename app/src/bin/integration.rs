use anyhow::Result;
use clap::Parser;
use warp_cli::WorkerCommand;
use warp_core::channel::{Channel, ChannelConfig, ChannelState, OzConfig, WarpServerConfig};
use warp_core::AppId;

#[derive(Debug, Default, Parser, Clone)]
#[command(name = "warp-integration")]
#[clap(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[command(subcommand)]
    command: Option<WorkerCommand>,
}

pub fn main() -> Result<()> {
    ChannelState::set(ChannelState::new(
        Channel::Integration,
        ChannelConfig {
            app_id: AppId::new(
                "dev",
                "warp",
                if cfg!(target_os = "macos") {
                    "Warp-Integration"
                } else {
                    "WarpIntegration"
                },
            ),
            logfile_name: "warp_integration.log".into(),
            server_config: WarpServerConfig {
                firebase_auth_api_key: "".into(),
                // Use an IP in the IANA testing range, with the TCP discard port, to
                // black-hole server traffic.
                server_root_url: "http://192.0.2.0:9".into(),
                rtc_server_url: "ws://192.0.2.0:9/graphql/v2".into(),
                session_sharing_server_url: None,
            },
            oz_config: OzConfig {
                // Use an IP in the IANA testing range, with the TCP discard port, to
                // black-hole server traffic.
                oz_root_url: "http://192.0.2.0:9".into(),
                workload_audience_url: None,
            },
            telemetry_config: None,
            crash_reporting_config: None,
            autoupdate_config: None,
            mcp_static_config: None,
        },
    ));

    let args = Args::parse();

    if let Some(command) = &args.command {
        match command {
            #[cfg(unix)]
            WorkerCommand::TerminalServer(args) => {
                // If we were asked to run as a terminal server (as opposed to the main
                // GUI application), do so.  This must occur before init_logging, as the
                // terminal server sets up its own logger, and attempting to set a second
                // logger leads to a panic.
                warp::terminal::local_tty::server::run_terminal_server(args);
                return Ok(());
            }
            #[cfg(not(target_family = "wasm"))]
            WorkerCommand::RemoteServerProxy(_) | WorkerCommand::RemoteServerDaemon(_) => {
                return warp::run();
            }
            // This is a catch-all to handle the plugin host, which the integration test crate doesn't have a feature flag for.
            #[allow(unreachable_patterns)]
            other => panic!("Worker not supported in integration tests: {other:?}"),
        }
    }

    warp::run()
}
