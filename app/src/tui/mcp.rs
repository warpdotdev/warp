use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;

use uuid::Uuid;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::mcp::file_based_manager::FileBasedMCPServerScope;
use crate::ai::mcp::gallery::MCPGalleryManagerEvent;
use crate::ai::mcp::parsing::resolve_json;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManagerEvent;
use crate::ai::mcp::{
    FileBasedMCPManager, FileMCPWatcher, MCPGalleryManager, MCPServer, MCPServerExt,
    MCPServerState, TemplatableMCPServer, TemplatableMCPServerInstallation,
    TemplatableMCPServerManager, TransportType, VariableType, VariableValue,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TuiMcpServerId {
    /// Stable content hash. File-based installation UUIDs are regenerated on
    /// every config parse, so they cannot preserve selection across reloads.
    FileBased(u64),
    Installation(Uuid),
    SyncedTemplate(Uuid),
    Gallery(Uuid),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiMcpTransport {
    Stdio,
    HttpOrSse,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TuiMcpFileScope {
    Global,
    Project,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TuiMcpFileSource {
    pub provider: String,
    pub root_path: PathBuf,
    pub scope: TuiMcpFileScope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiMcpSyncedTemplateProvenance {
    FromAnotherDevice,
    Shared { creator: Option<String> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiMcpServerSource {
    FileBased {
        sources: Vec<TuiMcpFileSource>,
    },
    Installation,
    SyncedTemplate {
        provenance: TuiMcpSyncedTemplateProvenance,
    },
    Gallery,
}
impl TuiMcpServerSource {
    pub fn label(&self) -> String {
        match self {
            Self::Installation => "CLI local".to_owned(),
            Self::SyncedTemplate {
                provenance: TuiMcpSyncedTemplateProvenance::FromAnotherDevice,
            } => "from another device".to_owned(),
            Self::SyncedTemplate {
                provenance: TuiMcpSyncedTemplateProvenance::Shared { creator },
            } => creator
                .as_ref()
                .map(|creator| format!("shared by {creator}"))
                .unwrap_or_else(|| "shared by a team member".to_owned()),
            Self::Gallery => "shared by Warp".to_owned(),
            Self::FileBased { sources } => {
                let labels = sources
                    .iter()
                    .map(|source| match source.scope {
                        TuiMcpFileScope::Global => format!("{} global", source.provider),
                        TuiMcpFileScope::Project => {
                            let root = source
                                .root_path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("project");
                            format!("{} · {root}", source.provider)
                        }
                    })
                    .collect::<Vec<_>>();
                if labels.is_empty() {
                    "file config".to_owned()
                } else {
                    labels.join(", ")
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiMcpServerStatus {
    Available,
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
    pub installation_uuid: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub source: TuiMcpServerSource,
    pub transport: Option<TuiMcpTransport>,
    pub status: TuiMcpServerStatus,
    pub tool_count: usize,
    pub resource_count: usize,
    pub can_log_out: bool,
    pub authorization_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiMcpConfigDiagnostic {
    pub provider: String,
    pub config_path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuiMcpSnapshot {
    pub diagnostics: Vec<TuiMcpConfigDiagnostic>,
    pub servers: Vec<TuiMcpServerSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiMcpTemplateVariable {
    pub key: String,
    pub allowed_values: Option<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiMcpInstallRequest {
    pub id: TuiMcpServerId,
    pub name: String,
    pub variables: Vec<TuiMcpTemplateVariable>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct TuiMcpVariableValue {
    pub key: String,
    pub value: String,
}
impl fmt::Debug for TuiMcpVariableValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TuiMcpVariableValue")
            .field("key", &self.key)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiMcpAction {
    Enable(TuiMcpServerId),
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

/// TUI-facing aggregate over installed, synced, gallery, and file-based MCPs.
///
/// Refreshing this model is a pure read. Available catalog items become
/// runnable only through [`Self::install_and_enable`], after the frontend has
/// collected any required values.
pub struct TuiMcpManager {
    snapshot: TuiMcpSnapshot,
}

impl TuiMcpManager {
    /// Creates an empty MCP aggregate for frontend tests.
    #[cfg(any(test, all(feature = "tui", feature = "test-util")))]
    pub(crate) fn new_for_test(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            snapshot: TuiMcpSnapshot::default(),
        }
    }

    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&FileBasedMCPManager::handle(ctx), |me, _, _, ctx| {
            me.refresh(ctx);
        });
        ctx.subscribe_to_model(
            &TemplatableMCPServerManager::handle(ctx),
            |me, _, event, ctx| {
                match event {
                    TemplatableMCPServerManagerEvent::StateChanged { uuid, state } => {
                        let _ = (uuid, state);
                    }
                    TemplatableMCPServerManagerEvent::AuthenticationRequired { uuid }
                    | TemplatableMCPServerManagerEvent::CredentialsChanged { uuid } => {
                        let _ = uuid;
                    }
                    TemplatableMCPServerManagerEvent::ServerInstallationAdded(uuid)
                    | TemplatableMCPServerManagerEvent::ServerInstallationDeleted(uuid) => {
                        let _ = uuid;
                    }
                    TemplatableMCPServerManagerEvent::TemplatableMCPServersUpdated
                    | TemplatableMCPServerManagerEvent::LegacyServerConverted => {}
                }
                me.refresh(ctx);
            },
        );
        ctx.subscribe_to_model(
            &MCPGalleryManager::handle(ctx),
            |me, _, event, ctx| match event {
                MCPGalleryManagerEvent::ItemsRefreshed => me.refresh(ctx),
            },
        );

        let mut model = Self {
            snapshot: TuiMcpSnapshot::default(),
        };
        model.refresh(ctx);
        model
    }

    pub fn snapshot(&self) -> &TuiMcpSnapshot {
        &self.snapshot
    }

    pub fn prepare_install(
        &self,
        id: TuiMcpServerId,
        ctx: &ModelContext<Self>,
    ) -> Result<TuiMcpInstallRequest, String> {
        if !self
            .snapshot
            .servers
            .iter()
            .any(|server| server.id == id && matches!(server.status, TuiMcpServerStatus::Available))
        {
            return Err("This MCP is no longer available to enable".to_owned());
        }

        let server = match id {
            TuiMcpServerId::SyncedTemplate(template_uuid) => {
                TemplatableMCPServerManager::as_ref(ctx)
                    .get_templatable_mcp_server(template_uuid)
                    .cloned()
                    .ok_or_else(|| "The synced MCP template is no longer available".to_owned())?
            }
            TuiMcpServerId::Gallery(gallery_uuid) => MCPGalleryManager::as_ref(ctx)
                .get_templatable_mcp_server(gallery_uuid)
                .cloned()
                .ok_or_else(|| "The gallery MCP template is no longer available".to_owned())?,
            TuiMcpServerId::FileBased(_) | TuiMcpServerId::Installation(_) => {
                return Err("This MCP is already installed".to_owned());
            }
        };

        Ok(TuiMcpInstallRequest {
            id,
            name: server.name,
            variables: server
                .template
                .variables
                .into_iter()
                .map(|variable| TuiMcpTemplateVariable {
                    key: variable.key,
                    allowed_values: variable.allowed_values,
                })
                .collect(),
        })
    }

    /// Installs and starts an available template after collecting any required
    /// values. Catalog refresh and selection never call this method.
    pub fn install_and_enable(
        &mut self,
        id: TuiMcpServerId,
        values: Vec<TuiMcpVariableValue>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<Uuid, String> {
        let request = self.prepare_install(id, ctx)?;
        let values = validate_variable_values(&request.variables, values)?;
        let server = match id {
            TuiMcpServerId::SyncedTemplate(template_uuid) => {
                TemplatableMCPServerManager::as_ref(ctx)
                    .get_templatable_mcp_server(template_uuid)
                    .cloned()
                    .ok_or_else(|| "The synced MCP template is no longer available".to_owned())?
            }
            TuiMcpServerId::Gallery(gallery_uuid) => MCPGalleryManager::as_ref(ctx)
                .get_templatable_mcp_server(gallery_uuid)
                .cloned()
                .ok_or_else(|| "The gallery MCP template is no longer available".to_owned())?,
            TuiMcpServerId::FileBased(_) | TuiMcpServerId::Installation(_) => {
                return Err("This MCP is already installed".to_owned());
            }
        };
        let installation = TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.install_from_template(server, values, true, ctx)
        });
        let installation =
            installation.ok_or_else(|| "Unable to install this MCP server".to_owned())?;
        let uuid = installation.uuid();
        self.refresh(ctx);
        Ok(uuid)
    }

    pub fn apply_action(&mut self, action: TuiMcpAction, ctx: &mut ModelContext<Self>) {
        match action {
            TuiMcpAction::Enable(_) => {}
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
            TuiMcpAction::Start(id) | TuiMcpAction::Retry(id) => match id {
                TuiMcpServerId::FileBased(hash) => {
                    let installation = FileBasedMCPManager::as_ref(ctx)
                        .installation_by_hash(hash)
                        .cloned();
                    if let Some(installation) = installation {
                        TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                            if !manager.is_server_active_or_pending(installation.uuid()) {
                                manager.spawn_ephemeral_server(installation, ctx);
                            }
                        });
                    }
                }
                TuiMcpServerId::Installation(uuid) => {
                    TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                        if !manager.is_server_active_or_pending(uuid) {
                            manager.spawn_server(uuid, ctx);
                        }
                    });
                }
                TuiMcpServerId::SyncedTemplate(_) | TuiMcpServerId::Gallery(_) => {}
            },
            TuiMcpAction::Stop(id) | TuiMcpAction::LogOut(id) => {
                let installation_uuid = match id {
                    TuiMcpServerId::FileBased(hash) => FileBasedMCPManager::as_ref(ctx)
                        .installation_by_hash(hash)
                        .map(TemplatableMCPServerInstallation::uuid),
                    TuiMcpServerId::Installation(uuid) => Some(uuid),
                    TuiMcpServerId::SyncedTemplate(_) | TuiMcpServerId::Gallery(_) => None,
                };
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
        let file_manager = FileBasedMCPManager::as_ref(ctx);
        let runtime_manager = TemplatableMCPServerManager::as_ref(ctx);
        let gallery_manager = MCPGalleryManager::as_ref(ctx);

        let mut diagnostics = file_manager
            .config_diagnostics()
            .into_iter()
            .map(|diagnostic| TuiMcpConfigDiagnostic {
                provider: diagnostic.provider.display_name().to_owned(),
                config_path: diagnostic.config_path,
                message: diagnostic.message,
            })
            .collect::<Vec<_>>();
        diagnostics.sort_by(|left, right| {
            left.provider
                .cmp(&right.provider)
                .then(left.config_path.cmp(&right.config_path))
                .then(left.message.cmp(&right.message))
        });

        let mut servers = Vec::new();
        let installations = runtime_manager
            .get_installed_templatable_servers()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let installed_template_uuids = installations
            .iter()
            .map(TemplatableMCPServerInstallation::template_uuid)
            .collect::<HashSet<_>>();
        let installed_gallery_uuids = installations
            .iter()
            .filter_map(TemplatableMCPServerInstallation::gallery_uuid)
            .collect::<HashSet<_>>();
        let global_warp_server_identities = file_manager
            .global_warp_servers()
            .into_iter()
            .filter_map(|installation| template_identity(installation.templatable_mcp_server()))
            .collect::<HashSet<_>>();

        for installation in installations {
            servers.push(snapshot_for_installation(
                TuiMcpServerId::Installation(installation.uuid()),
                TuiMcpServerSource::Installation,
                &installation,
                runtime_manager,
                ctx,
            ));
        }

        let synced_templates = runtime_manager
            .get_all_templatable_mcp_servers()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let reserved_gallery_uuids = synced_templates
            .iter()
            .filter_map(|template| template.gallery_data.map(|data| data.gallery_item_id))
            .chain(installed_gallery_uuids.iter().copied())
            .collect::<HashSet<_>>();
        let reserved_names = synced_templates
            .iter()
            .map(|template| template.name.to_ascii_lowercase())
            .chain(installations_names(
                runtime_manager.get_installed_templatable_servers(),
            ))
            .collect::<HashSet<_>>();

        for template in synced_templates {
            if installed_template_uuids.contains(&template.uuid) {
                continue;
            }
            let source = synced_template_source(template.uuid, runtime_manager, ctx);
            if is_represented_by_global_warp_server(
                &template,
                &source,
                &global_warp_server_identities,
            ) {
                continue;
            }
            servers.push(snapshot_for_available(
                TuiMcpServerId::SyncedTemplate(template.uuid),
                source,
                &template,
            ));
        }

        for gallery in gallery_manager.get_gallery() {
            if reserved_gallery_uuids.contains(&gallery.uuid())
                || reserved_names.contains(&gallery.title().to_ascii_lowercase())
            {
                continue;
            }
            let Some(template) = gallery_manager
                .get_templatable_mcp_server(gallery.uuid())
                .cloned()
            else {
                continue;
            };
            servers.push(snapshot_for_available(
                TuiMcpServerId::Gallery(gallery.uuid()),
                TuiMcpServerSource::Gallery,
                &template,
            ));
        }

        for file_server in file_manager.file_based_servers_with_sources() {
            let installation = file_server.installation;
            let Some(hash) = installation.hash() else {
                continue;
            };
            let mut sources = file_server
                .sources
                .into_iter()
                .map(|source| TuiMcpFileSource {
                    provider: source.provider.display_name().to_owned(),
                    root_path: source.root_path,
                    scope: match source.scope {
                        FileBasedMCPServerScope::Global => TuiMcpFileScope::Global,
                        FileBasedMCPServerScope::Project => TuiMcpFileScope::Project,
                    },
                })
                .collect::<Vec<_>>();
            sources.sort();
            sources.dedup();
            servers.push(snapshot_for_installation(
                TuiMcpServerId::FileBased(hash),
                TuiMcpServerSource::FileBased { sources },
                &installation,
                runtime_manager,
                ctx,
            ));
        }

        sort_servers(&mut servers);
        let snapshot = TuiMcpSnapshot {
            diagnostics,
            servers,
        };
        if self.snapshot != snapshot {
            self.snapshot = snapshot;
            ctx.emit(TuiMcpManagerEvent::Updated);
            ctx.notify();
        }
    }
}

fn installations_names(
    installations: &HashMap<Uuid, TemplatableMCPServerInstallation>,
) -> impl Iterator<Item = String> + '_ {
    installations.values().map(|installation| {
        installation
            .templatable_mcp_server()
            .name
            .to_ascii_lowercase()
    })
}

fn validate_variable_values(
    variables: &[TuiMcpTemplateVariable],
    values: Vec<TuiMcpVariableValue>,
) -> Result<HashMap<String, VariableValue>, String> {
    let expected = variables
        .iter()
        .map(|variable| variable.key.as_str())
        .collect::<HashSet<_>>();
    if values.len() != expected.len() {
        return Err("Every required MCP variable must have a value".to_owned());
    }

    let mut resolved = HashMap::new();
    for value in values {
        if value.value.is_empty() || !expected.contains(value.key.as_str()) {
            return Err("Every required MCP variable must have a value".to_owned());
        }
        let variable = variables
            .iter()
            .find(|variable| variable.key == value.key)
            .expect("expected keys were checked");
        if variable
            .allowed_values
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(&value.value))
        {
            return Err("Select one of the allowed values for this MCP variable".to_owned());
        }
        if resolved
            .insert(
                value.key,
                VariableValue {
                    variable_type: VariableType::Text,
                    value: value.value,
                },
            )
            .is_some()
        {
            return Err("Each MCP variable may only be provided once".to_owned());
        }
    }
    Ok(resolved)
}

fn snapshot_for_available(
    id: TuiMcpServerId,
    source: TuiMcpServerSource,
    template: &TemplatableMCPServer,
) -> TuiMcpServerSnapshot {
    TuiMcpServerSnapshot {
        id,
        installation_uuid: None,
        name: template.name.clone(),
        description: template.description.clone(),
        source,
        transport: transport_from_template(template),
        status: TuiMcpServerStatus::Available,
        tool_count: 0,
        resource_count: 0,
        can_log_out: false,
        authorization_url: None,
    }
}

fn snapshot_for_installation(
    id: TuiMcpServerId,
    source: TuiMcpServerSource,
    installation: &TemplatableMCPServerInstallation,
    runtime_manager: &TemplatableMCPServerManager,
    ctx: &ModelContext<TuiMcpManager>,
) -> TuiMcpServerSnapshot {
    let uuid = installation.uuid();
    TuiMcpServerSnapshot {
        id,
        installation_uuid: Some(uuid),
        name: installation.templatable_mcp_server().name.clone(),
        description: installation.templatable_mcp_server().description.clone(),
        source,
        transport: transport_from_installation(installation),
        status: runtime_status(uuid, runtime_manager),
        tool_count: runtime_manager.tools_for_server(uuid).len(),
        resource_count: runtime_manager.resources_for_server(uuid).len(),
        can_log_out: runtime_manager.can_log_out(uuid, ctx),
        authorization_url: runtime_manager
            .authorization_url(uuid)
            .map(ToOwned::to_owned),
    }
}

fn synced_template_source(
    template_uuid: Uuid,
    runtime_manager: &TemplatableMCPServerManager,
    ctx: &ModelContext<TuiMcpManager>,
) -> TuiMcpServerSource {
    let provenance = if runtime_manager.is_server_template_shared(template_uuid, ctx) {
        TuiMcpSyncedTemplateProvenance::Shared {
            creator: runtime_manager.get_creator(template_uuid, ctx),
        }
    } else {
        TuiMcpSyncedTemplateProvenance::FromAnotherDevice
    };
    TuiMcpServerSource::SyncedTemplate { provenance }
}

#[derive(Debug, Eq, Hash, PartialEq)]
enum TuiMcpServerIdentity {
    Stdio {
        name: String,
        command: String,
        args: Vec<String>,
        working_directory: Option<String>,
    },
    HttpOrSse {
        name: String,
        url: String,
    },
}

fn template_identity(template: &TemplatableMCPServer) -> Option<TuiMcpServerIdentity> {
    let mut servers = MCPServer::from_user_json(&template.template.json).ok()?;
    if servers.len() != 1 {
        return None;
    }
    let server = servers.pop()?;
    let name = server.name.to_ascii_lowercase();
    match server.transport_type {
        TransportType::CLIServer(server) => Some(TuiMcpServerIdentity::Stdio {
            name,
            command: server.command,
            args: server.args,
            working_directory: server.cwd_parameter,
        }),
        TransportType::ServerSentEvents(server) => Some(TuiMcpServerIdentity::HttpOrSse {
            name,
            url: server.url,
        }),
    }
}

fn is_represented_by_global_warp_server(
    template: &TemplatableMCPServer,
    source: &TuiMcpServerSource,
    global_warp_server_identities: &HashSet<TuiMcpServerIdentity>,
) -> bool {
    matches!(
        source,
        TuiMcpServerSource::SyncedTemplate {
            provenance: TuiMcpSyncedTemplateProvenance::FromAnotherDevice,
        }
    ) && template_identity(template)
        .is_some_and(|identity| global_warp_server_identities.contains(&identity))
}
fn transport_from_template(template: &TemplatableMCPServer) -> Option<TuiMcpTransport> {
    MCPServer::from_user_json(&template.template.json)
        .ok()?
        .pop()
        .map(|server| transport_type(server.transport_type))
}

fn transport_from_installation(
    installation: &TemplatableMCPServerInstallation,
) -> Option<TuiMcpTransport> {
    MCPServer::from_user_json(&resolve_json(installation))
        .ok()?
        .pop()
        .map(|server| transport_type(server.transport_type))
}

fn transport_type(transport: TransportType) -> TuiMcpTransport {
    match transport {
        TransportType::CLIServer(_) => TuiMcpTransport::Stdio,
        TransportType::ServerSentEvents(_) => TuiMcpTransport::HttpOrSse,
    }
}

fn runtime_status(uuid: Uuid, runtime_manager: &TemplatableMCPServerManager) -> TuiMcpServerStatus {
    match runtime_manager.get_server_state(uuid) {
        None | Some(MCPServerState::NotRunning) => TuiMcpServerStatus::Offline,
        Some(MCPServerState::Starting | MCPServerState::Reconnecting) => {
            TuiMcpServerStatus::Starting
        }
        Some(MCPServerState::Authenticating) => TuiMcpServerStatus::Authenticating,
        Some(MCPServerState::Running) => TuiMcpServerStatus::Running,
        Some(MCPServerState::ShuttingDown) => TuiMcpServerStatus::Stopping,
        Some(MCPServerState::FailedToStart) => TuiMcpServerStatus::Failed {
            message: runtime_manager
                .get_server_error_message(uuid)
                .unwrap_or("Failed to start")
                .to_owned(),
        },
    }
}

fn sort_servers(servers: &mut [TuiMcpServerSnapshot]) {
    servers.sort_by(|left, right| {
        server_priority(left)
            .cmp(&server_priority(right))
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then(left.id.cmp(&right.id))
    });
}

fn server_priority(server: &TuiMcpServerSnapshot) -> u8 {
    match server.status {
        TuiMcpServerStatus::Available => 1,
        TuiMcpServerStatus::Offline
        | TuiMcpServerStatus::Starting
        | TuiMcpServerStatus::Authenticating
        | TuiMcpServerStatus::Running
        | TuiMcpServerStatus::Stopping
        | TuiMcpServerStatus::Failed { .. } => 0,
    }
}

impl Entity for TuiMcpManager {
    type Event = TuiMcpManagerEvent;
}

impl SingletonEntity for TuiMcpManager {}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
