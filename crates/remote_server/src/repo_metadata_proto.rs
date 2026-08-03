//! Conversion between `repo_metadata` Rust types and proto-generated types.
//!
//! The Rust types in `repo_metadata::file_tree_update` were designed to mirror the
//! proto schema 1:1, so these conversions are straightforward field mappings.

use repo_metadata::file_tree_store::{FileTreeEntry, FileTreeEntryState};
use repo_metadata::file_tree_update::{
    DirectoryNodeMetadata, FileNodeMetadata, FileTreeEntryUpdate, RepoMetadataUpdate,
    RepoNodeMetadata,
};
use repo_metadata::{StandingQueryContent, StandingQueryResultsDelta};
use warp_util::standardized_path::StandardizedPath;

use crate::proto;

// ── Rust → Proto ────────────────────────────────────────────

impl From<&RepoMetadataUpdate> for proto::RepoMetadataUpdatePush {
    fn from(update: &RepoMetadataUpdate) -> Self {
        Self {
            repo_path: update.repo_path.to_string(),
            remove_entries: update
                .remove_entries
                .iter()
                .map(|p| p.to_string())
                .collect(),
            update_entries: update
                .update_entries
                .iter()
                .map(proto::RepoMetadataEntryUpdate::from)
                .collect(),
            standing_results_delta: Some((&update.standing_results_delta).into()),
        }
    }
}

#[cfg(test)]
#[path = "repo_metadata_proto_tests.rs"]
mod tests;

impl From<&StandingQueryContent> for proto::StandingQueryContent {
    fn from(content: &StandingQueryContent) -> Self {
        Self {
            path: content.path.to_string(),
            is_directory: content.is_directory,
        }
    }
}

impl From<&StandingQueryResultsDelta> for proto::StandingQueryResultsDelta {
    fn from(delta: &StandingQueryResultsDelta) -> Self {
        Self {
            upserted_project_skills: delta
                .upserted_project_skills
                .iter()
                .map(proto::StandingQueryContent::from)
                .collect(),
            removed_project_skills: delta
                .removed_project_skills
                .iter()
                .map(proto::StandingQueryContent::from)
                .collect(),
            upserted_project_rules: delta
                .upserted_project_rules
                .iter()
                .map(proto::StandingQueryContent::from)
                .collect(),
            removed_project_rules: delta
                .removed_project_rules
                .iter()
                .map(proto::StandingQueryContent::from)
                .collect(),
        }
    }
}

impl From<&FileTreeEntryUpdate> for proto::RepoMetadataEntryUpdate {
    fn from(update: &FileTreeEntryUpdate) -> Self {
        Self {
            parent_path_to_replace: update.parent_path_to_replace.to_string(),
            subtree_metadata: update
                .subtree_metadata
                .iter()
                .map(proto::RepoNodeMetadata::from)
                .collect(),
        }
    }
}

impl From<&RepoNodeMetadata> for proto::RepoNodeMetadata {
    fn from(node: &RepoNodeMetadata) -> Self {
        let node_oneof = match node {
            RepoNodeMetadata::Directory(dir) => {
                proto::repo_node_metadata::Node::Directory(proto::DirectoryNodeMetadata {
                    path: dir.path.to_string(),
                    ignored: dir.ignored,
                    loaded: dir.loaded,
                })
            }
            RepoNodeMetadata::File(file) => {
                proto::repo_node_metadata::Node::File(proto::FileNodeMetadata {
                    path: file.path.to_string(),
                    extension: file.extension.clone(),
                    ignored: file.ignored,
                })
            }
        };
        Self {
            node: Some(node_oneof),
        }
    }
}

/// Serializes a full `FileTreeEntry`
/// messages suitable for a `RepoMetadataSnapshot`.
///
/// Walks the tree breadth-first from the root, producing one `RepoMetadataEntryUpdate`
/// per parent directory containing its immediate children as `RepoNodeMetadata`.
pub fn file_tree_entry_to_snapshot_proto(
    entry: &FileTreeEntry,
) -> Vec<proto::RepoMetadataEntryUpdate> {
    let mut result = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(entry.root_directory().clone());

    while let Some(current_path) = queue.pop_front() {
        let children: Vec<_> = entry.child_paths(&current_path).cloned().collect();
        if children.is_empty() {
            continue;
        }

        let mut subtree_metadata = Vec::with_capacity(children.len());
        for child_path in &children {
            match entry.get(child_path) {
                Some(FileTreeEntryState::Directory(dir)) => {
                    subtree_metadata.push(proto::RepoNodeMetadata {
                        node: Some(proto::repo_node_metadata::Node::Directory(
                            proto::DirectoryNodeMetadata {
                                path: dir.path.to_string(),
                                ignored: dir.ignored,
                                loaded: dir.loaded,
                            },
                        )),
                    });
                    // Enqueue for breadth-first traversal.
                    queue.push_back(child_path.clone());
                }
                Some(FileTreeEntryState::File(file)) => {
                    subtree_metadata.push(proto::RepoNodeMetadata {
                        node: Some(proto::repo_node_metadata::Node::File(
                            proto::FileNodeMetadata {
                                path: file.path.to_string(),
                                extension: file.extension.clone(),
                                ignored: file.ignored,
                            },
                        )),
                    });
                }
                None => {}
            }
        }

        if !subtree_metadata.is_empty() {
            result.push(proto::RepoMetadataEntryUpdate {
                parent_path_to_replace: current_path.to_string(),
                subtree_metadata,
            });
        }
    }

    result
}

// ── Proto → Rust ──────────────────────────────────────────────────

/// Converts a `RepoMetadataUpdatePush` proto message into a `RepoMetadataUpdate`.
pub fn proto_to_repo_metadata_update(
    push: &proto::RepoMetadataUpdatePush,
) -> Option<RepoMetadataUpdate> {
    let repo_path = StandardizedPath::try_new(&push.repo_path).ok()?;

    let remove_entries: Vec<StandardizedPath> = push
        .remove_entries
        .iter()
        .filter_map(|p| match StandardizedPath::try_new(p) {
            Ok(path) => Some(path),
            Err(e) => {
                log::warn!("Skipping invalid remove_entry path {p:?}: {e}");
                None
            }
        })
        .collect();

    let update_entries: Vec<FileTreeEntryUpdate> = push
        .update_entries
        .iter()
        .filter_map(proto_to_entry_update)
        .collect();

    Some(RepoMetadataUpdate {
        repo_path,
        remove_entries,
        update_entries,
        standing_results_delta: push
            .standing_results_delta
            .as_ref()
            .map(proto_to_standing_results_delta)
            .unwrap_or_default(),
    })
}

fn proto_to_standing_query_content(
    content: &proto::StandingQueryContent,
) -> Option<StandingQueryContent> {
    Some(StandingQueryContent {
        path: StandardizedPath::try_new(&content.path).ok()?,
        is_directory: content.is_directory,
    })
}

fn proto_to_standing_results_delta(
    delta: &proto::StandingQueryResultsDelta,
) -> StandingQueryResultsDelta {
    StandingQueryResultsDelta {
        upserted_project_skills: delta
            .upserted_project_skills
            .iter()
            .filter_map(proto_to_standing_query_content)
            .collect(),
        removed_project_skills: delta
            .removed_project_skills
            .iter()
            .filter_map(proto_to_standing_query_content)
            .collect(),
        upserted_project_rules: delta
            .upserted_project_rules
            .iter()
            .filter_map(proto_to_standing_query_content)
            .collect(),
        removed_project_rules: delta
            .removed_project_rules
            .iter()
            .filter_map(proto_to_standing_query_content)
            .collect(),
    }
}

/// Converts a `RepoMetadataSnapshot` proto into a `RepoMetadataUpdate`
/// (with no removals) that can be applied to a `RemoteRepoMetadataModel`.
pub fn proto_snapshot_to_update(
    snapshot: &proto::RepoMetadataSnapshot,
) -> Option<RepoMetadataUpdate> {
    let repo_path = StandardizedPath::try_new(&snapshot.repo_path).ok()?;

    let update_entries: Vec<FileTreeEntryUpdate> = snapshot
        .entries
        .iter()
        .filter_map(proto_to_entry_update)
        .collect();

    Some(RepoMetadataUpdate {
        repo_path,
        remove_entries: Vec::new(),
        update_entries,
        standing_results_delta: snapshot
            .standing_results
            .as_ref()
            .map(proto_to_standing_results_delta)
            .unwrap_or_default(),
    })
}

fn proto_to_entry_update(
    proto_update: &proto::RepoMetadataEntryUpdate,
) -> Option<FileTreeEntryUpdate> {
    let parent_path = StandardizedPath::try_new(&proto_update.parent_path_to_replace).ok()?;

    let subtree_metadata: Vec<RepoNodeMetadata> = proto_update
        .subtree_metadata
        .iter()
        .filter_map(proto_to_repo_node_metadata)
        .collect();

    Some(FileTreeEntryUpdate {
        parent_path_to_replace: parent_path,
        subtree_metadata,
    })
}

/// Converts a `LoadRepoMetadataDirectoryResponse` proto into a `RepoMetadataUpdate`
/// (with no removals) that can be applied to a `RemoteRepoMetadataModel`.
pub fn proto_load_repo_metadata_directory_response_to_update(
    resp: &proto::LoadRepoMetadataDirectoryResponse,
) -> Option<RepoMetadataUpdate> {
    let repo_path = StandardizedPath::try_new(&resp.repo_path).ok()?;

    let update_entries: Vec<FileTreeEntryUpdate> = resp
        .entries
        .iter()
        .filter_map(proto_to_entry_update)
        .collect();

    Some(RepoMetadataUpdate {
        repo_path,
        remove_entries: Vec::new(),
        update_entries,
        standing_results_delta: StandingQueryResultsDelta::default(),
    })
}

/// Serializes the immediate children of a directory in a `FileTreeEntry` as
/// `RepoMetadataEntryUpdate` protos. Used to build a `LoadRepoMetadataDirectoryResponse`.
pub fn file_tree_children_to_proto_entries(
    entry: &FileTreeEntry,
    dir_path: &StandardizedPath,
) -> Vec<proto::RepoMetadataEntryUpdate> {
    let children: Vec<_> = entry.child_paths(dir_path).cloned().collect();
    if children.is_empty() {
        return Vec::new();
    }

    let mut subtree_metadata = Vec::with_capacity(children.len());
    for child_path in &children {
        match entry.get(child_path) {
            Some(FileTreeEntryState::Directory(dir)) => {
                subtree_metadata.push(proto::RepoNodeMetadata {
                    node: Some(proto::repo_node_metadata::Node::Directory(
                        proto::DirectoryNodeMetadata {
                            path: dir.path.to_string(),
                            ignored: dir.ignored,
                            loaded: dir.loaded,
                        },
                    )),
                });
            }
            Some(FileTreeEntryState::File(file)) => {
                subtree_metadata.push(proto::RepoNodeMetadata {
                    node: Some(proto::repo_node_metadata::Node::File(
                        proto::FileNodeMetadata {
                            path: file.path.to_string(),
                            extension: file.extension.clone(),
                            ignored: file.ignored,
                        },
                    )),
                });
            }
            None => {}
        }
    }

    if subtree_metadata.is_empty() {
        return Vec::new();
    }

    vec![proto::RepoMetadataEntryUpdate {
        parent_path_to_replace: dir_path.to_string(),
        subtree_metadata,
    }]
}

fn proto_to_repo_node_metadata(proto_node: &proto::RepoNodeMetadata) -> Option<RepoNodeMetadata> {
    match proto_node.node.as_ref()? {
        proto::repo_node_metadata::Node::Directory(dir) => {
            let path = StandardizedPath::try_new(&dir.path).ok()?;
            Some(RepoNodeMetadata::Directory(DirectoryNodeMetadata {
                path,
                ignored: dir.ignored,
                loaded: dir.loaded,
            }))
        }
        proto::repo_node_metadata::Node::File(file) => {
            let path = StandardizedPath::try_new(&file.path).ok()?;
            Some(RepoNodeMetadata::File(FileNodeMetadata {
                path,
                extension: file.extension.clone(),
                ignored: file.ignored,
            }))
        }
    }
}
