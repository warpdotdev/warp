//! Unified repository metadata model.
//!
//! [`RepoMetadataModel`] is the singleton entry point for all repository metadata
//! queries. It holds handles to [`LocalRepoMetadataModel`] and
//! [`RemoteRepoMetadataModel`] and dispatches operations based on
//! [`RepositoryIdentifier`].

#[cfg(feature = "local_fs")]
use std::path::Path;

use warp_core::HostId;
use warp_util::standardized_path::StandardizedPath;
use warpui_core::{AppContext, ModelContext, ModelHandle, SingletonEntity};

use crate::file_tree_store::FileTreeState;
use crate::file_tree_update::{MetadataUpdateType, RepoMetadataUpdate};
use crate::local_model::{
    GetContentsArgs, IndexedRepoState, LocalRepoMetadataModel, RepoContents,
    RepositoryMetadataEvent,
};
use crate::remote_model::{RemoteRepoMetadataModel, RemoteRepositoryMetadataEvent};
use crate::repository_identifier::{RemoteRepositoryIdentifier, RepositoryIdentifier};
use crate::{RepoMetadataError, StandingQueryResults, StandingQueryResultsDelta};

/// Unified events emitted by the [`RepoMetadataModel`] wrapper.
///
/// These are mapped from the sub-model events into a common enum keyed by
/// [`RepositoryIdentifier`].
#[derive(Debug)]
pub enum RepoMetadataEvent {
    /// A repository was added or updated.
    RepositoryUpdated { id: RepositoryIdentifier },
    /// A repository was removed.
    RepositoryRemoved { id: RepositoryIdentifier },
    /// File trees for repositories were updated.
    FileTreeUpdated { ids: Vec<RepositoryIdentifier> },
    /// A file tree entry was updated.
    FileTreeEntryUpdated {
        id: RepositoryIdentifier,
        /// Specifies whether this event contains a precise delta or requires a conservative
        /// refresh because the entry was replaced without one.
        update_type: MetadataUpdateType,
    },
    /// Stored standing-query paths changed for a repository.
    StandingQueryResultsUpdated {
        id: RepositoryIdentifier,
        delta: StandingQueryResultsDelta,
    },
    /// Updating a repository failed.
    UpdatingRepositoryFailed { id: RepositoryIdentifier },
    /// An incremental file tree update is ready to be sent to the remote
    /// client. Only emitted when the local model has
    /// `emit_incremental_updates` enabled.
    IncrementalUpdateReady { update: RepoMetadataUpdate },
}

/// Singleton wrapper that provides a unified API over local and remote
/// repository metadata models.
///
/// All consumers should interact with this type rather than accessing the
/// sub-models directly. The wrapper does **not** expose `.local()` or
/// `.remote()` accessors — encapsulation ensures consumers are decoupled
/// from the local/remote split.
pub struct RepoMetadataModel {
    local: ModelHandle<LocalRepoMetadataModel>,
    remote: ModelHandle<RemoteRepoMetadataModel>,
}

impl RepoMetadataModel {
    /// Creates a new `RepoMetadataModel`, instantiating both sub-models and
    /// subscribing to their events for forwarding.
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let local = ctx.add_model(LocalRepoMetadataModel::new);
        let remote = ctx.add_model(RemoteRepoMetadataModel::new);

        ctx.subscribe_to_model(&local, Self::forward_local_event);
        ctx.subscribe_to_model(&remote, Self::forward_remote_event);

        Self { local, remote }
    }

    /// Creates a new `RepoMetadataModel` with incremental update emission
    /// enabled on the local sub-model. Used by the remote server.
    pub fn new_with_incremental_updates(ctx: &mut ModelContext<Self>) -> Self {
        let local = ctx.add_model(|ctx| {
            let mut model = LocalRepoMetadataModel::new(ctx);
            model.set_emit_incremental_updates(true);
            model
        });
        let remote = ctx.add_model(RemoteRepoMetadataModel::new);

        ctx.subscribe_to_model(&local, Self::forward_local_event);
        ctx.subscribe_to_model(&remote, Self::forward_remote_event);

        Self { local, remote }
    }

    // ── Event forwarding ─────────────────────────────────────────────

    fn forward_local_event(
        &mut self,
        _: ModelHandle<LocalRepoMetadataModel>,
        event: &RepositoryMetadataEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let unified = match event {
            RepositoryMetadataEvent::RepositoryUpdated { path } => {
                RepoMetadataEvent::RepositoryUpdated {
                    id: RepositoryIdentifier::local(path.clone()),
                }
            }
            RepositoryMetadataEvent::RepositoryRemoved { path } => {
                RepoMetadataEvent::RepositoryRemoved {
                    id: RepositoryIdentifier::local(path.clone()),
                }
            }
            RepositoryMetadataEvent::FileTreeUpdated { paths } => {
                RepoMetadataEvent::FileTreeUpdated {
                    ids: paths
                        .iter()
                        .map(|p| RepositoryIdentifier::local(p.clone()))
                        .collect(),
                }
            }
            RepositoryMetadataEvent::FileTreeEntryUpdated { path, update_type } => {
                RepoMetadataEvent::FileTreeEntryUpdated {
                    id: RepositoryIdentifier::local(path.clone()),
                    update_type: update_type.clone(),
                }
            }
            RepositoryMetadataEvent::StandingQueryResultsUpdated { path, delta } => {
                RepoMetadataEvent::StandingQueryResultsUpdated {
                    id: RepositoryIdentifier::local(path.clone()),
                    delta: delta.clone(),
                }
            }
            RepositoryMetadataEvent::UpdatingRepositoryFailed { path } => {
                RepoMetadataEvent::UpdatingRepositoryFailed {
                    id: RepositoryIdentifier::local(path.clone()),
                }
            }
            RepositoryMetadataEvent::IncrementalUpdateReady { update } => {
                RepoMetadataEvent::IncrementalUpdateReady {
                    update: update.clone(),
                }
            }
        };
        ctx.emit(unified);
    }

    fn forward_remote_event(
        &mut self,
        _: ModelHandle<RemoteRepoMetadataModel>,
        event: &RemoteRepositoryMetadataEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let unified = match event {
            RemoteRepositoryMetadataEvent::RepositoryUpdated { id } => {
                RepoMetadataEvent::RepositoryUpdated {
                    id: RepositoryIdentifier::Remote(id.clone()),
                }
            }
            RemoteRepositoryMetadataEvent::RepositoryRemoved { id } => {
                RepoMetadataEvent::RepositoryRemoved {
                    id: RepositoryIdentifier::Remote(id.clone()),
                }
            }
            RemoteRepositoryMetadataEvent::FileTreeUpdated { ids } => {
                RepoMetadataEvent::FileTreeUpdated {
                    ids: ids
                        .iter()
                        .cloned()
                        .map(RepositoryIdentifier::Remote)
                        .collect(),
                }
            }
            RemoteRepositoryMetadataEvent::FileTreeEntryUpdated { id, update_type } => {
                RepoMetadataEvent::FileTreeEntryUpdated {
                    id: RepositoryIdentifier::Remote(id.clone()),
                    update_type: update_type.clone(),
                }
            }
            RemoteRepositoryMetadataEvent::StandingQueryResultsUpdated { id, delta } => {
                RepoMetadataEvent::StandingQueryResultsUpdated {
                    id: RepositoryIdentifier::Remote(id.clone()),
                    delta: delta.clone(),
                }
            }
        };
        ctx.emit(unified);
    }

    // ── Unified query API ────────────────────────────────────────────

    /// Returns the [`FileTreeState`] for a repository identified by `id`.
    pub fn get_repository<'a>(
        &self,
        id: &RepositoryIdentifier,
        ctx: &'a AppContext,
    ) -> Option<&'a FileTreeState> {
        match id {
            RepositoryIdentifier::Local(path) => self.local.as_ref(ctx).get_repository(path),
            RepositoryIdentifier::Remote(remote_id) => {
                self.remote.as_ref(ctx).get_repository(remote_id)
            }
        }
    }

    pub fn standing_query_results<'a>(
        &self,
        id: &RepositoryIdentifier,
        ctx: &'a AppContext,
    ) -> Option<&'a StandingQueryResults> {
        match id {
            RepositoryIdentifier::Local(path) => {
                self.local.as_ref(ctx).standing_query_results(path)
            }
            RepositoryIdentifier::Remote(remote_id) => {
                self.remote.as_ref(ctx).standing_query_results(remote_id)
            }
        }
    }

    /// Returns whether the given repository is indexed.
    pub fn has_repository(&self, id: &RepositoryIdentifier, ctx: &AppContext) -> bool {
        match id {
            RepositoryIdentifier::Local(path) => self.local.as_ref(ctx).has_repository(path),
            RepositoryIdentifier::Remote(remote_id) => {
                self.remote.as_ref(ctx).has_repository(remote_id)
            }
        }
    }

    /// Returns the current [`IndexedRepoState`] for a repository.
    pub fn repository_state<'a>(
        &self,
        id: &RepositoryIdentifier,
        ctx: &'a AppContext,
    ) -> Option<&'a IndexedRepoState> {
        match id {
            RepositoryIdentifier::Local(path) => self.local.as_ref(ctx).repository_state(path),
            RepositoryIdentifier::Remote(remote_id) => {
                self.remote.as_ref(ctx).repository_state(remote_id)
            }
        }
    }

    /// Returns a future that resolves once repository indexing has completed at least once.
    ///
    /// Callers should inspect [`Self::repository_state`] after awaiting this future to see whether
    /// indexing succeeded or failed.
    pub fn repository_indexed(
        &self,
        id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) -> futures::future::BoxFuture<'static, ()> {
        match id {
            RepositoryIdentifier::Local(path) => {
                let path = path.clone();
                self.local
                    .update(ctx, |local, _| local.repository_indexed(&path))
            }
            RepositoryIdentifier::Remote(remote_id) => {
                let remote_id = remote_id.clone();
                self.remote
                    .update(ctx, |remote, _| remote.repository_indexed(&remote_id))
            }
        }
    }

    /// Returns repository contents for the specified repository.
    ///
    /// The number of returned entries is capped; when the repository contains
    /// more matching entries, the result is truncated and
    /// [`RepoContents::truncated`] is set to `true`.
    ///
    /// Returns an error if the repository is not indexed, indexing is pending, or indexing failed.
    pub fn get_repo_contents<'a>(
        &self,
        id: &RepositoryIdentifier,
        args: GetContentsArgs,
        ctx: &'a AppContext,
    ) -> Result<RepoContents<'a>, RepoMetadataError> {
        match id {
            RepositoryIdentifier::Local(path) => {
                self.local.as_ref(ctx).get_repo_contents(path, args)
            }
            RepositoryIdentifier::Remote(remote_id) => {
                self.remote.as_ref(ctx).get_repo_contents(remote_id, args)
            }
        }
    }

    /// Finds the repository root that contains the given local path.
    #[cfg(feature = "local_fs")]
    pub fn find_repository_for_path(
        &self,
        path: &Path,
        ctx: &AppContext,
    ) -> Option<StandardizedPath> {
        self.local.as_ref(ctx).find_repository_for_path(path)
    }

    // ── Local-specific operations ────────────────────────────────────
    // These delegate to the local sub-model. Remote equivalents will be
    // added once the remote client ↔ server sync layer is in place.

    /// Fully indexes a local directory identified by a standardized path.
    #[cfg(feature = "local_fs")]
    pub fn index_local_directory_path(
        &self,
        path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        let path = path.clone();
        self.local
            .update(ctx, |local, ctx| local.index_directory_path(&path, ctx))
    }

    /// Indexes a local repository from the given repository handle.
    #[cfg(feature = "local_fs")]
    pub fn index_directory(
        &self,
        repository: ModelHandle<crate::repository::Repository>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        self.local
            .update(ctx, |local, ctx| local.index_directory(repository, ctx))
    }

    /// Lazily indexes a local standalone path with only the first level of children.
    #[cfg(feature = "local_fs")]
    pub fn index_lazy_loaded_path(
        &self,
        path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        let path = path.clone();
        self.local
            .update(ctx, |local, ctx| local.index_lazy_loaded_path(&path, ctx))
    }

    /// Lazily indexes a local git repository root with only the first level of
    /// children, keeping the repo's gitignores so ignore-tagging and on-demand
    /// loads match the eagerly-indexed tree.
    #[cfg(feature = "local_fs")]
    pub fn index_lazy_loaded_repo_root(
        &self,
        path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        let path = path.clone();
        self.local.update(ctx, |local, ctx| {
            local.index_lazy_loaded_repo_root(&path, ctx)
        })
    }

    /// Loads a specific directory inside an already-tracked local tree.
    #[cfg(feature = "local_fs")]
    pub fn load_directory(
        &self,
        repo_root: &StandardizedPath,
        dir_path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        let repo_root = repo_root.clone();
        let dir_path = dir_path.clone();
        self.local.update(ctx, |local, ctx| {
            local.load_directory(&repo_root, &dir_path, ctx)
        })
    }

    /// Registers paths that must be loaded even when gitignored or beyond the
    /// tree's size limit.
    ///
    /// This delegates to the local model because force-included path matching
    /// happens while building local file trees. Remote repositories receive the
    /// resulting file-tree metadata over the existing remote sync protocol.
    pub fn register_force_included_paths(
        &self,
        paths: impl IntoIterator<Item = std::path::PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        let paths: Vec<_> = paths.into_iter().collect();
        self.local.update(ctx, |local, _| {
            local.register_force_included_paths(paths);
        });
    }

    pub fn set_project_skill_provider_paths(
        &self,
        paths: impl IntoIterator<Item = std::path::PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        let paths: Vec<_> = paths.into_iter().collect();
        self.local.update(ctx, |local, _| {
            local.set_project_skill_provider_paths(paths);
        });
    }

    /// Removes a lazily-loaded local standalone path from tracking.
    #[cfg(feature = "local_fs")]
    pub fn remove_lazy_loaded_path(&self, path: &StandardizedPath, ctx: &mut ModelContext<Self>) {
        let path = path.clone();
        self.local
            .update(ctx, |local, ctx| local.remove_lazy_loaded_path(&path, ctx));
    }

    // ── Remote-specific operations ─────────────────────────────────
    // These delegate to the remote sub-model and are called by the
    // RemoteServerManager event subscription in the app layer.

    /// Inserts or replaces a remote repository from a snapshot push event.
    pub fn insert_remote_snapshot(
        &self,
        host_id: HostId,
        update: &RepoMetadataUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        self.remote.update(ctx, |remote, ctx| {
            remote.insert_from_snapshot(host_id, update, ctx);
        });
    }

    /// Applies an incremental remote repo metadata update.
    pub fn apply_remote_incremental_update(
        &self,
        host_id: &HostId,
        update: &RepoMetadataUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        let host_id = host_id.clone();
        self.remote.update(ctx, |remote, ctx| {
            remote.apply_incremental_update(&host_id, update, ctx);
        });
    }

    /// Removes all remote repositories for the given host (e.g. on disconnect).
    pub fn remove_remote_repositories_for_host(
        &self,
        host_id: &HostId,
        ctx: &mut ModelContext<Self>,
    ) {
        let host_id = host_id.clone();
        self.remote.update(ctx, |remote, ctx| {
            remote.remove_repositories_for_host(&host_id, ctx);
        });
    }

    /// Removes a repository (local or remote) from tracking.
    pub fn remove_repository(
        &self,
        id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), RepoMetadataError> {
        match id {
            RepositoryIdentifier::Local(path) => {
                let path = path.clone();
                self.local
                    .update(ctx, |local, ctx| local.remove_repository(&path, ctx))
            }
            RepositoryIdentifier::Remote(remote_id) => {
                let remote_id = remote_id.clone();
                self.remote
                    .update(ctx, |remote, ctx| remote.remove_repository(&remote_id, ctx));
                Ok(())
            }
        }
    }

    /// Returns all tracked remote repository identifiers.
    pub fn remote_repository_ids<'a>(
        &self,
        ctx: &'a AppContext,
    ) -> impl Iterator<Item = &'a RemoteRepositoryIdentifier> {
        self.remote.as_ref(ctx).remote_repository_ids()
    }

    /// Returns whether the given local path is tracked as a lazily-loaded standalone path.
    pub fn is_lazy_loaded_path(&self, path: &StandardizedPath, ctx: &AppContext) -> bool {
        self.local.as_ref(ctx).is_lazy_loaded_path(path)
    }
}

impl warpui_core::Entity for RepoMetadataModel {
    type Event = RepoMetadataEvent;
}

impl SingletonEntity for RepoMetadataModel {}

#[cfg(any(test, feature = "test-util"))]
impl RepoMetadataModel {
    /// Inserts repository state directly into the local sub-model for testing.
    pub fn insert_test_state(
        &self,
        repo_path: StandardizedPath,
        state: FileTreeState,
        ctx: &mut ModelContext<Self>,
    ) {
        self.local.update(ctx, |local, _ctx| {
            local.insert_test_state(repo_path, state);
        });
    }

    pub fn insert_test_standing_results(
        &self,
        repo_path: StandardizedPath,
        standing_results: StandingQueryResults,
        ctx: &mut ModelContext<Self>,
    ) {
        self.local.update(ctx, |local, _ctx| {
            local.insert_test_standing_results(repo_path, standing_results);
        });
    }
}
