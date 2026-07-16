use std::path::PathBuf;

use uuid::Uuid;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::mcp::parsing::resolve_json;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManagerEvent;
use crate::ai::mcp::{
    FileBasedMCPManager, FileMCPWatcher, MCPServer, MCPServerExt, MCPServerState,
    TemplatableMCPServerManager, TransportType,
};
use crate::warp_managed_paths_watcher::active_mcp_config_file_path;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TuiMcpServerId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiMcpTransport {
    Stdio,
    HttpOrSse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiMcpServerStatus {
    Offline,
    Starting,
    Authenticating,
    Running,
    Stopping,
    Failed { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiMcpServerSnapshot {
    pub id: TuiMcpServerId,
    pub installation_uuid: Uuid,
    pub name: String,
    pub transport: TuiMcpTransport,
    pub status: TuiMcpServerStatus,
    pub tool_count: usize,
    pub resource_count: usize,
    pub has_credentials: bool,
    pub authorization_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiMcpConfigState {
    Missing,
    Ready,
    Invalid { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiMcpSnapshot {
    pub config_path: PathBuf,
    pub config_state: TuiMcpConfigState,
    pub servers: Vec<TuiMcpServerSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiMcpAction {
    Start(TuiMcpServerId),
    Stop(TuiMcpServerId),
    Retry(TuiMcpServerId),
    LogOut(TuiMcpServerId),
    ReopenAuthorization(TuiMcpServerId),
    ReloadConfig,
}

#[derive(Clone, Copy, Debug)]
pub enum TuiMcpManagerEvent {
    Updated,
}

/// TUI-facing aggregate over every global Warp MCP server.
///
/// There is one singleton model for the TUI process, not one model per
/// server. Its snapshot joins file-config health with runtime state for all
/// configured servers; per-server actions are routed by [`TuiMcpServerId`].
pub struct TuiMcpManager {
    snapshot: TuiMcpSnapshot,
}

impl TuiMcpManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&FileBasedMCPManager::handle(ctx), |me, _, _, ctx| {
            me.refresh(ctx);
        });
        ctx.subscribe_to_model(
            &TemplatableMCPServerManager::handle(ctx),
            |me, _, event, ctx| match event {
                TemplatableMCPServerManagerEvent::StateChanged { .. } => me.refresh(ctx),
                TemplatableMCPServerManagerEvent::AuthenticationRequired { uuid }
                | TemplatableMCPServerManagerEvent::CredentialsChanged { uuid } => {
                    let _ = uuid;
                    me.refresh(ctx);
                }
                TemplatableMCPServerManagerEvent::ServerInstallationAdded(_)
                | TemplatableMCPServerManagerEvent::ServerInstallationDeleted(_)
                | TemplatableMCPServerManagerEvent::TemplatableMCPServersUpdated
                | TemplatableMCPServerManagerEvent::LegacyServerConverted => {}
            },
        );

        let mut model = Self {
            snapshot: TuiMcpSnapshot {
                config_path: active_mcp_config_file_path().unwrap_or_default(),
                config_state: TuiMcpConfigState::Missing,
                servers: Vec::new(),
            },
        };
        model.refresh(ctx);
        model
    }

    pub fn snapshot(&self) -> &TuiMcpSnapshot {
        &self.snapshot
    }

    pub fn apply_action(&mut self, action: TuiMcpAction, ctx: &mut ModelContext<Self>) {
        match action {
            TuiMcpAction::ReloadConfig => {
                FileMCPWatcher::handle(ctx).update(ctx, |watcher, ctx| {
                    watcher.reload_global_config(ctx);
                });
            }
            TuiMcpAction::ReopenAuthorization(id) => {
                if let Some(url) = self
                    .snapshot
                    .servers
                    .iter()
                    .find(|server| server.id == id)
                    .and_then(|server| server.authorization_url.as_deref())
                {
                    ctx.open_url(url);
                }
            }
            TuiMcpAction::Start(id) | TuiMcpAction::Retry(id) => {
                let installation = FileBasedMCPManager::as_ref(ctx)
                    .global_warp_installation_by_hash(id.0)
                    .cloned();
                if let Some(installation) = installation {
                    TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                        if !manager.is_server_active_or_pending(installation.uuid()) {
                            manager.spawn_ephemeral_server(installation, ctx);
                        }
                    });
                }
            }
            TuiMcpAction::Stop(id) | TuiMcpAction::LogOut(id) => {
                let installation_uuid = FileBasedMCPManager::as_ref(ctx)
                    .global_warp_installation_by_hash(id.0)
                    .map(|installation| installation.uuid());
                if let Some(installation_uuid) = installation_uuid {
                    TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                        manager.shutdown_server(installation_uuid, ctx);
                        if matches!(action, TuiMcpAction::LogOut(_)) {
                            manager.delete_credentials_from_secure_storage(installation_uuid, ctx);
                        }
                    });
                }
            }
        }
    }

    fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let config_path = active_mcp_config_file_path().unwrap_or_default();
        let file_manager = FileBasedMCPManager::as_ref(ctx);
        let runtime_manager = TemplatableMCPServerManager::as_ref(ctx);
        let config_state = if let Some(diagnostic) = file_manager.config_diagnostic(&config_path) {
            TuiMcpConfigState::Invalid {
                message: diagnostic.message.clone(),
            }
        } else if config_path.exists() {
            TuiMcpConfigState::Ready
        } else {
            TuiMcpConfigState::Missing
        };

        let mut servers = file_manager
            .global_warp_servers()
            .into_iter()
            .filter_map(|installation| {
                let hash = installation.hash()?;
                let uuid = installation.uuid();
                let transport = MCPServer::from_user_json(&resolve_json(installation))
                    .ok()?
                    .pop()
                    .map(|server| match server.transport_type {
                        TransportType::CLIServer(_) => TuiMcpTransport::Stdio,
                        TransportType::ServerSentEvents(_) => TuiMcpTransport::HttpOrSse,
                    })?;
                let status = match runtime_manager.get_server_state(uuid) {
                    None | Some(MCPServerState::NotRunning) => TuiMcpServerStatus::Offline,
                    Some(MCPServerState::Starting) => TuiMcpServerStatus::Starting,
                    Some(MCPServerState::Authenticating) => TuiMcpServerStatus::Authenticating,
                    Some(MCPServerState::Running) => TuiMcpServerStatus::Running,
                    Some(MCPServerState::ShuttingDown) => TuiMcpServerStatus::Stopping,
                    Some(MCPServerState::FailedToStart) => TuiMcpServerStatus::Failed {
                        message: runtime_manager
                            .get_server_error_message(uuid)
                            .unwrap_or("Failed to start")
                            .to_string(),
                    },
                };
                Some(TuiMcpServerSnapshot {
                    id: TuiMcpServerId(hash),
                    installation_uuid: uuid,
                    name: installation.templatable_mcp_server().name.clone(),
                    transport,
                    status,
                    tool_count: runtime_manager.tools_for_server(uuid).len(),
                    resource_count: runtime_manager.resources_for_server(uuid).len(),
                    has_credentials: runtime_manager.has_credentials(uuid, ctx),
                    authorization_url: runtime_manager
                        .authorization_url(uuid)
                        .map(ToString::to_string),
                })
            })
            .collect::<Vec<_>>();
        servers.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then(left.id.cmp(&right.id))
        });

        let snapshot = TuiMcpSnapshot {
            config_path,
            config_state,
            servers,
        };
        if self.snapshot != snapshot {
            self.snapshot = snapshot;
            ctx.emit(TuiMcpManagerEvent::Updated);
            ctx.notify();
        }
    }
}

impl Entity for TuiMcpManager {
    type Event = TuiMcpManagerEvent;
}

impl SingletonEntity for TuiMcpManager {}
