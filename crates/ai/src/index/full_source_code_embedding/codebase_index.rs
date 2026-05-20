use anyhow::anyhow;
use async_channel;
use chrono::{DateTime, Utc};
use futures::stream::AbortHandle;
use ignore::gitignore::Gitignore;
#[cfg(feature = "local_fs")]
use repo_metadata::entry::IgnoredPathStrategy;
use repo_metadata::Repository;
use std::{path::Path, sync::Arc};
use warp_core::safe_error;
use warpui::{Entity, ModelContext, ModelHandle};

use super::{
    fragment_metadata::{FragmentMetadata, LeafToFragmentMetadata, LeafToFragmentMetadataUpdates},
    manager::{
        CodebaseIndexFinishedStatus, CodebaseIndexStatus, FragmentMetadataLookupError,
        RetrieveFileError,
    },
    merkle_tree::{MerkleTree, SerializedCodebaseIndex},
    store_client::StoreClient,
    sync_client::{FlushFragmentResult, SyncOperationError},
    CodebaseContextConfig, ContentHash, EmbeddingConfig, Error, Fragment, NodeHash, RepoMetadata,
};
use crate::{
    index::locations::{CodeContextLocation, FileFragmentLocation},
    telemetry::{AITelemetryEvent, CodebaseContextSyncType},
    workspace::{WorkspaceMetadata, WorkspaceMetadataEvent},
};
use instant::Instant;
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use super::{
            changed_files::ChangedFiles,
            merkle_tree::{NodeId, NodeLens, TreeUpdateResult},
            DiffMerkleTreeError::*,
            sync_client::SyncTask,
        };
        use crate::index::{
            Entry,
            matches_gitignores,
            full_source_code_embedding::sync_client::CodebaseIndexSyncOperation,
            full_source_code_embedding::FragmentLocation
        };
        use warp_core::send_telemetry_from_ctx;
        use warp_core::interval_timer::IntervalTimer;
        use warpui::r#async::Timer;
        use warpui::SingletonEntity;
        use warp_core::sync_queue::SyncQueue;
        use sha2::Digest;
    }
}

#[cfg(feature = "local_fs")]
const MAX_DEPTH: usize = 200;

/// The interval for periodic reindexing (20 minutes).
const REINDEX_INTERVAL: Duration = Duration::from_secs(20 * 60);
const DEFAULT_INCREMENAL_SYNC_FLUSH_INTERVAL: Duration = Duration::from_secs(60 * 60);

const FILE_TRAVERSAL_TIME: &str = "file_traversal_time";
const MERKLE_TREE_BUILD_TIME: &str = "merkle_tree_build_time";
const SYNC_TIME: &str = "sync_time";

const SUPPORTED_IGNORES: [&str; 4] = [
    ".warpindexingignore",
    ".cursorignore",
    ".cursorindexingignore",
    ".codeiumignore",
];

/// We are not attaching any context lines right now since it does not seem to improve quality
/// and is causing extra token efficiency issues.
const RETRIEVE_FRAGMENT_CONTEXT_LENGTH: usize = 0;

#[derive(Debug, Copy, Clone)]
pub enum SyncProgress {
    /// We're in the process of discovering how many nodes we need to sync.
    Discovering { total_nodes: usize },

    /// We're syncing the nodes to the server.
    Syncing {
        completed_nodes: usize,
        total_nodes: usize,
    },
}

#[cfg(feature = "local_fs")]
struct BuildFileTreeResult {
    file_tree: Entry,
    gitignores: Vec<Gitignore>,
    time_tracker: IntervalTimer,
}

#[cfg(feature = "local_fs")]
struct IndexBuildResult {
    tree: MerkleTree,
    leaf_to_fragment_metadata: LeafToFragmentMetadata,
    server_sync_result: SyncOperationResult,
    time_tracker: IntervalTimer,
}

/// Successful output of the background snapshot parse + filesystem diff task.
#[cfg(feature = "local_fs")]
struct SnapshotLoaded {
    repo_path: PathBuf,
    tree: Box<MerkleTree>,
    fragment_metadata: LeafToFragmentMetadata,
    changed_files: ChangedFiles,
    gitignores: Vec<Gitignore>,
    diff_duration: Duration,
}

/// Error loading a codebase snapshot.
#[cfg(feature = "local_fs")]
#[derive(thiserror::Error, Debug)]
enum SnapshotLoadError {
    /// Parsing the snapshot bytes failed.
    #[error("failed to parse snapshot: {0}")]
    ParseFailed(anyhow::Error),
    /// Filesystem diff against the parsed tree failed.
    #[error("failed to diff filesystem with snapshot: {0}")]
    DiffFailed(Error),
}

#[derive(Default)]
pub(crate) struct CodebaseIndexTimeStampMetadata {
    last_edited: Option<DateTime<Utc>>,
    last_snapshot: Option<DateTime<Utc>>,
    earliest_unsynced_change: Option<DateTime<Utc>>,
}

impl CodebaseIndexTimeStampMetadata {
    pub fn from_metadata(metadata: WorkspaceMetadata) -> Self {
        Self {
            last_edited: metadata.modified_ts,
            last_snapshot: None,
            earliest_unsynced_change: None,
        }
    }
}

pub struct CodebaseIndex {
    repo_path: PathBuf,
    repository: ModelHandle<Repository>,
    leaf_node_to_fragment_metadatas: LeafToFragmentMetadata,
    embedding_config: EmbeddingConfig,
    gitignores: Arc<Vec<Gitignore>>,
    tree_sync_state: TreeSourceSyncState,
    retrieval_requests: HashMap<RetrievalID, AbortHandle>,
    store_client: Arc<dyn StoreClient>,
    ts_metadata: CodebaseIndexTimeStampMetadata,
    next_incremental_flush_handle: Option<AbortHandle>,
    incremental_sync_interval: Duration,
    sync_progress_tx: async_channel::Sender<SyncProgress>,
    embedding_generation_batch_size: usize,

    #[cfg(feature = "local_fs")]
    pending_file_changes: Option<ChangedFiles>,
}

#[derive(Debug)]
enum TreeSourceSyncState {
    /// We've successfully generated and updated the Merkle tree on the client
    /// and it may or may not be synced to the server.
    Synced {
        tree: MerkleTree,
        server_sync_result: ServerSyncResult,
    },

    /// We're in the process of applying updates to the client-side
    /// Merkle tree to match the source state (usually a filesystem).
    /// While this is happening, we have a root node hash we can use
    /// for context retrieval requests to the server.
    Syncing {
        last_server_synced_root_node: Option<NodeHash>,
        abort_handle: Option<AbortHandle>,
        sync_progress: Option<SyncProgress>,
    },

    /// Tree failed to be initialized. Likely due to some file system or tree build
    /// error.
    InitializeTreeFailure(Error),
}

impl TreeSourceSyncState {
    fn last_server_synced_root_node(&self) -> Option<NodeHash> {
        match self {
            TreeSourceSyncState::Synced {
                tree,
                server_sync_result,
            } => match server_sync_result {
                ServerSyncResult::Success => Some(tree.root_node().hash()),
                ServerSyncResult::Failed {
                    last_server_synced_root_node,
                    ..
                } => last_server_synced_root_node.clone(),
            },
            TreeSourceSyncState::Syncing {
                last_server_synced_root_node,
                ..
            } => last_server_synced_root_node.clone(),
            TreeSourceSyncState::InitializeTreeFailure(_) => None,
        }
    }

    /// A brand-new, empty codebase index that still needs to be built from source
    /// and synced to the server.
    fn unsynced() -> Self {
        Self::Syncing {
            last_server_synced_root_node: None,
            abort_handle: None,
            sync_progress: None,
        }
    }

    /// Set the sync abort handle. This will return an error if the tree is not in a syncing state.
    fn set_sync_abort_handle(&mut self, new_abort_handle: AbortHandle) -> anyhow::Result<()> {
        match self {
            Self::Syncing { abort_handle, .. } => {
                *abort_handle = Some(new_abort_handle);
                Ok(())
            }
            Self::Synced { .. } | Self::InitializeTreeFailure(_) => Err(anyhow!(
                "Trying to set abort handle when tree is not syncing"
            )),
        }
    }
}

enum SyncOperationResult {
    Success {
        flushed_node_count: usize,
        flushed_fragment_result: FlushFragmentResult,
        updated_codebase_config: Option<CodebaseContextConfig>,
        cache_population_error: Option<Error>,
    },
    Error(SyncOperationError),
}

impl SyncOperationResult {
    fn telemetry_event(
        &self,
        sync_duration: Duration,
        sync_type: CodebaseContextSyncType,
    ) -> AITelemetryEvent {
        match self {
            SyncOperationResult::Success {
                flushed_node_count,
                flushed_fragment_result,
                cache_population_error,
                ..
            } => AITelemetryEvent::SyncCodebaseContextSuccess {
                total_sync_duration: sync_duration,
                sync_type,
                flushed_node_count: *flushed_node_count,
                flushed_fragment_count: flushed_fragment_result.fragment_count,
                total_fragment_size_bytes: flushed_fragment_result.total_fragment_size_bytes,
                cache_population_error: cache_population_error.as_ref().map(|e| e.to_string()),
            },
            SyncOperationResult::Error(err) => AITelemetryEvent::SyncCodebaseContextFailed {
                error: err.to_string(),
                sync_type,
            },
        }
    }

    fn server_sync_result(
        self,
        last_server_synced_root_node: Option<NodeHash>,
    ) -> ServerSyncResult {
        match self {
            SyncOperationResult::Success { .. } => ServerSyncResult::Success,
            SyncOperationResult::Error(err) => ServerSyncResult::Failed {
                error: err.into(),
                last_server_synced_root_node,
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[derive(Debug)]
pub(super) enum ServerSyncResult {
    /// The current client tree is synced with the server.
    Success,
    /// The current client tree was not synced with the server.
    /// Use the last synced root node hash for context retrieval requests.
    Failed {
        error: Error,
        last_server_synced_root_node: Option<NodeHash>,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RetrievalID(usize);

impl RetrievalID {
    fn new() -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        RetrievalID(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

pub enum CodebaseIndexEvent {
    RetrievalRequestCompleted {
        retrieval_id: RetrievalID,
        fragments: Arc<HashSet<CodeContextLocation>>,
        out_of_sync_delay: Option<Duration>,
    },
    RetrievalRequestFailed {
        retrieval_id: RetrievalID,
        error: Error,
    },
    SyncStateUpdated,
    IndexMetadataUpdated {
        root_path: PathBuf,
        event: WorkspaceMetadataEvent,
    },
    #[cfg(feature = "local_fs")]
    GitignoresUpdated {
        repo_root_path: PathBuf,
        gitignores: Arc<Vec<Gitignore>>,
    },
    LocalIndexBuilt {
        repo_root_path: PathBuf,
    },
    /// The index was synced to the server for the first time.
    #[cfg(feature = "local_fs")]
    InitialSyncCompleted {
        repo_path: PathBuf,
        has_pending_change: bool,
    },
}

struct IncrementalUpdateResult {
    tree: MerkleTree,
    build_result: IncrementalUpdateBuildResult,
}

impl IncrementalUpdateResult {
    fn telemetry_event(&self, sync_start_time: Instant) -> AITelemetryEvent {
        match &self.build_result {
            IncrementalUpdateBuildResult::Success {
                operation_result, ..
            } => operation_result.telemetry_event(
                sync_start_time.elapsed(),
                CodebaseContextSyncType::Incremental,
            ),
            IncrementalUpdateBuildResult::Error { error, .. } => {
                AITelemetryEvent::BuildTreeFailed {
                    error: error.to_string(),
                }
            }
        }
    }
}

enum IncrementalUpdateBuildResult {
    Success {
        fragment_metadata_updates: LeafToFragmentMetadataUpdates,
        operation_result: SyncOperationResult,
    },
    Error {
        error: Error,
        restore_server_sync_status: Option<ServerSyncResult>,
    },
}

impl Entity for CodebaseIndex {
    type Event = CodebaseIndexEvent;
}

impl CodebaseIndex {
    /// A brand-new, empty codebase index. We'll need to sync it with the local filesystem state.
    pub fn new_from_scratch(
        repository: ModelHandle<Repository>,
        store_client: Arc<dyn StoreClient>,
        embedding_config: EmbeddingConfig,
        max_files_repo_limit: usize,
        embedding_generation_batch_size: usize,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut index = Self::new(
            repository,
            store_client,
            embedding_config,
            embedding_generation_batch_size,
            ctx,
        );

        if let Err(err) = index.build_and_sync_from_repository_root(max_files_repo_limit, ctx) {
            safe_error!(
                safe: ("Failed to build index: {err:?}"),
                full: ("Failed to build index at root {}: {err:?}", index.repo_path.display())
            );
        }

        ctx.emit(CodebaseIndexEvent::IndexMetadataUpdated {
            root_path: index.repo_path.clone(),
            event: WorkspaceMetadataEvent::Created,
        });

        index
    }

    fn new(
        repository: ModelHandle<Repository>,
        store_client: Arc<dyn StoreClient>,
        embedding_config: EmbeddingConfig,
        embedding_generation_batch_size: usize,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let (sync_progress_tx, sync_progress_rx) = async_channel::unbounded();

        let _ = ctx.spawn_stream_local(
            sync_progress_rx,
            |me, progress, ctx| {
                me.update_sync_progress(progress, ctx);
            },
            |_, _| (),
        );

        let repo_path = repository.as_ref(ctx).root_dir().to_local_path_lossy();

        Self {
            repo_path,
            repository,
            ts_metadata: CodebaseIndexTimeStampMetadata::default(),
            embedding_config,
            gitignores: Arc::new(vec![]),
            tree_sync_state: TreeSourceSyncState::unsynced(),
            leaf_node_to_fragment_metadatas: LeafToFragmentMetadata::default(),
            retrieval_requests: Default::default(),
            store_client,
            next_incremental_flush_handle: None,
            incremental_sync_interval: DEFAULT_INCREMENAL_SYNC_FLUSH_INTERVAL,
            sync_progress_tx,
            embedding_generation_batch_size,
            #[cfg(feature = "local_fs")]
            pending_file_changes: None,
        }
    }

    #[cfg(feature = "local_fs")]
    pub(super) fn incremental_update(
        &mut self,
        changed_files: ChangedFiles,
        store_client: Arc<dyn StoreClient>,
        force_flush: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if changed_files.is_empty() {
            return;
        }

        self.pending_file_changes
            .get_or_insert_default()
            .merge_subsequent(changed_files);

        self.on_modification(ctx);

        if !force_flush {
            if self.next_incremental_flush_handle.is_some() {
                return;
            }

            let interval = self.incremental_sync_interval;
            // Schedule the next incremental update flush.
            self.next_incremental_flush_handle = Some(
                ctx.spawn(
                    async move { Timer::after(interval).await },
                    move |me, _, ctx| {
                        // Only attempt to run an incremental sync if the last sync was successful.
                        // Otherwise we should wait for the next full sync to fix the out-of-sync issues.
                        if me.codebase_index_status().last_sync_successful() == Some(true) {
                            me.flush_incremental_update(store_client, ctx);
                        }
                    },
                )
                .abort_handle(),
            );
        } else {
            if let Some(abort_handle) = self.next_incremental_flush_handle.take() {
                abort_handle.abort();
            }
            self.flush_incremental_update(store_client, ctx);
        }
    }

    #[cfg(feature = "local_fs")]
    fn flush_incremental_update(
        &mut self,
        store_client: Arc<dyn StoreClient>,
        ctx: &mut ModelContext<Self>,
    ) {
        let last_server_synced_root_node = self.last_server_synced_root_node();
        let old_state = self.update_tree_sync_state(
            TreeSourceSyncState::Syncing {
                last_server_synced_root_node,
                abort_handle: None,
                sync_progress: None,
            },
            ctx,
        );

        let (tree, last_server_sync_state) = match old_state {
            TreeSourceSyncState::Synced {
                tree,
                server_sync_result,
            } => (tree, Some(server_sync_result)),
            TreeSourceSyncState::Syncing { .. } | TreeSourceSyncState::InitializeTreeFailure(_) => {
                self.update_tree_sync_state(old_state, ctx);
                return;
            }
        };

        let Some(final_changed_files) = self.pending_file_changes.take() else {
            return;
        };

        let embedding_config = self.embedding_config();
        let repo_metadata = self.repo_metadata();
        let sync_start_time = Instant::now();
        let sync_queue = SyncQueue::as_ref(ctx).clone();
        let sync_progress_tx = self.sync_progress_tx.clone();
        let embedding_generation_batch_size = self.embedding_generation_batch_size;

        let abort_handle = ctx
            .spawn(
                async move {
                    Self::incremental_update_internal_operation(
                        tree,
                        final_changed_files,
                        last_server_sync_state,
                        store_client,
                        sync_queue,
                        embedding_config,
                        repo_metadata,
                        sync_progress_tx,
                        embedding_generation_batch_size,
                    )
                    .await
                },
                move |me, incremental_update_sync_result, ctx| {
                    send_telemetry_from_ctx!(
                        incremental_update_sync_result.telemetry_event(sync_start_time),
                        ctx
                    );
                    me.process_sync_update_result(incremental_update_sync_result, ctx);
                },
            )
            .abort_handle();

        let _ = self.tree_sync_state.set_sync_abort_handle(abort_handle);
    }

    /// Applies the file updates and starts an incremental sync.
    #[allow(clippy::too_many_arguments)]
    #[cfg(feature = "local_fs")]
    async fn incremental_update_internal_operation(
        mut tree: MerkleTree,
        changed_files: ChangedFiles,
        last_server_sync_status: Option<ServerSyncResult>,
        store_client: Arc<dyn StoreClient>,
        sync_queue: SyncQueue<SyncTask>,
        embedding_config: EmbeddingConfig,
        repo_metadata: RepoMetadata,
        sync_progress_tx: async_channel::Sender<SyncProgress>,
        embedding_generation_batch_size: usize,
    ) -> IncrementalUpdateResult {
        use super::fragment_metadata::LeafToFragmentMetadataMapping;

        // We should not start incremental sync if changed files are empty.
        debug_assert!(!changed_files.is_empty());

        let mut fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
        let mut server_sync_result = Ok(());
        let mut total_nodes_to_flush = 0;
        let mut flushed_fragment_result = FlushFragmentResult::default();

        if !changed_files.deletions().is_empty() {
            if warp_core::channel::ChannelState::enable_debug_features() {
                log::info!(
                    "Trying to remove changed files: {:?}",
                    changed_files.deletions()
                );
            }
            let (deleted_nodes, deletion_fragment_metadata_updates) =
                match tree.remove_files(changed_files.deletions).await {
                    Err(remove_err) => {
                        log::warn!("Failed to remove files during index: {remove_err:?}");
                        return IncrementalUpdateResult {
                            tree,
                            build_result: IncrementalUpdateBuildResult::Error {
                                restore_server_sync_status: last_server_sync_status,
                                error: remove_err,
                            },
                        };
                    }
                    Ok(TreeUpdateResult {
                        node_lens,
                        leaf_to_fragment_meta_updates,
                    }) => (node_lens, leaf_to_fragment_meta_updates),
                };

            let new_root_node_hash = deleted_nodes.last().unwrap().hash();

            match CodebaseIndexSyncOperation::incremental_sync(
                deleted_nodes,
                store_client.clone(),
                sync_queue.clone(),
                embedding_config,
                sync_progress_tx.clone(),
                embedding_generation_batch_size,
            )
            .await
            {
                Ok(sync_operation) => {
                    total_nodes_to_flush += sync_operation.total_pending_nodes_count();

                    match sync_operation
                        .flush_nodes_pending_sync(
                            &repo_metadata,
                            new_root_node_hash,
                            &LeafToFragmentMetadataMapping::default(),
                            sync_progress_tx.clone(),
                        )
                        .await
                    {
                        Ok(flush_result) => {
                            flushed_fragment_result += flush_result;
                        }
                        Err(err) => {
                            server_sync_result = Err(err);
                        }
                    }
                }
                Err(e) => {
                    log::debug!("Failed to prepare removal sync operation: {e}");
                    server_sync_result = Err(e)
                }
            };

            fragment_metadata_updates.merge(deletion_fragment_metadata_updates);
        }

        if !changed_files.upsertions.is_empty() {
            if warp_core::channel::ChannelState::enable_debug_features() {
                log::debug!(
                    "Trying to upsert changed files: {:?}",
                    &changed_files.upsertions
                );
            }
            let (upserted_nodes, upsertion_fragment_metadata_updates) =
                match tree.upsert_files(changed_files.upsertions).await {
                    Err(upsert_err) => {
                        log::warn!("Failed to upsert files during index: {upsert_err:?}");
                        return IncrementalUpdateResult {
                            tree,
                            build_result: IncrementalUpdateBuildResult::Error {
                                restore_server_sync_status: last_server_sync_status,
                                error: upsert_err,
                            },
                        };
                    }
                    Ok(TreeUpdateResult {
                        node_lens,
                        leaf_to_fragment_meta_updates,
                    }) => (node_lens, leaf_to_fragment_meta_updates),
                };

            let new_root_node_hash = upserted_nodes.last().unwrap().hash();
            match CodebaseIndexSyncOperation::incremental_sync(
                upserted_nodes,
                store_client,
                sync_queue,
                embedding_config,
                sync_progress_tx.clone(),
                embedding_generation_batch_size,
            )
            .await
            {
                Ok(sync_operation) => {
                    total_nodes_to_flush += sync_operation.total_pending_nodes_count();

                    match sync_operation
                        .flush_nodes_pending_sync(
                            &repo_metadata,
                            new_root_node_hash,
                            upsertion_fragment_metadata_updates.insertions(),
                            sync_progress_tx,
                        )
                        .await
                    {
                        Ok(flush_result) => {
                            flushed_fragment_result += flush_result;
                        }
                        Err(err) => {
                            server_sync_result = Err(err);
                        }
                    }
                }
                Err(e) => {
                    log::debug!("Failed to prepare upsert sync operation: {e}");
                    server_sync_result = Err(e)
                }
            };

            fragment_metadata_updates.merge(upsertion_fragment_metadata_updates);
        }

        IncrementalUpdateResult {
            tree,
            build_result: IncrementalUpdateBuildResult::Success {
                fragment_metadata_updates,
                operation_result: match server_sync_result {
                    Ok(()) => SyncOperationResult::Success {
                        flushed_node_count: total_nodes_to_flush,
                        flushed_fragment_result,
                        updated_codebase_config: None,
                        cache_population_error: None,
                    },
                    Err(e) => SyncOperationResult::Error(e),
                },
            },
        }
    }

    #[cfg(feature = "local_fs")]
    fn process_sync_update_result(
        &mut self,
        sync_update_result: IncrementalUpdateResult,
        ctx: &mut ModelContext<Self>,
    ) {
        let IncrementalUpdateResult { tree, build_result } = sync_update_result;

        match build_result {
            IncrementalUpdateBuildResult::Success {
                fragment_metadata_updates,
                operation_result,
            } => {
                self.leaf_node_to_fragment_metadatas
                    .apply_update(fragment_metadata_updates);

                let should_flush_pending_changes = if let SyncOperationResult::Error(
                    SyncOperationError::ReadFragmentError(changed_files),
                ) = &operation_result
                {
                    self.pending_file_changes
                        .get_or_insert_default()
                        .merge_subsequent(changed_files.clone());
                    true
                } else {
                    false
                };

                self.handle_sync_operation_result(tree, operation_result, ctx);
                if should_flush_pending_changes {
                    self.flush_pending_file_changes(ctx);
                }
            }
            IncrementalUpdateBuildResult::Error {
                restore_server_sync_status,
                error,
            } => {
                self.update_tree_sync_state(
                    TreeSourceSyncState::Synced {
                        tree,
                        server_sync_result: match restore_server_sync_status {
                            Some(restore_server_sync_status) => restore_server_sync_status,
                            None => ServerSyncResult::Failed {
                                error,
                                last_server_synced_root_node: self.last_server_synced_root_node(),
                            },
                        },
                    },
                    ctx,
                );
            }
        };
    }

    /// Builds up the index from root of a repository.
    /// Only a max of `max_num_files_limit` will be indexed in the repo. If None, all files are
    /// indexed.
    #[cfg(feature = "local_fs")]
    fn build_and_sync_from_repository_root(
        &mut self,
        max_files_repo_limit: usize,
        ctx: &mut ModelContext<'_, Self>,
    ) -> Result<(), Error> {
        let repo_path = self.repo_path.clone();
        let repo_path_clone = self.repo_path.clone();
        let repo_metadata = RepoMetadata {
            path: Some(repo_path.to_string_lossy().to_string()),
        };
        let store_client = self.store_client.clone();
        let max_num_files_limit = Some(max_files_repo_limit);

        let abort_handle = ctx
            .spawn(
                async move { Self::build_file_tree(repo_path, max_num_files_limit).await },
                move |me, file_tree_result, ctx| {
                    match file_tree_result {
                        Ok(res) => me.process_build_file_tree_result(
                            res,
                            repo_metadata,
                            repo_path_clone,
                            store_client,
                            ctx,
                        ),
                        Err(e) => {
                            log::error!("Failed to build tree {e}");
                            send_telemetry_from_ctx!(
                                AITelemetryEvent::BuildTreeFailed {
                                    error: e.to_string(),
                                },
                                ctx
                            );
                            me.update_tree_sync_state(
                                TreeSourceSyncState::InitializeTreeFailure(e),
                                ctx,
                            );
                        }
                    };
                },
            )
            .abort_handle();

        self.tree_sync_state.set_sync_abort_handle(abort_handle)?;
        Ok(())
    }

    /// Takes the result from a sync operation and updates the tree sync state accordingly.
    /// This also applies any codebase context config change returned.
    fn handle_sync_operation_result(
        &mut self,
        tree: MerkleTree,
        sync_operation_result: SyncOperationResult,
        ctx: &mut ModelContext<Self>,
    ) -> TreeSourceSyncState {
        if let SyncOperationResult::Success {
            updated_codebase_config,
            ..
        } = &sync_operation_result
        {
            // Apply any updated codebase config
            if let Some(updated_codebase_config) = updated_codebase_config {
                self.embedding_config = updated_codebase_config.embedding_config;
                self.incremental_sync_interval = updated_codebase_config.embedding_cadence;
            }

            // Clear the earliest unsynced change on successful sync
            self.ts_metadata.earliest_unsynced_change = None;
        }

        self.update_tree_sync_state(
            TreeSourceSyncState::Synced {
                tree,
                server_sync_result: sync_operation_result
                    .server_sync_result(self.last_server_synced_root_node()),
            },
            ctx,
        )
    }

    /// Update the codebase's tree sync state with the input. This returns the old state.
    fn update_tree_sync_state(
        &mut self,
        new_state: TreeSourceSyncState,
        ctx: &mut ModelContext<Self>,
    ) -> TreeSourceSyncState {
        let old_state = std::mem::replace(&mut self.tree_sync_state, new_state);
        ctx.emit(CodebaseIndexEvent::SyncStateUpdated);
        old_state
    }

    fn update_sync_progress(&mut self, progress: SyncProgress, ctx: &mut ModelContext<Self>) {
        // Only update if we're currently syncing
        if let TreeSourceSyncState::Syncing { sync_progress, .. } = &mut self.tree_sync_state {
            *sync_progress = Some(progress);

            ctx.emit(CodebaseIndexEvent::SyncStateUpdated);
        }
    }

    fn construct_initial_ignores(repo_path: &Path) -> Vec<Gitignore> {
        let mut gitignores = vec![];
        let (global_gitignore, _) = Gitignore::global();
        gitignores.push(global_gitignore);

        for option in SUPPORTED_IGNORES {
            let gitignore_path = repo_path.join(option);
            if gitignore_path.exists() {
                let (gitignore, _) = Gitignore::new(gitignore_path);
                gitignores.push(gitignore);
            }
        }

        gitignores
    }

    /// Build the file tree and gitignores.
    #[cfg(feature = "local_fs")]
    async fn build_file_tree(
        repo_path: PathBuf,
        max_num_files_limit: Option<usize>,
    ) -> Result<BuildFileTreeResult, Error> {
        log::info!("Started creating codebase index for repository root: {repo_path:?}");
        let time_tracker = IntervalTimer::new();

        // We need to canonicalize the path to make sure build tree can apply gitignores correctly.
        let mut gitignores = Self::construct_initial_ignores(&repo_path);

        // First traverse the repo path to retrieve all files we want to parse.
        let mut files = Vec::new();
        let mut remaining_file_quotas = max_num_files_limit;
        let entry = Entry::build_tree(
            &repo_path,
            &mut files,
            &mut gitignores,
            remaining_file_quotas.as_mut(),
            MAX_DEPTH,
            0,
            &IgnoredPathStrategy::Exclude, // override_ignore_for_files
        )?;

        Ok(BuildFileTreeResult {
            file_tree: entry,
            gitignores,
            time_tracker,
        })
    }

    /// Save the gitignores so that the CodebaseIndexManager can register the filewatcher,
    /// then use the file tree to build and sync the Merkle tree.
    #[cfg(feature = "local_fs")]
    fn process_build_file_tree_result(
        &mut self,
        build_file_tree_result: BuildFileTreeResult,
        repo_metadata: RepoMetadata,
        repo_path: PathBuf,
        store_client: Arc<dyn StoreClient>,
        ctx: &mut ModelContext<Self>,
    ) {
        let BuildFileTreeResult {
            file_tree,
            gitignores,
            mut time_tracker,
        } = build_file_tree_result;

        time_tracker.mark_interval_end(FILE_TRAVERSAL_TIME);

        self.gitignores = Arc::new(gitignores);
        ctx.emit(CodebaseIndexEvent::GitignoresUpdated {
            repo_root_path: repo_path.clone(),
            gitignores: self.gitignores.clone(),
        });

        let sync_queue = SyncQueue::as_ref(ctx).clone();

        let sync_progress_tx = self.sync_progress_tx.clone();
        let embedding_generation_batch_size = self.embedding_generation_batch_size;
        let abort_handle = ctx
            .spawn(
                async move {
                    Self::build_merkle_tree_and_sync(
                        file_tree,
                        repo_metadata,
                        repo_path,
                        store_client,
                        sync_queue,
                        time_tracker,
                        sync_progress_tx,
                        embedding_generation_batch_size,
                    )
                    .await
                },
                move |me, index_build_result, ctx| {
                    me.process_merkle_tree_result(index_build_result, ctx);
                },
            )
            .abort_handle();
        let _ = self.tree_sync_state.set_sync_abort_handle(abort_handle);
    }

    #[cfg(feature = "local_fs")]
    #[allow(clippy::too_many_arguments)]
    async fn build_merkle_tree_and_sync(
        file_tree: Entry,
        repo_metadata: RepoMetadata,
        repo_path: PathBuf,
        store_client: Arc<dyn StoreClient>,
        sync_queue: SyncQueue<SyncTask>,
        mut time_tracker: IntervalTimer,
        sync_progress_tx: async_channel::Sender<SyncProgress>,
        embedding_generation_batch_size: usize,
    ) -> Result<IndexBuildResult, Error> {
        let (tree, leaf_to_fragment_metadata) = MerkleTree::try_new(file_tree).await?;

        time_tracker.mark_interval_end(MERKLE_TREE_BUILD_TIME);

        log::info!("Created index for repository: {repo_path:?}");

        let server_sync_result = Self::full_sync_internal(
            &tree,
            store_client.clone(),
            sync_queue,
            &repo_metadata,
            &leaf_to_fragment_metadata,
            sync_progress_tx,
            true, /* should_populate_cache */
            embedding_generation_batch_size,
        )
        .await;

        time_tracker.mark_interval_end(SYNC_TIME);

        log::info!("Finished syncing index for repository: {repo_path:?}");

        Ok(IndexBuildResult {
            tree,
            leaf_to_fragment_metadata,
            server_sync_result,
            time_tracker,
        })
    }

    /// Fully sync the input merkle tree state with the server and returns the result of the sync operation.
    #[cfg(feature = "local_fs")]
    #[allow(clippy::too_many_arguments)]
    async fn full_sync_internal(
        tree: &MerkleTree,
        store_client: Arc<dyn StoreClient>,
        sync_queue: SyncQueue<SyncTask>,
        repo_metadata: &RepoMetadata,
        fragment_metadata: &LeafToFragmentMetadata,
        sync_progress_tx: async_channel::Sender<SyncProgress>,
        should_populate_cache: bool,
        embedding_generation_batch_size: usize,
    ) -> SyncOperationResult {
        let res = CodebaseIndexSyncOperation::full_sync(
            tree,
            store_client.clone(),
            sync_queue,
            sync_progress_tx.clone(),
            embedding_generation_batch_size,
        )
        .await;

        match res {
            Ok((sync_operation, updated_config)) => {
                let root_node_hash = tree.root_node().hash();
                let total_nodes_to_sync = sync_operation.total_pending_nodes_count();
                match sync_operation
                    .flush_nodes_pending_sync(
                        repo_metadata,
                        root_node_hash,
                        fragment_metadata.mapping(),
                        sync_progress_tx,
                    )
                    .await
                {
                    Ok(flush_result) => {
                        let mut cache_population_error = None;
                        // Populate cache if needed.
                        if should_populate_cache {
                            if let Err(e) = store_client
                                .populate_merkle_tree_cache(
                                    updated_config.embedding_config,
                                    tree.root_node().hash(),
                                    repo_metadata.clone(),
                                )
                                .await
                            {
                                cache_population_error = Some(e);
                            }
                        }
                        SyncOperationResult::Success {
                            flushed_node_count: total_nodes_to_sync,
                            flushed_fragment_result: flush_result,
                            updated_codebase_config: Some(updated_config),
                            cache_population_error,
                        }
                    }
                    Err(err) => SyncOperationResult::Error(err),
                }
            }
            Err(err) => SyncOperationResult::Error(err),
        }
    }

    pub(super) fn update_timestamps_from_metadata(&mut self, ts_metadata: WorkspaceMetadata) {
        if ts_metadata.modified_ts.is_some() {
            self.ts_metadata.last_edited = ts_metadata.modified_ts;
        }
    }

    /// Performs a full reparse of the merkle tree, followed by a full server sync. This force evicts the
    /// existing merkle tree state.
    #[cfg(feature = "local_fs")]
    pub(super) fn full_sync_index(
        &mut self,
        max_files_repo_limit: usize,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), Error> {
        self.update_tree_sync_state(TreeSourceSyncState::unsynced(), ctx);
        self.build_and_sync_from_repository_root(max_files_repo_limit, ctx)
    }

    /// Attempt to perform a full SERVER sync on the current index. We only proceed with the sync if there is
    /// no other in progress syncs.
    #[cfg(feature = "local_fs")]
    pub(super) fn try_full_server_sync_index(&mut self, ctx: &mut ModelContext<Self>) {
        let last_server_synced_root_node = self.last_server_synced_root_node();
        let sync_state = self.update_tree_sync_state(
            TreeSourceSyncState::Syncing {
                last_server_synced_root_node,
                abort_handle: None,
                sync_progress: None,
            },
            ctx,
        );

        let tree = match sync_state {
            TreeSourceSyncState::Synced { tree, .. } => tree,
            // We are already syncing. We can skip this full sync.
            TreeSourceSyncState::Syncing { .. } => {
                self.schedule_next_index(ctx);
                return;
            }
            TreeSourceSyncState::InitializeTreeFailure(_) => {
                log::warn!("Tried to periodic sync but encountered tree initialization failure");
                // Note that we're not rebuilding the entire tree for periodic syncs yet since it is potentially
                // expensive.
                self.update_tree_sync_state(sync_state, ctx);
                self.schedule_next_index(ctx);
                return;
            }
        };

        let sync_start_time = Instant::now();

        let store_client = self.store_client.clone();
        let repo_metadata = self.repo_metadata();

        // TODO(kevin): this is not ideal. Think about how we could avoid the cloning here.
        let fragment_metadata = self.leaf_node_to_fragment_metadatas.clone();
        let sync_queue = SyncQueue::as_ref(ctx).clone();

        let sync_progress_tx = self.sync_progress_tx.clone();
        let embedding_generation_batch_size = self.embedding_generation_batch_size;

        let abort_handle = ctx
            .spawn(
                async move {
                    let sync_result = Self::full_sync_internal(
                        &tree,
                        store_client,
                        sync_queue,
                        &repo_metadata,
                        &fragment_metadata,
                        sync_progress_tx,
                        false, /* should_populate_cache */
                        embedding_generation_batch_size,
                    )
                    .await;

                    (tree, sync_result)
                },
                move |me, (tree, server_sync_result), ctx| {
                    send_telemetry_from_ctx!(
                        server_sync_result.telemetry_event(
                            sync_start_time.elapsed(),
                            CodebaseContextSyncType::Full
                        ),
                        ctx
                    );

                    // We should only flush pending changes when we know the sync failed because of a read fragment error.
                    let should_flush_pending_changes = if let SyncOperationResult::Error(
                        SyncOperationError::ReadFragmentError(changed_files),
                    ) = &server_sync_result
                    {
                        me.pending_file_changes
                            .get_or_insert_default()
                            .merge_subsequent(changed_files.clone());
                        true
                    } else {
                        false
                    };

                    me.on_modification(ctx);
                    me.handle_sync_operation_result(tree, server_sync_result, ctx);
                    if should_flush_pending_changes {
                        me.flush_pending_file_changes(ctx);
                    }
                    me.schedule_next_index(ctx);
                },
            )
            .abort_handle();

        // We just set tree sync state to syncing above.
        self.tree_sync_state
            .set_sync_abort_handle(abort_handle)
            .expect("Should be syncing");
    }

    /// Schedules the next periodic sync task.
    #[cfg(feature = "local_fs")]
    fn schedule_next_index(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async move { Timer::after(REINDEX_INTERVAL).await },
            move |me, _, ctx| {
                // Schedule the next re-sync cycle
                me.try_full_server_sync_index(ctx);
            },
        );
    }

    #[cfg(feature = "local_fs")]
    fn process_merkle_tree_result(
        &mut self,
        index_build_result: Result<IndexBuildResult, Error>,
        ctx: &mut ModelContext<Self>,
    ) {
        match index_build_result {
            Ok(IndexBuildResult {
                tree,
                leaf_to_fragment_metadata,
                server_sync_result,
                time_tracker,
            }) => {
                // Emit telemetries for the initial sync result.
                if let Some(sync_time) = time_tracker.compute_duration_for_interval(SYNC_TIME) {
                    send_telemetry_from_ctx!(
                        server_sync_result
                            .telemetry_event(sync_time, CodebaseContextSyncType::Initial),
                        ctx
                    );
                }

                if let Some((file_traversal_duration, merkle_tree_parse_duration)) = time_tracker
                    .compute_duration_for_interval(FILE_TRAVERSAL_TIME)
                    .zip(time_tracker.compute_duration_for_interval(MERKLE_TREE_BUILD_TIME))
                {
                    send_telemetry_from_ctx!(
                        AITelemetryEvent::BuildTreeSuccess {
                            file_traversal_duration,
                            merkle_tree_parse_duration
                        },
                        ctx
                    );
                }

                if let SyncOperationResult::Error(SyncOperationError::ReadFragmentError(
                    changed_files,
                )) = &server_sync_result
                {
                    self.pending_file_changes
                        .get_or_insert_default()
                        .merge_subsequent(changed_files.clone());
                }

                self.handle_sync_operation_result(tree, server_sync_result, ctx);

                ctx.emit(CodebaseIndexEvent::InitialSyncCompleted {
                    repo_path: self.repo_path.clone(),
                    has_pending_change: self.pending_file_changes.is_some(),
                });

                self.leaf_node_to_fragment_metadatas = leaf_to_fragment_metadata;

                self.flush_pending_file_changes(ctx);
                self.schedule_next_index(ctx);
            }
            Err(err) => {
                safe_error!(
                    safe: ("Failed to build index: {err:?}"),
                    full: ("Failed to build index at root {}: {err:?}", self.repo_path.display())
                );
                send_telemetry_from_ctx!(
                    AITelemetryEvent::BuildTreeFailed {
                        error: err.to_string()
                    },
                    ctx
                );
                self.update_tree_sync_state(TreeSourceSyncState::InitializeTreeFailure(err), ctx);
            }
        }
    }

    #[cfg(feature = "local_fs")]
    pub fn flush_pending_file_changes(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(pending_file_changes) = self.pending_file_changes.take() {
            self.incremental_update(pending_file_changes, self.store_client.clone(), true, ctx);
        }
    }

    #[cfg(not(feature = "local_fs"))]
    pub fn build_and_sync_from_repository_root(
        &mut self,
        _max_num_files_limit: usize,
        _ctx: &mut ModelContext<'_, Self>,
    ) -> Result<(), Error> {
        Err(Error::UnsupportedPlatform)
    }

    fn fragment_metadatas_from_hash<T: AsRef<ContentHash>>(
        &self,
        leaf_hash: T,
    ) -> Option<&Vec<FragmentMetadata>> {
        self.leaf_node_to_fragment_metadatas.get(leaf_hash.as_ref())
    }

    pub(super) fn fragment_metadatas_from_hashes(
        &self,
        root_hash: &NodeHash,
        content_hashes: &[ContentHash],
    ) -> Result<HashMap<ContentHash, Vec<FragmentMetadata>>, FragmentMetadataLookupError> {
        let current_root_hash = self
            .last_server_synced_root_node()
            .ok_or(FragmentMetadataLookupError::IndexNotSynced)?;
        if &current_root_hash != root_hash {
            return Err(FragmentMetadataLookupError::RootHashMismatch {
                requested: root_hash.clone(),
                current: current_root_hash,
            });
        }

        Ok(content_hashes
            .iter()
            .filter_map(|hash| {
                self.fragment_metadatas_from_hash(hash)
                    .map(|metadata| (hash.clone(), metadata.clone()))
            })
            .collect())
    }

    fn repo_metadata(&self) -> RepoMetadata {
        RepoMetadata {
            path: Some(self.repo_path.to_string_lossy().to_string()),
        }
    }

    fn embedding_config(&self) -> EmbeddingConfig {
        self.embedding_config
    }

    pub(super) fn update_embedding_generation_batch_size(&mut self, new_batch_size: usize) {
        self.embedding_generation_batch_size = new_batch_size;
    }

    fn merkle_tree(&self) -> Option<&MerkleTree> {
        match &self.tree_sync_state {
            TreeSourceSyncState::Synced { tree, .. } => Some(tree),
            TreeSourceSyncState::InitializeTreeFailure(_) | TreeSourceSyncState::Syncing { .. } => {
                None
            }
        }
    }

    pub(super) fn codebase_index_status(&self) -> CodebaseIndexStatus {
        let root_hash = self.last_server_synced_root_node();
        let has_synced_version = root_hash.is_some();
        match &self.tree_sync_state {
            TreeSourceSyncState::Synced {
                server_sync_result, ..
            } => CodebaseIndexStatus {
                has_pending: false,
                has_synced_version,
                last_sync_successful: Some(match server_sync_result {
                    ServerSyncResult::Success => CodebaseIndexFinishedStatus::Completed,
                    ServerSyncResult::Failed { error, .. } => {
                        CodebaseIndexFinishedStatus::Failed(error.into())
                    }
                }),
                sync_progress: None,
                root_hash: root_hash.clone(),
            },
            TreeSourceSyncState::InitializeTreeFailure(e) => CodebaseIndexStatus {
                has_pending: false,
                has_synced_version,
                last_sync_successful: Some(CodebaseIndexFinishedStatus::Failed(e.into())),
                sync_progress: None,
                root_hash: root_hash.clone(),
            },
            TreeSourceSyncState::Syncing { sync_progress, .. } => CodebaseIndexStatus {
                has_pending: true,
                has_synced_version,
                last_sync_successful: None,
                sync_progress: *sync_progress,
                root_hash: root_hash.clone(),
            },
        }
    }

    fn last_server_synced_root_node(&self) -> Option<NodeHash> {
        self.tree_sync_state.last_server_synced_root_node()
    }

    pub(super) fn retrieve_relevant_files(
        &mut self,
        query: String,
        store_client: Arc<dyn StoreClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<RetrievalID, RetrieveFileError> {
        match &self.last_server_synced_root_node() {
            Some(root_node_hash) => {
                ctx.emit(CodebaseIndexEvent::IndexMetadataUpdated {
                    root_path: self.repo_path.clone(),
                    event: WorkspaceMetadataEvent::Queried,
                });

                let request_id = RetrievalID::new();

                let embedding_config = self.embedding_config();
                let root_node_hash = root_node_hash.clone();
                let repo_metadata = self.repo_metadata();

                let query_clone = query.clone();
                let store_client_clone = store_client.clone();
                let request_id_clone = request_id.clone();

                let fetch_relevant_fragments_future = ctx.spawn(
                    async move {
                        store_client
                            .get_relevant_fragments(
                                embedding_config,
                                query,
                                root_node_hash,
                                repo_metadata,
                            )
                            .await
                    },
                    |me, relevant_fragments, ctx| {
                        me.process_relevant_fragments(
                            relevant_fragments,
                            store_client_clone,
                            query_clone,
                            request_id_clone,
                            ctx,
                        )
                    },
                );

                self.retrieval_requests.insert(
                    request_id.clone(),
                    fetch_relevant_fragments_future.abort_handle(),
                );
                Ok(request_id)
            }
            None => match &self.tree_sync_state {
                TreeSourceSyncState::Syncing { .. } => Err(RetrieveFileError::IndexSyncing),
                TreeSourceSyncState::InitializeTreeFailure(error)
                | TreeSourceSyncState::Synced {
                    server_sync_result: ServerSyncResult::Failed { error, .. },
                    ..
                } => Err(RetrieveFileError::IndexFailed(error.into())),
                _ => {
                    panic!("Impossible state: successfully synced tree must have a valid root node")
                }
            },
        }
    }

    /// Given content hashes, fetch their associated metadata. Then, build and rerank the fragments
    /// in a background thread.
    fn process_relevant_fragments(
        &mut self,
        relevant_fragments_result: Result<Vec<ContentHash>, Error>,
        store_client: Arc<dyn StoreClient>,
        query: String,
        retrieval_id: RetrievalID,
        ctx: &mut ModelContext<Self>,
    ) {
        match relevant_fragments_result {
            Err(err) => {
                log::error!(
                    "Failed to retrieve relevant fragment on root {:?}",
                    self.last_server_synced_root_node()
                );
                ctx.emit(CodebaseIndexEvent::RetrievalRequestFailed {
                    retrieval_id,
                    error: err,
                });
            }
            Ok(hashes) => {
                let fragment_metadatas = self.hashes_to_fragment_metadata(&hashes);
                let retrieval_id_clone = retrieval_id.clone();
                let build_and_rerank_future = ctx.spawn(
                    Self::build_and_rerank_fragments(fragment_metadatas, store_client, query),
                    |me, reranked_fragments_result, ctx| {
                        me.process_reranked_fragments(
                            retrieval_id_clone,
                            reranked_fragments_result,
                            ctx,
                        );
                    },
                );
                self.retrieval_requests
                    .insert(retrieval_id, build_and_rerank_future.abort_handle());
            }
        }
    }

    fn hashes_to_fragment_metadata(
        &self,
        hashes: &[ContentHash],
    ) -> Vec<(ContentHash, FragmentMetadata)> {
        let mut fragment_metadatas = Vec::new();
        for hash in hashes {
            let Some(metadatas) = self.fragment_metadatas_from_hash(hash) else {
                log::warn!("Could not find metadata for leaf hash {hash:?}");
                continue;
            };
            for metadata in metadatas {
                fragment_metadatas.push((hash.clone(), metadata.clone()));
            }
        }
        fragment_metadatas
    }

    async fn build_and_rerank_fragments(
        fragment_metadata: Vec<(ContentHash, FragmentMetadata)>,
        store_client: Arc<dyn StoreClient>,
        query: String,
    ) -> Result<Vec<Fragment>, Error> {
        let fragments = build_fragments_from_metadata(fragment_metadata).await;
        store_client
            .rerank_fragments(query, fragments.successfully_read)
            .await
    }

    /// Turn the list of fragments into a list of paths with fragments, then send them back through the provided channel.
    fn process_reranked_fragments(
        &self,
        retrieval_id: RetrievalID,
        reranked_fragments: Result<Vec<Fragment>, Error>,
        ctx: &mut ModelContext<Self>,
    ) {
        match reranked_fragments {
            Ok(reranked_fragments) => {
                // Create a HashSet of CodeContextLocation::Fragment instances
                let code_fragments =
                    self.process_fragments(reranked_fragments, RETRIEVE_FRAGMENT_CONTEXT_LENGTH);

                ctx.emit(CodebaseIndexEvent::RetrievalRequestCompleted {
                    retrieval_id,
                    fragments: Arc::new(code_fragments),
                    out_of_sync_delay: self.out_of_sync_delay(),
                });
            }
            Err(err) => {
                ctx.emit(CodebaseIndexEvent::RetrievalRequestFailed {
                    retrieval_id,
                    error: err,
                });
            }
        };
    }

    pub(super) fn abort_retrieval_request(&mut self, retrieval_id: RetrievalID) {
        if let Some(abort_handle) = self.retrieval_requests.remove(&retrieval_id) {
            abort_handle.abort();
        }
    }

    pub(super) fn abort_in_progress_sync(&self) {
        if let TreeSourceSyncState::Syncing {
            abort_handle: Some(abort_handle),
            ..
        } = &self.tree_sync_state
        {
            abort_handle.abort();
        }
    }

    pub(super) fn generate_snapshot(&self) -> anyhow::Result<SerializedCodebaseIndex> {
        let Some(merkle_tree) = self.merkle_tree() else {
            return Err(anyhow::anyhow!(
                "No Merkle tree available to serialize index at {:?}",
                self.repo_path
            ));
        };

        SerializedCodebaseIndex::new(merkle_tree, &self.leaf_node_to_fragment_metadatas)
    }

    pub(super) fn update_snapshot_ts(&mut self, snapshot_ts: DateTime<Utc>) {
        self.ts_metadata.last_snapshot = Some(snapshot_ts);
    }

    pub(super) fn has_unsnapshotted_changes(&self) -> bool {
        // If there's never been a snapshot, the whole index still needs
        // to be snapshotted.
        let Some(snapshot_ts) = self.ts_metadata.last_snapshot else {
            return true;
        };

        // If the index was snapshotted but there have been no edits,
        // there are no outstanding changes.
        let Some(edited_ts) = self.ts_metadata.last_edited else {
            return false;
        };

        // If we have an edit after the last snapshot was generated,
        // there are outstanding changes.
        edited_ts >= snapshot_ts
    }

    /// Deserializes a snapshot from bytes into a Merkle tree and fragment metadata.
    pub(super) async fn deserialize_snapshot(
        snapshot_bytes: Vec<u8>,
    ) -> anyhow::Result<(MerkleTree, LeafToFragmentMetadata)> {
        let deserialize = move || {
            let serialized_codebase_index: SerializedCodebaseIndex =
                bincode::deserialize(&snapshot_bytes)
                    .or_else(|_| serde_json::from_slice(&snapshot_bytes))?;
            MerkleTree::from_serialized_tree(serialized_codebase_index.into_tree())
        };

        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::spawn_blocking(deserialize)
                .await
                .map_err(|e| anyhow::anyhow!("spawn_blocking join error: {e}"))?
        } else {
            deserialize()
        }
    }

    /// Serializes a `SerializedCodebaseIndex` into bytes.
    pub(super) async fn serialize_snapshot(
        serializable_index: SerializedCodebaseIndex,
    ) -> anyhow::Result<Vec<u8>> {
        let serialize =
            move || bincode::serialize(&serializable_index).map_err(anyhow::Error::from);

        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::spawn_blocking(serialize)
                .await
                .map_err(|e| anyhow::anyhow!("spawn_blocking join error: {e}"))?
        } else {
            serialize()
        }
    }

    // Convert fragments into CodeContextLocations. This function groups and dedupes fragments in the same file.
    // It also allows the caller to define a context line number surrounding the relevant fragment.
    fn process_fragments(
        &self,
        fragments: Vec<Fragment>,
        context_lines: usize,
    ) -> HashSet<CodeContextLocation> {
        // Map to collect fragments by file path
        let mut fragments_by_path: HashMap<&PathBuf, Vec<Range<usize>>> = HashMap::new();
        let mut whole_files = HashSet::new();

        // First pass - collect all fragments and their line ranges by file path
        for fragment in &fragments {
            if let Some(metadata) = self
                .fragment_metadatas_from_hash(&fragment.content_hash)
                .and_then(|metadatas| {
                    metadatas.iter().find(|m| {
                        m.absolute_path == fragment.location.absolute_path
                            && m.location.byte_range == fragment.location.byte_range
                    })
                })
            {
                // Add line range with context to the appropriate file's collection
                let path = &fragment.location.absolute_path;
                let start = metadata.location.start_line.saturating_sub(context_lines);
                let end = metadata.location.end_line + 1 + context_lines; // Make the range inclusive on both ends

                fragments_by_path.entry(path).or_default().push(start..end);
            } else {
                // Fallback to whole file if metadata not found
                whole_files.insert(fragment.location.absolute_path.clone());
            }
        }

        // Second pass - process each file's fragments
        let mut result = HashSet::new();

        // Process each file's fragments
        for (path, mut line_ranges) in fragments_by_path {
            if line_ranges.is_empty() {
                continue;
            }

            // We can skip the fragments if the entire file is already included in the context.
            if whole_files.contains(path) {
                continue;
            }

            // Sort ranges by start position
            line_ranges.sort_by_key(|range| range.start);

            // Merge overlapping or adjacent ranges
            let mut merged_ranges: Vec<Range<usize>> = Vec::new();
            for range in line_ranges {
                if let Some(last) = merged_ranges.last_mut() {
                    // If current range overlaps or is adjacent to the last one, merge them
                    if range.start <= last.end {
                        last.end = last.end.max(range.end);
                    } else {
                        merged_ranges.push(range);
                    }
                } else {
                    merged_ranges.push(range);
                }
            }

            // Add file fragment location with all merged ranges
            result.insert(CodeContextLocation::Fragment(FileFragmentLocation {
                path: path.clone(),
                line_ranges: merged_ranges,
            }));
        }

        // Add whole files to the result set
        result.extend(whole_files.into_iter().map(CodeContextLocation::WholeFile));
        result
    }

    /// A new index built from a snapshot. This constructor builds the index and starts
    /// a full sync with the server.
    #[cfg(feature = "local_fs")]
    pub fn new_from_snapshot(
        repository: ModelHandle<Repository>,
        store_client: Arc<dyn StoreClient>,
        embedding_config: EmbeddingConfig,
        snapshot_bytes: Vec<u8>,
        max_files_repo_limit: usize,
        embedding_generation_batch_size: usize,
        ctx: &mut ModelContext<Self>,
    ) -> Result<Self, Error> {
        let mut index = Self::new(
            repository,
            store_client,
            embedding_config,
            embedding_generation_batch_size,
            ctx,
        );
        index.rebuild_and_sync_from_snapshot(snapshot_bytes, max_files_repo_limit, ctx);
        Ok(index)
    }

    #[cfg(feature = "local_fs")]
    fn rebuild_and_sync_from_snapshot(
        &mut self,
        snapshot_bytes: Vec<u8>,
        max_files_repo_limit: usize,
        ctx: &mut ModelContext<'_, Self>,
    ) {
        let repo_metadata = RepoMetadata {
            path: Some(self.repo_path.to_string_lossy().to_string()),
        };
        let store_client = self.store_client.clone();
        let embedding_generation_batch_size = self.embedding_generation_batch_size;

        self.update_tree_sync_state(TreeSourceSyncState::unsynced(), ctx);

        let sync_queue = SyncQueue::as_ref(ctx).clone();
        let repo_path = self.repo_path.clone();
        let sync_progress_tx = self.sync_progress_tx.clone();

        // Parse snapshot and diff filesystem in a single background task.
        let abort_handle = ctx
            .spawn(
                async move {
                    log::info!("Reading snapshot from file for repo {repo_path:?}");
                    let (tree, fragment_metadata) =
                        Self::deserialize_snapshot(snapshot_bytes)
                            .await
                            .map_err(SnapshotLoadError::ParseFailed)?;

                    log::info!(
                        "Diffing filesystem with tree from snapshot for repo {repo_path:?}"
                    );
                    let diff_start_time = Instant::now();
                    let (changed_files, gitignores) = Self::diff_filesystem_with_tree(
                        repo_path.clone(),
                        &tree,
                        max_files_repo_limit,
                    )
                    .map_err(SnapshotLoadError::DiffFailed)?;

                    Ok(SnapshotLoaded {
                        repo_path,
                        tree: Box::new(tree),
                        fragment_metadata,
                        changed_files,
                        gitignores,
                        diff_duration: diff_start_time.elapsed(),
                    })
                },
                move |me, load_result, ctx| match load_result {
                    Ok(SnapshotLoaded {
                        repo_path,
                        tree: boxed_tree,
                        fragment_metadata,
                        changed_files,
                        gitignores,
                        diff_duration,
                    }) => {
                        let tree = *boxed_tree;
                        send_telemetry_from_ctx!(
                            AITelemetryEvent::MerkleTreeSnapshotDiffSuccess {
                                duration: diff_duration
                            },
                            ctx
                        );

                        log::info!(
                            "Diffed filesystem with tree from snapshot for repo {repo_path:?}"
                        );

                        let abort_handle = ctx
                            .spawn(
                                async move {
                                    log::info!(
                                        "Syncing from snapshot for repo {repo_path:?}"
                                    );
                                    let sync_operation_result = Self::full_sync_internal(
                                        &tree,
                                        store_client,
                                        sync_queue,
                                        &repo_metadata,
                                        &fragment_metadata,
                                        sync_progress_tx,
                                        true, /* should_populate_cache */
                                        embedding_generation_batch_size,
                                    )
                                    .await;

                                    (
                                        repo_path,
                                        tree,
                                        sync_operation_result,
                                        fragment_metadata,
                                        changed_files,
                                        gitignores,
                                    )
                                },
                                move |me,
                                      (
                                    repo_path,
                                    tree,
                                    sync_operation_result,
                                    fragment_metadata,
                                    mut changed_files,
                                    gitignores,
                                ),
                                      ctx| {
                                    // Incremental sync assumes the previous sync was successful,
                                    // so only flush pending file changes if the sync operation
                                    // was successful or we failed to read fragments (which is the
                                    // only error that an incremental sync can recover from).
                                    let should_flush_pending_file_changes =
                                        match &sync_operation_result {
                                            SyncOperationResult::Success { .. } => {
                                                log::info!(
                                                    "Synced from snapshot for repo {repo_path:?}"
                                                );
                                                true
                                            }
                                            SyncOperationResult::Error(
                                                SyncOperationError::ReadFragmentError(changed_files_from_read_fragment_error),
                                            ) => {
                                                log::info!(
                                                    "Failed to sync from snapshot for repo {repo_path:?}: read fragments error",
                                                );
                                                changed_files.merge_subsequent(
                                                    changed_files_from_read_fragment_error
                                                        .clone(),
                                                );
                                                true
                                            }
                                            SyncOperationResult::Error(err) => {
                                                safe_error!(
                                                    safe: ("Failed to sync index from snapshot: {err:?}"),
                                                    full: ("Failed to sync index from snapshot for repo {repo_path:?}: {err:?}")
                                                );
                                                false
                                            }
                                        };

                                    me.gitignores = Arc::new(gitignores);
                                    ctx.emit(CodebaseIndexEvent::GitignoresUpdated {
                                        repo_root_path: me.repo_path.clone(),
                                        gitignores: me.gitignores.clone(),
                                    });
                                    me.handle_sync_operation_result(
                                        tree,
                                        sync_operation_result,
                                        ctx,
                                    );
                                    ctx.emit(CodebaseIndexEvent::LocalIndexBuilt {
                                        repo_root_path: me.repo_path.clone(),
                                    });
                                    me.leaf_node_to_fragment_metadatas = fragment_metadata;
                                    me.pending_file_changes
                                        .get_or_insert_default()
                                        .merge_subsequent(changed_files);

                                    if should_flush_pending_file_changes {
                                        me.flush_pending_file_changes(ctx);
                                    }
                                },
                            )
                            .abort_handle();

                        let _ = me.tree_sync_state.set_sync_abort_handle(abort_handle);
                    }
                    Err(SnapshotLoadError::DiffFailed(err)) => {
                        send_telemetry_from_ctx!(
                            AITelemetryEvent::MerkleTreeSnapshotDiffFailed {
                                error: err.to_string()
                            },
                            ctx
                        );
                        log::error!(
                            "Failed to diff filesystem with tree from snapshot: {err:?}"
                        );
                        me.update_tree_sync_state(
                            TreeSourceSyncState::InitializeTreeFailure(err),
                            ctx,
                        );
                    }
                    Err(SnapshotLoadError::ParseFailed(e)) => {
                        log::error!("Failed to parse snapshot: {e:?}");
                        me.update_tree_sync_state(
                            TreeSourceSyncState::InitializeTreeFailure(
                                Error::SnapshotParsingFailed,
                            ),
                            ctx,
                        );
                    }
                },
            )
            .abort_handle();

        let _ = self.tree_sync_state.set_sync_abort_handle(abort_handle);
    }

    #[cfg(feature = "local_fs")]
    fn diff_filesystem_with_tree(
        repo_path: PathBuf,
        tree: &MerkleTree,
        max_files_repo_limit: usize,
    ) -> Result<(ChangedFiles, Vec<Gitignore>), Error> {
        let mut gitignores = Self::construct_initial_ignores(&repo_path);

        let mut changed_files = ChangedFiles::default();
        let mut remaining_file_quotas = Some(max_files_repo_limit);
        CodebaseIndex::diff_merkle_node(
            &mut changed_files,
            &tree.root_node(),
            repo_path.clone(),
            &mut gitignores,
            remaining_file_quotas.as_mut(),
            MAX_DEPTH,
            0,
        )?;
        Ok((changed_files, gitignores))
    }

    /// For a given merkle node and path:
    ///   - we assume that the node is either a file or a directory
    ///   - the file / directory is not in gitignore and is not a symlink
    ///   - the file / directory exists in the filesystem
    ///
    /// If the node is a directory, we:
    ///   1. fetch the set of children from the filesystem
    ///   2. compare the set of children from the filesystem with the set of children from the merkle tree into three sets:
    ///    - children only in the merkle tree
    ///    - children only in the filesystem
    ///    - children in both the merkle tree and the filesystem
    ///   3. for each child only in the merkle tree, recurse down delete_merkle_node (e.g. the helper function will return a list of files to delete)
    ///   4. for each child only in the filesystem, recurse down add_merkle_node (e.g. the helper function will return a list of files to add)
    ///   5. for each child in both the merkle tree and the filesystem, recurse down diff_merkle_node (e.g. the helper function will return a list of files to update)
    ///
    /// If the node is a file, we:
    ///  - check if file has been modified
    ///  - if it has been modified, add the file to the list of files to update
    #[cfg(feature = "local_fs")]
    fn diff_merkle_node(
        changed_files: &mut ChangedFiles,
        node: &NodeLens<'_>,
        curr_path: PathBuf,
        gitignores: &mut Vec<Gitignore>,
        mut remaining_file_quota: Option<&mut usize>,
        max_depth: usize,
        current_depth: usize,
    ) -> Result<(), Error> {
        if current_depth > max_depth {
            return Err(Error::DiffMerkleTreeError(MaxDepthExceeded));
        }

        // Sanity check that the node path is the same as the current path
        if node.path() != curr_path {
            return Err(Error::DiffMerkleTreeError(CurrentNodeMismatch(curr_path)));
        }

        let is_dir = curr_path.is_dir();

        match node.node_id() {
            NodeId::File {
                file_size,
                fs_modified_time,
                file_contents_hash,
                ..
            } => {
                // Handle the case where a file has become a directory
                if is_dir {
                    // If current node is a file but current path is a directory, then add the file
                    // to the list of files to delete and recurse down the directory to add any children
                    // to the list of files to add
                    changed_files.deletions.insert(curr_path.clone());
                    CodebaseIndex::add_merkle_node(
                        changed_files,
                        &curr_path,
                        gitignores,
                        remaining_file_quota.as_deref_mut(),
                        max_depth,
                        current_depth, // Pass current depth without incrementing it because we haven't traversed down a level yet.
                    )?;
                    return Ok(());
                }
                if let Some(remaining_file_quota) = remaining_file_quota {
                    if *remaining_file_quota == 0 {
                        return Err(Error::DiffMerkleTreeError(ExceededMaxFileLimit));
                    }

                    *remaining_file_quota -= 1
                }

                // Sanity check that file node has children, and that all children are fragment nodes with no children.
                // If not, then add file to the list of files to upsert.
                let is_valid_file_node = node.children().count() > 0
                    && node.children().all(|child| {
                        if let NodeId::Fragment { .. } = child.node_id() {
                            child.children().next().is_none()
                        } else {
                            false
                        }
                    });
                if !is_valid_file_node {
                    changed_files.upsertions.insert(curr_path.clone());
                    return Ok(());
                }

                // If the current node is a file and the current path is a file, check if it has been modified:
                // 1. check if file sizes match (fast)
                // 2. check if file timestamps match (fast)
                // 3. check if file contents match (slow - only if size and time are the same)
                let file_path = curr_path.clone();

                if let Ok(metadata) = std::fs::metadata(&file_path) {
                    let filesystem_file_size = metadata.len() as usize;
                    if let Ok(filesystem_modified_time) = metadata.modified() {
                        // Convert the SystemTime to DateTime<Utc>
                        let filesystem_modified_time: DateTime<Utc> =
                            filesystem_modified_time.into();

                        // Fast checks first: if size or modification time changed, file is definitely changed
                        if filesystem_file_size != *file_size
                            || filesystem_modified_time != *fs_modified_time
                        {
                            changed_files.upsertions.insert(file_path.clone());
                            return Ok(());
                        }

                        // Size and modification time are the same, now check contents hash (slower)
                        if let Ok(file_contents) = std::fs::read_to_string(&file_path) {
                            let mut hasher = sha2::Sha256::new();
                            hasher.update(file_contents.as_bytes());
                            let current_hash = format!("{:x}", hasher.finalize());
                            if current_hash != *file_contents_hash {
                                changed_files.upsertions.insert(file_path.clone());
                                return Ok(());
                            }
                        } else {
                            // If we can't read the file, consider it changed
                            changed_files.upsertions.insert(file_path.clone());
                            return Ok(());
                        }
                    } else {
                        log::trace!(
                            "Failed to get modified time for file {}",
                            file_path.display()
                        );
                        return Err(Error::FailedToGetMetadata(file_path.clone()));
                    }
                } else {
                    log::trace!("Failed to get metadata for file {}", file_path.display());
                    return Err(Error::FailedToGetMetadata(file_path.clone()));
                }
            }
            NodeId::Directory { .. } => {
                // Handle the case where a directory has become a file
                if !is_dir {
                    // If current node is a directory but current path is a file, then recurse down
                    // the directory to delete any children and add the file to the list of files to add
                    CodebaseIndex::delete_merkle_node(changed_files, node)?;
                    changed_files.upsertions.insert(curr_path.clone());
                    return Ok(());
                }

                // Populate gitignores if there is a .gitignore file in the current directory
                let gitignore_path = curr_path.join(".gitignore");
                if gitignore_path.exists() {
                    let (gitignore, _) = Gitignore::new(gitignore_path);
                    gitignores.push(gitignore);
                }

                let entries = std::fs::read_dir(&curr_path)?;

                // Get the set of children from the filesystem.
                let mut filesystem_children = HashSet::new();
                for entry in entries {
                    match entry.and_then(|entry| dunce::canonicalize(entry.path())) {
                        Ok(child_path) => {
                            // Ignore paths that are excluded by .gitignore, end with .git, or are symlinks.
                            if matches_gitignores(
                                &child_path,
                                is_dir,
                                &*gitignores,
                                false, /* check_ancestors */
                            ) || child_path.ends_with(".git")
                                || child_path.is_symlink()
                            {
                                continue;
                            }

                            // At this point, we know that the child path is a file or directory that
                            // is not in gitignore, not a symlink, and not a .git directory.
                            filesystem_children.insert(child_path);
                        }
                        Err(err) => {
                            log::trace!("Failed to canonicalize path: {err:?}");
                        }
                    }
                }

                // Get the map of children from the merkle tree.
                let mut merkle_tree_children_map = HashMap::new();
                let mut merkle_tree_paths = HashSet::new();
                for child in node.children() {
                    let path = child.path().to_path_buf();
                    merkle_tree_children_map.insert(path.clone(), child);
                    merkle_tree_paths.insert(path);
                }

                // Process deletions (in merkle tree but not in filesystem)
                for path in merkle_tree_paths.difference(&filesystem_children) {
                    if let Some(node) = merkle_tree_children_map.get(path) {
                        CodebaseIndex::delete_merkle_node(changed_files, node)?;
                    }
                }

                // Process additions (in filesystem but not in merkle tree)
                for path in filesystem_children.difference(&merkle_tree_paths) {
                    CodebaseIndex::add_merkle_node(
                        changed_files,
                        path,
                        gitignores,
                        remaining_file_quota.as_deref_mut(),
                        max_depth,
                        current_depth + 1,
                    )?;
                }

                // Get the set of children that are in both the merkle tree and the filesystem.
                let in_both = merkle_tree_paths.intersection(&filesystem_children);
                for path in in_both {
                    if let Some(node) = merkle_tree_children_map.get(path) {
                        // Recursively diff any children that are in both the merkle tree and the filesystem.
                        CodebaseIndex::diff_merkle_node(
                            changed_files,
                            node,
                            path.clone(),
                            gitignores,
                            remaining_file_quota.as_deref_mut(),
                            max_depth,
                            current_depth + 1,
                        )?;
                    }
                }
            }
            NodeId::Fragment { .. } => {
                // We should never see a fragment node in the diffing process.
                return Err(Error::DiffMerkleTreeError(Fragment(curr_path)));
            }
        }

        Ok(())
    }

    /// Returns the list of files that correspond to the deletion of the given merkle node.
    /// 1. if the node is a directory, recurse down to the children of the directory
    /// 2. if the node is a file, add the file to the list of files to delete
    #[cfg(feature = "local_fs")]
    fn delete_merkle_node(
        changed_files: &mut ChangedFiles,
        node: &NodeLens<'_>,
    ) -> Result<(), Error> {
        fn delete_merkle_node_internal(
            changed_files: &mut ChangedFiles,
            node: &NodeLens<'_>,
        ) -> Result<(), Error> {
            match node.node_id() {
                NodeId::Directory { .. } => {
                    for child in node.children() {
                        delete_merkle_node_internal(changed_files, &child)?;
                    }
                }
                NodeId::File { absolute_path, .. } => {
                    changed_files.deletions.insert(absolute_path.to_path_buf());
                }
                _ => {}
            }
            Ok(())
        }
        delete_merkle_node_internal(changed_files, node)
    }

    /// Returns the list of files that correspond to the addition of the given filesystem path.
    /// 1. if the path is a directory, recurse down to the children of the directory
    /// 2. if the path is a file, add the file to the list of files to add
    ///
    /// We maintain a gitignore to make sure we don't add files that are ignored.
    #[cfg(feature = "local_fs")]
    fn add_merkle_node(
        changed_files: &mut ChangedFiles,
        path: &PathBuf,
        gitignores: &mut Vec<Gitignore>,
        remaining_file_quota: Option<&mut usize>,
        max_depth: usize,
        current_depth: usize,
    ) -> Result<(), Error> {
        fn add_merkle_node_internal(
            changed_files: &mut ChangedFiles,
            path: &PathBuf,
            gitignores: &mut Vec<Gitignore>,
            mut remaining_file_quota: Option<&mut usize>,
            max_depth: usize,
            current_depth: usize,
        ) -> Result<(), Error> {
            if current_depth > max_depth {
                return Err(Error::DiffMerkleTreeError(MaxDepthExceeded));
            }

            let is_dir = path.is_dir();

            if is_dir {
                let gitignore_path = path.join(".gitignore");
                if gitignore_path.exists() {
                    let (gitignore, _) = Gitignore::new(gitignore_path);
                    gitignores.push(gitignore);
                }

                let entries = std::fs::read_dir(path)?;
                let mut filesystem_children = HashSet::new();
                for entry in entries {
                    match entry.and_then(|entry| dunce::canonicalize(entry.path())) {
                        Ok(child_path) => {
                            // Ignore paths that are excluded by .gitignore, end with .git, or are symlinks.
                            if matches_gitignores(
                                &child_path,
                                is_dir,
                                &*gitignores,
                                false, /* check_ancestors */
                            ) || child_path.ends_with(".git")
                                || child_path.is_symlink()
                            {
                                continue;
                            }
                            filesystem_children.insert(child_path);
                        }
                        Err(err) => {
                            log::trace!("Failed to canonicalize path: {err:?}");
                        }
                    }
                }
                for child_path in filesystem_children {
                    add_merkle_node_internal(
                        changed_files,
                        &child_path,
                        gitignores,
                        remaining_file_quota.as_deref_mut(),
                        max_depth,
                        current_depth + 1,
                    )?;
                }
            } else if path.is_file() {
                if let Some(remaining_file_quota) = remaining_file_quota {
                    if *remaining_file_quota == 0 {
                        return Err(Error::DiffMerkleTreeError(ExceededMaxFileLimit));
                    }

                    *remaining_file_quota -= 1
                }

                changed_files.upsertions.insert(path.to_path_buf());
            } else {
                return Err(Error::DiffMerkleTreeError(Symlink));
            }
            Ok(())
        }
        add_merkle_node_internal(
            changed_files,
            path,
            gitignores,
            remaining_file_quota,
            max_depth,
            current_depth,
        )
    }

    fn on_modification(&mut self, ctx: &mut ModelContext<Self>) {
        let now = Utc::now();
        self.ts_metadata.last_edited = Some(now);

        // Track the earliest unsynced change if we don't have one yet
        if self.ts_metadata.earliest_unsynced_change.is_none() {
            self.ts_metadata.earliest_unsynced_change = Some(now);
        }

        ctx.emit(CodebaseIndexEvent::IndexMetadataUpdated {
            root_path: self.repo_path.clone(),
            event: WorkspaceMetadataEvent::Modified,
        });
    }

    /// Calculate the duration between the earliest unsynced change and now.
    /// Returns None if there are no unsynced changes.
    fn out_of_sync_delay(&self) -> Option<Duration> {
        self.ts_metadata
            .earliest_unsynced_change
            .and_then(|earliest| Utc::now().signed_duration_since(earliest).to_std().ok())
    }
}

#[derive(Default)]
pub struct ReadFragmentResult {
    pub successfully_read: Vec<Fragment>,
    pub fail_to_read: Vec<ContentHash>,
    pub fail_to_read_path: Vec<PathBuf>,
}

#[cfg(feature = "local_fs")]
pub(super) async fn build_fragments_from_metadata(
    metadatas: impl IntoIterator<Item = (ContentHash, FragmentMetadata)>,
) -> ReadFragmentResult {
    let mut fragments = Vec::new();
    let mut fail_to_read = Vec::new();
    let mut fail_to_read_path = Vec::new();

    // Group fragments by file path
    let mut fragments_by_path: HashMap<_, Vec<_>> = HashMap::new();
    for (content_hash, metadata) in metadatas {
        fragments_by_path
            .entry(metadata.absolute_path)
            .or_default()
            .push((content_hash, metadata.location.byte_range));
    }

    // Process each file and its fragments
    for (file_path, file_fragments) in fragments_by_path {
        let mut has_failed_to_read_fragments = false;
        // Read the file content once
        if let Ok(file_content) = async_fs::read_to_string(&file_path).await {
            // Process all fragments for this file
            for (content_hash, fragment_ranges) in file_fragments {
                let start_idx = fragment_ranges.start.as_usize();
                let end_idx = fragment_ranges.end.as_usize();

                if start_idx <= end_idx
                    && end_idx <= file_content.len()
                    && file_content.is_char_boundary(start_idx)
                    && file_content.is_char_boundary(end_idx)
                {
                    let content = file_content[start_idx..end_idx].to_string();
                    if content.is_empty() {
                        log::trace!(
                            "Fragment for {:?} with range {:?} is empty",
                            file_path.display(),
                            fragment_ranges
                        );
                        fail_to_read.push(content_hash);
                        has_failed_to_read_fragments = true;
                    } else if ContentHash::from_content(&content) != content_hash {
                        log::trace!(
                            "Fragment for {:?} with range {:?} does not match its content hash",
                            file_path.display(),
                            fragment_ranges
                        );
                        fail_to_read.push(content_hash);
                        has_failed_to_read_fragments = true;
                    } else {
                        fragments.push(Fragment {
                            content,
                            content_hash,
                            location: FragmentLocation {
                                absolute_path: file_path.clone(),
                                byte_range: fragment_ranges,
                            },
                        });
                    }
                } else {
                    log::trace!("Invalid byte range {fragment_ranges:?} for file: {file_path:?}");
                    fail_to_read.push(content_hash);
                    has_failed_to_read_fragments = true;
                }
            }
        } else {
            log::trace!("Failed to read file: {file_path:?}");
            fail_to_read.extend(
                file_fragments
                    .into_iter()
                    .map(|(content_hash, _)| content_hash),
            );
            has_failed_to_read_fragments = true;
        }

        if has_failed_to_read_fragments {
            fail_to_read_path.push(file_path);
        }
    }

    ReadFragmentResult {
        successfully_read: fragments,
        fail_to_read,
        fail_to_read_path,
    }
}

#[cfg(not(feature = "local_fs"))]
pub(super) async fn build_fragments_from_metadata(
    _metadatas: impl IntoIterator<Item = (ContentHash, FragmentMetadata)>,
) -> ReadFragmentResult {
    ReadFragmentResult::default()
}

#[cfg(test)]
#[path = "codebase_index_tests.rs"]
mod tests;
