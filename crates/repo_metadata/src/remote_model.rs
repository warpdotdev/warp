//! Remote repository metadata model (client-side).
//!
//! Holds file tree state for repositories on remote servers. In this initial phase
//! there is no syncing or indexing — state is populated externally (e.g. by a future
//! remote client model or via test helpers).

use futures::future::{self, BoxFuture, FutureExt as _};
use std::collections::HashMap;
use std::sync::Arc;

use warp_core::HostId;
use warpui::ModelContext;

use crate::file_tree_store::{FileTreeEntry, FileTreeState};
use crate::file_tree_update::RepoMetadataUpdate;
use crate::local_model::{GetContentsArgs, IndexedRepoState, RepoContent};
use crate::repository_identifier::RemoteRepositoryIdentifier;

use super::local_model::collect_contents_recursive;

/// Events emitted by the [`RemoteRepoMetadataModel`].
#[derive(Debug)]
pub enum RemoteRepositoryMetadataEvent {
    /// A remote repository was added or updated.
    RepositoryUpdated { id: RemoteRepositoryIdentifier },
    /// A remote repository was removed.
    RepositoryRemoved { id: RemoteRepositoryIdentifier },
    /// The file tree for remote repositories was updated.
    FileTreeUpdated {
        ids: Vec<RemoteRepositoryIdentifier>,
    },
    /// The file tree entry for a remote repository was updated.
    FileTreeEntryUpdated { id: RemoteRepositoryIdentifier },
}

/// Client-side model for remote repository metadata.
///
/// This model holds file tree state for repositories living on remote servers.
/// It provides the same read-only query surface as the local model, and write
/// methods that will be the integration points for the future remote sync layer.
///
/// Consumers should access this through the [`RepoMetadataModel`](crate::wrapper_model::RepoMetadataModel)
/// wrapper rather than using this type directly.
pub struct RemoteRepoMetadataModel {
    repositories: HashMap<RemoteRepositoryIdentifier, IndexedRepoState>,
}

impl RemoteRepoMetadataModel {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            repositories: HashMap::new(),
        }
    }

    /// Returns a future that resolves once remote repository indexing reaches a terminal state.
    ///
    /// Callers should check [`Self::repository_state`] after awaiting this future to see whether
    /// indexing succeeded or failed.
    pub fn repository_indexed(&self, id: &RemoteRepositoryIdentifier) -> BoxFuture<'static, ()> {
        match self.repositories.get(id) {
            Some(state) => state.wait_until_indexed(),
            None => future::ready(()).boxed(),
        }
    }

    // ── Read-only query API ──────────────────────────────────────────

    /// Returns the [`FileTreeState`] for a remote repository, if it is indexed.
    pub fn get_repository(&self, id: &RemoteRepositoryIdentifier) -> Option<&FileTreeState> {
        match self.repositories.get(id)? {
            IndexedRepoState::Indexed(state) => Some(state),
            IndexedRepoState::Pending(_) | IndexedRepoState::Failed(_) => None,
        }
    }

    /// Returns whether the given remote repository is indexed.
    pub fn has_repository(&self, id: &RemoteRepositoryIdentifier) -> bool {
        matches!(
            self.repositories.get(id),
            Some(IndexedRepoState::Indexed(_))
        )
    }

    /// Returns the current [`IndexedRepoState`] for a remote repository.
    pub fn repository_state(&self, id: &RemoteRepositoryIdentifier) -> Option<&IndexedRepoState> {
        self.repositories.get(id)
    }

    /// Returns repository contents for the specified remote repository.
    pub fn get_repo_contents(
        &self,
        id: &RemoteRepositoryIdentifier,
        args: GetContentsArgs,
    ) -> Option<Vec<RepoContent<'_>>> {
        let state = match self.repositories.get(id)? {
            IndexedRepoState::Indexed(state) => state,
            IndexedRepoState::Pending(_) | IndexedRepoState::Failed(_) => return None,
        };
        let mut contents = Vec::new();
        collect_contents_recursive(
            &state.entry,
            state.entry.root_directory(),
            &mut contents,
            &args,
        );
        Some(contents)
    }

    /// Returns all tracked remote repository identifiers, including those in
    /// `Pending` or `Failed` states. Callers that only need indexed repos
    /// should filter via [`get_repository`](Self::get_repository).
    pub fn remote_repository_ids(&self) -> impl Iterator<Item = &RemoteRepositoryIdentifier> {
        self.repositories.keys()
    }

    // ── Write API (for future sync + test use) ───────────────────────

    /// Inserts or replaces file tree state for a remote repository.
    pub fn insert_repository(
        &mut self,
        id: RemoteRepositoryIdentifier,
        state: FileTreeState,
        ctx: &mut ModelContext<Self>,
    ) {
        self.replace_repository_state(id.clone(), IndexedRepoState::Indexed(state));
        ctx.emit(RemoteRepositoryMetadataEvent::RepositoryUpdated { id });
    }

    /// Removes a remote repository from tracking.
    pub fn remove_repository(
        &mut self,
        id: &RemoteRepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.remove_repository_state(id).is_some() {
            ctx.emit(RemoteRepositoryMetadataEvent::RepositoryRemoved { id: id.clone() });
        }
    }

    /// Replaces the file tree entry within an existing remote repository's state.
    pub fn update_file_tree_entry(
        &mut self,
        id: &RemoteRepositoryIdentifier,
        entry: FileTreeEntry,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(IndexedRepoState::Indexed(state)) = self.repositories.get_mut(id) {
            state.entry = entry;
            ctx.emit(RemoteRepositoryMetadataEvent::FileTreeEntryUpdated { id: id.clone() });
        }
    }

    /// Inserts or replaces a remote repository from a snapshot update.
    ///
    /// Creates a `FileTreeEntry` from the update by starting with an empty
    /// root and applying the snapshot entries, then wraps it in a
    /// `FileTreeState`. This is the primary entry point for populating
    /// remote repo state from server push events.
    pub fn insert_from_snapshot(
        &mut self,
        host_id: HostId,
        update: &RepoMetadataUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut entry = FileTreeEntry::new_for_directory(Arc::new(update.repo_path.clone()));
        entry.apply_repo_metadata_update(update);
        let state = FileTreeState::from_file_tree_entry(entry);
        let id = RemoteRepositoryIdentifier::new(host_id, update.repo_path.clone());
        self.insert_repository(id, state, ctx);
    }

    /// Removes all remote repositories associated with the given host.
    pub fn remove_repositories_for_host(&mut self, host_id: &HostId, ctx: &mut ModelContext<Self>) {
        let ids_to_remove: Vec<RemoteRepositoryIdentifier> = self
            .repositories
            .keys()
            .filter(|id| id.host_id == *host_id)
            .cloned()
            .collect();
        for id in ids_to_remove {
            self.remove_repository(&id, ctx);
        }
    }

    /// Applies an incremental update received from the remote server.
    ///
    /// Looks up the repository by matching `(host_id, repo_path)` against
    /// tracked [`RemoteRepositoryIdentifier`]s, then delegates to
    /// [`FileTreeEntry::apply_repo_metadata_update`].
    pub fn apply_incremental_update(
        &mut self,
        host_id: &HostId,
        update: &RepoMetadataUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        let matching_id = self
            .repositories
            .keys()
            .find(|id| id.host_id == *host_id && id.path == update.repo_path)
            .cloned();

        let Some(id) = matching_id else {
            log::warn!(
                "No remote repository found for incremental update: {}",
                update.repo_path
            );
            return;
        };

        if let Some(IndexedRepoState::Indexed(state)) = self.repositories.get_mut(&id) {
            state.entry.apply_repo_metadata_update(update);
            ctx.emit(RemoteRepositoryMetadataEvent::FileTreeEntryUpdated { id });
        }
    }
}

impl warpui::Entity for RemoteRepoMetadataModel {
    type Event = RemoteRepositoryMetadataEvent;
}

impl RemoteRepoMetadataModel {
    fn replace_repository_state(
        &mut self,
        id: RemoteRepositoryIdentifier,
        state: IndexedRepoState,
    ) -> Option<IndexedRepoState> {
        let previous = self.repositories.insert(id, state);
        if let Some(previous) = &previous {
            previous.complete_if_pending();
        }
        previous
    }

    fn remove_repository_state(
        &mut self,
        id: &RemoteRepositoryIdentifier,
    ) -> Option<IndexedRepoState> {
        let previous = self.repositories.remove(id);
        if let Some(previous) = &previous {
            previous.complete_if_pending();
        }
        previous
    }
}

#[cfg(any(test, feature = "test-util"))]
impl RemoteRepoMetadataModel {
    /// Insert a repository state directly for testing purposes.
    pub fn insert_test_state(&mut self, id: RemoteRepositoryIdentifier, state: FileTreeState) {
        self.replace_repository_state(id, IndexedRepoState::Indexed(state));
    }
}
