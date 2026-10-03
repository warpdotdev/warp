use comfy_table::Cell;
use cynic::QueryBuilder;
use serde::Serialize;
use warp_cli::GlobalOptions;
use warp_cli::mcp::MCPCommand;
use warp_graphql::queries::managed_mcp_servers::{
    ManagedMcpServer, ManagedMcpServersInput, ManagedMcpServersQuery, ManagedMcpServersResult,
    ManagedMcpServersVariables, ManagedMcpStatus,
};
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::ai::agent_sdk::output::{self, TableFormat};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};
use crate::server::server_api::ServerApiProvider;

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
        let server_api = ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(initial_sync, move |_, _, ctx| {
            let mut local_servers = TemplatableMCPServerManager::get_all_runnable_mcp_servers(ctx);
            local_servers.sort_by_key(|(uuid, _)| *uuid);

            let mut servers: Vec<MCPServerInfo> = local_servers
                .into_iter()
                .map(|(uuid, name)| MCPServerInfo::local(uuid, name))
                .collect();

            // Managed MCP servers (team/user-owned installations) live entirely behind
            // GraphQL — there's no REST list endpoint by design — so fetch them
            // separately and merge them into the same listing, clearly labeled.
            let operation = ManagedMcpServersQuery::build(ManagedMcpServersVariables {
                input: ManagedMcpServersInput::default(),
                request_context: get_request_context(),
            });
            let fetch_managed =
                async move { server_api.send_graphql_request(operation, None).await };

            ctx.spawn(fetch_managed, move |_, result, ctx| {
                match result {
                    Ok(response) => match response.managed_mcp_servers {
                        ManagedMcpServersResult::ManagedMcpServersOutput(output) => {
                            let mut managed_servers: Vec<ManagedMcpServer> = output.servers;
                            managed_servers.sort_by(|a, b| a.display_name.cmp(&b.display_name));
                            servers.extend(managed_servers.into_iter().map(MCPServerInfo::managed));
                        }
                        ManagedMcpServersResult::UserFacingError(error) => {
                            log::warn!(
                                "Failed to fetch managed MCP servers: {}; showing local installs only",
                                get_user_facing_error_message(error)
                            );
                        }
                        ManagedMcpServersResult::Unknown => {
                            log::warn!(
                                "Failed to fetch managed MCP servers: unknown server error; showing local installs only"
                            );
                        }
                    },
                    Err(err) => {
                        log::warn!(
                            "Failed to fetch managed MCP servers: {err}; showing local installs only"
                        );
                    }
                }

                output::print_list(servers, global_options.output_format);

                ctx.terminate_app(warpui::platform::TerminationMode::ForceTerminate, None);
            });
        });
    }
}

impl warpui::Entity for MCPCommandRunner {
    type Event = ();
}
impl SingletonEntity for MCPCommandRunner {}

/// Where an MCP server entry comes from: a local templatable install, or a
/// team/user-managed installation (see the `oauth-managed-mcp` server spec).
/// Local installs and managed installs are both usable with `agent run --mcp`,
/// but only managed installs carry a lifecycle [`ManagedMcpStatus`].
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum MCPServerSource {
    Local,
    Managed,
}

impl std::fmt::Display for MCPServerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MCPServerSource::Local => write!(f, "local"),
            MCPServerSource::Managed => write!(f, "managed"),
        }
    }
}

/// MCP server information that's shown in the `list` command.
#[derive(Serialize)]
struct MCPServerInfo {
    uuid: String,
    name: String,
    source: MCPServerSource,
    /// Lifecycle status, e.g. whether a managed server is usable yet. Empty
    /// for local installs, which have no equivalent concept.
    status: Option<&'static str>,
}

impl MCPServerInfo {
    fn local(uuid: uuid::Uuid, name: String) -> Self {
        MCPServerInfo {
            uuid: uuid.to_string(),
            name,
            source: MCPServerSource::Local,
            status: None,
        }
    }

    fn managed(server: ManagedMcpServer) -> Self {
        MCPServerInfo {
            uuid: server.uid.into_inner(),
            name: server.display_name,
            source: MCPServerSource::Managed,
            status: Some(match server.status {
                ManagedMcpStatus::Draft => "draft",
                ManagedMcpStatus::Active => "active",
                ManagedMcpStatus::Error => "error",
            }),
        }
    }
}

impl TableFormat for MCPServerInfo {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("UUID"),
            Cell::new("Name"),
            Cell::new("Source"),
            Cell::new("Status"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.uuid),
            Cell::new(&self.name),
            Cell::new(self.source),
            Cell::new(self.status.unwrap_or("-")),
        ]
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
