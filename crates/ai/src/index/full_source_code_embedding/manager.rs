use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use itertools::Itertools;
use repo_metadata::{BuildTreeError, DirectoryWatcher, Repository};
use thiserror::Error;

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use chrono::Utc;
        use super::changed_files::ChangedFiles;
        use crate::index::path_passes_filters;
        use ignore::gitignore::Gitignore;
        use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
        use warp_core::features::FeatureFlag;
        use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};
        use warpui::r#async::Timer;
        use warp_core::{send_telemetry_from_ctx, report_if_error};
        use crate::telemetry::AITelemetryEvent;
        use instant::Instant;
        use warp_core::channel::ChannelState;
        use warp_core::safe_warn;
    }
}
use warp_core::safe_anyhow;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use super::{
    codebase_index::{CodebaseIndexEvent, RetrievalID, SyncProgress},
    fragment_metadata::FragmentMetadata,
    priority_queue::{BuildQueue, Priority},
    snapshot::*,
    store_client::StoreClient,
    CodebaseIndex, ContentHash, EmbeddingConfig, Error as CodebaseIndexError, NodeHash,
};

use crate::{
    index::locations::CodeContextLocation,
    workspace::{WorkspaceMetadata, WorkspaceMetadataEvent},
};

/// The interval for debouncing filesystem events.
const REPO_WATCHER_DEBOUNCE_DURATION: Duration = Duration::from_secs(10);

/// The number of minutes between writing index snapshots.
const REPO_SNAPSHOT_PERSISTENCE_MINUTES: u64 = 10;

/// The interval for writing index snapshots.
const REPO_SNAPSHOT_PERSISTENCE_INTERVAL: Duration =
    Duration::from_secs(60 * REPO_SNAPSHOT_PERSISTENCE_MINUTES);

/// User-facing indexing completion status.
pub enum CodebaseIndexFinishedStatus {
    Completed,
    Failed(CodebaseIndexingError),
}

#[derive(Error, Debug)]
pub enum RetrieveFileError {
    #[error("Codebase index still indexing")]
    IndexSyncing,
    #[error("Codebase index failed: {0:#}")]
    IndexFailed(CodebaseIndexingError),
    #[error("Codebase index not found")]
    IndexNotFound,
}

#[derive(Error, Debug)]
pub enum FragmentMetadataLookupError {
    #[error("Codebase index not found")]
    IndexNotFound,
    #[error("Codebase index has no synced root hash")]
    IndexNotSynced,
    #[error("Codebase index root hash mismatch: requested {requested}, current {current}")]
    RootHashMismatch {
        requested: NodeHash,
        current: NodeHash,
    },
}

pub enum CodebaseIndexManagerEvent {
    RetrievalRequestCompleted {
        retrieval_id: RetrievalID,
        fragments: Arc<HashSet<CodeContextLocation>>,
        out_of_sync_delay: Option<Duration>,
    },
    RetrievalRequestFailed {
        retrieval_id: RetrievalID,
        error_message: String,
    },
    SyncStateUpdated,
    IndexMetadataUpdated {
        root_path: PathBuf,
        event: WorkspaceMetadataEvent,
    },
    RemoveExpiredIndexMetadata {
        expired_metadata: Arc<Vec<PathBuf>>,
    },
    NewIndexCreated,
}

/// User-facing indexing errors.
#[derive(Error, Debug)]
pub enum CodebaseIndexingError {
    #[error("Build tree error")]
    BuildTreeError,
    #[error("Repo size exceeded max file limit")]
    ExceededMaxFileLimit,
    #[error("Maximum directory depth exceeded")]
    MaxDepthExceeded,
    #[error("Failed to generate embeddings for some hashes:\n{0:#?}")]
    FailedToGenerateEmbeddings(Vec<FragmentMetadata>),
    #[error("Failed to sync intermediate nodes:\n{0:#?}")]
    FailedToSyncIntermediateNodes(Vec<NodeHash>),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<&CodebaseIndexError> for CodebaseIndexingError {
    fn from(value: &CodebaseIndexError) -> Self {
        match value {
            CodebaseIndexError::BuildTreeError(build_tree_error) => match build_tree_error {
                BuildTreeError::ExceededMaxFileLimit => Self::ExceededMaxFileLimit,
                BuildTreeError::MaxDepthExceeded => Self::MaxDepthExceeded,
                _ => Self::BuildTreeError,
            },
            CodebaseIndexError::FailedToGenerateEmbeddings(failed_fragments) => {
                Self::FailedToGenerateEmbeddings(failed_fragments.clone())
            }
            CodebaseIndexError::FailedToSyncIntermediateNodes(failed_hashes) => {
                Self::FailedToSyncIntermediateNodes(failed_hashes.clone())
            }
            _ => Self::Other(anyhow::anyhow!(value.to_string())),
        }
    }
}

/// User-facing codebase index status.
pub struct CodebaseIndexStatus {
    pub(super) has_pending: bool,
    pub(super) has_synced_version: bool,
    pub(super) last_sync_successful: Option<CodebaseIndexFinishedStatus>,
    pub(super) sync_progress: Option<SyncProgress>,
    pub(super) root_hash: Option<NodeHash>,
}

impl CodebaseIndexStatus {
    pub fn has_pending(&self) -> bool {
        self.has_pending
    }

    pub fn has_synced_version(&self) -> bool {
        self.has_synced_version
    }

    pub fn last_sync_successful(&self) -> Option<bool> {
        self.last_sync_successful
            .as_ref()
            .map(|res| matches!(res, CodebaseIndexFinishedStatus::Completed))
    }

    pub fn last_sync_result(&self) -> Option<&CodebaseIndexFinishedStatus> {
        self.last_sync_successful.as_ref()
    }

    pub fn sync_progress(&self) -> Option<&SyncProgress> {
        self.sync_progress.as_ref()
    }

    pub fn root_hash(&self) -> Option<&NodeHash> {
        self.root_hash.as_ref()
    }
}

pub enum BuildSource<'a> {
    FromPath(&'a Path),
    FromPersistedMetadata(WorkspaceMetadata),
}
pub struct CodebaseIndexManagerConfig {
    persisted_index_metadata: Vec<WorkspaceMetadata>,
    max_index_count: Option<usize>,
    max_files_repo_limit: usize,
    embedding_generation_batch_size: usize,
    store_client: Arc<dyn StoreClient>,
    indexing_enabled: bool,
}

impl CodebaseIndexManagerConfig {
    pub fn new(
        persisted_index_metadata: Vec<WorkspaceMetadata>,
        max_index_count: Option<usize>,
        max_files_repo_limit: usize,
        embedding_generation_batch_size: usize,
        store_client: Arc<dyn StoreClient>,
        indexing_enabled: bool,
    ) -> Self {
        Self {
            persisted_index_metadata,
            max_index_count,
            max_files_repo_limit,
            embedding_generation_batch_size,
            store_client,
            indexing_enabled,
        }
    }
}

/// Manager for the codebase index states across the app.
pub struct CodebaseIndexManager {
    codebase_indices: HashMap<PathBuf, ModelHandle<CodebaseIndex>>,

    store_client: Arc<dyn StoreClient>,

    #[cfg(feature = "local_fs")]
    watcher: ModelHandle<BulkFilesystemWatcher>,

    build_queue: BuildQueue,

    max_indices: Option<usize>,

    max_files_repo_limit: usize,

    embedding_generation_batch_size: usize,

    indexing_enabled: bool,

    #[cfg(feature = "local_fs")]
    snapshot_storage: Option<SnapshotStorage>,
}

impl CodebaseIndexManager {
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn new(
        persisted_index_metadata: Vec<WorkspaceMetadata>,
        max_index_count: Option<usize>,
        max_files_repo_limit: usize,
        embedding_generation_batch_size: usize,
        store_client: Arc<dyn StoreClient>,
        indexing_enabled: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let config = CodebaseIndexManagerConfig::new(
            persisted_index_metadata,
            max_index_count,
            max_files_repo_limit,
            embedding_generation_batch_size,
            store_client,
            indexing_enabled,
        );
        Self::new_with_config(config, ctx)
    }

    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn new_with_config(
        config: CodebaseIndexManagerConfig,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        #[cfg(feature = "local_fs")]
        {
            report_if_error!(migrate_snapshots_to_secure_dir_if_needed());
            Self::new_with_snapshot_storage(config, SnapshotStorage::app_default(), ctx)
        }

        #[cfg(not(feature = "local_fs"))]
        {
            Self::new_internal(config, ctx)
        }
    }

    #[cfg(feature = "local_fs")]
    pub fn new_with_snapshot_storage(
        config: CodebaseIndexManagerConfig,
        snapshot_storage: Option<SnapshotStorage>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self::new_internal(config, snapshot_storage, ctx)
    }

    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    fn new_internal(
        config: CodebaseIndexManagerConfig,
        #[cfg(feature = "local_fs")] snapshot_storage: Option<SnapshotStorage>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let CodebaseIndexManagerConfig {
            persisted_index_metadata,
            max_index_count,
            max_files_repo_limit,
            embedding_generation_batch_size,
            store_client,
            indexing_enabled,
        } = config;
        cfg_if::cfg_if! {
            if #[cfg(feature = "local_fs")] {
                let file_watcher = ctx.add_model(|ctx| BulkFilesystemWatcher::new(REPO_WATCHER_DEBOUNCE_DURATION, ctx));
                ctx.subscribe_to_model(&file_watcher, Self::handle_watcher_event);
            }
        }
        if !indexing_enabled {
            log::debug!(
                "Codebase indexing disabled for this launch mode; skipping restore of {:?} persisted codebase indices",
                persisted_index_metadata.len()
            );

            return Self {
                codebase_indices: HashMap::new(),
                store_client,
                #[cfg(feature = "local_fs")]
                watcher: file_watcher,
                build_queue: BuildQueue::empty(),
                max_indices: max_index_count,
                max_files_repo_limit,
                embedding_generation_batch_size,
                indexing_enabled,
                #[cfg(feature = "local_fs")]
                snapshot_storage,
            };
        }

        log::debug!(
            "Received {:?} persisted codebase indices",
            persisted_index_metadata.len()
        );
        #[cfg(feature = "local_fs")]
        let (invalid_metadata, valid_metadata) = split_snapshot_metadata_by_validity(
            persisted_index_metadata,
            snapshot_storage.as_ref(),
        );
        #[cfg(not(feature = "local_fs"))]
        let (invalid_metadata, valid_metadata) = (persisted_index_metadata, Vec::new());

        ctx.emit(CodebaseIndexManagerEvent::RemoveExpiredIndexMetadata {
            expired_metadata: Arc::new(
                invalid_metadata
                    .into_iter()
                    .map(|metadata| metadata.path)
                    .collect(),
            ),
        });
        #[cfg(feature = "local_fs")]
        if let Some(snapshot_storage) = snapshot_storage.as_ref() {
            clean_up_snapshot_files(snapshot_storage.path(), &valid_metadata);
        }

        // For the moment, we've decided to load all snapshots regardless of the index count.
        let build_queue = BuildQueue::new_with_persisted(valid_metadata);

        let mut me = Self {
            codebase_indices: HashMap::new(),
            store_client,
            #[cfg(feature = "local_fs")]
            watcher: file_watcher,
            build_queue,
            max_indices: max_index_count,
            max_files_repo_limit,
            embedding_generation_batch_size,
            indexing_enabled,
            #[cfg(feature = "local_fs")]
            snapshot_storage,
        };

        // Start building the first index in the queue.
        if let Some(next_repo) = me.build_queue.pick_next_sync() {
            me.build_and_sync_codebase_index(BuildSource::FromPersistedMetadata(next_repo), ctx);
        }

        me
    }

    #[cfg(feature = "test-util")]
    pub fn new_for_test(store_client: Arc<dyn StoreClient>, ctx: &mut ModelContext<Self>) -> Self {
        #[cfg(feature = "local_fs")]
        let file_watcher = ctx.add_model(|_| BulkFilesystemWatcher::new_for_test());
        Self {
            codebase_indices: HashMap::new(),
            store_client,
            #[cfg(feature = "local_fs")]
            watcher: file_watcher,
            build_queue: BuildQueue::empty(),
            max_indices: None,
            max_files_repo_limit: 0,
            embedding_generation_batch_size: 100,
            indexing_enabled: true,
            #[cfg(feature = "local_fs")]
            snapshot_storage: SnapshotStorage::app_default(),
        }
    }

    /// Check whether any of the codebases' root path was deleted and clean up its persisted
    /// artifacts.
    #[cfg(feature = "local_fs")]
    pub fn clean_up_deleted_indices(&mut self, ctx: &mut ModelContext<Self>) {
        let codebase_roots = self.codebase_indices.keys().cloned().collect_vec();
        ctx.spawn(
            async move {
                // Check all codebase roots for deletion.
                codebase_roots
                    .into_iter()
                    .filter(|codebase_root_path| !codebase_root_path.exists())
                    .collect()
            },
            |me, to_clean_up, ctx| {
                me.drop_indices(to_clean_up, ctx);
            },
        );
    }

    /// Cleans up all indexed codebases.
    fn drop_all_indices(&mut self, ctx: &mut ModelContext<Self>) {
        self.drop_indices(self.codebase_indices.keys().cloned().collect_vec(), ctx);

        // Replace the HashMap with the default (empty) one.
        // Unlike `.clear()` and `.drain()`, this releases the allocated memory.
        self.codebase_indices = HashMap::new();
    }

    /// Checks if the codebase still exists in the filesystem.
    async fn should_clean_up_index(root_path: &Path) -> bool {
        let Ok(exists) = std::fs::exists(root_path) else {
            return true;
        };
        !exists
    }

    /// Fully clears all persisted data related to the given indices and
    /// stops receiving watcher events for it.
    fn drop_indices(&mut self, to_drop: Vec<PathBuf>, ctx: &mut ModelContext<Self>) {
        // Drop the in-memory indices and unregister the filewatcher.
        for codebase_root in &to_drop {
            self.drop_index_from_memory(codebase_root, ctx);
        }

        // Remove snapshots from disk.
        let to_drop_clone = to_drop.clone();
        #[cfg(feature = "local_fs")]
        let snapshot_storage = self.snapshot_storage.clone();
        ctx.spawn(
            async move {
                #[cfg(feature = "local_fs")]
                Self::drop_index_snapshots(snapshot_storage, to_drop_clone).await;
                #[cfg(not(feature = "local_fs"))]
                let _ = to_drop_clone;
            },
            |_, _, _| {},
        );

        // Remove metadata from SQLite.
        ctx.emit(CodebaseIndexManagerEvent::RemoveExpiredIndexMetadata {
            expired_metadata: Arc::new(to_drop),
        });
    }

    /// Remove the given index snapshots from disk.
    #[cfg(feature = "local_fs")]
    async fn drop_index_snapshots(
        snapshot_storage: Option<SnapshotStorage>,
        to_drop: Vec<PathBuf>,
    ) {
        if let Some(snapshot_storage) = snapshot_storage {
            for codebase_root in &to_drop {
                Self::drop_index_snapshot(snapshot_storage.path(), codebase_root).await;
            }
        }
    }

    async fn drop_index_snapshot(snapshot_dir: &Path, codebase_root: &Path) {
        if let Err(err) = std::fs::remove_file(snapshot_path(snapshot_dir, codebase_root)) {
            log::warn!(
                "Failed to remove codebase index snapshot file for {codebase_root:?}: {err:#?}"
            );
        }
    }

    /// Removes an index from in-memory data structures.
    fn drop_index_from_memory(&mut self, root_path: &Path, ctx: &mut ModelContext<Self>) {
        // Cancel any pending sync for this codebase
        if let Some(index) = self.codebase_indices.get(root_path) {
            index.update(ctx, |index, _| {
                index.abort_in_progress_sync();
            });
        }

        // Drop the in-memory index.
        self.codebase_indices.remove(root_path);

        // Stop the filewatcher from receiving events for this codebase.
        #[cfg(feature = "local_fs")]
        self.unwatch_path(root_path, ctx);
    }

    /// Fully clears all persisted data related to a single codebase index
    /// and stops receiving watcher events for it.
    pub fn drop_index(&mut self, root_path: PathBuf, ctx: &mut ModelContext<Self>) {
        let root_path = dunce::canonicalize(&root_path).unwrap_or(root_path);
        self.drop_index_from_memory(root_path.as_path(), ctx);

        // Remove snapshot from disk.
        let root_path_clone = root_path.clone();
        #[cfg(feature = "local_fs")]
        let snapshot_storage = self.snapshot_storage.clone();
        ctx.spawn(
            async move {
                #[cfg(feature = "local_fs")]
                if let Some(snapshot_storage) = snapshot_storage {
                    Self::drop_index_snapshot(snapshot_storage.path(), &root_path_clone).await;
                }
                #[cfg(not(feature = "local_fs"))]
                let _ = root_path_clone;
            },
            |_, _, _| {},
        );

        // Remove metadata from SQLite.
        ctx.emit(CodebaseIndexManagerEvent::RemoveExpiredIndexMetadata {
            expired_metadata: Arc::new(vec![root_path]),
        });
    }

    #[cfg(feature = "local_fs")]
    fn group_file_events(
        &self,
        event: &BulkFilesystemWatcherEvent,
    ) -> HashMap<PathBuf, ChangedFiles> {
        let mut added_or_updated = event.added_or_updated_set();
        let mut deleted = event.deleted.clone();

        // For now, treat a move as a deletion followed by an addition.
        // This means deletions must be processed before additions/updates.
        for (old_path, new_path) in event.moved.iter() {
            deleted.insert(old_path.to_path_buf());
            added_or_updated.insert(new_path.to_path_buf());
        }

        let mut updates_by_root: HashMap<PathBuf, ChangedFiles> = HashMap::new();

        for path in deleted {
            if let Some(root_path) = self.root_path_for_codebase(&path) {
                updates_by_root
                    .entry(root_path)
                    .or_default()
                    .deletions
                    .insert(path);
            } else {
                log::warn!(
                    "Could not find index root for deleted file: {}",
                    path.display()
                );
            }
        }

        for path in added_or_updated {
            if let Some(root_path) = self.root_path_for_codebase(&path) {
                updates_by_root
                    .entry(root_path)
                    .or_default()
                    .upsertions
                    .insert(path);
            } else {
                log::warn!(
                    "Could not find index root for updated file: {}",
                    path.display()
                );
            }
        }

        updates_by_root
    }

    #[cfg(feature = "local_fs")]
    fn incremental_update_codebase_index(
        &mut self,
        root_path: PathBuf,
        changed_files: ChangedFiles,
        ctx: &mut ModelContext<Self>,
    ) {
        if changed_files.is_empty() {
            return;
        }

        let Some(index_state) = self.codebase_indices.get(root_path.as_path()) else {
            log::warn!(
                "No prior index state for root path: {}",
                root_path.display()
            );
            return;
        };

        index_state.update(ctx, |codebase_index, ctx| {
            codebase_index.incremental_update(changed_files, self.store_client.clone(), false, ctx);
        });
    }

    #[cfg(feature = "local_fs")]
    fn handle_watcher_event(
        &mut self,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let updates_by_root = self.group_file_events(event);

        for (root_path, changed_files) in updates_by_root {
            self.incremental_update_codebase_index(root_path, changed_files, ctx);
        }
    }

    pub fn handle_active_session_changed(&mut self, active_directory: &Path) {
        if !self.is_indexing_enabled() {
            return;
        }
        let Some(root_path) = self.root_path_for_codebase(active_directory) else {
            return;
        };

        self.build_queue
            .update_path_priority(root_path, Priority::ActiveSession);
    }

    pub fn update_max_limits(
        &mut self,
        new_max_indices: Option<usize>,
        new_max_files_per_repo: usize,
        new_embedding_generation_batch_size: usize,
        _ctx: &mut ModelContext<Self>,
    ) {
        self.max_indices = new_max_indices;

        if self.max_files_repo_limit != new_max_files_per_repo {
            self.max_files_repo_limit = new_max_files_per_repo;

            #[cfg(feature = "local_fs")]
            for index in self.codebase_indices.values() {
                // If the max file repo limit changed, kick off a new full sync to retry indexing.
                if matches!(
                    index
                        .as_ref(_ctx)
                        .codebase_index_status()
                        .last_sync_result(),
                    Some(CodebaseIndexFinishedStatus::Failed(
                        CodebaseIndexingError::ExceededMaxFileLimit
                    ))
                ) {
                    index.update(_ctx, |code_index, ctx| {
                        let _ = code_index.full_sync_index(self.max_files_repo_limit, ctx);
                    });
                }
            }
        }

        // Update the embedding generation batch size for existing indices
        if self.embedding_generation_batch_size != new_embedding_generation_batch_size {
            self.embedding_generation_batch_size = new_embedding_generation_batch_size;

            for index in self.codebase_indices.values() {
                index.update(_ctx, |code_index, _| {
                    code_index.update_embedding_generation_batch_size(
                        new_embedding_generation_batch_size,
                    );
                });
            }
        }
    }

    /// Ensures the current number of indices is below the maximum.
    pub fn can_create_new_indices(&self) -> bool {
        if !self.is_indexing_enabled() {
            return false;
        }
        self.max_indices
            .is_none_or(|max_indices| self.codebase_indices.len() < max_indices)
    }

    pub fn handle_session_bootstrapped(&mut self, working_directory: &Path) {
        if !self.is_indexing_enabled() {
            return;
        }
        let Some(root_path) = self.root_path_for_codebase(working_directory) else {
            return;
        };

        self.build_queue
            .update_path_priority(root_path, Priority::OpenSession);
    }

    pub fn get_codebase_index_statuses<'a>(
        &'a self,
        app: &'a AppContext,
    ) -> impl Iterator<Item = (&'a PathBuf, CodebaseIndexStatus)> {
        self.codebase_indices.iter().map(|(path, codebase_index)| {
            let index_state = codebase_index.as_ref(app);
            let status = index_state.codebase_index_status();
            (path, status)
        })
    }

    pub fn get_codebase_index_status_for_path<'a>(
        &'a self,
        root_path: &Path,
        app: &'a AppContext,
    ) -> Option<CodebaseIndexStatus> {
        let root_path = dunce::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
        self.codebase_indices.get(&root_path).map(|codebase_index| {
            let index_state = codebase_index.as_ref(app);
            index_state.codebase_index_status()
        })
    }

    pub fn fragment_metadatas_from_hashes(
        &self,
        repo_path: &Path,
        root_hash: &NodeHash,
        content_hashes: &[ContentHash],
        app: &AppContext,
    ) -> Result<HashMap<ContentHash, Vec<FragmentMetadata>>, FragmentMetadataLookupError> {
        let (codebase_index, _) = self
            .get_codebase_index_internal(repo_path)
            .map_err(|_| FragmentMetadataLookupError::IndexNotFound)?;

        codebase_index
            .as_ref(app)
            .fragment_metadatas_from_hashes(root_hash, content_hashes)
    }

    pub fn get_codebase_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.codebase_indices.keys()
    }

    pub fn num_active_indices(&self) -> usize {
        self.codebase_indices.len()
    }

    pub fn is_indexing_enabled(&self) -> bool {
        self.indexing_enabled
    }

    pub fn index_directory(&mut self, directory: PathBuf, ctx: &mut ModelContext<Self>) {
        if !self.is_indexing_enabled() {
            return;
        }
        let directory = dunce::canonicalize(&directory).unwrap_or(directory);
        if !self.codebase_indices.contains_key(&directory) {
            self.build_and_sync_codebase_index(BuildSource::FromPath(&directory), ctx);
            // Starting a new codebase index should be considered into sync state updates.
            ctx.emit(CodebaseIndexManagerEvent::SyncStateUpdated);
        }
    }

    #[cfg(feature = "local_fs")]
    fn watch_path(
        &self,
        root_path: &Path,
        gitignores: Arc<Vec<Gitignore>>,
        ctx: &mut ModelContext<Self>,
    ) {
        let watch_filter = WatchFilter::with_filter(Arc::new(move |path| {
            path_passes_filters(path, gitignores.as_slice())
        }));
        self.watcher.update(ctx, |watcher, _ctx| {
            std::mem::drop(watcher.register_path(
                root_path,
                watch_filter,
                RecursiveMode::Recursive,
            ));
        });
    }

    #[cfg(feature = "local_fs")]
    fn unwatch_path(&self, root_path: &Path, ctx: &mut ModelContext<Self>) {
        self.watcher.update(ctx, |watcher, _ctx| {
            std::mem::drop(watcher.unregister_path(root_path));
        });
    }

    #[cfg(feature = "local_fs")]
    fn unwatch_all_paths(&self, ctx: &mut ModelContext<Self>) {
        for path in self.get_codebase_paths() {
            self.unwatch_path(path, ctx);
        }
    }

    pub fn build_and_sync_codebase_index(
        &mut self,
        build_source: BuildSource,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.is_indexing_enabled() {
            return;
        }
        if !self.can_create_new_indices() {
            return;
        }

        let repo_path = match build_source {
            BuildSource::FromPath(path) => path,
            BuildSource::FromPersistedMetadata(ref metadata) => metadata.path.as_path(),
        };

        let standardized_path =
            match warp_util::standardized_path::StandardizedPath::from_local_canonicalized(
                repo_path,
            ) {
                Ok(path) => path,
                Err(e) => {
                    log::error!("Failed to canonicalize repository path: {e:?}");
                    return;
                }
            };

        // Ensure the repository is registered with RepoWatcher.
        let handle = match DirectoryWatcher::handle(ctx).update(ctx, |repo_watcher, ctx| {
            repo_watcher.add_directory(standardized_path, ctx)
        }) {
            Ok(handle) => handle,
            Err(e) => {
                log::error!("Failed to start tracking repository: {e:?}");
                return;
            }
        };

        let canonical_key =
            dunce::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());

        let index = self
            .codebase_indices
            .entry(canonical_key)
            .or_insert_with(|| {
                #[cfg(feature = "local_fs")]
                let snapshot_storage = self.snapshot_storage.clone();
                let index = Self::build_and_sync_codebase_index_internal(
                    self.store_client.clone(),
                    handle,
                    self.max_files_repo_limit,
                    self.embedding_generation_batch_size,
                    #[cfg(feature = "local_fs")]
                    snapshot_storage,
                    ctx,
                );

                #[cfg(feature = "local_fs")]
                Self::schedule_next_snapshot_write(repo_path.to_path_buf(), ctx);

                index
            })
            .clone();

        if let BuildSource::FromPersistedMetadata(metadata) = build_source {
            index.update(ctx, |index, _| {
                index.update_timestamps_from_metadata(metadata);
            });
        }
    }

    /// Checks whether a snapshot exists for the index and attempts to load it;
    /// otherwise, falls back to creating a brand-new index.
    fn build_and_sync_codebase_index_internal(
        store_client: Arc<dyn StoreClient>,
        repository: ModelHandle<Repository>,
        max_files_repo_limit: usize,
        embedding_generation_batch_size: usize,
        #[cfg(feature = "local_fs")] snapshot_storage: Option<SnapshotStorage>,
        ctx: &mut ModelContext<Self>,
    ) -> ModelHandle<CodebaseIndex> {
        let codebase_index = ctx.add_model(|ctx| {
            #[cfg(feature = "local_fs")]
            if FeatureFlag::CodebaseIndexPersistence.is_enabled()
                && repository
                    .as_ref(ctx)
                    .root_dir()
                    .to_local_path()
                    .is_some_and(|p| {
                        snapshot_storage
                            .as_ref()
                            .is_some_and(|storage| storage.has_snapshot(&p))
                    })
            {
                if let Some(snapshot_storage) = snapshot_storage.as_ref() {
                    let read_snapshot_start_time = Instant::now();
                    match read_snapshot(
                        store_client.clone(),
                        snapshot_storage.path(),
                        repository.clone(),
                        max_files_repo_limit,
                        embedding_generation_batch_size,
                        ctx,
                    ) {
                        Ok(snapshot_index) => {
                            send_telemetry_from_ctx!(
                                AITelemetryEvent::MerkleTreeSnapshotRebuildSuccess {
                                    duration: read_snapshot_start_time.elapsed()
                                },
                                ctx
                            );
                            return snapshot_index;
                        }
                        Err(err) => {
                            send_telemetry_from_ctx!(
                                AITelemetryEvent::MerkleTreeSnapshotRebuildFailed {
                                    error: err.to_string()
                                },
                                ctx
                            );
                        }
                    }
                }
            }

            CodebaseIndex::new_from_scratch(
                repository,
                store_client,
                EmbeddingConfig::default(),
                max_files_repo_limit,
                embedding_generation_batch_size,
                ctx,
            )
        });
        ctx.subscribe_to_model(&codebase_index, Self::handle_codebase_index_event);

        codebase_index
    }

    fn handle_codebase_index_event(
        &mut self,
        event: &CodebaseIndexEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            CodebaseIndexEvent::RetrievalRequestFailed {
                retrieval_id,
                error,
            } => ctx.emit(CodebaseIndexManagerEvent::RetrievalRequestFailed {
                retrieval_id: retrieval_id.clone(),
                error_message: error.to_string(),
            }),
            CodebaseIndexEvent::RetrievalRequestCompleted {
                retrieval_id,
                fragments,
                out_of_sync_delay,
            } => ctx.emit(CodebaseIndexManagerEvent::RetrievalRequestCompleted {
                retrieval_id: retrieval_id.clone(),
                fragments: fragments.clone(),
                out_of_sync_delay: *out_of_sync_delay,
            }),
            CodebaseIndexEvent::SyncStateUpdated => {
                ctx.emit(CodebaseIndexManagerEvent::SyncStateUpdated)
            }
            CodebaseIndexEvent::IndexMetadataUpdated { root_path, event } => {
                ctx.emit(CodebaseIndexManagerEvent::IndexMetadataUpdated {
                    root_path: root_path.to_path_buf(),
                    event: *event,
                })
            }
            #[cfg(feature = "local_fs")]
            CodebaseIndexEvent::GitignoresUpdated {
                repo_root_path,
                gitignores,
            } => {
                self.unwatch_path(repo_root_path, ctx);
                self.watch_path(repo_root_path, gitignores.clone(), ctx);
            }
            CodebaseIndexEvent::LocalIndexBuilt { repo_root_path } => {
                self.on_index_build_finished(repo_root_path, ctx);
            }
            #[cfg(feature = "local_fs")]
            CodebaseIndexEvent::InitialSyncCompleted {
                repo_path,
                has_pending_change,
            } => {
                if !has_pending_change {
                    self.write_snapshot(repo_path, ctx);
                }
            }
        }
    }

    fn on_index_build_finished(&mut self, finished_repo: &Path, ctx: &mut ModelContext<Self>) {
        let Ok(_) = self.get_codebase_index_internal(finished_repo) else {
            return;
        };

        if let Some(next_repo) = self.build_queue.pick_next_sync() {
            self.build_and_sync_codebase_index(BuildSource::FromPersistedMetadata(next_repo), ctx);
        }
    }

    /// Aborts any in-progress syncs and drops all codebase indices.
    pub fn reset_codebase_indexing(&mut self, ctx: &mut ModelContext<Self>) {
        for index in self.codebase_indices.values() {
            index.as_ref(ctx).abort_in_progress_sync();
        }

        self.drop_all_indices(ctx);
    }

    pub fn root_path_for_codebase(&self, path: &Path) -> Option<PathBuf> {
        self.get_codebase_index_internal(path)
            .map(|(_, path)| path)
            .ok()
    }
    pub fn with_indexed_codebase<T>(
        &mut self,
        path: &Path,
        on_found: impl FnOnce(&mut Self, &Path, &mut ModelContext<Self>) -> T,
        on_missing: impl FnOnce(&mut Self, &Path, &mut ModelContext<Self>) -> T,
        ctx: &mut ModelContext<Self>,
    ) -> T {
        let Some(indexed_repo_path) = self.root_path_for_codebase(path) else {
            return on_missing(self, path, ctx);
        };

        on_found(self, indexed_repo_path.as_path(), ctx)
    }

    fn get_codebase_index_internal(
        &self,
        path: &Path,
    ) -> anyhow::Result<(&ModelHandle<CodebaseIndex>, PathBuf)> {
        let mut path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_owned());

        loop {
            if let Some(outline) = self.codebase_indices.get(&path) {
                return Ok((outline, path));
            }

            if !path.pop() {
                break;
            }
        }

        Err(safe_anyhow!(
            safe: ("Codebase index not found"),
            full: ("Codebase index for repo {path:?} not found")
        ))
    }

    /// Try to manually perform a full sync on the given codebase. This will fail if the codebase has a sync already in-progress.
    #[cfg(feature = "local_fs")]
    pub fn try_manual_resync_codebase(&self, repo_path: &Path, ctx: &mut ModelContext<Self>) {
        let Ok((codebase_index, _)) = self.get_codebase_index_internal(repo_path) else {
            return;
        };

        codebase_index.update(ctx, |index, ctx| {
            let _ = index.full_sync_index(self.max_files_repo_limit, ctx);
        })
    }

    #[cfg(not(feature = "local_fs"))]
    pub fn try_manual_resync_codebase(&self, _repo_path: &Path, _ctx: &mut ModelContext<Self>) {}

    pub fn retrieve_relevant_files(
        &self,
        query: String,
        repo_path: &Path,
        ctx: &mut ModelContext<Self>,
    ) -> Result<RetrievalID, RetrieveFileError> {
        let Ok((codebase_index, _)) = self.get_codebase_index_internal(repo_path) else {
            return Err(RetrieveFileError::IndexNotFound);
        };

        codebase_index.update(ctx, |codebase_index, ctx| {
            codebase_index.retrieve_relevant_files(query, self.store_client.clone(), ctx)
        })
    }

    pub fn abort_retrieval_request(
        &self,
        repo_path: &Path,
        retrieval_id: RetrievalID,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), anyhow::Error> {
        let (codebase_index, _) = self.get_codebase_index_internal(repo_path)?;

        codebase_index.update(ctx, |codebase_index, _ctx| {
            codebase_index.abort_retrieval_request(retrieval_id);
        });

        Ok(())
    }

    #[cfg(feature = "local_fs")]
    pub fn write_snapshot(&mut self, working_directory: &Path, ctx: &mut ModelContext<Self>) {
        let Some(repo_path) = self.root_path_for_codebase(working_directory) else {
            safe_warn!(
                safe: ("No root codebase for path"),
                full: ("No root codebase for path {working_directory:?}")
            );
            return;
        };

        let Ok((codebase_index_model, _)) = self.get_codebase_index_internal(repo_path.as_path())
        else {
            return;
        };

        let codebase_index = codebase_index_model.as_ref(ctx);

        if !codebase_index.has_unsnapshotted_changes() {
            Self::schedule_next_snapshot_write(repo_path, ctx);
            return;
        }

        let snapshot_generation_time = Utc::now();
        let serializable_index = match codebase_index.generate_snapshot() {
            Ok(index) => index,
            Err(err) => {
                log::warn!("Unable to generate snapshot: {err:?}");
                Self::schedule_next_snapshot_write(repo_path, ctx);
                return;
            }
        };

        let snapshot_storage = match self.snapshot_storage.as_ref() {
            Some(storage) => storage,
            None => {
                log::warn!("No snapshot directory to write to");
                Self::schedule_next_snapshot_write(repo_path, ctx);
                return;
            }
        };
        let snapshot_path = snapshot_storage.snapshot_path(repo_path.as_path());

        // Update timestamp eagerly so concurrent calls to has_unsnapshotted_changes()
        // won't trigger a duplicate snapshot while the background write is in progress.
        codebase_index_model.update(ctx, |codebase_index, _ctx| {
            codebase_index.update_snapshot_ts(snapshot_generation_time)
        });

        // Move the expensive serialization and file I/O to a background thread.
        ctx.spawn(
            async move {
                let result = async {
                    let serialized = CodebaseIndex::serialize_snapshot(serializable_index).await?;
                    async_fs::write(&snapshot_path, serialized).await?;
                    anyhow::Ok(())
                }
                .await;
                (repo_path, result)
            },
            |_me, (repo_path, result), ctx| {
                if let Err(err) = result {
                    if ChannelState::enable_debug_features() {
                        log::error!("Unable to write snapshot for {repo_path:?}: {err:?}");
                    } else {
                        log::warn!("Unable to write snapshot: {err:?}");
                    }
                }
                Self::schedule_next_snapshot_write(repo_path, ctx);
            },
        );
    }

    /// Schedules the next periodic snapshot write.
    #[cfg(feature = "local_fs")]
    fn schedule_next_snapshot_write(repo_path: PathBuf, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async move {
                Timer::after(REPO_SNAPSHOT_PERSISTENCE_INTERVAL).await;
                let should_remove_index = Self::should_clean_up_index(&repo_path).await;
                (repo_path, should_remove_index)
            },
            move |me, (repo_path, should_remove_index), ctx| {
                if should_remove_index {
                    me.drop_index(repo_path, ctx);
                } else {
                    me.write_snapshot(&repo_path, ctx);
                }
            },
        );
    }

    /// Triggers an incremental sync for the codebase at the given path.
    pub fn trigger_incremental_sync_for_path(
        &mut self,
        directory_path: &Path,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        if !self.is_indexing_enabled() {
            return Ok(());
        }
        // Find the root path for this directory's codebase
        let Some(repo_path) = self.root_path_for_codebase(directory_path) else {
            return Err(anyhow::anyhow!("Failed to find root path for directory"));
        };

        // Check if there's an existing index for this repository
        let (codebase_index, _) = self.get_codebase_index_internal(repo_path.as_path())?;

        // Trigger an incremental sync by checking for file changes
        // This will detect any changes since the last sync and update the index accordingly
        codebase_index.update(ctx, |index, _ctx| {
            // Check if the index is in a state where it can perform incremental updates
            let status = index.codebase_index_status();
            if status.has_pending {
                return;
            }

            log::debug!(
                "Triggering incremental sync for repo: {}",
                repo_path.display()
            );

            // For now, we'll trigger a check that may lead to an incremental sync
            // The actual sync will only happen if the file watcher has detected changes
            // or if there are pending file changes that need to be processed
            // This is a lightweight operation that won't do unnecessary work
            #[cfg(feature = "local_fs")]
            index.flush_pending_file_changes(_ctx);
        });

        Ok(())
    }
}

impl Entity for CodebaseIndexManager {
    type Event = CodebaseIndexManagerEvent;
}

impl SingletonEntity for CodebaseIndexManager {}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
