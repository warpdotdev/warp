use clap::{Args, Subcommand};

use crate::config_file::ConfigFileArgs;
use crate::environment::{EnvironmentCreateArgs, EnvironmentUpdateArgs};
use crate::mcp::MCPSpec;
use crate::model::ModelArgs;
use crate::provider::ProviderType;

/// Integration-related subcommands.
#[derive(Debug, Clone, Subcommand)]
#[command(visible_alias = "i")]
pub enum IntegrationCommand {
    /// Create a new integration.
    Create(CreateIntegrationArgs),
    /// Update an integration.
    Update(UpdateIntegrationArgs),
    /// List simple integrations and their connection status.
    List,
}

impl IntegrationCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            IntegrationCommand::Create(_) => "integration create",
            IntegrationCommand::Update(_) => "integration update",
            IntegrationCommand::List => "integration list",
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct CreateIntegrationArgs {
    /// Provider to create the integration for.
    #[arg(value_enum)]
    pub provider: ProviderType,

    #[command(flatten)]
    pub model: ModelArgs,

    #[clap(flatten)]
    pub environment: EnvironmentCreateArgs,

    #[command(flatten)]
    pub config_file: ConfigFileArgs,

    /// MCP servers to configure for this integration.
    ///
    /// Can be specified as:
    /// - A path to a JSON file containing MCP configuration
    /// - Inline JSON with MCP server configuration
    ///
    /// Can be specified multiple times to include multiple servers.
    #[arg(long = "mcp", value_name = "SPEC")]
    pub mcp_specs: Vec<MCPSpec>,

    /// Custom instructions for the integration.
    #[arg(long = "prompt", short = 'p')]
    pub prompt: Option<String>,

    /// Worker host ID for self-hosted workers.
    /// If not specified or set to "warp", tasks will run on Warp-hosted workers.
    #[arg(long = "host")]
    pub worker_host: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateIntegrationArgs {
    /// Provider to update the integration for.
    #[arg(value_enum)]
    pub provider: ProviderType,

    #[command(flatten)]
    pub model: ModelArgs,

    #[command(flatten)]
    pub environment: EnvironmentUpdateArgs,

    #[command(flatten)]
    pub config_file: ConfigFileArgs,

    /// MCP servers to configure for this integration.
    ///
    /// Can be specified as:
    /// - A path to a JSON file containing MCP configuration
    /// - Inline JSON with MCP server configuration
    ///
    /// Can be specified multiple times to include multiple servers.
    #[arg(long = "mcp", value_name = "SPEC")]
    pub mcp_specs: Vec<MCPSpec>,

    /// Remove MCP servers from this integration by server name.
    ///
    /// This removes the server entry whose key matches `SERVER_NAME`.
    #[arg(long = "remove-mcp", value_name = "SERVER_NAME")]
    pub remove_mcp: Vec<String>,

    /// Custom instructions for the integration.
    #[arg(long = "prompt", short = 'p')]
    pub prompt: Option<String>,

    /// Worker host ID for self-hosted workers.
    /// If not specified or set to "warp", tasks will run on Warp-hosted workers.
    #[arg(long = "host")]
    pub worker_host: Option<String>,
}
