mod native;
pub use native::McpIntegration;
mod oauth;

use std::collections::HashMap;

use crate::ai::mcp::FileBasedMCPManager;
use crate::ai::mcp::{
    templatable_installation::TemplatableMCPServerInstallation, MCPServerState,
    TemplatableMCPServer,
};
use futures_util::stream::AbortHandle;
use uuid::Uuid;
use warpui::ModelSpawner;
use warpui::{Entity, SingletonEntity};

type ReconnectResultSender =
    tokio::sync::oneshot::Sender<Result<rmcp::Peer<rmcp::RoleClient>, String>>;

/// Singleton model to manage state of MCP server lifecycles and panes across multiple windows
/// (where only one MCP server pane can exist per window).
///
/// Specifically:
/// - Maintains MCP server view handles to preserve state when panes are hidden
/// - Tracks currently open MCP server panes and their location
///
/// The core implementation is in the native module.
#[derive(Default)]
pub struct TemplatableMCPServerManager {
    templatable_mcp_servers: HashMap<Uuid, TemplatableMCPServer>,
    locally_installed_servers: HashMap<Uuid, TemplatableMCPServerInstallation>,
    server_states: HashMap<Uuid, MCPServerState>,
    active_servers: HashMap<Uuid, TemplatableMCPServerInfo>,

    spawned_servers: HashMap<Uuid, SpawnedServerInfo>,
    /// Cached credentials for each server.
    ///
    /// We persist these to secure storage, and if they are present when the server is started,
    /// we use them instead of going through the OAuth flow again.
    server_credentials: oauth::PersistedCredentialsMap,
    /// Cached credentials for file-based servers, keyed by installation hash.
    file_based_server_credentials: oauth::FileBasedPersistedCredentialsMap,
    credentials_loaded_from_secure_storage: bool,
    /// Error messages for failed servers, keyed by installation UUID.
    server_error_messages: HashMap<Uuid, String>,
    /// Spawner for running tasks in the context of this manager.
    ///
    /// Used by `ReconnectingPeer` to trigger reconnection from async contexts.
    spawner: Option<ModelSpawner<Self>>,
    /// Pending reconnection waiters, keyed by installation UUID.
    ///
    /// When a reconnection is in progress, subsequent reconnect requests for the same server
    /// will add their result channels here instead of starting a new reconnection. When the
    /// reconnection completes, all waiters are notified with the result.
    pending_reconnections: HashMap<Uuid, Vec<ReconnectResultSender>>,
    /// Maps the OAuth CSRF `state` token to the installation UUID of the server whose
    /// authorization flow is in progress.
    ///
    /// Populated just before opening the authorization URL; removed once the callback
    /// is received or the spawn task terminates.
    pending_oauth_csrf: HashMap<String, Uuid>,
}

/// Information about a spawned server task.
struct SpawnedServerInfo {
    abort_handle: AbortHandle,
    oauth_result_tx: async_channel::Sender<oauth::CallbackResult>,
}

/// Information about a single connected MCP server.
pub struct TemplatableMCPServerInfo {
    name: String,
    service: rmcp::service::RunningService<
        rmcp::RoleClient,
        Box<dyn rmcp::service::DynService<rmcp::RoleClient>>,
    >,
    resources: Vec<rmcp::model::Resource>,
    tools: Vec<rmcp::model::Tool>,
    installation_id: Uuid,
    description: Option<String>,
    /// Whether the underlying transport uses authentication.
    ///
    /// TODO(vorporeal): Use this to display a toast when MCP transport authentication and connection is complete, and
    /// to provide a "log out" button.
    #[allow(dead_code)]
    is_authenticated_transport: bool,
}

impl TemplatableMCPServerInfo {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn resources(&self) -> &Vec<rmcp::model::Resource> {
        &self.resources
    }

    pub fn tools(&self) -> &Vec<rmcp::model::Tool> {
        &self.tools
    }

    pub fn installation_id(&self) -> Uuid {
        self.installation_id
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// The current status of the Figma MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FigmaMcpStatus {
    /// The Figma MCP server is not installed.
    NotInstalled,
    /// The Figma MCP server is installed but not currently running.
    Installed,
    /// The Figma MCP server is in the process of enabling (e.g. OAuth flow in progress).
    Enabling,
    /// The Figma MCP server is running.
    Running,
}

impl TemplatableMCPServerManager {
    pub fn get_installed_templatable_servers(
        &self,
    ) -> &HashMap<Uuid, TemplatableMCPServerInstallation> {
        &self.locally_installed_servers
    }

    pub fn get_installed_server(
        &self,
        installation_uuid: &Uuid,
    ) -> Option<&TemplatableMCPServerInstallation> {
        self.locally_installed_servers.get(installation_uuid)
    }

    /// Returns the UUID of the locally-installed Figma MCP server installation, if any.
    pub fn get_figma_installation_uuid(&self) -> Option<Uuid> {
        self.locally_installed_servers
            .iter()
            .find(|(_, installation)| {
                installation
                    .template_json()
                    .contains("https://mcp.figma.com/mcp")
            })
            .map(|(uuid, _)| *uuid)
    }

    /// Returns the current status of the Figma MCP server.
    pub fn get_figma_mcp_status(&self) -> FigmaMcpStatus {
        let Some(uuid) = self.get_figma_installation_uuid() else {
            return FigmaMcpStatus::NotInstalled;
        };
        if self.active_servers.contains_key(&uuid) {
            FigmaMcpStatus::Running
        } else if self.spawned_servers.contains_key(&uuid) {
            FigmaMcpStatus::Enabling
        } else {
            FigmaMcpStatus::Installed
        }
    }

    pub fn get_template_uuid(&self, installation_uuid: Uuid) -> Option<Uuid> {
        self.locally_installed_servers
            .get(&installation_uuid)
            .map(|server_installation| server_installation.template_uuid())
    }
    pub fn is_server_active(&self, installation_uuid: Uuid) -> bool {
        self.active_servers.contains_key(&installation_uuid)
    }
    pub fn is_server_active_or_pending(&self, uuid: Uuid) -> bool {
        self.is_server_active(uuid) || self.spawned_servers.contains_key(&uuid)
    }

    pub fn get_server_state(&self, installation_uuid: Uuid) -> Option<MCPServerState> {
        self.server_states.get(&installation_uuid).copied()
    }

    pub fn get_server_error_message(&self, installation_uuid: Uuid) -> Option<&str> {
        self.server_error_messages
            .get(&installation_uuid)
            .map(|s| s.as_str())
    }

    pub fn resources(&self) -> impl Iterator<Item = &rmcp::model::Resource> {
        self.active_servers
            .values()
            .flat_map(|server| server.resources.iter())
    }

    pub fn tools(&self) -> impl Iterator<Item = &rmcp::model::Tool> {
        self.active_servers
            .values()
            .flat_map(|server| server.tools.iter())
    }

    /// Returns a reconnecting peer for a server that has the given resource.
    ///
    /// The returned peer will automatically reconnect if the underlying transport is closed.
    pub fn server_with_resource(
        &self,
        resource: &rmcp::model::Resource,
    ) -> Option<super::reconnecting_peer::ReconnectingPeer> {
        let spawner = self.spawner.as_ref()?;
        self.active_servers
            .iter()
            .find(|(_, server)| {
                server
                    .resources
                    .iter()
                    .any(|other_resource| resource.uri == other_resource.uri)
            })
            .map(|(installation_uuid, _)| {
                super::reconnecting_peer::ReconnectingPeer::new(*installation_uuid, spawner.clone())
            })
    }

    pub fn tools_for_server(&self, uuid: Uuid) -> Vec<rmcp::model::Tool> {
        self.active_servers
            .get(&uuid)
            .map(|server| server.tools.clone())
            .unwrap_or_default()
    }

    /// Returns the JSON Schema `input_schema` for a named tool across active MCP servers.
    ///
    /// If `installation_id` is `Some`, only that server is considered; otherwise, the
    /// first active server providing a matching tool name wins (matching the existing
    /// `server_with_tool_name` lookup behavior).
    ///
    /// Used by the MCP tool executor to coerce integer-typed args before dispatch, since
    /// `structpb.NumberValue` on the wire cannot preserve the integer/float distinction.
    /// See <https://json-schema.org/understanding-json-schema/reference/type>.
    pub fn tool_input_schema(
        &self,
        installation_id: Option<Uuid>,
        tool_name: &str,
    ) -> Option<std::sync::Arc<rmcp::model::JsonObject>> {
        let candidates: Box<dyn Iterator<Item = &TemplatableMCPServerInfo>> =
            if let Some(uuid) = installation_id {
                Box::new(self.active_servers.get(&uuid).into_iter())
            } else {
                Box::new(self.active_servers.values())
            };

        candidates
            .flat_map(|server| server.tools.iter())
            .find(|t| t.name == tool_name)
            .map(|t| t.input_schema.clone())
    }
    pub fn server_from_tool(&self, tool: String) -> Option<&Uuid> {
        self.active_servers
            .iter()
            .find(|(_, server)| server.tools.iter().any(|t| t.name == tool))
            .map(|(uuid, _)| uuid)
    }

    /// Returns the installation UUID of the server that provides a resource matching the given
    /// name or URI.
    pub fn server_from_resource(&self, name: &str, uri: Option<&str>) -> Option<&Uuid> {
        self.active_servers
            .iter()
            .find(|(_, server)| {
                server.resources.iter().any(|r| {
                    if let Some(uri) = uri {
                        r.uri == uri
                    } else {
                        r.name == name
                    }
                })
            })
            .map(|(uuid, _)| uuid)
    }

    /// Returns installed templatable servers that are currently active.
    pub fn get_active_templatable_servers(&self) -> HashMap<Uuid, &TemplatableMCPServerInfo> {
        self.locally_installed_servers
            .keys()
            .filter_map(|uuid| self.active_servers.get(uuid).map(|info| (*uuid, info)))
            .collect()
    }

    /// Returns file-based MCP servers that are currently active and in scope for the given working directory.
    pub fn get_active_file_based_servers(
        &self,
        cwd: &std::path::Path,
        app: &warpui::AppContext,
    ) -> HashMap<Uuid, &TemplatableMCPServerInfo> {
        FileBasedMCPManager::as_ref(app)
            .get_servers_for_working_directory(cwd, app)
            .iter()
            .filter_map(|installation| {
                let uuid = installation.uuid();
                self.active_servers.get(&uuid).map(|info| (uuid, info))
            })
            .collect()
    }
}

#[derive(Debug)]
pub enum TemplatableMCPServerManagerEvent {
    StateChanged,
    // TODO(aeybel) Right now most of the app doesn't use these events to communicate
    // We should change them so this manager is source of truth and all communication goes through here
    #[allow(dead_code)]
    ServerInstallationAdded(Uuid),
    #[allow(dead_code)]
    ServerInstallationDeleted(Uuid),
    TemplatableMCPServersUpdated,
}

impl Entity for TemplatableMCPServerManager {
    type Event = TemplatableMCPServerManagerEvent;
}

impl SingletonEntity for TemplatableMCPServerManager {}
