use comfy_table::Cell;
use serde::Serialize;
use warp_cli::mcp::MCPCommand;
use warp_cli::GlobalOptions;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::ai::agent_sdk::output::{self, TableFormat};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::server::cloud_objects::update_manager::UpdateManager;

/// Handle MCP-related CLI commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: MCPCommand,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| MCPCommandRunner);
    match command {
        MCPCommand::List => {
            runner.update(ctx, |runner, ctx| runner.list(global_options, ctx));
            Ok(())
        }
    }
}

/// Singleton model for running async work as part of MCP CLI commands.
struct MCPCommandRunner;

impl MCPCommandRunner {
    fn list(&self, global_options: GlobalOptions, ctx: &mut ModelContext<Self>) {
        let initial_sync = UpdateManager::as_ref(ctx).initial_load_complete();

        ctx.spawn(initial_sync, move |_, _, ctx| {
            let mut servers = TemplatableMCPServerManager::get_all_runnable_mcp_servers(ctx);
            servers.sort_by_key(|(uuid, _)| *uuid);

            output::print_list(
                servers
                    .into_iter()
                    .map(|(uuid, name)| MCPServerInfo { uuid, name }),
                global_options.output_format,
            );

            ctx.terminate_app(warpui::platform::TerminationMode::ForceTerminate, None);
        });
    }
}

impl warpui::Entity for MCPCommandRunner {
    type Event = ();
}
impl SingletonEntity for MCPCommandRunner {}

/// MCP server information that's shown in the `list` command.
#[derive(Serialize)]
struct MCPServerInfo {
    uuid: uuid::Uuid,
    name: String,
}

impl TableFormat for MCPServerInfo {
    fn header() -> Vec<Cell> {
        vec![Cell::new("UUID"), Cell::new("Name")]
    }

    fn row(&self) -> Vec<Cell> {
        vec![Cell::new(self.uuid), Cell::new(&self.name)]
    }
}
