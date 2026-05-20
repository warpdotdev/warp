use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{collections::HashSet, sync::mpsc::SyncSender};

use lsp::LanguageId;
#[cfg(feature = "local_fs")]
use lsp::{LspManagerModel, LspServerConfig};
#[cfg(feature = "local_fs")]
use repo_metadata::repositories::{DetectedRepositories, DetectedRepositoriesEvent};
use serde::{Deserialize, Serialize};

use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::AIRequestUsageModel;
use crate::persistence::ModelEvent;
use crate::report_if_error;
use crate::send_telemetry_from_ctx;
use crate::settings::CodeSettings;
use crate::terminal::TerminalView;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
use crate::TelemetryEvent;
use ai::{
    index::full_source_code_embedding::manager::{CodebaseIndexManager, CodebaseIndexManagerEvent},
    workspace::{WorkspaceMetadata, WorkspaceMetadataEvent},
};
use anyhow::Context;
use chrono::Utc;
use itertools::Itertools;
use lsp::supported_servers::LSPServerType;
use warp_core::features::FeatureFlag;
#[cfg(feature = "local_fs")]
use warpui::windowing::WindowManager;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

#[cfg(feature = "local_fs")]
use crate::code::language_server_shutdown_manager::LanguageServerShutdownManager;
#[cfg(feature = "local_fs")]
use crate::code::lsp_telemetry::LspTelemetryEvent;
#[cfg(feature = "local_fs")]
use crate::server::server_api::ServerApiProvider;
#[cfg(feature = "local_fs")]
use crate::terminal::local_shell::LocalShellState;
#[cfg(feature = "local_fs")]
use crate::{view_components::DismissibleToast, workspace::ToastStack};
#[cfg(feature = "local_fs")]
use lsp::LspEvent;
#[cfg(feature = "local_fs")]
use warp_core::channel::ChannelState;

use ai::project_context::model::{ProjectContextModel, ProjectContextModelEvent};

/// Represents whether an LSP server is enabled or disabled for a workspace.
///
/// This is also used in underlying sqlite type persistence. We should be careful
/// not to rename an existing variant, as it will break persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnablementState {
    Yes,
    No,
    /// Server was detected as available for a repo but not yet explicitly
    /// enabled/disabled by the user. Entries with this state live only in
    /// memory and are never persisted to SQLite.
    Suggested,
}

/// Describes an LSP operation to be executed after capturing the interactive shell PATH.
#[cfg(feature = "local_fs")]
pub enum LspTask {
    /// Install and enable an LSP server for a file path.
    Install {
        file_path: PathBuf,
        repo_root: PathBuf,
        server_type: LSPServerType,
    },
    /// Spawn LSP servers for a file path.
    Spawn { file_path: PathBuf },
}

pub enum LSPEnablementResultForFile {
    Enabled,
    UnsupportedLanguage,
    LSPNotEnabled { root_name: Option<String> },
}

/// Tracks whether an LSP server is relevant/installed/enabled for a repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LspRepoStatus {
    /// LSP is enabled and running (view will set this when subscribed to a live server).
    Ready,
    /// LSP is enabled (we don't block on installation checks when enabled).
    Enabled,
    /// We are checking installation status (only for disabled case).
    CheckingForInstallation,
    /// LSP is disabled and globally installed.
    DisabledAndInstalled { server_type: LSPServerType },
    /// LSP is disabled and not installed.
    DisabledAndNotInstalled { server_type: LSPServerType },
    /// LSP is currently being installed.
    Installing { server_type: LSPServerType },
}

/// Global installation status for an LSP server (across all projects).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LSPInstallationStatus {
    Installed,
    NotInstalled,
    Checking,
    Installing,
}

impl LspRepoStatus {
    /// Converts an [`LSPInstallationStatus`] (global, per-server-type) into an
    /// [`LspRepoStatus`] (per-repo view of enablement/installation).
    pub fn from_installation_status(
        status: &LSPInstallationStatus,
        server_type: LSPServerType,
    ) -> Self {
        match status {
            LSPInstallationStatus::Installed => Self::DisabledAndInstalled { server_type },
            LSPInstallationStatus::NotInstalled => Self::DisabledAndNotInstalled { server_type },
            LSPInstallationStatus::Checking => Self::CheckingForInstallation,
            LSPInstallationStatus::Installing => Self::Installing { server_type },
        }
    }
}

pub struct Workspace {
    metadata: WorkspaceMetadata,
    language_servers: HashMap<LSPServerType, EnablementState>,
}

impl Workspace {
    /// Returns `true` if this workspace has been persisted to SQLite.
    ///
    /// A workspace created solely from available-server detection will have
    /// all metadata timestamps set to `None` and is considered non-persisted.
    fn is_persisted(&self) -> bool {
        let persisted = self.metadata.navigated_ts.is_some()
            || self.metadata.modified_ts.is_some()
            || self.metadata.queried_ts.is_some();

        if !persisted {
            debug_assert!(
                self.language_servers
                    .values()
                    .all(|s| *s == EnablementState::Suggested),
                "non-persisted workspace has Yes/No server state; persist metadata first"
            );
        }

        persisted
    }
}

/// Manages a set of code workspaces that the app recognizes. These workspaces define
/// the scope of various repo-based code features like codebase indexing, project rules and LSP.
pub struct PersistedWorkspace {
    workspaces: HashMap<PathBuf, Workspace>,
    model_event_sender: Option<SyncSender<ModelEvent>>,
    /// Global installation status per LSP server type.
    #[cfg(feature = "local_fs")]
    lsp_installation_status: HashMap<LSPServerType, LSPInstallationStatus>,
}

#[derive(Debug, Clone)]
pub enum PersistedWorkspaceEvent {
    /// Emitted when LSP installation status changes.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    InstallStatusUpdate {
        server_type: LSPServerType,
        status: LSPInstallationStatus,
    },
    /// Emitted when LSP installation completes successfully.
    /// Toast notification is shown directly by PersistedWorkspace.
    /// The server is also spawned automatically by PersistedWorkspace.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    InstallationSucceeded,
    /// Emitted when LSP installation fails.
    /// Toast notification is shown directly by PersistedWorkspace.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    InstallationFailed,
    /// Emitted when async detection of available servers for a workspace completes.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    AvailableServersDetected {
        workspace_path: PathBuf,
        servers: Vec<LSPServerType>,
    },
    /// Emitted when the user explicitly adds a repo via a picker (e.g. the tab-config
    /// params modal's repo dropdown). Subscribers can use this to refresh their list.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    WorkspaceAdded { path: PathBuf },
}

impl Entity for PersistedWorkspace {
    type Event = PersistedWorkspaceEvent;
}

impl SingletonEntity for PersistedWorkspace {}

impl PersistedWorkspace {
    #[cfg(test)]
    pub fn new_for_test(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            workspaces: HashMap::new(),
            model_event_sender: None,
            #[cfg(feature = "local_fs")]
            lsp_installation_status: HashMap::new(),
        }
    }

    pub fn new(
        metadata: Vec<WorkspaceMetadata>,
        workspace_language_servers: HashMap<PathBuf, HashMap<LSPServerType, EnablementState>>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let metadata: HashMap<PathBuf, Workspace> = metadata
            .into_iter()
            .map(|metadata| {
                let path = metadata.path.clone();
                let language_servers = workspace_language_servers
                    .get(&path)
                    .cloned()
                    .unwrap_or_default();

                (
                    path,
                    Workspace {
                        metadata,
                        language_servers,
                    },
                )
            })
            .collect();

        if FeatureFlag::FullSourceCodeEmbedding.is_enabled() {
            ctx.subscribe_to_model(
                &CodebaseIndexManager::handle(ctx),
                |me, event, ctx| match event {
                    CodebaseIndexManagerEvent::IndexMetadataUpdated { root_path, event } => {
                        me.handle_index_metadata_event(root_path, *event);
                    }
                    CodebaseIndexManagerEvent::NewIndexCreated => {
                        send_active_indexed_repos_changed_telemetry(ctx);
                    }
                    CodebaseIndexManagerEvent::RemoveExpiredIndexMetadata { expired_metadata } => {
                        // TODO: Disable expired metadata removal once we have other consumers of the workspace metadata.
                        me.clean_up_expired_metadata(expired_metadata.clone(), ctx);
                        send_active_indexed_repos_changed_telemetry(ctx);
                    }
                    _ => {}
                },
            );

            // Subscribe to AI conversation events to trigger incremental sync
            ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), |me, event, ctx| {
                if let BlocklistAIHistoryEvent::StartedNewConversation {
                    terminal_view_id, ..
                } = event
                {
                    #[cfg(feature = "local_fs")]
                    me.clean_up_deleted_indices(ctx);

                    me.trigger_incremental_sync_for_conversation(*terminal_view_id, ctx);
                }
            });

            // Subscribe to changes in workspace settings.
            ctx.subscribe_to_model(
                &UserWorkspaces::handle(ctx),
                |me, user_workspaces_event, ctx| {
                    if let UserWorkspacesEvent::CodebaseContextEnablementChanged =
                        user_workspaces_event
                    {
                        me.on_settings_changed(ctx);
                    }
                },
            );

            // Subscribe to ProjectContextModel events to persist rule changes
            ctx.subscribe_to_model(&ProjectContextModel::handle(ctx), |me, event, _ctx| {
                if let ProjectContextModelEvent::KnownRulesChanged(delta) = event {
                    let mut events = vec![];

                    if !delta.discovered_rules.is_empty() {
                        events.push(ModelEvent::UpsertProjectRules {
                            project_rule_paths: delta.discovered_rules.clone(),
                        });
                    }

                    if !delta.deleted_rules.is_empty() {
                        events.push(ModelEvent::DeleteProjectRules {
                            path: delta.deleted_rules.clone(),
                        });
                    }

                    if !events.is_empty() {
                        me.save_to_db(events);
                    }
                }
            });
        }

        #[cfg(feature = "local_fs")]
        if !cfg!(any(
            test,
            feature = "fast_dev",
            feature = "integration_tests"
        )) && CodebaseIndexManager::as_ref(ctx).is_indexing_enabled()
        {
            ctx.subscribe_to_model(&DetectedRepositories::handle(ctx), |me, event, ctx| {
                let DetectedRepositoriesEvent::DetectedGitRepo { repository, .. } = event;
                let repo_path = repository.as_ref(ctx).root_dir().to_local_path_lossy();

                me.index_repo(repo_path, ctx);
            });
        }

        // Collect workspace paths before metadata is moved into Self.
        #[cfg(feature = "local_fs")]
        let startup_workspace_paths: Vec<PathBuf> = metadata.keys().cloned().collect();

        #[allow(unused_mut)]
        let mut result = Self {
            workspaces: metadata,
            model_event_sender,
            #[cfg(feature = "local_fs")]
            lsp_installation_status: HashMap::new(),
        };

        // Kick off LSP suggestion scanning for all existing workspaces so that
        // the available-server state is fresh by the time any footer is created.
        // We pass skip_cached=true so workspaces with persisted entries are still
        // re-scanned to discover newly relevant server types.
        #[cfg(feature = "local_fs")]
        if !cfg!(any(
            test,
            feature = "fast_dev",
            feature = "integration_tests"
        )) && !startup_workspace_paths.is_empty()
        {
            result.detect_available_servers_for_workspaces(startup_workspace_paths, true, ctx);
        }

        result
    }

    /// Given a repo path, enables the specified LSP server. If the workspace doesn't exist, it will be created.
    pub fn enable_lsp_server_for_path(&mut self, path: &Path, server_type: LSPServerType) {
        self.set_lsp_server_for_path(path, server_type, EnablementState::Yes);
    }

    /// Given a repo path, disables the specified LSP server.
    pub fn disable_lsp_server_for_path(&mut self, path: &Path, server_type: LSPServerType) {
        self.set_lsp_server_for_path(path, server_type, EnablementState::No);
    }

    /// Returns the enabled LSP server type (if any) for this file path.
    pub fn has_enabled_lsp_server_for_file_path(&self, path: &Path) -> LSPEnablementResultForFile {
        let Some(language_id) = LanguageId::from_path(path) else {
            return LSPEnablementResultForFile::UnsupportedLanguage;
        };
        let Some(root) = self.root_for_workspace(path) else {
            return LSPEnablementResultForFile::LSPNotEnabled { root_name: None };
        };
        let Some(workspace) = self.workspaces.get(root) else {
            return LSPEnablementResultForFile::LSPNotEnabled {
                root_name: root
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string()),
            };
        };

        for (language_server, enablement) in &workspace.language_servers {
            if *enablement == EnablementState::Yes
                && language_server.languages().contains(&language_id)
            {
                return LSPEnablementResultForFile::Enabled;
            }
        }

        LSPEnablementResultForFile::LSPNotEnabled {
            root_name: root
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string()),
        }
    }

    /// Internal method to set LSP server state for a path.
    fn set_lsp_server_for_path(
        &mut self,
        path: &Path,
        server_type: LSPServerType,
        state: EnablementState,
    ) {
        // Check if the workspace needs to be persisted before we take a
        // mutable borrow, so we can call save_to_db without conflicting borrows.
        let needs_persist = self
            .workspaces
            .get(path)
            .is_some_and(|ws| !ws.is_persisted());

        if needs_persist {
            // Materialize the workspace: set a timestamp and persist metadata
            // so the FK-dependent workspace_language_server row can be written.
            let workspace = self.workspaces.get_mut(path).unwrap();
            workspace.metadata.modified_ts = Some(Utc::now());
            let metadata = workspace.metadata.clone();
            self.save_to_db(vec![ModelEvent::UpsertCodebaseIndexMetadata {
                index_metadata: Box::new(metadata),
            }]);
        }

        match self.workspaces.get_mut(path) {
            Some(workspace) => {
                workspace.language_servers.insert(server_type, state);
            }
            None => {
                let metadata = WorkspaceMetadata {
                    path: path.to_path_buf(),
                    navigated_ts: None,
                    // Consider creation as a modification event.
                    modified_ts: Some(Utc::now()),
                    queried_ts: None,
                };

                self.save_to_db(vec![ModelEvent::UpsertCodebaseIndexMetadata {
                    index_metadata: Box::new(metadata.clone()),
                }]);

                self.workspaces.insert(
                    path.to_path_buf(),
                    Workspace {
                        metadata,
                        language_servers: HashMap::from([(server_type, state)]),
                    },
                );
            }
        }

        // Persist the language server setting to database
        self.save_to_db(vec![ModelEvent::UpsertWorkspaceLanguageServer {
            workspace_path: path.to_path_buf(),
            lsp_type: server_type,
            enabled: state,
        }]);
    }

    pub fn root_for_workspace<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.ancestors()
            .find(|&path| self.workspaces.contains_key(path))
    }

    /// Returns the enabled lsp servers for a given repo path.
    pub fn enabled_lsp_servers(
        &self,
        path: &Path,
    ) -> Option<impl Iterator<Item = LSPServerType> + use<'_>> {
        let root = self.root_for_workspace(path)?;

        self.workspaces.get(root).map(|workspace| {
            workspace
                .language_servers
                .iter()
                .filter_map(|(server_type, state)| {
                    if *state == EnablementState::Yes {
                        Some(*server_type)
                    } else {
                        None
                    }
                })
        })
    }

    /// Returns LSP servers for a given workspace path.
    ///
    /// When `include_suggested` is `false`, only persisted entries (`Yes`/`No`)
    /// are returned.  When `true`, in-memory `Suggested` entries are included as
    /// well (useful for showing available-for-download servers in the UI).
    pub fn all_lsp_servers(
        &self,
        path: &Path,
        include_suggested: bool,
    ) -> Option<impl Iterator<Item = (LSPServerType, EnablementState)> + use<'_>> {
        let root = self.root_for_workspace(path)?;

        self.workspaces.get(root).map(move |workspace| {
            workspace
                .language_servers
                .iter()
                .filter(move |(_, state)| {
                    include_suggested || **state != EnablementState::Suggested
                })
                .map(|(server_type, state)| (*server_type, *state))
        })
    }

    /// Asynchronously detects which LSP server types are relevant for the given workspaces
    /// by calling `should_suggest_for_repo` on each `LSPServerType`. Results are stored
    /// as `Suggested` entries in the workspaces map and emitted via `AvailableServersDetected`.
    ///
    /// Workspaces that already have language server entries are skipped (results emitted
    /// immediately) unless `skip_cached` is true, in which case all workspaces are scanned
    /// unconditionally. The workspaces to scan share a single background task and one
    /// interactive PATH capture.
    #[cfg(feature = "local_fs")]
    pub fn detect_available_servers_for_workspaces(
        &mut self,
        workspace_paths: Vec<PathBuf>,
        skip_cached: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // Workspaces that already have entries get an immediate emit; the rest need scanning.
        // When skip_cached is true (initial startup), always scan to pick up new server types.
        let mut paths_to_scan = Vec::new();
        for workspace_path in workspace_paths {
            if !skip_cached {
                if let Some(workspace) = self.workspaces.get(&workspace_path) {
                    if !workspace.language_servers.is_empty() {
                        let servers: Vec<LSPServerType> =
                            workspace.language_servers.keys().copied().collect();
                        ctx.emit(PersistedWorkspaceEvent::AvailableServersDetected {
                            workspace_path,
                            servers,
                        });
                        continue;
                    }
                }
            }
            paths_to_scan.push(workspace_path);
        }

        if paths_to_scan.is_empty() {
            return;
        }

        // Get interactive PATH for should_suggest_for_repo checks
        let path_future = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
            shell_state.get_interactive_path_env_var(ctx)
        });
        let http_client = ServerApiProvider::as_ref(ctx).get_http_client();

        ctx.spawn(
            async move {
                let path_env_var = path_future.await;
                let executor = lsp::CommandBuilder::new(path_env_var);

                let mut results: Vec<(PathBuf, Vec<LSPServerType>)> = Vec::new();
                for workspace_path in paths_to_scan {
                    let mut suggested = Vec::new();
                    for server_type in LSPServerType::all() {
                        let candidate = server_type.candidate(http_client.clone());
                        if candidate
                            .should_suggest_for_repo(&workspace_path, &executor)
                            .await
                        {
                            suggested.push(server_type);
                        }
                    }
                    if !suggested.is_empty() {
                        results.push((workspace_path, suggested));
                    }
                }
                results
            },
            move |me, results, ctx| {
                for (workspace_path, servers) in results {
                    // Insert Suggested entries into the workspace, without
                    // overwriting existing Yes/No entries.
                    let workspace =
                        me.workspaces
                            .entry(workspace_path.clone())
                            .or_insert_with(|| Workspace {
                                metadata: WorkspaceMetadata {
                                    path: workspace_path.clone(),
                                    navigated_ts: None,
                                    modified_ts: None,
                                    queried_ts: None,
                                },
                                language_servers: HashMap::new(),
                            });

                    for &server_type in &servers {
                        workspace
                            .language_servers
                            .entry(server_type)
                            .or_insert(EnablementState::Suggested);
                    }

                    ctx.emit(PersistedWorkspaceEvent::AvailableServersDetected {
                        workspace_path,
                        servers,
                    });
                }
            },
        );
    }

    /// Returns the total count of LSP servers across all workspaces.
    ///
    /// When `include_suggested` is `false`, only persisted entries (`Yes`/`No`)
    /// are counted.  When `true`, in-memory `Suggested` entries are counted too.
    pub fn total_lsp_server_count(&self, include_suggested: bool) -> usize {
        self.workspaces
            .values()
            .map(|workspace| {
                workspace
                    .language_servers
                    .values()
                    .filter(|state| include_suggested || **state != EnablementState::Suggested)
                    .count()
            })
            .sum()
    }

    fn on_settings_changed(&mut self, ctx: &mut ModelContext<Self>) {
        Self::maybe_enable_codebase_indexing(ctx);
    }

    pub fn on_user_changed(&self, ctx: &mut ModelContext<Self>) {
        Self::maybe_enable_codebase_indexing(ctx);
    }

    /// Enables or disables codebase indexing according to the setting.
    fn maybe_enable_codebase_indexing(ctx: &mut ModelContext<Self>) {
        CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
            if !manager.is_indexing_enabled() {
                return;
            }
            if UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx) {
                Self::enable_codebase_indexing(manager, ctx);
            } else {
                manager.reset_codebase_indexing(ctx);
            }
        });
    }

    fn enable_codebase_indexing(
        manager: &mut CodebaseIndexManager,
        ctx: &mut ModelContext<CodebaseIndexManager>,
    ) {
        let request_model = AIRequestUsageModel::handle(ctx);
        let codebase_limits = request_model.as_ref(ctx).codebase_context_limits();
        manager.update_max_limits(
            codebase_limits.max_indices_allowed,
            codebase_limits.max_files_per_repo,
            codebase_limits.embedding_generation_batch_size,
            ctx,
        );

        #[cfg(feature = "local_fs")]
        for dir in all_working_directories(ctx) {
            // Auto-index working directory ONLY if the user has "Read files" set to "Always allow" OR this directory is in the allowlist.
            let auto_indexing_enabled = *CodeSettings::as_ref(ctx).auto_indexing_enabled;

            if auto_indexing_enabled {
                if let Some(root) = DetectedRepositories::as_ref(ctx)
                    .get_root_for_path(&warp_util::local_or_remote_path::LocalOrRemotePath::Local(
                        dir.clone(),
                    ))
                    .and_then(|r| r.to_local_path().map(std::path::Path::to_path_buf))
                {
                    manager.index_directory(root, ctx);
                }
            }
        }
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn index_repo(&self, directory_path: PathBuf, ctx: &mut ModelContext<Self>) {
        ProjectContextModel::handle(ctx).update(ctx, |model, ctx| {
            let _ = model.index_and_store_rules(directory_path.clone(), ctx);
        });

        if FeatureFlag::FullSourceCodeEmbedding.is_enabled() {
            let auto_indexing_enabled = UserWorkspaces::as_ref(ctx)
                .is_codebase_context_enabled(ctx)
                && *CodeSettings::as_ref(ctx).auto_indexing_enabled;

            if auto_indexing_enabled {
                CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.index_directory(directory_path, ctx);
                });
            }
        }
    }

    /// Explicitly registers a directory as a workspace, as if the user had navigated there.
    ///
    /// Creates or updates the entry with `navigated_ts = now`, persists to SQLite,
    /// triggers project-rules and codebase-index scanning, and emits
    /// [`PersistedWorkspaceEvent::WorkspaceAdded`] so subscribers can refresh their UI.
    pub fn user_added_workspace(&mut self, path: PathBuf, ctx: &mut ModelContext<Self>) {
        let now = Utc::now();

        match self.workspaces.get_mut(&path) {
            Some(workspace) => {
                workspace.metadata.navigated_ts = Some(now);
            }
            None => {
                self.workspaces.insert(
                    path.clone(),
                    Workspace {
                        metadata: WorkspaceMetadata {
                            path: path.clone(),
                            navigated_ts: Some(now),
                            modified_ts: None,
                            queried_ts: None,
                        },
                        language_servers: HashMap::new(),
                    },
                );
            }
        }

        self.persist_metadata_for_index(&path);
        self.index_repo(path.clone(), ctx);
        ctx.emit(PersistedWorkspaceEvent::WorkspaceAdded { path });
    }

    pub fn workspaces<'a>(&'a self) -> impl Iterator<Item = WorkspaceMetadata> + use<'a> {
        self.workspaces
            .values()
            .filter(|workspace| workspace.is_persisted())
            .map(|workspace| workspace.metadata.clone())
            .sorted_by(WorkspaceMetadata::most_recently_touched)
            .dedup_by(|a, b| a.path == b.path)
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn navigated_to_path(&mut self, directory: &PathBuf) {
        if let Some(workspace) = self.workspaces.get_mut(directory) {
            workspace.metadata.navigated_ts = Some(Utc::now());
            self.persist_metadata_for_index(directory);
        }
    }

    fn handle_index_metadata_event(&mut self, root_path: &PathBuf, event: WorkspaceMetadataEvent) {
        match event {
            WorkspaceMetadataEvent::Queried => {
                if let Some(workspace) = self.workspaces.get_mut(root_path) {
                    workspace.metadata.queried_ts = Some(Utc::now());
                }
                self.persist_metadata_for_index(root_path);
            }
            WorkspaceMetadataEvent::Modified => {
                if let Some(workspace) = self.workspaces.get_mut(root_path) {
                    workspace.metadata.modified_ts = Some(Utc::now());
                }
                self.persist_metadata_for_index(root_path);
            }
            WorkspaceMetadataEvent::Created => {
                let new_metadata = WorkspaceMetadata {
                    path: root_path.clone(),
                    navigated_ts: None,
                    // Count creation as a modification event.
                    modified_ts: Some(Utc::now()),
                    queried_ts: None,
                };

                if let Some(existing) = self.workspaces.get_mut(root_path) {
                    // Preserve existing language server settings when re-creating
                    // workspace metadata (e.g. after an expired index is cleaned up
                    // and the user navigates back to the same directory).
                    existing.metadata = new_metadata;
                } else {
                    self.workspaces.insert(
                        root_path.clone(),
                        Workspace {
                            metadata: new_metadata,
                            language_servers: HashMap::new(),
                        },
                    );
                }
                self.persist_metadata_for_index(root_path);
            }
        }
    }

    pub fn workspace_for_path(&self, root_path: &Path) -> Option<WorkspaceMetadata> {
        self.workspaces
            .get(root_path)
            .map(|workspace| workspace.metadata.clone())
    }

    fn persist_metadata_for_index(&self, path: &PathBuf) {
        log::info!("Saving workspace metadata for {path:?} to SQLite");

        if let Some(single_metadata) = self.workspace_for_path(path) {
            self.save_to_db(vec![ModelEvent::UpsertCodebaseIndexMetadata {
                index_metadata: Box::new(single_metadata),
            }]);
        }
    }

    /// Triggers an incremental sync for the codebase context when a new conversation starts.
    /// This ensures that the codebase index is up-to-date before the conversation begins.
    fn trigger_incremental_sync_for_conversation(
        &mut self,
        terminal_view_id: warpui::EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx) {
            return;
        }

        // Get the current working directory for the terminal view that started the conversation
        // Collect window IDs first to avoid borrowing conflicts
        let window_ids: Vec<_> = ctx.window_ids().collect();

        for window_id in window_ids {
            let terminal_views = ctx.views_of_type::<TerminalView>(window_id);

            for terminal_view in terminal_views.into_iter().flatten() {
                let terminal_view_ref = terminal_view.as_ref(ctx);
                if terminal_view_ref.view_id() == terminal_view_id {
                    if let Some(pwd) = terminal_view_ref.pwd() {
                        let directory_path = Path::new(&pwd);

                        // Trigger an incremental sync through the CodebaseIndexManager
                        CodebaseIndexManager::handle(ctx).update(ctx, |codebase_manager, ctx| {
                            if let Err(e) = codebase_manager
                                .trigger_incremental_sync_for_path(directory_path, ctx)
                            {
                                log::warn!("Failed to trigger incremental sync {e}");
                            }
                        });
                    }
                    return; // Found the terminal view, exit both loops
                }
            }
        }
    }

    fn clean_up_expired_metadata(
        &self,
        indices_to_remove: Arc<Vec<PathBuf>>,
        _ctx: &mut ModelContext<Self>,
    ) {
        log::info!("Cleaning up index metadata from SQLite");

        let indices_to_remove = indices_to_remove.as_ref();
        self.save_to_db(indices_to_remove.iter().filter_map(|path| {
            let Some(ws) = self.workspaces.get(path) else {
                return Some(ModelEvent::DeleteCodebaseIndexMetadata {
                    repo_path: path.to_path_buf(),
                });
            };

            // Skip non-persisted workspaces — they have no DB row to delete.
            if !ws.is_persisted() {
                return None;
            }

            // Don't delete workspace metadata rows for workspaces that have
            // persisted LSP server settings (Yes/No).
            //
            // Deleting workspace_metadata rows would orphan corresponding
            // workspace_language_server rows (FK'd without ON DELETE CASCADE).
            // On next app load, the inner_join used to load workspace language
            // servers will silently drop orphaned rows, making enabled
            // language servers appear disabled.
            let has_persisted_servers = ws
                .language_servers
                .values()
                .any(|s| *s != EnablementState::Suggested);
            if has_persisted_servers {
                return None;
            }

            Some(ModelEvent::DeleteCodebaseIndexMetadata {
                repo_path: path.to_path_buf(),
            })
        }));
    }

    #[cfg(feature = "local_fs")]
    fn clean_up_deleted_indices(&self, ctx: &mut ModelContext<Self>) {
        CodebaseIndexManager::handle(ctx).update(ctx, |codebase_manager, ctx| {
            codebase_manager.clean_up_deleted_indices(ctx);
        });
    }

    fn save_to_db(&self, events: impl IntoIterator<Item = ModelEvent>) {
        let model_event_sender = self.model_event_sender.clone();
        if let Some(model_event_sender) = &model_event_sender {
            for event in events {
                report_if_error!(model_event_sender
                    .send(event)
                    .with_context(|| "Unable to save codebase index metadata to sqlite"));
            }
        }
    }

    /// Installs the LSP server for the given file path and enables it.
    /// This is used when the server is not yet installed.
    #[cfg(feature = "local_fs")]
    fn handle_install_lsp(
        &mut self,
        file_path: PathBuf,
        repo_root: PathBuf,
        server_type: LSPServerType,
        path_env_var: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Early return if already installing to prevent duplicate installations from repeated clicks
        if self.lsp_installation_status.get(&server_type)
            == Some(&LSPInstallationStatus::Installing)
        {
            return;
        }

        // Set Installing state before spawning async installation
        self.lsp_installation_status
            .insert(server_type, LSPInstallationStatus::Installing);
        ctx.emit(PersistedWorkspaceEvent::InstallStatusUpdate {
            server_type,
            status: LSPInstallationStatus::Installing,
        });

        let repo_root_clone = repo_root.clone();
        let file_path_clone = file_path.clone();
        let executor = lsp::CommandBuilder::new(path_env_var);
        let http_client = ServerApiProvider::as_ref(ctx).get_http_client();
        ctx.spawn(
            async move {
                let candidate = server_type.candidate(http_client);
                let metadata = candidate.fetch_latest_server_metadata().await?;
                candidate.install(metadata, &executor).await?;
                Ok::<_, anyhow::Error>(())
            },
            move |me, result, ctx| match result {
                Ok(()) => {
                    // Enable the LSP server
                    me.enable_lsp_server_for_path(&repo_root_clone, server_type);

                    // Update installation status cache
                    me.lsp_installation_status
                        .insert(server_type, LSPInstallationStatus::Installed);

                    // Show success toast
                    if let Some(window_id) = WindowManager::as_ref(ctx).active_window() {
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            toast_stack.add_ephemeral_toast(
                                DismissibleToast::success(format!(
                                    "{} installed and enabled successfully.",
                                    server_type.binary_name()
                                )),
                                window_id,
                                ctx,
                            );
                        });
                    }

                    ctx.emit(PersistedWorkspaceEvent::InstallationSucceeded);

                    // Also emit status update so listeners can update their UI
                    ctx.emit(PersistedWorkspaceEvent::InstallStatusUpdate {
                        server_type,
                        status: LSPInstallationStatus::Installed,
                    });

                    // Spawn the server now that it's installed and enabled.
                    // This is done here so it happens exactly once, rather
                    // than relying on each subscriber to spawn independently.
                    me.execute_lsp_task(
                        LspTask::Spawn {
                            file_path: file_path_clone,
                        },
                        ctx,
                    );
                }
                Err(e) => {
                    log::info!("Failed to install LSP server: {e}");

                    // Update installation status to NotInstalled
                    me.lsp_installation_status
                        .insert(server_type, LSPInstallationStatus::NotInstalled);

                    // Show error toast
                    if let Some(window_id) = WindowManager::as_ref(ctx).active_window() {
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            toast_stack.add_ephemeral_toast(
                                DismissibleToast::error(format!(
                                    "Failed to install {}: {}",
                                    server_type.binary_name(),
                                    e
                                )),
                                window_id,
                                ctx,
                            );
                        });
                    }

                    ctx.emit(PersistedWorkspaceEvent::InstallationFailed);

                    // Also emit status update so listeners can update their UI
                    ctx.emit(PersistedWorkspaceEvent::InstallStatusUpdate {
                        server_type,
                        status: LSPInstallationStatus::NotInstalled,
                    });
                }
            },
        );
    }

    /// Starts all enabled LSP servers for the given file path.
    /// This looks up the workspace root and starts any servers that are enabled but not yet running.
    #[cfg(feature = "local_fs")]
    fn handle_spawn_lsp(
        &self,
        file_path: &Path,
        path_env_var: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(workspace_root) = self.root_for_workspace(file_path) else {
            return;
        };

        let Some(servers) = self.enabled_lsp_servers(workspace_root) else {
            return;
        };

        let supported_servers = servers.collect::<Vec<LSPServerType>>();

        if supported_servers.is_empty() {
            return;
        }

        let mut new_servers_available_to_start = false;
        let workspace_root = workspace_root.to_path_buf();

        for server in supported_servers {
            if LspManagerModel::as_ref(ctx).server_registered_and_started(
                &workspace_root,
                server,
                ctx,
            ) {
                continue;
            }

            log::info!(
                "Starting {} LSP server for {}",
                server.binary_name(),
                workspace_root.display()
            );
            let log_relative_path =
                crate::code::lsp_logs::relative_log_path(server, &workspace_root);
            let http_client = ServerApiProvider::as_ref(ctx).get_http_client();
            let config = LspServerConfig::new(
                server,
                workspace_root.clone(),
                path_env_var.clone(),
                ChannelState::app_id().application_name().to_string(),
                http_client,
            )
            .with_log_relative_path(log_relative_path);

            LspManagerModel::handle(ctx).update(ctx, |manager, m_ctx| {
                manager.register(workspace_root.clone(), config, m_ctx);
            });
            new_servers_available_to_start = true;
        }

        if !new_servers_available_to_start {
            return;
        }

        let lsp_manager_handle = LspManagerModel::handle(ctx);
        lsp_manager_handle.update(ctx, |manager, m_ctx| {
            manager.start_all(workspace_root.clone(), m_ctx);
        });

        // Subscribe to LSP server events to show error toast on failure.
        let workspace_root_display = workspace_root.display().to_string();
        let servers = lsp_manager_handle
            .as_ref(ctx)
            .servers_for_workspace(&workspace_root)
            .cloned()
            .unwrap_or_default();

        for server in servers {
            let workspace_root_display = workspace_root_display.clone();
            let server_type_name = server.as_ref(ctx).server_name();
            ctx.subscribe_to_model(&server, move |_me, event, ctx| match event {
                LspEvent::Started => {
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ServerStarted {
                            server_type: server_type_name.clone(),
                        },
                        ctx
                    );
                }
                LspEvent::Failed(e) => {
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ServerFailed {
                            server_type: server_type_name.clone(),
                        },
                        ctx
                    );
                    if let Some(window_id) = WindowManager::as_ref(ctx).active_window()
                    {
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            let toast = DismissibleToast::error(format!(
                                "Failed to start LSP server for {workspace_root_display} with error {e}",
                            ));
                            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                        });
                    }
                }
                _ => {}
            });
        }

        // Once we start a LSP server, also start the garbage collection process if it is not active.
        LanguageServerShutdownManager::handle(ctx).update(ctx, |shutdown_manager, ctx| {
            if !shutdown_manager.has_in_progress_scan() {
                shutdown_manager.schedule_next_scan(ctx);
            }
        });
    }

    /// Executes an LSP task after capturing the interactive shell PATH.
    /// This is the main entry point for LSP operations that need the full PATH.
    #[cfg(feature = "local_fs")]
    pub fn execute_lsp_task(&mut self, task: LspTask, ctx: &mut ModelContext<Self>) {
        // For Spawn tasks, check synchronously whether there are any enabled LSP
        // servers for this workspace before kicking off the expensive interactive
        // shell PATH capture.
        if let LspTask::Spawn { ref file_path } = task {
            let has_servers = self
                .root_for_workspace(file_path)
                .and_then(|root| self.enabled_lsp_servers(root))
                .is_some_and(|mut servers| servers.next().is_some());
            if !has_servers {
                return;
            }
        }

        // Get a future for the interactive PATH
        let path_future = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
            shell_state.get_interactive_path_env_var(ctx)
        });

        ctx.spawn(path_future, move |me, path_env_var, ctx| match task {
            LspTask::Install {
                file_path,
                repo_root,
                server_type,
            } => {
                me.handle_install_lsp(file_path, repo_root, server_type, path_env_var, ctx);
            }
            LspTask::Spawn { file_path } => {
                me.handle_spawn_lsp(&file_path, path_env_var, ctx);
            }
        });
    }

    /// Kicks off detection (deduped via Checking) and returns the best immediate status.
    /// Uses the interactive shell PATH for detection to ensure gopls and other tools
    /// installed in user-specific locations (like ~/go/bin) are found.
    ///
    /// Logic:
    /// 1. If enabled for repo => Enabled
    /// 2. If not enabled and Installed => DisabledAndInstalled
    /// 3. If NotInstalled => DisabledAndNotInstalled
    /// 4. If Installing => Installing
    /// 5. If Checking or Unknown => set Checking, start detection, return CheckingForInstallation
    #[cfg(feature = "local_fs")]
    pub fn detect_lsp_workspace_status(
        &mut self,
        repo_root: PathBuf,
        server_type: LSPServerType,
        ctx: &mut ModelContext<Self>,
    ) -> LspRepoStatus {
        // Determine enablement
        let is_enabled = self
            .enabled_lsp_servers(&repo_root)
            .map(|mut it| it.any(|s| s == server_type))
            .unwrap_or(false);

        // If enabled, do not check installation.
        if is_enabled {
            return LspRepoStatus::Enabled;
        }

        match self.lsp_installation_status.get(&server_type).copied() {
            Some(LSPInstallationStatus::Installed) => {
                LspRepoStatus::DisabledAndInstalled { server_type }
            }
            Some(LSPInstallationStatus::NotInstalled) => {
                LspRepoStatus::DisabledAndNotInstalled { server_type }
            }
            Some(LSPInstallationStatus::Checking) => LspRepoStatus::CheckingForInstallation,
            Some(LSPInstallationStatus::Installing) => LspRepoStatus::Installing { server_type },
            None => {
                // Mark as checking and start async detection with interactive PATH
                self.lsp_installation_status
                    .insert(server_type, LSPInstallationStatus::Checking);

                // Get a future for the interactive PATH
                let path_future = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
                    shell_state.get_interactive_path_env_var(ctx)
                });

                let http_client = ServerApiProvider::as_ref(ctx).get_http_client();
                ctx.spawn(
                    async move {
                        // Wait for interactive PATH, then check installation
                        let path_env_var = path_future.await;
                        let executor = lsp::CommandBuilder::new(path_env_var);
                        let candidate = server_type.candidate(http_client);
                        candidate.is_installed(&executor).await
                    },
                    move |me, is_installed, ctx| {
                        let status = if is_installed {
                            LSPInstallationStatus::Installed
                        } else {
                            LSPInstallationStatus::NotInstalled
                        };
                        me.lsp_installation_status.insert(server_type, status);
                        ctx.emit(PersistedWorkspaceEvent::InstallStatusUpdate {
                            server_type,
                            status,
                        });
                    },
                );

                LspRepoStatus::CheckingForInstallation
            }
        }
    }
}

fn send_active_indexed_repos_changed_telemetry<T: Entity>(ctx: &mut ModelContext<T>) {
    let total = CodebaseIndexManager::as_ref(ctx).num_active_indices();
    let hit_max = AIRequestUsageModel::as_ref(ctx).hit_codebase_index_limit(total);
    send_telemetry_from_ctx!(
        TelemetryEvent::ActiveIndexedReposChanged {
            updated_number_of_codebase_indices: total,
            hit_max_indices: hit_max
        },
        ctx
    );
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub fn all_working_directories(app: &AppContext) -> HashSet<PathBuf> {
    let mut working_directories = HashSet::new();
    for window_id in app.window_ids() {
        for terminal_view in app
            .views_of_type::<TerminalView>(window_id)
            .into_iter()
            .flatten()
            .map(|handle| handle.as_ref(app))
        {
            let working_directory = terminal_view.pwd();
            if let Some(dir) = working_directory {
                working_directories.insert(dir.into());
            }
        }
    }
    working_directories
}
