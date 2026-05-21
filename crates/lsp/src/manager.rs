use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{
    config::LanguageId, model::LanguageServerId, supported_servers::LSPServerType, LspEvent,
    LspServerConfig, LspServerModel,
};
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

#[derive(Debug)]
pub enum LspManagerModelEvent {
    /// ServerStarted is fired when the server is successfully started and reports ready status.
    /// ServerStopped is fired when the server has completed its shutdown.
    /// Both are routed from individual LspServerModel events.
    ServerStarted(PathBuf),
    ServerStopped(PathBuf),
    /// ServerRemoved is fired when a server is removed from the manager.
    /// This happens when the user explicitly removes the server (e.g., from settings or footer menu).
    /// Subscribers should drop their references to the server model.
    /// Contains the workspace path, server type, and the unique server ID.
    ServerRemoved {
        workspace_root: PathBuf,
        server_type: LSPServerType,
        server_id: LanguageServerId,
    },
}

#[derive(Default)]
pub struct LspManagerModel {
    /// Map from workspace root path to server info
    servers: HashMap<PathBuf, Vec<ModelHandle<LspServerModel>>>,
    /// Map from external file paths to the LSP server that should handle them.
    /// This is populated when navigating to definitions in files outside the workspace.
    external_file_servers: HashMap<PathBuf, LanguageServerId>,
}

impl LspManagerModel {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
            external_file_servers: HashMap::new(),
        }
    }

    /// Returns an iterator over all workspace root paths that currently have an LSP server.
    pub fn workspace_roots(&self) -> impl Iterator<Item = &PathBuf> {
        self.servers.keys()
    }

    /// Returns the server handles for a given workspace root path.
    pub fn servers_for_workspace(&self, path: &Path) -> Option<&Vec<ModelHandle<LspServerModel>>> {
        self.servers.get(path)
    }

    /// Returns true if a server of the given type is already registered for this workspace.
    /// This is used to prevent duplicate registrations.
    pub fn server_registered(
        &self,
        path: &Path,
        server_type: LSPServerType,
        ctx: &AppContext,
    ) -> bool {
        let Some(servers) = self.servers.get(path) else {
            return false;
        };

        for server in servers {
            if server.as_ref(ctx).server_type() == server_type {
                return true;
            }
        }

        false
    }

    pub fn server_registered_and_started(
        &self,
        path: &Path,
        server_type: LSPServerType,
        ctx: &AppContext,
    ) -> bool {
        let Some(servers) = self.servers.get(path) else {
            return false;
        };

        for server in servers {
            if server.as_ref(ctx).server_type() == server_type {
                return server.as_ref(ctx).has_started();
            }
        }

        false
    }

    pub fn server_for_path(
        &self,
        path: &Path,
        ctx: &AppContext,
    ) -> Option<ModelHandle<LspServerModel>> {
        // Resolve the language ID - early return if unknown
        let path_lang = LanguageId::from_path(path)?;

        // First check if this is an external file that was registered via goto-definition
        if let Some(server_id) = self.external_file_servers.get(path) {
            if let Some(server) = self.server_by_id(*server_id, ctx) {
                // Validate that the server supports this file's language
                if server.as_ref(ctx).supports_language(&path_lang) {
                    return Some(server);
                }
                log::debug!(
                    "External file server for {} does not support language {:?}, falling back to workspace lookup",
                    path.display(),
                    path_lang
                );
            }
        }

        // Then try workspace-based lookup
        let lsp_model = self.lsp_model_for_path(path)?;

        for server in lsp_model {
            let supported = server.as_ref(ctx).supports_language(&path_lang);

            if supported {
                return Some(server.clone());
            }
        }

        log::debug!(
            "LSP server found for path: {}, but language does not match",
            path.display()
        );

        None
    }

    /// Registers an external file (outside any workspace) to be handled by a specific LSP server.
    /// This is called when navigating to a definition in an external file.
    pub fn maybe_register_external_file(&mut self, path: &Path, server_id: LanguageServerId) {
        // Skip registration if the path is already under an existing workspace scope
        if self.lsp_model_for_path(path).is_some() {
            log::debug!(
                "Skipping external file registration for {} - already under workspace scope",
                path.display()
            );
            return;
        }

        self.external_file_servers
            .insert(path.to_path_buf(), server_id);
    }

    /// Finds an LSP server by its unique ID.
    pub fn server_by_id(
        &self,
        id: LanguageServerId,
        ctx: &AppContext,
    ) -> Option<ModelHandle<LspServerModel>> {
        self.servers
            .values()
            .flatten()
            .find(|server| server.as_ref(ctx).id() == id)
            .cloned()
    }

    /// Register a new LSP server at the given path.
    /// Returns false if a server of the same type is already registered for this workspace.
    pub fn register(
        &mut self,
        path: PathBuf,
        config: LspServerConfig,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        // Check if a server of the same type is already registered for this workspace.
        if self.server_registered(&path, config.server_type(), ctx) {
            log::debug!(
                "LSP server {} already registered for path: {}",
                config.server_type().binary_name(),
                path.display()
            );
            return false;
        }

        log::info!("Registering LSP server for path: {}", path.display());

        let lsp = ctx.add_model(|_| LspServerModel::new(config));

        let path_clone = path.clone();
        ctx.subscribe_to_model(&lsp, move |_, event, ctx| match event {
            LspEvent::Started => {
                ctx.emit(LspManagerModelEvent::ServerStarted(path_clone.clone()));
            }
            LspEvent::Stopped => {
                ctx.emit(LspManagerModelEvent::ServerStopped(path_clone.clone()));
            }
            _ => {}
        });

        self.servers.entry(path).or_default().push(lsp);
        true
    }

    pub fn start_all(&mut self, path: PathBuf, ctx: &mut ModelContext<Self>) {
        let Some(servers) = self.servers.get(&path) else {
            log::warn!(
                "No server registered for startup at path: {}",
                path.display()
            );
            return;
        };

        for server in servers.iter() {
            // Skip servers that were manually stopped by the user
            if !server.as_ref(ctx).can_auto_start() {
                log::info!(
                    "Skipping auto-start for manually stopped LSP server at path: {}",
                    path.display()
                );
                continue;
            }

            let result = server.update(ctx, |server, ctx| server.start(ctx));

            if let Err(e) = &result {
                log::warn!(
                    "Failed to start LSP server at path: {}: {e}",
                    path.display()
                );
            }
        }
    }

    pub fn stop_all(&mut self, path: PathBuf, ctx: &mut ModelContext<Self>) {
        let Some(servers) = self.servers.get(&path) else {
            log::warn!("No server registered to stop at path: {}", path.display());
            return;
        };

        for server in servers {
            let result = server.update(ctx, |server, ctx| server.stop(false, ctx));

            if let Err(e) = &result {
                log::warn!("Failed to stop LSP server at path: {}: {e}", path.display())
            }
        }
    }

    /// Removes a specific LSP server from the manager.
    /// This stops the server and removes it from the internal HashMap.
    /// Emits a ServerRemoved event so subscribers can drop their references.
    pub fn remove_server(
        &mut self,
        workspace_root: &Path,
        server_type: LSPServerType,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(servers) = self.servers.get_mut(workspace_root) else {
            log::warn!(
                "No server registered to remove at path: {}",
                workspace_root.display()
            );
            return;
        };

        // Find and remove the server with matching type, capturing its ID first
        let mut removed_server_id: Option<LanguageServerId> = None;
        servers.retain(|server| {
            let server_ref = server.as_ref(ctx);
            if server_ref.server_type() == server_type {
                // Capture the server ID before removing
                removed_server_id = Some(server_ref.id());
                // Always attempt to stop the server before removing (manually_stopped = true).
                // The stop() method handles state checks internally.
                let _ = server.update(ctx, |s, ctx| s.stop(true, ctx));
                false // Remove from vec
            } else {
                true // Keep in vec
            }
        });

        // Clean up empty entries
        if servers.is_empty() {
            self.servers.remove(workspace_root);
        }

        if let Some(server_id) = removed_server_id {
            log::info!(
                "Removed {} LSP server for {}",
                server_type.binary_name(),
                workspace_root.display()
            );
            ctx.emit(LspManagerModelEvent::ServerRemoved {
                workspace_root: workspace_root.to_path_buf(),
                server_type,
                server_id,
            });
        }
    }

    /// Terminate all LSP servers for all workspaces.
    /// This should be called during app shutdown.
    pub fn terminate(&mut self, ctx: &mut ModelContext<Self>) {
        log::info!(
            "Terminating all LSP servers for {} workspaces",
            self.servers.len()
        );
        let workspace_roots: Vec<_> = self.workspace_roots().cloned().collect();
        for root in workspace_roots {
            log::debug!(
                "Shutting down LSP servers for workspace: {}",
                root.display()
            );
            self.stop_all(root, ctx);
        }
    }

    /// Given a path, return the path of the registered LSP workspace for that path, if any
    pub fn lsp_model_for_path(&self, path: &Path) -> Option<&[ModelHandle<LspServerModel>]> {
        for ancestor in path.ancestors() {
            if let Some(servers) = self.servers.get(ancestor) {
                return Some(servers);
            }
        }
        None
    }

    #[cfg(target_arch = "wasm32")]
    pub fn repo_path_for_path(_path: &Path, _ctx: &AppContext) -> Option<PathBuf> {
        None
    }
}

impl Entity for LspManagerModel {
    type Event = LspManagerModelEvent;
}

impl SingletonEntity for LspManagerModel {}
