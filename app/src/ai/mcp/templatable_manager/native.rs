use crate::ai::mcp::file_based_manager::FileBasedMCPManagerEvent;
use crate::ai::mcp::templatable_manager::oauth::{
    load_credentials_from_secure_storage, write_to_secure_storage, FILE_BASED_MCP_CREDENTIALS_KEY,
    TEMPLATABLE_MCP_CREDENTIALS_KEY,
};
use crate::ai::mcp::FileBasedMCPManager;
use core::fmt;
use std::collections::HashSet;
use std::sync::Arc;
use std::{collections::HashMap, future::Future};

use crate::ai::mcp::http_client::build_client_with_headers;
use crate::ai::mcp::templatable::GalleryData;
use crate::ai::mcp::templatable_manager::FigmaMcpStatus;
use crate::ai::mcp::{
    Author, CloudMCPServer, JsonTemplate, MCPGalleryManager, MCPServerUpdate,
    ParsedTemplatableMCPServerResult,
};

use crate::ai::mcp::parsing::resolve_json;
use crate::ai::mcp::TemplatableMCPServer;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::{CloudObject, CloudObjectLocation, CloudObjectMetadataExt, Space};
use crate::server::cloud_objects::update_manager::InitiatedBy;
use crate::server::ids::{ClientId, ServerId};
use crate::server::telemetry::{
    MCPServerModel, MCPServerTelemetryTransportType, MCPTemplateCreationSource,
};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{
    ai::mcp::{
        logs, templatable::CloudTemplatableMCPServer, templatable_installation::VariableValue,
        MCPServer, StaticEnvVar, TemplatableMCPServerInstallation, TransportType,
    },
    cloud_object::{GenericStringObjectFormat, JsonObjectType},
    drive::CloudObjectTypeAndId,
    persistence::{
        database_file_path_for_scope, establish_ro_connection, ModelEvent, PersistenceScope,
    },
    send_telemetry_from_ctx,
    server::{
        cloud_objects::update_manager::UpdateManager, ids::SyncId, telemetry::TelemetryEvent,
    },
    settings::AISettings,
    view_components::DismissibleToast,
    workspace::ToastStack,
    GlobalResourceHandlesProvider,
};
use async_compat::CompatExt as _;
use cfg_if::cfg_if;
use futures::FutureExt as _;
use parking_lot::Mutex;
use rmcp::{transport::ConfigureCommandExt as _, ServiceExt as _};
use simple_logger::manager::LogManager;
use simple_logger::SimpleLogger;
use tokio::io::AsyncBufReadExt as _;
use uuid::Uuid;
use warp_core::safe_error;
use warp_core::{execution_mode::AppExecutionMode, features::FeatureFlag, settings::Setting as _};
use warpui::AppContext;
use warpui::{windowing::WindowManager, ModelContext, SingletonEntity};

use super::{
    oauth::{self, AuthContext, FileBasedPersistedCredentialsMap, PersistedCredentialsMap},
    utils::{query_resources_for, query_tools_for},
    MCPServerState, SpawnedServerInfo, TemplatableMCPServerInfo, TemplatableMCPServerManager,
    TemplatableMCPServerManagerEvent,
};

/// Controls the behavior of `spawn_server_impl`.
enum SpawnMode {
    /// Initial spawn - clears logs and sends telemetry.
    Initial {
        /// Whether to persist running state to SQLite.
        persist_running_state_to_sqlite: bool,
    },
    /// Reconnection after transport closed - preserves logs, no telemetry.
    ///
    /// Waiters are notified via `pending_reconnections` when the connection completes.
    Reconnect,
}

impl SpawnMode {
    fn should_send_telemetry(&self) -> bool {
        matches!(self, SpawnMode::Initial { .. })
    }

    fn should_persist_running_state_to_sqlite(&self) -> bool {
        matches!(
            self,
            SpawnMode::Initial {
                persist_running_state_to_sqlite: true
            }
        )
    }

    fn is_reconnect(&self) -> bool {
        matches!(self, SpawnMode::Reconnect)
    }
}

enum LegacyToTemplatableMCPConversionError {
    TemplateAlreadyExists,
    NoDBConnection,
    InstallationFailed,
}

impl fmt::Display for LegacyToTemplatableMCPConversionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::TemplateAlreadyExists => write!(f, "templatable MCP server already exists"),
            Self::NoDBConnection => write!(f, "failed to connect to database"),
            Self::InstallationFailed => write!(
                f,
                "created template successfully, but could not create installation"
            ),
        }
    }
}

/// Convert an rmcp error to a user-friendly error message.
fn error_to_user_message(error: &rmcp::RmcpError) -> String {
    match error {
        rmcp::RmcpError::ClientInitialize(err) => {
            format!("Failed to initialize client: {}", err)
        }
        rmcp::RmcpError::ServerInitialize(err) => {
            format!("Failed to initialize server: {}", err)
        }
        rmcp::RmcpError::TransportCreation { error, .. } => {
            format!("Failed to establish connection: {}", error)
        }
        rmcp::RmcpError::Runtime(err) => {
            format!("Runtime error: {}", err)
        }
        rmcp::RmcpError::Service(err) => match err {
            rmcp::ServiceError::McpError(_) => {
                "Server returned an error. Please check server logs for details.".to_string()
            }
            rmcp::ServiceError::TransportSend(_) => {
                "Failed to send data to server. Connection may have been lost.".to_string()
            }
            rmcp::ServiceError::TransportClosed => {
                "Connection closed unexpectedly. The server may have crashed.".to_string()
            }
            rmcp::ServiceError::UnexpectedResponse => {
                "Server sent an unexpected response. The server may be incompatible.".to_string()
            }
            rmcp::ServiceError::Cancelled { reason } => format!(
                "Operation was cancelled with reason: {}",
                reason.clone().unwrap_or("Unknown reason".to_string())
            ),
            rmcp::ServiceError::Timeout { timeout } => {
                format!(
                    "Connection timed out after {} seconds. The server may be unresponsive.",
                    timeout.as_secs()
                )
            }
            _ => format!("Service error: {}", err),
        },
        // The enum is marked as non-exhaustive, so we need a catch-all.
        _ => {
            format!("Error: {error}")
        }
    }
}

/// An MCP server integration that Warp ships with bundled skills for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpIntegration {
    Figma,
}

impl TemplatableMCPServerManager {
    /// Returns `true` if the given MCP integration is currently running.
    pub fn is_mcp_server_running(&self, integration: McpIntegration) -> bool {
        match integration {
            McpIntegration::Figma => self.get_figma_mcp_status() == FigmaMcpStatus::Running,
        }
    }

    /// Creates a new [`TemplatableMCPServerManager`] instance.
    pub fn new(
        locally_installed_servers: HashMap<Uuid, TemplatableMCPServerInstallation>,
        running_server_uuids: Vec<Uuid>,
        running_legacy_server_uuids: &[Uuid],
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Subscribe to FileBasedMCPManager events.
        let file_based_mcp_manager = FileBasedMCPManager::handle(ctx);
        ctx.subscribe_to_model(&file_based_mcp_manager, |me, event, ctx| match event {
            FileBasedMCPManagerEvent::SpawnServers { installations } => {
                me.spawn_file_based_servers(installations, ctx);
            }
            FileBasedMCPManagerEvent::DespawnServers { installation_uuids } => {
                me.despawn_file_based_servers(installation_uuids.clone(), ctx);
            }
            FileBasedMCPManagerEvent::PurgeCredentials {
                installation_hashes,
            } => {
                me.purge_file_based_server_credentials(installation_hashes, ctx);
            }
            // Notification for cloud-environment readiness; handled by the AgentDriver.
            FileBasedMCPManagerEvent::CloudEnvMcpScanComplete { .. } => {}
        });

        // TemplatableMCPServerManager is the source of truth for templatable MCP servers stored on the cloud
        let cloud_model = CloudModel::handle(ctx);
        ctx.subscribe_to_model(&cloud_model, |me, event, ctx| match event {
            CloudModelEvent::ObjectUpdated {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                        id: _,
                    },
                source: _,
            }
            | CloudModelEvent::ObjectTrashed {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                        id: _,
                    },
                source: _,
            }
            | CloudModelEvent::ObjectUntrashed {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                        id: _,
                    },
                source: _,
            }
            | CloudModelEvent::ObjectDeleted {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                        id: _,
                    },
                folder_id: _,
            }
            | CloudModelEvent::ObjectSynced {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                        id: _,
                    },
                client_id: _,
                server_id: _,
            }
            | CloudModelEvent::ObjectMoved {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                        id: _,
                    },
                source: _,
                from_folder: _,
                to_folder: _,
            } => {
                me.fetch_cloud_servers(ctx);
            },
            CloudModelEvent::ObjectCreated {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                        id: new_sync_id
                    },
            } => {
                log::debug!("A new MCP server template was found with sync id {new_sync_id}");
                if let Some(new_server) = CloudTemplatableMCPServer::get_by_id(new_sync_id, ctx) {
                    let uuid = new_server.model().string_model.uuid;
                    if let Some(legacy_server) = CloudMCPServer::get_by_uuid(&uuid, ctx) {
                        let old_sync_id = legacy_server.sync_id();
                        me.delete_legacy_mcp_server(old_sync_id, InitiatedBy::System, ctx);
                        log::info!("Successfully converted MCP server {old_sync_id} into {uuid} with sync id {new_sync_id}.");
                        ctx.emit(TemplatableMCPServerManagerEvent::LegacyServerConverted);
                    }
                }
                me.fetch_cloud_servers(ctx);
            },
            _ => {}
        });

        let database_connection = database_file_path_for_scope(&PersistenceScope::App)
            .to_str()
            .and_then(|db_url| {
                establish_ro_connection(db_url)
                    .ok()
                    .map(|conn| Arc::new(Mutex::new(conn)))
            });

        let mut me = Self {
            cloud_templatable_mcp_servers: Default::default(),
            server_states: Default::default(),
            active_servers: Default::default(),
            spawned_servers: Default::default(),
            server_credentials: Default::default(),
            file_based_server_credentials: Default::default(),
            locally_installed_servers,
            database_connection,
            server_error_messages: Default::default(),
            spawner: Some(ctx.spawner()),
            pending_reconnections: Default::default(),
            pending_oauth_csrf: Default::default(),
            cli_spawned_server_uuids: Default::default(),
        };

        me.fetch_cloud_servers(ctx);

        // If we're not in a test, try to load credentials from secure storage.
        if !cfg!(test) {
            me.server_credentials = load_credentials_from_secure_storage::<PersistedCredentialsMap>(
                ctx,
                TEMPLATABLE_MCP_CREDENTIALS_KEY,
            );

            if FeatureFlag::FileBasedMcp.is_enabled() {
                me.file_based_server_credentials = load_credentials_from_secure_storage::<
                    FileBasedPersistedCredentialsMap,
                >(
                    ctx, FILE_BASED_MCP_CREDENTIALS_KEY
                );
            }
        }

        if AppExecutionMode::as_ref(ctx).can_autostart_mcp_servers() {
            for installation_uuid in running_server_uuids {
                me.spawn_server(installation_uuid, ctx)
            }
        }

        // Migrate legacy MCPs to be templatables on app start. Uses UpdateManager
        let servers_to_restart: HashSet<Uuid> =
            running_legacy_server_uuids.iter().cloned().collect();
        me.convert_all_legacy_to_templatable(servers_to_restart, ctx);

        me
    }

    fn delete_orphaned_installations(&mut self, ctx: &mut ModelContext<Self>) {
        let orphaned_installations: Vec<Uuid> = self.locally_installed_servers
            .iter()
            .filter(|(_, installation)| {
                // Gallery-sourced installations don't have a corresponding cloud template
                // and should never be treated as orphans.
                installation.gallery_uuid().is_none()
                    && !self.cloud_templatable_mcp_servers.contains_key(&installation.template_uuid())
            })
            .map(|(installation_uuid, installation)| {
                log::info!("Deleting orphaned MCP server installation {installation_uuid} named {} with no corresponding cloud template {}", installation.templatable_mcp_server().name, installation.template_uuid());
                *installation_uuid
            })
            .collect();

        self.delete_templatable_mcp_server_installations(orphaned_installations, ctx);
    }

    fn fetch_cloud_servers(&mut self, ctx: &mut ModelContext<Self>) {
        self.cloud_templatable_mcp_servers = Self::get_cloud_servers(ctx);
        self.delete_orphaned_installations(ctx);
        ctx.emit(TemplatableMCPServerManagerEvent::TemplatableMCPServersUpdated);
    }

    fn get_cloud_servers(ctx: &mut ModelContext<Self>) -> HashMap<Uuid, CloudTemplatableMCPServer> {
        let cloud_templatable_mcp_servers: Vec<CloudTemplatableMCPServer> =
            CloudTemplatableMCPServer::get_all(ctx);
        cloud_templatable_mcp_servers
            .into_iter()
            .map(|server| (server.model().string_model.uuid, server))
            .collect()
    }

    pub fn get_cloud_server(
        &self,
        template_uuid: Uuid,
        _ctx: &mut ModelContext<Self>,
    ) -> Option<&CloudTemplatableMCPServer> {
        self.cloud_templatable_mcp_servers.get(&template_uuid)
    }

    pub fn is_server_installation_shared(&self, installation_uuid: Uuid, app: &AppContext) -> bool {
        match self.get_installed_server(&installation_uuid) {
            Some(installation) => self.is_server_template_shared(installation.template_uuid(), app),
            None => false,
        }
    }

    pub fn change_server_state(
        &mut self,
        installation_uuid: Uuid,
        new_state: MCPServerState,
        ctx: &mut ModelContext<Self>,
    ) {
        // If a server is rapidly stopped and started there is a race condition
        // Checking the server's actual status helps us avoid this
        if matches!(new_state, MCPServerState::NotRunning)
            && (self.is_server_active(installation_uuid)
                || self.is_server_active_or_pending(installation_uuid))
        {
            return;
        }
        self.server_states.insert(installation_uuid, new_state);
        ctx.emit(TemplatableMCPServerManagerEvent::StateChanged {
            uuid: installation_uuid,
            state: new_state,
        });
    }

    pub fn is_server_template_shared(&self, template_uuid: Uuid, app: &AppContext) -> bool {
        match self.get_space(template_uuid, app) {
            Some(Space::Personal) => false,
            Some(Space::Team { team_uid: _ }) => true,
            Some(Space::Shared) => true,
            None => false,
        }
    }

    fn get_space(&self, template_uuid: Uuid, app: &AppContext) -> Option<Space> {
        self.cloud_templatable_mcp_servers
            .get(&template_uuid)
            .map(|template| template.space(app))
    }

    /// Gets a CloudTemplatableMCPServer by its UUID.
    /// Returns the CloudTemplatableMCPServer model if found, otherwise None.
    pub fn get_cloud_templatable_mcp_server(
        &self,
        uuid: Uuid,
    ) -> Option<&CloudTemplatableMCPServer> {
        self.cloud_templatable_mcp_servers.get(&uuid)
    }

    pub fn get_creator(&self, template_uuid: Uuid, app: &AppContext) -> Option<String> {
        let server = self.get_cloud_templatable_mcp_server(template_uuid);
        server.map(|server| server.metadata().semantic_creator(app))?
    }

    /// Gets a TemplatableMCPServer by its UUID.
    /// Returns the TemplatableMCPServer model if found, otherwise None.
    pub fn get_templatable_mcp_server(&self, uuid: Uuid) -> Option<&TemplatableMCPServer> {
        self.get_cloud_templatable_mcp_server(uuid)
            .map(|server| &server.model().string_model)
    }

    /// Creates a new templatable MCP server in the specified space.
    pub fn create_templatable_mcp_server(
        &mut self,
        templatable_mcp_server: TemplatableMCPServer,
        space: Space,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) {
        let owner = UserWorkspaces::as_ref(ctx).space_to_owner(space, ctx);
        if let Some(owner) = owner {
            let update_manager = UpdateManager::handle(ctx);
            update_manager.update(ctx, |update_manager, ctx| {
                let client_id = ClientId::default();
                update_manager.create_templatable_mcp_server(
                    templatable_mcp_server.clone(),
                    client_id,
                    owner,
                    initiated_by,
                    ctx,
                );
            });
        }
    }

    pub fn get_all_templatable_mcp_servers(&self) -> Vec<&TemplatableMCPServer> {
        self.cloud_templatable_mcp_servers
            .values()
            .map(|server| &server.model().string_model)
            .collect()
    }

    pub fn update_templatable_mcp_server(
        &mut self,
        template_server: TemplatableMCPServer,
        ctx: &mut ModelContext<Self>,
    ) {
        let cloud_templatable_mcp_server =
            self.get_cloud_templatable_mcp_server(template_server.uuid);
        if let Some(cloud_templatable_mcp_server) = cloud_templatable_mcp_server {
            let update_manager = UpdateManager::handle(ctx);
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.update_templatable_mcp_server(
                    template_server,
                    cloud_templatable_mcp_server.id,
                    cloud_templatable_mcp_server.metadata.revision.clone(),
                    ctx,
                );
            });
        }
    }

    pub fn delete_templatable_mcp_server(&mut self, uuid: Uuid, ctx: &mut ModelContext<Self>) {
        // Delete any existing installations of this template
        let installation = self.get_installation_by_template_uuid(uuid);
        if let Some(installation) = installation {
            let installation_uuid = installation.uuid();
            self.delete_credentials_from_secure_storage(installation_uuid, ctx);
            self.delete_templatable_mcp_server_installation(installation_uuid, ctx);
        }

        let cloud_templatable_mcp_server = self.get_cloud_templatable_mcp_server(uuid);
        if let Some(cloud_templatable_mcp_server) = cloud_templatable_mcp_server {
            let cloud_object_type_and_id = CloudObjectTypeAndId::GenericStringObject {
                object_type: GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                id: cloud_templatable_mcp_server.id,
            };

            let update_manager = UpdateManager::handle(ctx);
            update_manager.update(ctx, |update_manager, ctx| {
                update_manager.delete_object_by_user(cloud_object_type_and_id, ctx);
            });
        }
    }

    pub fn delete_legacy_mcp_server(
        &mut self,
        sync_id: SyncId,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) {
        // The legacy MCPServerManager no longer runs servers, so we only need
        // to delete the cloud object. OAuth credentials were already copied
        // during conversion.
        let cloud_object_type_and_id = CloudObjectTypeAndId::GenericStringObject {
            object_type: GenericStringObjectFormat::Json(JsonObjectType::MCPServer),
            id: sync_id,
        };

        let update_manager = UpdateManager::handle(ctx);
        update_manager.update(ctx, |update_manager, ctx| {
            update_manager.delete_object_with_initiated_by(
                cloud_object_type_and_id,
                initiated_by,
                ctx,
            );
        });
    }

    /// Get all runnable MCP servers (templatable installations).
    pub fn get_all_runnable_mcp_servers(ctx: &AppContext) -> Vec<(Uuid, String)> {
        TemplatableMCPServerManager::as_ref(ctx)
            .get_installed_templatable_servers()
            .iter()
            .map(|(uuid, installation)| (*uuid, installation.templatable_mcp_server().name.clone()))
            .collect()
    }

    /// Get all cloud synced MCP servers (templatable templates).
    pub fn get_all_cloud_synced_mcp_servers(ctx: &AppContext) -> HashMap<Uuid, String> {
        TemplatableMCPServerManager::as_ref(ctx)
            .get_all_templatable_mcp_servers()
            .iter()
            .map(|&server| (server.uuid, server.name.clone()))
            .collect()
    }

    /// Get the name for an MCP server based on uuid.
    pub fn get_mcp_name(uuid: &Uuid, app: &AppContext) -> Option<String> {
        TemplatableMCPServerManager::as_ref(app)
            .get_installed_server_name(uuid)
            .or_else(|| TemplatableMCPServerManager::as_ref(app).get_template_server_name(uuid))
    }

    /// Extracts some piece of server info for all servers (template & installation) and returns it in a HashSet.
    pub fn extract_server_info<T: std::cmp::Eq + std::hash::Hash>(
        &self,
        template_fn: fn(&TemplatableMCPServer) -> Option<T>,
        installation_fn: fn(&TemplatableMCPServerInstallation) -> Option<T>,
        _app: &AppContext,
    ) -> HashSet<T> {
        let template_results = self
            .get_all_templatable_mcp_servers()
            .into_iter()
            .filter_map(template_fn);

        let installation_results = self
            .get_installed_templatable_servers()
            .values()
            .filter_map(installation_fn);

        template_results
            .chain(installation_results)
            .collect::<HashSet<T>>()
    }

    fn get_installed_server_name(&self, installation_uuid: &Uuid) -> Option<String> {
        self.get_installed_server(installation_uuid)
            .map(|server| server.templatable_mcp_server().name.clone())
    }

    fn get_template_server_name(&self, template_uuid: &Uuid) -> Option<String> {
        self.get_templatable_mcp_server(*template_uuid)
            .map(|template| template.name.clone())
    }

    fn persist_is_mcp_running(
        installation_uuid: Uuid,
        running: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get();

        let Some(sender) = &global_resource_handles.model_event_sender else {
            return;
        };
        let event = ModelEvent::UpdateMCPInstallationRunning {
            installation_uuid,
            running,
        };
        if let Err(err) = sender.send(event) {
            log::error!(
                "Failed to save TemplatableMCPServerInstallation running status to database: {err}"
            );
        }
    }

    /// Spawns an ephemeral MCP server from a given [`TemplatableMCPServerInstallation`].
    ///
    /// Unlike `spawn_server`, this method takes the installation directly and does not
    /// persist it to the database. The server exists only for the duration of the current
    /// session (e.g., an agent CLI run).
    pub fn spawn_ephemeral_server(
        &mut self,
        installation: TemplatableMCPServerInstallation,
        ctx: &mut ModelContext<Self>,
    ) {
        let installation_uuid = installation.uuid();
        log::debug!("Spawning ephemeral server with installation_uuid {installation_uuid}");

        self.spawn_server_impl(
            installation,
            SpawnMode::Initial {
                persist_running_state_to_sqlite: false,
            },
            ctx,
        );
    }

    /// Spawns an ephemeral MCP server started via the CLI (`oz agent run --mcp`).
    pub fn spawn_cli_ephemeral_server(
        &mut self,
        installation: TemplatableMCPServerInstallation,
        ctx: &mut ModelContext<Self>,
    ) {
        self.cli_spawned_server_uuids.insert(installation.uuid());
        self.spawn_ephemeral_server(installation, ctx);
    }

    /// Spawns a new MCP server from a given installation UUID.
    ///
    /// This looks up the installation from `locally_installed_servers` and persists
    /// running state changes to the database.
    ///
    /// Depending on the configuration, this will either spawn a child process for a server
    /// configured to use the stdio transport or start a Streamable HTTP transport client.
    ///
    /// The server will be started in the background and the result will be handled by the
    /// [`TemplatableMCPServerManager`].
    pub fn spawn_server(&mut self, installation_uuid: Uuid, ctx: &mut ModelContext<Self>) {
        log::debug!("Trying to spawn a server with installation_uuid {installation_uuid}");

        // Look up installation to resolve template variables and use its UUID as the MCP server id.
        let Some(installation) = self
            .locally_installed_servers
            .get(&installation_uuid)
            .cloned()
        else {
            log::error!(
                "No templatable MCP installation found for installation_uuid {installation_uuid}; cannot resolve template variables"
            );

            self.change_server_state(installation_uuid, MCPServerState::FailedToStart, ctx);
            return;
        };

        self.spawn_server_impl(
            installation,
            SpawnMode::Initial {
                persist_running_state_to_sqlite: true,
            },
            ctx,
        );
    }

    /// Internal implementation of server spawning.
    fn spawn_server_impl(
        &mut self,
        installation: TemplatableMCPServerInstallation,
        mode: SpawnMode,
        ctx: &mut ModelContext<Self>,
    ) {
        let installation_uuid = installation.uuid();

        let resolved_json = resolve_json(&installation);
        let template_uuid = installation.template_uuid();

        // Parse the resolved JSON into an MCPServer.
        let mut server = match MCPServer::from_user_json(&resolved_json) {
            Ok(mut servers) => match servers.pop() {
                Some(mut s) => {
                    // We want to use installation uuid, not template uuid.
                    s.uuid = installation_uuid;
                    s
                }
                None => {
                    log::error!(
                        "Templatable MCP server template contains no servers: {template_uuid}",
                    );
                    self.change_server_state(installation_uuid, MCPServerState::FailedToStart, ctx);
                    if mode.is_reconnect() {
                        self.notify_reconnect_waiters(
                            installation_uuid,
                            Err("Template contains no servers".to_string()),
                        );
                    }
                    return;
                }
            },
            Err(err) => {
                log::error!(
                    "Failed to parse resolved MCP server JSON for '{template_uuid}': {err:#}",
                );
                self.change_server_state(installation_uuid, MCPServerState::FailedToStart, ctx);
                if mode.is_reconnect() {
                    self.notify_reconnect_waiters(
                        installation_uuid,
                        Err(format!("Failed to parse MCP server: {err:#}")),
                    );
                }
                return;
            }
        };

        // If we're executing a CLI MCP server, ensure that the environment variables includes
        // PATH.
        if let TransportType::CLIServer(cli_server) = &mut server.transport_type {
            let Some(execution_path) = AISettings::as_ref(ctx).mcp_execution_path.value().clone()
            else {
                // This can only happen if the user is trying to launch an MCP server
                // without ever having had a successfully bootstrapped session, which
                // should basically never happen.
                log::warn!("Unknown PATH when trying to launch MCP command.");

                self.change_server_state(installation_uuid, MCPServerState::FailedToStart, ctx);

                if let Some(window_id) = WindowManager::as_ref(ctx).active_window() {
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(
                                "PATH required to launch MCP server. Please open a new terminal session to autopopulate PATH."
                                    .to_string(),
                            ),
                            window_id,
                            ctx,
                        );
                    });
                }

                if mode.is_reconnect() {
                    self.notify_reconnect_waiters(
                        installation_uuid,
                        Err("PATH not available".to_string()),
                    );
                }
                return;
            };

            // Prepend our PATH to the static env vars, in case the user has
            // specified a custom PATH in the MCP server settings.
            cli_server.static_env_vars.insert(
                0,
                StaticEnvVar {
                    name: "PATH".to_string(),
                    value: execution_path,
                },
            );

            // For file-based MCP installations without an explicit `working_directory`,
            // default the spawn cwd to the directory the config was discovered in
            // (repo root for project-scoped configs, ~/.warp/ or ~ for globals). This
            // matches user expectations for repo-relative commands in `.mcp.json`.
            // Cloud-templated installations (lookup returns None) are unaffected and
            // continue to inherit Warp's process cwd.
            if cli_server.cwd_parameter.is_none() {
                if let Some(spawn_root) =
                    FileBasedMCPManager::as_ref(ctx).spawn_root_for_installation(installation_uuid)
                {
                    cli_server.cwd_parameter = Some(spawn_root.to_string_lossy().into_owned());
                }
            }
        }

        let executor = ctx.background_executor().clone();
        let logger = match LogManager::handle(ctx).update(ctx, |mgr, _| {
            mgr.register_namespace("mcp", true);
            mgr.register(
                "mcp",
                logs::relative_log_file_path_from_uuid(&template_uuid),
                executor,
            )
        }) {
            Ok(logger) => logger,
            Err(e) => {
                safe_error!(
                    safe: ("Failed to register MCP log file: {}", e.safe_message()),
                    full: ("Failed to register MCP log file for {template_uuid}: {e}")
                );
                return;
            }
        };
        let logger_clone = logger.clone();

        // Create channel that we can use to send OAuth callback results
        // to the server initialization task, if the server requires OAuth.
        let (oauth_result_tx, oauth_result_rx) = async_channel::unbounded();

        let is_headless = AppExecutionMode::as_ref(ctx).is_autonomous();

        let mut persisted_credentials = self.server_credentials.get(&template_uuid).cloned();
        if persisted_credentials.is_none() && FeatureFlag::FileBasedMcp.is_enabled() {
            persisted_credentials = installation
                .hash()
                .and_then(|hash| self.file_based_server_credentials.get(&hash).cloned());
        }

        let is_file_based = FeatureFlag::FileBasedMcp.is_enabled()
            && FileBasedMCPManager::as_ref(ctx)
                .get_hash_by_uuid(installation_uuid)
                .is_some();

        let auth_context = AuthContext {
            oauth_result_rx,
            spawner: ctx.spawner(),
            uuid: installation_uuid,
            persisted_credentials,
            is_headless,
            is_file_based,
        };

        let server_name = server.name.clone();
        let description = installation.templatable_mcp_server().description.clone();

        // Extract values from mode before moving it into the closure.
        let should_persist = mode.should_persist_running_state_to_sqlite();
        let should_send_telemetry = mode.should_send_telemetry();
        let is_reconnect = mode.is_reconnect();

        self.change_server_state(installation_uuid, MCPServerState::Starting, ctx);
        let task = ctx.spawn(
            spawn_server(
                server_name,
                description,
                installation_uuid,
                server.transport_type.clone(),
                logger.clone(),
                auth_context,
            )
            .compat(),
            move |me, server_info: Result<_, rmcp::RmcpError>, ctx| {
                me.spawned_servers.remove(&installation_uuid);
                me.pending_oauth_csrf.retain(|_, v| *v != installation_uuid);

                let error = match server_info {
                    Ok(info) => {
                        let peer = info.service.clone();
                        me.active_servers.insert(installation_uuid, info);

                        // Clear any previous error message on successful connection.
                        me.server_error_messages.remove(&installation_uuid);

                        if should_persist {
                            ctx.dispatch_global_action("workspace:save_app", ());
                            Self::persist_is_mcp_running(installation_uuid, true, ctx);
                        }
                        me.change_server_state(installation_uuid, MCPServerState::Running, ctx);

                        if is_reconnect {
                            me.notify_reconnect_waiters(installation_uuid, Ok(peer));
                        }
                        None
                    }
                    Err(e) => {
                        logger_clone
                            .log(format!("[error] MCP: Failed to connect to server: {e:#}"));
                        // Close the logger to make sure we flush any remaining data.
                        logger_clone.close();
                        log::warn!("Failed to spawn MCP server: {e:#}");

                        // Store user-friendly error message.
                        let error_message = error_to_user_message(&e);
                        me.server_error_messages
                            .insert(installation_uuid, error_message.clone());

                        me.change_server_state(
                            installation_uuid,
                            MCPServerState::FailedToStart,
                            ctx,
                        );

                        me.delete_credentials_from_secure_storage(installation_uuid, ctx);

                        if is_reconnect {
                            me.notify_reconnect_waiters(installation_uuid, Err(error_message));
                        }

                        Some(e.into())
                    }
                };

                if should_send_telemetry {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::MCPServerSpawned {
                            transport_type: match server.transport_type {
                                TransportType::CLIServer { .. } =>
                                    MCPServerTelemetryTransportType::CLIServer,
                                TransportType::ServerSentEvents { .. } =>
                                    MCPServerTelemetryTransportType::ServerSentEvents,
                            },
                            server_model: MCPServerModel::Templatable,
                            error
                        },
                        ctx
                    );
                }
            },
        );

        self.spawned_servers.insert(
            installation_uuid,
            SpawnedServerInfo {
                abort_handle: task.abort_handle(),
                oauth_result_tx,
            },
        );

        log::debug!(
            "We have successfully spawned a server with installation_uuid {installation_uuid}!"
        );
    }

    /// Shuts down a running MCP server.
    pub fn shutdown_server(&mut self, installation_uuid: Uuid, ctx: &mut ModelContext<'_, Self>) {
        log::debug!("Trying to shut down a MCP server with installation uuid {installation_uuid}");

        Self::persist_is_mcp_running(installation_uuid, false, ctx);

        // There's 2 possibilities:
        // 1. The server is still connecting, in which case we cancel the spawn
        // 2. The server is already running, in which case we shutdown the server
        // We do both to avoid race conditions and have to do it in this order to avoid the server connecting between the shutdown_server and cancel_spawn calls
        if let Some(spawned_info) = self.spawned_servers.remove(&installation_uuid) {
            spawned_info.abort_handle.abort();
        }
        self.pending_oauth_csrf
            .retain(|_, v| *v != installation_uuid);
        if let Some(server_info) = self.active_servers.remove(&installation_uuid) {
            self.change_server_state(installation_uuid, MCPServerState::ShuttingDown, ctx);
            // Cancel the server, and emit NotRunning state once it has stopped.
            ctx.spawn(server_info.service.cancel(), move |me, _, ctx| {
                me.change_server_state(installation_uuid, MCPServerState::NotRunning, ctx);
                ctx.dispatch_global_action("workspace:save_app", ());
            });
        } else {
            self.change_server_state(installation_uuid, MCPServerState::NotRunning, ctx);
        }

        log::debug!("Successfully shut down server with installation uuid {installation_uuid}");
    }

    pub fn get_installation_by_template_uuid(
        &self,
        template_uuid: Uuid,
    ) -> Option<&TemplatableMCPServerInstallation> {
        self.locally_installed_servers
            .values()
            .find(|installation| installation.template_uuid() == template_uuid)
    }

    pub fn install_from_template(
        &mut self,
        templatable_mcp_server: TemplatableMCPServer,
        variable_values: HashMap<String, VariableValue>,
        start_automatically: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TemplatableMCPServerInstallation> {
        let installation_uuid = Uuid::new_v4();

        // Idempotent safety checks
        // If the server is already installed, we delete the existing installation
        let existing_installation =
            self.get_installation_by_template_uuid(templatable_mcp_server.uuid);
        if let Some(existing_installation) = existing_installation {
            log::warn!(
                "A server with template uuid {} is already installed. The existing installation with uuid {} will be deleted.",
                templatable_mcp_server.uuid,
                existing_installation.uuid()
            );
            self.delete_templatable_mcp_server_installation(existing_installation.uuid(), ctx);
        }

        let mcp_server_installation = TemplatableMCPServerInstallation::new(
            installation_uuid,
            templatable_mcp_server,
            variable_values,
        );

        // Add it locally so the UI updates
        self.locally_installed_servers
            .insert(installation_uuid, mcp_server_installation.clone());

        // Persist it to the local database
        let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get();

        if let Some(sender) = &global_resource_handles.model_event_sender {
            let event = ModelEvent::UpsertMCPServerInstallation {
                mcp_server_installation: mcp_server_installation.clone(),
            };
            if let Err(err) = sender.send(event) {
                log::error!("Failed to save TemplatableMCPServerInstallation to database: {err}");
            }
        }

        ctx.emit(TemplatableMCPServerManagerEvent::ServerInstallationAdded(
            installation_uuid,
        ));

        // Spawn the server
        if start_automatically {
            self.spawn_server(installation_uuid, ctx);
        }

        Some(mcp_server_installation)
    }

    /// Enables (starts) the installed Figma MCP server.
    pub fn enable_figma_mcp(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(uuid) = self.get_figma_installation_uuid() {
            self.spawn_server(uuid, ctx);
        } else {
            log::warn!("Could not find Figma MCP server installation to enable");
        }
    }

    /// Installs the Figma MCP server from the MCP gallery.
    pub fn install_figma_from_gallery(&mut self, ctx: &mut ModelContext<Self>) {
        let figma_gallery_server = MCPGalleryManager::as_ref(ctx)
            .get_gallery()
            .into_iter()
            .find(|item| item.title() == "Figma");
        let Some(figma_gallery_server) = figma_gallery_server else {
            log::warn!("Could not find Figma MCP server in gallery");
            return;
        };
        let Ok(templatable_mcp_server) = TemplatableMCPServer::try_from(figma_gallery_server)
        else {
            log::warn!("Failed to convert Figma gallery item to TemplatableMCPServer");
            return;
        };
        self.install_from_template(templatable_mcp_server, HashMap::new(), true, ctx);
    }

    pub fn delete_templatable_mcp_server_installation(
        &mut self,
        installation_uuid: Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        self.delete_templatable_mcp_server_installations(vec![installation_uuid], ctx);
    }

    fn delete_templatable_mcp_server_installations(
        &mut self,
        installation_uuids: Vec<Uuid>,
        ctx: &mut ModelContext<Self>,
    ) {
        if installation_uuids.is_empty() {
            return;
        }

        for installation_uuid in &installation_uuids {
            self.shutdown_server(*installation_uuid, ctx);

            // Delete log files using template_uuid
            if let Some(installation) = self.locally_installed_servers.get(installation_uuid) {
                let template_uuid = installation.template_uuid();
                let log_file_path = logs::log_file_path_from_uuid(&template_uuid);
                if log_file_path.exists() {
                    let _ = std::fs::remove_file(log_file_path);
                }
            }

            self.locally_installed_servers.remove(installation_uuid);
        }

        // Delete the entries from the local database
        let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get();

        if let Some(sender) = &global_resource_handles.model_event_sender {
            let event = ModelEvent::DeleteMCPServerInstallations {
                installation_uuids: installation_uuids.clone(),
            };
            if let Err(err) = sender.send(event) {
                log::error!("Failed to delete installations from local database: {err}");
            }
        }

        for uuid in installation_uuids {
            ctx.emit(TemplatableMCPServerManagerEvent::ServerInstallationDeleted(
                uuid,
            ));
        }
        ctx.notify();
    }

    fn get_update_from_cloud_server(
        &self,
        installation_uuid: Uuid,
        app: &AppContext,
    ) -> Option<MCPServerUpdate> {
        let installation = self.get_installed_server(&installation_uuid)?;
        let templatable_mcp_server =
            self.get_templatable_mcp_server(installation.template_uuid())?;

        // Return early if the currently installed version isn't out of date
        if templatable_mcp_server.version <= installation.templatable_mcp_server().version {
            return None;
        }

        let author = if self.is_author(templatable_mcp_server.uuid, app) {
            Author::CurrentUser
        } else {
            let creator = self.get_creator(templatable_mcp_server.uuid, app);
            match creator {
                Some(creator) => Author::OtherUser { name: creator },
                None => Author::Unknown,
            }
        };

        Some(MCPServerUpdate::CloudTemplate {
            publisher: author,
            new_version_ts: templatable_mcp_server.version,
            json_template: templatable_mcp_server.template.clone(),
        })
    }

    fn get_update_from_gallery(
        &self,
        installation_uuid: Uuid,
        app: &AppContext,
    ) -> Option<MCPServerUpdate> {
        let installation = self.get_installed_server(&installation_uuid)?;

        let GalleryData {
            gallery_item_id,
            version: installed_gallery_version,
        } = installation.templatable_mcp_server().gallery_data?;

        let gallery_item = MCPGalleryManager::as_ref(app).get_gallery_item(gallery_item_id)?;

        // Return early if the gallery version isn't out of date
        if gallery_item.version() <= installed_gallery_version {
            return None;
        }

        let gallery_templatable_mcp_server =
            MCPGalleryManager::as_ref(app).get_templatable_mcp_server(gallery_item_id)?;

        Some(MCPServerUpdate::Gallery {
            name: gallery_item.title(),
            new_version: gallery_item.version(),
            json_template: gallery_templatable_mcp_server.template.clone(),
        })
    }

    fn deduplicate_updates(
        &self,
        installation_uuid: Uuid,
        updates: Vec<MCPServerUpdate>,
    ) -> Vec<MCPServerUpdate> {
        let Some(installation) = self.get_installed_server(&installation_uuid) else {
            log::error!("Could not find installed server {installation_uuid}");
            return updates.to_vec();
        };

        let installed_template = &installation.templatable_mcp_server().template;

        let mut templates_to_keep: std::collections::HashMap<JsonTemplate, usize> =
            std::collections::HashMap::new();

        for (index, update) in updates.iter().enumerate() {
            let json_template = match update {
                MCPServerUpdate::CloudTemplate { json_template, .. } => json_template,
                MCPServerUpdate::Gallery { json_template, .. } => json_template,
            };

            // De-duplicate those that are the same as the currently installed template
            if installed_template == json_template {
                log::info!(
                    "De-duplicating one update for {installation_uuid} which is the same as the current template."
                );
                continue;
            }

            // De-duplicate those that are identical to each other
            if let Some(other_template_index) = templates_to_keep.get(json_template).copied() {
                log::info!("De-duplicating one identical update for {installation_uuid}.");

                // We should prioritize the gallery version & the newest version
                let other_update = &updates[other_template_index];
                let should_replace = match (update, other_update) {
                    (
                        MCPServerUpdate::CloudTemplate { new_version_ts, .. },
                        MCPServerUpdate::CloudTemplate {
                            new_version_ts: other_new_version_ts,
                            ..
                        },
                    ) => new_version_ts > other_new_version_ts,
                    (MCPServerUpdate::CloudTemplate { .. }, MCPServerUpdate::Gallery { .. }) => {
                        false
                    }
                    (MCPServerUpdate::Gallery { .. }, MCPServerUpdate::CloudTemplate { .. }) => {
                        true
                    }
                    (
                        MCPServerUpdate::Gallery { new_version, .. },
                        MCPServerUpdate::Gallery {
                            new_version: other_new_version,
                            ..
                        },
                    ) => new_version > other_new_version,
                };

                if should_replace {
                    templates_to_keep.insert(json_template.clone(), index);
                }
            } else {
                // This is a new template, so we can add it
                templates_to_keep.insert(json_template.clone(), index);
            }
        }

        templates_to_keep
            .values()
            .map(|&template_index| updates[template_index].clone())
            .collect()
    }

    pub fn is_update_available_for_installation(
        &self,
        installation_uuid: Uuid,
        app: &AppContext,
    ) -> bool {
        !self
            .get_updates_available_for_installation(installation_uuid, app)
            .is_empty()
    }

    pub fn get_updates_available_for_installation(
        &self,
        installation_uuid: Uuid,
        app: &AppContext,
    ) -> Vec<MCPServerUpdate> {
        let options: Vec<MCPServerUpdate> = [
            self.get_update_from_cloud_server(installation_uuid, app),
            self.get_update_from_gallery(installation_uuid, app),
        ]
        .iter()
        .filter_map(|option| option.clone())
        .collect();
        for option in options.clone() {
            log::debug!("Updates: {:?}", option);
        }
        self.deduplicate_updates(installation_uuid, options)
    }

    pub fn update_templatable_mcp_server_installation(
        &mut self,
        installation_uuid: Uuid,
        templatable_mcp_server: &TemplatableMCPServer,
        reuse_variable_values: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let existing_variable_values = self
            .get_installed_server(&installation_uuid)
            .map(|installation| installation.variable_values().clone());

        self.delete_templatable_mcp_server_installation(installation_uuid, ctx);

        if reuse_variable_values {
            if let Some(existing_variable_values) = existing_variable_values {
                self.install_from_template(
                    templatable_mcp_server.clone(),
                    existing_variable_values,
                    true,
                    ctx,
                );
            }
        }
    }

    pub fn is_authorized_editor(&self, template_uuid: Uuid, ctx: &AppContext) -> bool {
        let cloud_templatable_mcp_server = self.get_cloud_templatable_mcp_server(template_uuid);

        if let Some(cloud_templatable_mcp_server) = cloud_templatable_mcp_server {
            let auth_state = AuthStateProvider::as_ref(ctx).get();
            let current_team = UserWorkspaces::as_ref(ctx).current_team();

            let has_admin_permissions = current_team.is_some_and(|team| {
                team.has_admin_permissions(&auth_state.user_email().unwrap_or_default())
            });
            let is_author = cloud_templatable_mcp_server.metadata().creator_uid
                == auth_state.user_id().map(|user_id| user_id.as_string());

            has_admin_permissions || is_author
        } else {
            false
        }
    }

    pub fn is_author(&self, template_uuid: Uuid, ctx: &AppContext) -> bool {
        let cloud_templatable_mcp_server = self.get_cloud_templatable_mcp_server(template_uuid);
        if let Some(cloud_templatable_mcp_server) = cloud_templatable_mcp_server {
            let auth_state = AuthStateProvider::as_ref(ctx).get();
            cloud_templatable_mcp_server.metadata().creator_uid
                == auth_state.user_id().map(|user_id| user_id.as_string())
        } else {
            false
        }
    }

    fn copy_oauth_from_legacy_to_templatable(
        &mut self,
        sync_id: SyncId,
        template_uuid: Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        // Read legacy credentials directly from secure storage rather than
        // going through the (now-removed) MCPServerManager singleton.
        let legacy_credentials: HashMap<SyncId, oauth::PersistedCredentials> =
            oauth::load_credentials_from_secure_storage(
                ctx,
                crate::ai::mcp::manager::oauth::LEGACY_MCP_CREDENTIALS_KEY,
            );
        if let Some(legacy_cred) = legacy_credentials.get(&sync_id) {
            log::info!(
                "Copying OAuth credentials from legacy server {sync_id} to template {template_uuid}"
            );
            self.server_credentials
                .insert(template_uuid, legacy_cred.clone());
            write_to_secure_storage(
                ctx,
                TEMPLATABLE_MCP_CREDENTIALS_KEY,
                &self.server_credentials,
            );
        }
    }

    fn convert_legacy_to_templatable(
        &mut self,
        sync_id: SyncId,
        mut legacy_mcp_server: MCPServer,
        space: Space,
        automatically_start_server: bool,
        initiated_by: InitiatedBy,
        ctx: &mut ModelContext<Self>,
    ) -> Result<ParsedTemplatableMCPServerResult, LegacyToTemplatableMCPConversionError> {
        let template_uuid = legacy_mcp_server.uuid;
        if self.get_templatable_mcp_server(template_uuid).is_some() {
            self.delete_legacy_mcp_server(sync_id, InitiatedBy::System, ctx);
            return Err(LegacyToTemplatableMCPConversionError::TemplateAlreadyExists);
        }

        if let Some(conn) = &self.database_connection {
            let mut conn = conn.lock();
            legacy_mcp_server.fill_environment_variables(&mut conn);
        } else {
            return Err(LegacyToTemplatableMCPConversionError::NoDBConnection);
        }

        let parsed_result = legacy_mcp_server.to_parsed_templatable_mcp_server_result();
        let ParsedTemplatableMCPServerResult {
            templatable_mcp_server,
            templatable_mcp_server_installation,
        } = parsed_result.clone();
        let template_uuid = templatable_mcp_server.uuid;
        self.create_templatable_mcp_server(templatable_mcp_server, space, initiated_by, ctx);
        self.copy_oauth_from_legacy_to_templatable(sync_id, template_uuid, ctx);
        if let Some(templatable_mcp_server_installation) = templatable_mcp_server_installation {
            let installation = self.install_from_template(
                templatable_mcp_server_installation
                    .templatable_mcp_server()
                    .clone(),
                templatable_mcp_server_installation
                    .variable_values()
                    .clone(),
                automatically_start_server,
                ctx,
            );
            if installation.is_some() {
                return Ok(parsed_result);
            }
        }
        Err(LegacyToTemplatableMCPConversionError::InstallationFailed)
    }

    /// To support deprecating the legacy MCPServerManager,
    /// we need to convert all legacy MCP to templatable MCP on app start up
    fn convert_all_legacy_to_templatable(
        &mut self,
        servers_to_restart: HashSet<Uuid>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Import inline because of circular dependencies
        use crate::ai::mcp::CloudMCPServer;
        let cloud_legacy_servers = CloudMCPServer::get_all(ctx);
        log::info!(
            "Converting {} legacy MCP servers into templatable MCP servers",
            cloud_legacy_servers.len()
        );
        for cloud_legacy_server in cloud_legacy_servers {
            let sync_id = cloud_legacy_server.sync_id();
            let legacy_mcp_server = cloud_legacy_server.model().string_model.clone();
            let uuid = legacy_mcp_server.uuid;
            let result = self.convert_legacy_to_templatable(
                sync_id,
                legacy_mcp_server,
                Space::Personal,
                servers_to_restart.contains(&uuid),
                InitiatedBy::System,
                ctx,
            );
            match result {
                Ok(result) => {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::MCPTemplateCreated {
                            source: MCPTemplateCreationSource::Conversion,
                            variables: result.templatable_mcp_server.template.variables,
                            name: result.templatable_mcp_server.name,
                        },
                        ctx
                    );
                }
                Err(e) => log::error!("{e}"),
            }
        }
    }

    pub fn share_templatable_mcp_server(
        &mut self,
        template_uuid: Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        let sync_id = self
            .get_cloud_templatable_mcp_server(template_uuid)
            .map(|server| server.sync_id());
        let team_uid = TemplatableMCPServerManager::get_first_team_space_id(ctx);

        if let Some(sync_id) = sync_id {
            if let Some(team_uid) = team_uid {
                let object_type_and_id = CloudObjectTypeAndId::GenericStringObject {
                    object_type: GenericStringObjectFormat::Json(
                        JsonObjectType::TemplatableMCPServer,
                    ),
                    id: sync_id,
                };
                UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                    update_manager.move_object_to_location(
                        object_type_and_id,
                        CloudObjectLocation::Space(Space::Team { team_uid }),
                        ctx,
                    );
                });
                send_telemetry_from_ctx!(TelemetryEvent::MCPTemplateShared, ctx);
            }
        }
    }

    pub fn share_templatable_mcp_server_installation(
        &mut self,
        installation_uuid: Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        let template_uuid = self.get_template_uuid(installation_uuid);
        if let Some(template_uuid) = template_uuid {
            self.share_templatable_mcp_server(template_uuid, ctx);
        }
    }

    pub fn unshare_templatable_mcp_server(
        &mut self,
        template_uuid: Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        let cloud_templatable_mcp_server = self.get_cloud_templatable_mcp_server(template_uuid);

        if let Some(cloud_templatable_mcp_server) = cloud_templatable_mcp_server {
            let sync_id = cloud_templatable_mcp_server.sync_id();

            let object_type_and_id = CloudObjectTypeAndId::GenericStringObject {
                object_type: GenericStringObjectFormat::Json(JsonObjectType::TemplatableMCPServer),
                id: sync_id,
            };
            UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                update_manager.move_object_to_location(
                    object_type_and_id,
                    CloudObjectLocation::Space(Space::Personal),
                    ctx,
                );
            });
        }
    }

    pub fn unshare_templatable_mcp_server_installation(
        &mut self,
        installation_uuid: Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        let template_uuid = self.get_template_uuid(installation_uuid);
        if let Some(template_uuid) = template_uuid {
            self.unshare_templatable_mcp_server(template_uuid, ctx);
        }
    }

    pub fn get_first_team_space_id(app: &AppContext) -> Option<ServerId> {
        let user_workspaces = UserWorkspaces::as_ref(app);
        let all_user_spaces = user_workspaces.all_user_spaces(app);
        all_user_spaces.into_iter().find_map(|space| match space {
            Space::Team { team_uid } => Some(team_uid),
            _ => None,
        })
    }

    pub fn has_oauth_credentials_for_server(&self, template_uuid: Uuid) -> bool {
        self.server_credentials.contains_key(&template_uuid)
    }

    /// Returns the peer for the given installation UUID if it is connected and the transport is not closed.
    pub fn get_peer_if_connected(
        &self,
        installation_uuid: Uuid,
    ) -> Option<rmcp::Peer<rmcp::RoleClient>> {
        self.active_servers
            .get(&installation_uuid)
            .and_then(|server| {
                if server.service.is_transport_closed() {
                    None
                } else {
                    Some(server.service.clone())
                }
            })
    }

    /// Triggers reconnection of a server by its installation UUID.
    ///
    /// If a reconnection is already in progress for this server, the caller is added to the
    /// waiting list and will be notified when the existing reconnection completes.
    /// Otherwise, a new reconnection is started.
    ///
    /// The result is sent via the provided oneshot channel when the connection completes (or fails).
    pub fn reconnect_server(
        &mut self,
        installation_uuid: Uuid,
        result_tx: tokio::sync::oneshot::Sender<Result<rmcp::Peer<rmcp::RoleClient>, String>>,
        ctx: &mut ModelContext<Self>,
    ) {
        log::debug!("Reconnecting MCP server with installation uuid {installation_uuid}");

        // If a reconnection is already in progress, add this caller to the waiting list.
        if let Some(waiters) = self.pending_reconnections.get_mut(&installation_uuid) {
            log::debug!(
                "Reconnection already in progress for {installation_uuid}, adding to waiters"
            );
            waiters.push(result_tx);
            return;
        }

        // Start tracking this reconnection with this caller as the first waiter.
        self.pending_reconnections
            .insert(installation_uuid, vec![result_tx]);

        // Remove the old server from active_servers if it exists.
        self.active_servers.remove(&installation_uuid);

        // Cancel any in-flight spawn.
        if let Some(spawned_info) = self.spawned_servers.remove(&installation_uuid) {
            spawned_info.abort_handle.abort();
        }
        self.pending_oauth_csrf
            .retain(|_, v| *v != installation_uuid);

        // Look up the installation to get server details.
        let Some(installation) = self
            .locally_installed_servers
            .get(&installation_uuid)
            .cloned()
        else {
            self.notify_reconnect_waiters(
                installation_uuid,
                Err("Installation not found".to_string()),
            );
            return;
        };

        self.spawn_server_impl(installation, SpawnMode::Reconnect, ctx);
    }

    /// Notifies all pending reconnection waiters for the given installation UUID.
    ///
    /// This removes the waiters from `pending_reconnections` and sends the result to each.
    fn notify_reconnect_waiters(
        &mut self,
        installation_uuid: Uuid,
        result: Result<rmcp::Peer<rmcp::RoleClient>, String>,
    ) {
        if let Some(waiters) = self.pending_reconnections.remove(&installation_uuid) {
            for tx in waiters {
                // Clone the result for each waiter. For Ok, we clone the peer.
                // For Err, we clone the error message.
                let _ = tx.send(result.clone());
            }
        }
    }

    /// Returns a reconnecting peer for a server that has the given tool.
    ///
    /// The returned peer will automatically reconnect if the underlying transport is closed.
    pub fn server_with_tool_name(
        &self,
        tool_name: String,
    ) -> Option<crate::ai::mcp::reconnecting_peer::ReconnectingPeer> {
        let spawner = self.spawner.as_ref()?;
        self.active_servers
            .iter()
            .find(|(_, server)| server.tools.iter().any(|t| t.name == tool_name))
            .map(|(installation_uuid, _)| {
                crate::ai::mcp::reconnecting_peer::ReconnectingPeer::new(
                    *installation_uuid,
                    spawner.clone(),
                )
            })
    }

    /// Returns a reconnecting peer for a server with the given installation ID and tool.
    ///
    /// The returned peer will automatically reconnect if the underlying transport is closed.
    pub fn server_with_installation_id_and_tool_name(
        &self,
        installation_id: Uuid,
        tool_name: String,
    ) -> Option<crate::ai::mcp::reconnecting_peer::ReconnectingPeer> {
        let spawner = self.spawner.as_ref()?;
        let server = self.active_servers.get(&installation_id)?;
        if server.tools.iter().any(|t| t.name == tool_name) {
            Some(crate::ai::mcp::reconnecting_peer::ReconnectingPeer::new(
                installation_id,
                spawner.clone(),
            ))
        } else {
            None
        }
    }

    fn spawn_file_based_servers(
        &mut self,
        installations: &[TemplatableMCPServerInstallation],
        ctx: &mut ModelContext<Self>,
    ) {
        // First, check if the servers are already spawned.
        let mut new_installations = Vec::new();
        for installation in installations {
            let uuid = installation.uuid();
            let server_name = &installation.templatable_mcp_server().name;
            if self.active_servers.contains_key(&uuid) {
                log::info!(
                    "Skipping file-based MCP server '{server_name}' ({uuid}); already running"
                );
                continue;
            }
            if self.spawned_servers.contains_key(&uuid) {
                log::info!(
                    "Skipping file-based MCP server '{server_name}' ({uuid}); already starting"
                );
                continue;
            }
            log::info!("Spawning file-based MCP server '{server_name}' ({uuid})");
            new_installations.push(installation.clone());
        }

        // If not, spawn them.
        for installation in new_installations {
            self.spawn_ephemeral_server(installation, ctx);
        }
    }

    fn despawn_file_based_servers(
        &mut self,
        installation_uuids: Vec<Uuid>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.delete_templatable_mcp_server_installations(installation_uuids, ctx);
    }

    pub fn purge_file_based_server_credentials(
        &mut self,
        installation_hashes: &Vec<u64>,
        ctx: &mut ModelContext<Self>,
    ) {
        for hash in installation_hashes {
            self.file_based_server_credentials.remove(hash);
        }
        if !installation_hashes.is_empty() {
            write_to_secure_storage(
                ctx,
                FILE_BASED_MCP_CREDENTIALS_KEY,
                &self.file_based_server_credentials,
            );
        }
    }

    pub fn has_oauth_credentials_for_file_based_server(&self, installation_hash: u64) -> bool {
        self.file_based_server_credentials
            .contains_key(&installation_hash)
    }
}

type ReqwestHttpTransport = rmcp::transport::StreamableHttpClientTransport<reqwest::Client>;
type ReqwestSseTransport = mcp::sse_transport::SseClientTransport<reqwest::Client>;

/// Spawns a new MCP server from a given [`TransportType`].
async fn spawn_server(
    server_name: String,
    description: Option<String>,
    uuid: Uuid,
    transport_type: TransportType,
    logger: SimpleLogger,
    auth_context: AuthContext,
) -> Result<TemplatableMCPServerInfo, rmcp::RmcpError> {
    logger.log("[note] Attention! There may be sensitive information (such as API keys) in these logs. Make sure to redact any secrets before sharing with others.".to_string());

    let mut is_authenticated_transport = false;
    let service = match transport_type {
        TransportType::CLIServer(cli_server) => {
            logger.log("[info] MCP: Using stdio transport".to_string());

            cfg_if! {
                if #[cfg(windows)] {
                    // We wrap the command in cmd.exe /c to allow Windows to be responsible for resolving the
                    // PATH variable rather than depending on the `Command` implementation, which only looks for
                    // `.exe` files in directories found in PATH.
                    // https://github.com/rust-lang/rust/issues/37519
                    let command = "cmd.exe".to_owned();
                    let args = std::iter::once("/c".to_owned())
                        .chain(std::iter::once(cli_server.command))
                        .chain(cli_server.args)
                        .collect::<Vec<String>>();
                } else {
                    let command = cli_server.command;
                    let args = cli_server.args;
                }
            }

            // Capture the command and configured cwd for diagnostics before they're
            // moved into the Command builder closure.
            let command_for_log = command.clone();
            let cwd_for_log = cli_server.cwd_parameter.clone();

            // Try to spawn the child process.
            let (transport, stderr) = rmcp::transport::TokioChildProcess::builder(
                tokio::process::Command::new(command).configure(|cmd| {
                    cmd.args(args);
                    if let Some(cwd) = cli_server.cwd_parameter {
                        cmd.current_dir(cwd);
                    }
                    for StaticEnvVar { name, value } in cli_server.static_env_vars.iter() {
                        if value.is_empty() {
                            // Skip empty/unset environment variables so that, in the CLI, they can be inherited.
                            logger.log(format!(
                                "[warn] MCP: Skipping empty environment variable: {name}"
                            ));
                            continue;
                        }
                        cmd.env(name, value);
                    }

                    // On Windows, ensure that no console window is shown.
                    #[cfg(windows)]
                    cmd.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
                }),
            )
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    let cwd_display = cwd_for_log
                        .as_deref()
                        .unwrap_or("<inherited from Warp's process cwd>");
                    logger.log(format!(
                        "[error] MCP: Failed to spawn '{server_name}': command '{command_for_log}' \
                         not found (cwd: {cwd_display}). If your MCP server depends on a specific \
                         working directory, set the `working_directory` field in your config to \
                         override the default."
                    ));
                }
                rmcp::RmcpError::transport_creation::<rmcp::transport::TokioChildProcess>(err)
            })?;

            let pid = transport
                .id()
                .map(|pid| pid.to_string())
                .unwrap_or("??".to_string());

            // We always expect to have an stderr, but this is marginally safer than unwrapping.
            if let Some(stderr) = stderr {
                let logger = logger.clone();
                // Spawn a background task to forward from the child process's stderr to our logger.
                tokio::spawn(async move {
                    let mut buf = String::new();
                    let mut reader = tokio::io::BufReader::new(stderr);
                    loop {
                        match reader.read_line(&mut buf).await {
                            // EOF.
                            Ok(0) => return,
                            // Read some data.
                            Ok(_) => logger.log(format!("[info] MCP [pid: {pid}] stderr: {buf}")),
                            // Failed to read from the child process's stderr.
                            Err(e) => {
                                log::error!("Failed to read stderr: {e}");
                                return;
                            }
                        }
                    }
                });
            }

            // Wrap the transport in a logging wrapper.
            let transport = TransportLoggingWrapper {
                transport,
                logger: logger.clone(),
            };

            // Create the MCP client and connect to the server.
            Ok::<_, rmcp::RmcpError>(make_client_info().into_dyn().serve(transport).await?)
        }
        TransportType::ServerSentEvents(sse_server) => {
            let headers: std::collections::HashMap<String, String> = sse_server
                .headers
                .iter()
                .map(|h| (h.name.clone(), h.value.clone()))
                .collect();
            match determine_transport(server_name.clone(), &sse_server.url, &headers, auth_context)
                .await
            {
                // TODO: these need headers also?
                Ok(Transport::Http(Some(client))) => {
                    is_authenticated_transport = true;

                    logger.log("[info] MCP: Using Streaming HTTP transport".to_string());
                    let transport = rmcp::transport::StreamableHttpClientTransport::with_client(
                        client,
                        rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                            sse_server.url.clone(),
                        ),
                    );
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
                    };
                    Ok(make_client_info().into_dyn().serve(transport).await?)
                }
                Ok(Transport::Http(None)) => {
                    logger.log("[info] MCP: Using Streaming HTTP transport".to_string());
                    let transport = if headers.is_empty() {
                        rmcp::transport::StreamableHttpClientTransport::from_uri(
                            sse_server.url.clone(),
                        )
                    } else {
                        let client = build_client_with_headers(&headers)?;
                        rmcp::transport::StreamableHttpClientTransport::with_client(
                            client,
                            rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                                sse_server.url.clone(),
                            ),
                        )
                    };
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
                    };
                    Ok(make_client_info().into_dyn().serve(transport).await?)
                }
                Ok(Transport::Sse(Some(client))) => {
                    is_authenticated_transport = true;

                    logger.log("[info] MCP: Using (legacy) SSE transport (due to preflight failing with a 404)".to_string());
                    let transport = mcp::sse_transport::SseClientTransport::start_with_client(
                        client,
                        mcp::sse_transport::SseClientConfig {
                            sse_endpoint: sse_server.url.into(),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(rmcp::RmcpError::transport_creation::<ReqwestSseTransport>)?;
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
                    };
                    Ok(make_client_info().into_dyn().serve(transport).await?)
                }
                Ok(Transport::Sse(None)) => {
                    logger.log("[info] MCP: Using (legacy) SSE transport (due to preflight failing with a 404)".to_string());
                    let transport = if headers.is_empty() {
                        mcp::sse_transport::SseClientTransport::start(sse_server.url.clone())
                            .await
                            .map_err(|e| {
                                rmcp::RmcpError::transport_creation::<ReqwestSseTransport>(e)
                            })?
                    } else {
                        let client = build_client_with_headers(&headers)?;
                        mcp::sse_transport::SseClientTransport::start_with_client(
                            client,
                            mcp::sse_transport::SseClientConfig {
                                sse_endpoint: sse_server.url.clone().into(),
                                ..Default::default()
                            },
                        )
                        .await
                        .map_err(rmcp::RmcpError::transport_creation::<ReqwestSseTransport>)?
                    };
                    let transport = TransportLoggingWrapper {
                        transport,
                        logger: logger.clone(),
                    };
                    Ok(make_client_info().into_dyn().serve(transport).await?)
                }
                Err(err) => {
                    logger.log(format!(
                        "[error] MCP: preflight connection to MCP server failed: {err:#}"
                    ));
                    Err(err)?
                }
            }
        }
    }?;

    let server_info = service.peer_info();
    logger.log(format!("[info] MCP: Connected to server: {server_info:#?}"));

    let capabilities = server_info.map(|info| &info.capabilities);

    let resources =
        query_resources_for(capabilities, &server_name, || service.list_all_resources()).await;
    let tools = query_tools_for(capabilities, &server_name, || service.list_all_tools()).await;

    Ok(TemplatableMCPServerInfo {
        name: server_name,
        service,
        resources,
        tools,
        installation_id: uuid,
        description,
        is_authenticated_transport,
    })
}

/// The transport to use for MCP.
enum Transport {
    /// The HTTP transport, with an optional authenticated client.
    Http(Option<rmcp::transport::auth::AuthClient<reqwest::Client>>),
    /// The SSE transport, with an optional authenticated client.
    Sse(Option<rmcp::transport::auth::AuthClient<reqwest::Client>>),
}

/// Determines which transport to use.
///
/// This sends a "preflight" InitializeRequest to the server to determine whether the
/// server supports the HTTP transport (or needs to use the SSE transport), and if
/// authentication is required.
async fn determine_transport(
    server_name: String,
    url: &str,
    headers: &std::collections::HashMap<String, String>,
    auth_context: AuthContext,
) -> Result<Transport, rmcp::RmcpError> {
    use reqwest::StatusCode;

    fn unexpected_error(status: reqwest::StatusCode) -> rmcp::RmcpError {
        rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>(format!(
            "Unexpected status code: {status}"
        ))
    }
    match send_initialize_request(url, headers, None).await? {
        StatusCode::OK => Ok(Transport::Http(None)),
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => Ok(Transport::Sse(None)),
        StatusCode::UNAUTHORIZED => {
            if !FeatureFlag::McpOauth.is_enabled() {
                return Err(rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>(
                    "Server requires authentication, which is not yet supported.".to_string(),
                ));
            }

            let spawner = auth_context.spawner.clone();
            // Go through the OAuth flow to get an authenticated client.
            // This will first attempt to use cached credentials before starting interactive OAuth.
            let (client, did_require_login) = oauth::make_authenticated_client(url, auth_context)
                .boxed()
                .await
                .map_err(rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>)?;
            let transport = match send_initialize_request(url, headers, Some(&client)).await? {
                StatusCode::OK => Ok(Transport::Http(Some(client))),
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
                    Ok(Transport::Sse(Some(client)))
                }
                other => Err(unexpected_error(other)),
            };
            if transport.is_ok() && did_require_login {
                let _ = spawner
                    .spawn(move |_, ctx| {
                        if let Some(active_window_id) = ctx.windows().active_window() {
                            ToastStack::handle(ctx).update(ctx, |stack, ctx| {
                                stack.add_ephemeral_toast(
                                    DismissibleToast::default(format!(
                                        "Successfully authenticated {server_name} MCP server"
                                    )),
                                    active_window_id,
                                    ctx,
                                );
                            });
                        }
                    })
                    .await;
            }

            transport
        }
        status => Err(unexpected_error(status)),
    }
}

/// Sends an InitializeRequest to the server, and returns the HTTP status code from the response.
async fn send_initialize_request(
    url: &str,
    headers: &std::collections::HashMap<String, String>,
    auth_client: Option<&rmcp::transport::auth::AuthClient<reqwest::Client>>,
) -> Result<reqwest::StatusCode, rmcp::RmcpError> {
    use rmcp::transport::common::http_header::{EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE};

    let request = rmcp::model::InitializeRequest::new(make_client_info());
    let request = rmcp::model::ClientJsonRpcMessage::request(
        rmcp::model::ClientRequest::InitializeRequest(request),
        rmcp::model::RequestId::Number(0),
    );

    let mut request = build_client_with_headers(headers)?
        .post(url)
        .header(
            http::header::ACCEPT,
            [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "),
        )
        .json(&request);

    if let Some(auth_client) = auth_client.as_ref() {
        let access_token = auth_client
            .get_access_token()
            .await
            .map_err(rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>)?;
        request = request.bearer_auth(access_token);
    }

    let response = request
        .send()
        .await
        .map_err(rmcp::RmcpError::transport_creation::<ReqwestHttpTransport>)?;

    Ok(response.status())
}

/// Creates a [`ClientInfo`] for the MCP client.
///
/// This tells the MCP server who we are and what capabilities we have.
fn make_client_info() -> rmcp::model::ClientInfo {
    rmcp::model::ClientInfo::new(
        Default::default(),
        rmcp::model::Implementation::new(
            warp_core::channel::ChannelState::app_id().to_string(),
            warp_core::channel::ChannelState::app_version()
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ),
    )
}

/// A wrapper around a [`rmcp::transport::Transport`] that logs all requests and responses.
struct TransportLoggingWrapper<T> {
    transport: T,
    logger: SimpleLogger,
}

impl<T: rmcp::transport::Transport<R>, R: rmcp::service::ServiceRole> rmcp::transport::Transport<R>
    for TransportLoggingWrapper<T>
{
    type Error = T::Error;

    fn send(
        &mut self,
        item: rmcp::service::TxJsonRpcMessage<R>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        if let Ok(json) = serde_json::to_string(&item) {
            self.logger
                .log(format!("[info] MCP: Sending request: {json}"));
        }

        let logger = self.logger.clone();
        self.transport.send(item).map(move |result| {
            if let Err(e) = &result {
                logger.log(format!("[warn] MCP: Failed to send request: {e:#}"));
            }
            result
        })
    }

    fn receive(
        &mut self,
    ) -> impl Future<Output = Option<rmcp::service::RxJsonRpcMessage<R>>> + Send {
        let logger = self.logger.clone();
        async move {
            let result = self.transport.receive().await;
            if let Some(item) = &result {
                if let Ok(json) = serde_json::to_string(item) {
                    logger.log(format!("[info] MCP: Received response: {json}"));
                }
            }
            result
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.transport.close()
    }
}
