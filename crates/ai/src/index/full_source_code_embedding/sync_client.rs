use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::mem;
use std::ops::AddAssign;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use itertools::Itertools;
use warp_core::sync_queue::{IsTransientError, SyncQueue, SyncQueueTaskTrait};

use super::changed_files::ChangedFiles;
use super::codebase_index::{build_fragments_from_metadata, SyncProgress};
use super::fragment_metadata::LeafToFragmentMetadataMapping;
use super::merkle_tree::{MerkleTree, NodeLens};
use super::store_client::StoreClient;
use super::{
    CodebaseContextConfig, ContentHash, EmbeddingConfig, Error, Fragment, NodeHash, RepoMetadata,
};
use crate::index::full_source_code_embedding::store_client::IntermediateNode;

const SYNC_NODE_BATCH_SIZE: usize = 500;
// Minimum node batch size used for updates.
const MIN_UPDATE_NODE_BATCH_SIZE: usize = 100;

/// Maximum total raw content bytes per `GenerateCodeEmbeddings` request.
/// Set to 4 MB to stay under the 5 MB Cloud Armor limit after JSON serialization overhead.
const MAX_BATCH_CONTENT_BYTES: usize = 4_000_000;

#[derive(Debug, Clone, Default)]
pub struct FlushFragmentResult {
    pub fragment_count: usize,
    pub total_fragment_size_bytes: usize,
}

impl AddAssign for FlushFragmentResult {
    fn add_assign(&mut self, rhs: Self) {
        self.fragment_count += rhs.fragment_count;
        self.total_fragment_size_bytes += rhs.total_fragment_size_bytes;
    }
}

pub struct GenerateEmbeddingsTask {
    store_client: Arc<dyn StoreClient>,
    embedding_config: EmbeddingConfig,
    fragments: Vec<Fragment>,
    root_node_hash: NodeHash,
    repo_metadata: RepoMetadata,
}

pub struct UpdateIntermediateNodesTask {
    store_client: Arc<dyn StoreClient>,
    embedding_config: EmbeddingConfig,
    nodes: Vec<IntermediateNode>,
}

pub struct SyncMerkleTreeTask {
    store_client: Arc<dyn StoreClient>,
    embedding_config: EmbeddingConfig,
    nodes: Vec<NodeHash>,
}

pub enum SyncTask {
    GenerateEmbeddings(GenerateEmbeddingsTask),
    UpdateIntermediateNodes(UpdateIntermediateNodesTask),
    SyncMerkleTree(SyncMerkleTreeTask),
}

#[derive(Debug)]
pub enum SyncQueueResult {
    GenerateEmbeddings(HashMap<ContentHash, bool>),
    UpdateIntermediateNodes(HashMap<NodeHash, bool>),
    SyncMerkleTree(HashSet<NodeHash>),
}

impl SyncQueueTaskTrait for SyncTask {
    type Error = Error;
    type Result = SyncQueueResult;
    #[cfg(not(target_arch = "wasm32"))]
    type Fut = Pin<Box<dyn Future<Output = Result<Self::Result, Self::Error>> + Send>>;
    #[cfg(target_arch = "wasm32")]
    type Fut = Pin<Box<dyn Future<Output = Result<Self::Result, Self::Error>>>>;
    fn run(&mut self) -> Self::Fut {
        match self {
            SyncTask::GenerateEmbeddings(task) => {
                let store_client = task.store_client.clone();
                let embedding_config = task.embedding_config;
                let fragments = task.fragments.clone();
                let root_node_hash = task.root_node_hash.clone();
                let repo_metadata = task.repo_metadata.clone();
                Box::pin(async move {
                    store_client
                        .generate_embeddings(
                            embedding_config,
                            fragments,
                            root_node_hash,
                            repo_metadata,
                        )
                        .await
                        .map(SyncQueueResult::GenerateEmbeddings)
                })
            }
            SyncTask::SyncMerkleTree(task) => {
                let store_client = task.store_client.clone();
                let embedding_config = task.embedding_config;
                let nodes = task.nodes.clone();
                Box::pin(async move {
                    store_client
                        .sync_merkle_tree(nodes, embedding_config)
                        .await
                        .map(SyncQueueResult::SyncMerkleTree)
                })
            }
            SyncTask::UpdateIntermediateNodes(task) => {
                let store_client = task.store_client.clone();
                let embedding_config = task.embedding_config;
                let nodes = task.nodes.clone();
                Box::pin(async move {
                    store_client
                        .update_intermediate_nodes(embedding_config, nodes)
                        .await
                        .map(SyncQueueResult::UpdateIntermediateNodes)
                })
            }
        }
    }
}

/// A sync client that is used to update the server merkle tree state so it is up-to-date with the client
///
/// A sync is broken down into two steps:
/// 1) We need to walk the tree to find a list of nodes that need to be updated. This could be done by either
///    a full scan of the tree or walking the tree from bottom up with a known changed leaf node.
/// 2) With the list of nodes pending sync known, we could then generate embeddings for the leaf nodes and update
///    intermediate nodes.
pub(super) struct CodebaseIndexSyncOperation<'a> {
    /// A list of dirty nodes that need to be synced with the server. Note that the nodes _MUST_ be ordered from children
    /// to parent as we cannot sync parents that don't have their children synced.
    nodes_pending_sync: Vec<NodeLens<'a>>,
    store_client: Arc<dyn StoreClient>,
    embedding_config: EmbeddingConfig,
    sync_queue: SyncQueue<SyncTask>,
    embedding_generation_batch_size: usize,
}

impl<'a> CodebaseIndexSyncOperation<'a> {
    /// Perform a full sync of the merkle tree with the server. This guarantees we will add all inconsistent nodes
    /// to the nodes_pending_sync list.
    pub async fn full_sync(
        tree: &'a MerkleTree,
        store_client: Arc<dyn StoreClient>,
        sync_queue: SyncQueue<SyncTask>,
        sync_progress_tx: async_channel::Sender<SyncProgress>,
        embedding_generation_batch_size: usize,
    ) -> Result<(Self, CodebaseContextConfig), SyncOperationError> {
        let config = store_client
            .codebase_context_config()
            .await
            .map_err(SyncOperationError::ServerSyncError)?;

        let mut operation = Self {
            nodes_pending_sync: Vec::new(),
            store_client,
            embedding_config: config.embedding_config,
            sync_queue,
            embedding_generation_batch_size: embedding_generation_batch_size
                .max(MIN_UPDATE_NODE_BATCH_SIZE),
        };

        let root_node = tree.root_node();
        let mut nodes_pending_check = vec![root_node];

        loop {
            nodes_pending_check = operation
                .check_if_nodes_synced(&nodes_pending_check, sync_progress_tx.clone())
                .await?;

            if nodes_pending_check.is_empty() {
                break;
            }
        }

        // We need to reverse the node orders here so we go from children -> parent;
        operation.nodes_pending_sync.reverse();
        Ok((operation, config))
    }

    /// Perform an incremental sync for a set of updated nodes.
    /// This is used when we know exactly which nodes were modified through file system events.
    ///
    /// Returns the operation without flushing the nodes. The caller must call flush_nodes_pending_sync.
    pub async fn incremental_sync(
        updated_nodes: Vec<NodeLens<'a>>,
        store_client: Arc<dyn StoreClient>,
        sync_queue: SyncQueue<SyncTask>,
        embedding_config: EmbeddingConfig,
        sync_progress_tx: async_channel::Sender<SyncProgress>,
        embedding_generation_batch_size: usize,
    ) -> Result<Self, SyncOperationError> {
        if updated_nodes.is_empty() {
            log::info!("No nodes to sync incrementally");
        } else {
            log::debug!(
                "Starting incremental sync preparation for {} updated nodes",
                updated_nodes.len()
            );
        }

        let mut operation = Self {
            nodes_pending_sync: Vec::new(),
            store_client,
            embedding_config,
            sync_queue,
            embedding_generation_batch_size: embedding_generation_batch_size
                .max(MIN_UPDATE_NODE_BATCH_SIZE),
        };

        // We need to check if nodes are synced in incremental sync since the to-be-update
        // nodes could already exist on the server (e.g. switching between git branches).
        operation
            .check_if_nodes_synced(&updated_nodes, sync_progress_tx)
            .await?;
        Ok(operation)
    }

    pub async fn flush_nodes_pending_sync(
        mut self,
        repo_metadata: &RepoMetadata,
        root_node_hash: NodeHash,
        mapping_updates: &LeafToFragmentMetadataMapping,
        sync_progress_tx: async_channel::Sender<SyncProgress>,
    ) -> Result<FlushFragmentResult, SyncOperationError> {
        let mut leaves = Vec::new();
        let mut intermediate_nodes = Vec::new();

        for node in mem::take(&mut self.nodes_pending_sync) {
            if node.is_leaf() {
                leaves.push(node);
            } else {
                intermediate_nodes.push(node);
            }
        }

        let mut failed_to_sync_nodes: HashSet<NodeHash> = HashSet::new();
        let mut files_need_resync = ChangedFiles::default();
        let mut total_fragment_count = 0;
        let mut total_fragment_size_bytes = 0;

        let total_nodes_to_sync = leaves.len() + intermediate_nodes.len();
        let mut completed_nodes = 0;

        let leaf_batches = batch_leaves_by_size(
            &leaves,
            mapping_updates,
            self.embedding_generation_batch_size,
            MAX_BATCH_CONTENT_BYTES,
        )
        .map_err(SyncOperationError::Other)?;

        for chunk in &leaf_batches {
            let mut fragment_metadatas = HashMap::new();

            for node in chunk {
                let content_hash = node.content_hash().expect("Node should be leaf");
                let metadatas = mapping_updates
                    .get(content_hash.as_ref())
                    .ok_or(anyhow!("Couldn't find metadata for hash"))?;

                for metadata in metadatas {
                    fragment_metadatas.insert(content_hash.clone(), metadata.clone());
                }
            }

            let fragment_metadata_clone = fragment_metadatas.clone();
            let res = build_fragments_from_metadata(fragment_metadata_clone.into_iter()).await;

            if !res.fail_to_read.is_empty() {
                let failed_node_count = res.fail_to_read.len();
                failed_to_sync_nodes.extend(
                    res.fail_to_read
                        .into_iter()
                        .map(|content_hash| content_hash.into()),
                );
                files_need_resync.add_paths(res.fail_to_read_path).await;
                log::warn!("Failed to read {failed_node_count} fragments from disk");
            }

            // Add retry for embedding generation
            let fragments_to_sync = res.successfully_read;

            // Track fragment metrics
            total_fragment_count += fragments_to_sync.len();
            total_fragment_size_bytes += fragments_to_sync
                .iter()
                .map(|f| f.content.len())
                .sum::<usize>();

            let rx = self
                .sync_queue
                .enqueue_with_result(
                    SyncTask::GenerateEmbeddings(GenerateEmbeddingsTask {
                        store_client: self.store_client.clone(),
                        embedding_config: self.embedding_config,
                        fragments: fragments_to_sync,
                        root_node_hash: root_node_hash.clone(),
                        repo_metadata: repo_metadata.clone(),
                    }),
                    None,
                    "generate_embeddings".to_string(),
                )
                .await;

            let Ok(task_result) = rx.await else {
                return Err(SyncOperationError::Other(anyhow::anyhow!(
                    "Sync queue task cancelled"
                )));
            };

            let res = match task_result.inspect(|res| {
                if let SyncQueueResult::GenerateEmbeddings(res) = res {
                    let failed_fragments = res
                        .iter()
                        .filter_map(|(hash, &success)| {
                            if !success {
                                failed_to_sync_nodes.insert(hash.into());
                                fragment_metadatas.remove(hash)
                            } else {
                                None
                            }
                        })
                        .collect_vec();
                    if !failed_fragments.is_empty() {
                        log::warn!(
                            "Failed to generate embeddings for some hashes:\n{failed_fragments:#?}"
                        );
                    }
                }
            }) {
                Ok(res) => res,
                Err(err) => {
                    log::error!("Failed to generate embeddings: {err:?}");
                    if files_need_resync.is_empty() {
                        return Err(SyncOperationError::ServerSyncError(err));
                    } else {
                        return Err(SyncOperationError::ReadFragmentError(files_need_resync));
                    }
                }
            };

            completed_nodes += chunk.len();
            let _ = sync_progress_tx.try_send(SyncProgress::Syncing {
                completed_nodes: completed_nodes.saturating_sub(failed_to_sync_nodes.len()),
                total_nodes: total_nodes_to_sync,
            });

            log::debug!("Generated embedding for the following nodes: {res:?}");
        }

        for chunk in intermediate_nodes.chunks(self.embedding_generation_batch_size) {
            let nodes_to_sync: Vec<IntermediateNode> = chunk
                .iter()
                .filter_map(|node| {
                    let outdated = node
                        .children()
                        .any(|child| failed_to_sync_nodes.contains(&child.hash()));
                    if outdated {
                        failed_to_sync_nodes.insert(node.hash());
                        None
                    } else {
                        Some(IntermediateNode {
                            hash: node.hash(),
                            children: node.children().map(|child| child.hash()).collect(),
                        })
                    }
                })
                .collect();

            let rx = self
                .sync_queue
                .enqueue_with_result(
                    SyncTask::UpdateIntermediateNodes(UpdateIntermediateNodesTask {
                        store_client: self.store_client.clone(),
                        embedding_config: self.embedding_config,
                        nodes: nodes_to_sync.clone(),
                    }),
                    None,
                    "update intermediate nodes".to_string(),
                )
                .await;

            let Ok(task_result) = rx.await else {
                return Err(SyncOperationError::Other(anyhow::anyhow!(
                    "Sync queue task cancelled"
                )));
            };

            let update_result = match task_result.inspect(|res| {
                if let SyncQueueResult::UpdateIntermediateNodes(res) = res {
                    let failed_nodes = res
                        .iter()
                        .filter_map(|(hash, success)| {
                            if !success {
                                failed_to_sync_nodes.insert(hash.clone());
                                Some(hash.clone())
                            } else {
                                None
                            }
                        })
                        .collect_vec();

                    if !failed_nodes.is_empty() {
                        log::warn!("Failed to sync some intermediate nodes");
                    }
                }
            }) {
                Ok(res) => res,
                Err(err) => {
                    log::error!("Failed to sync intermediate node: {err:?}");
                    if files_need_resync.is_empty() {
                        return Err(SyncOperationError::ServerSyncError(err));
                    } else {
                        return Err(SyncOperationError::ReadFragmentError(files_need_resync));
                    }
                }
            };

            completed_nodes += chunk.len();
            let _ = sync_progress_tx.try_send(SyncProgress::Syncing {
                completed_nodes: completed_nodes.saturating_sub(failed_to_sync_nodes.len()),
                total_nodes: total_nodes_to_sync,
            });

            log::debug!("Updated the following nodes: {update_result:?}");
        }

        if !failed_to_sync_nodes.is_empty() {
            log::warn!(
                "Failed to sync {} nodes to the server for root {}",
                failed_to_sync_nodes.len(),
                root_node_hash
            );

            if files_need_resync.is_empty() {
                return Err(SyncOperationError::ServerSyncError(Error::Other(
                    anyhow::anyhow!("Failed to sync some nodes to the server"),
                )));
            } else {
                return Err(SyncOperationError::ReadFragmentError(files_need_resync));
            }
        }

        log::info!("Successfully flushed nodes pending sync for root {root_node_hash}");
        Ok(FlushFragmentResult {
            fragment_count: total_fragment_count,
            total_fragment_size_bytes,
        })
    }

    pub fn total_pending_nodes_count(&self) -> usize {
        self.nodes_pending_sync.len()
    }

    async fn check_if_nodes_synced(
        &mut self,
        nodes_to_check: &[NodeLens<'a>],
        sync_progress_tx: async_channel::Sender<SyncProgress>,
    ) -> Result<Vec<NodeLens<'a>>, SyncOperationError> {
        let chunks = nodes_to_check.chunks(SYNC_NODE_BATCH_SIZE);
        let mut res = Vec::new();

        for chunk in chunks {
            let mut node_hashes = Vec::new();

            for node in chunk {
                node_hashes.push(node.hash());
            }

            let rx = self
                .sync_queue
                .enqueue_with_result(
                    SyncTask::SyncMerkleTree(SyncMerkleTreeTask {
                        store_client: self.store_client.clone(),
                        embedding_config: self.embedding_config,
                        nodes: node_hashes.clone(),
                    }),
                    None,
                    "update intermediate nodes".to_string(),
                )
                .await;

            let nodes_need_sync = match rx.await {
                Ok(Ok(SyncQueueResult::SyncMerkleTree(res))) => res,
                Ok(Ok(_)) => {
                    return Err(SyncOperationError::Other(anyhow::anyhow!(
                        "Shouldn't receive other task result in channel"
                    )))
                }
                Ok(Err(e)) => return Err(SyncOperationError::ServerSyncError(e)),
                Err(_) => {
                    return Err(SyncOperationError::Other(anyhow::anyhow!(
                        "Sync queue task cancelled"
                    )))
                }
            };

            // Iterate over the nodes that need to be synced and add their children to the next queue.
            for node in chunk {
                if nodes_need_sync.contains(&node.hash()) {
                    self.nodes_pending_sync.push(*node);
                    res.extend(node.children());
                }
            }
        }

        let _ = sync_progress_tx.try_send(SyncProgress::Discovering {
            total_nodes: self.nodes_pending_sync.len(),
        });

        Ok(res)
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum SyncOperationError {
    #[error("Error reading some fragments {0:#?}")]
    ReadFragmentError(ChangedFiles),
    #[error("Error syncing nodes with server {0:#}")]
    ServerSyncError(Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Partitions `leaves` into batches where each batch contains at most `max_count` leaves
/// AND at most `max_bytes` of estimated content.
///
/// Content size for each leaf is estimated from the `byte_range` in its `FragmentMetadata`,
/// which is available without reading from disk.
fn batch_leaves_by_size<'a>(
    leaves: &[NodeLens<'a>],
    mapping_updates: &LeafToFragmentMetadataMapping,
    max_count: usize,
    max_bytes: usize,
) -> Result<Vec<Vec<NodeLens<'a>>>> {
    if leaves.is_empty() {
        return Ok(vec![]);
    }

    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut current_bytes: usize = 0;

    for leaf in leaves {
        let content_hash = leaf.content_hash().expect("Node should be leaf");
        let metadatas = mapping_updates
            .get(content_hash.as_ref())
            .ok_or_else(|| anyhow!("Couldn't find metadata for hash {content_hash:?}"))?;

        let leaf_bytes: usize = metadatas.iter().map(|m| m.content_byte_size()).sum();

        // If the current batch is non-empty and adding this leaf would exceed either limit,
        // finalize the current batch and start a new one.
        if !current_batch.is_empty()
            && (current_batch.len() >= max_count || current_bytes + leaf_bytes > max_bytes)
        {
            batches.push(std::mem::take(&mut current_batch));
            current_bytes = 0;
        }

        current_batch.push(*leaf);
        current_bytes += leaf_bytes;
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    Ok(batches)
}

impl IsTransientError for Error {
    fn is_transient(&self) -> bool {
        // TODO: match on the error type and only return true of actual transient error.
        true
    }
}

impl From<SyncOperationError> for Error {
    fn from(error: SyncOperationError) -> Self {
        match error {
            SyncOperationError::Other(e) => Self::Other(e),
            SyncOperationError::ServerSyncError(e) => e,
            SyncOperationError::ReadFragmentError(_) => Self::FileSystemStateChanged,
        }
    }
}

#[cfg(test)]
#[path = "sync_client_tests.rs"]
mod tests;
