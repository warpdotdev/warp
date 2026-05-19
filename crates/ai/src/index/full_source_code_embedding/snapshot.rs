use chrono::Utc;
#[cfg(feature = "local_fs")]
use repo_metadata::Repository;
use std::{
    collections::HashSet,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    time::Duration,
};
#[cfg(feature = "local_fs")]
use warpui::ModelHandle;

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use super::Error as CodebaseIndexError;
        use std::sync::Arc;
        use warpui::ModelContext;
        use anyhow::Context;
        use warp_core::safe_info;
        use super::{store_client::StoreClient, CodebaseIndex, EmbeddingConfig};
    }
}

use crate::workspace::WorkspaceMetadata;

/// Number of days after which an index snapshot should be considered expired.
pub(super) const REPO_SNAPSHOT_SHELF_LIFE_DAYS: u64 = 30;

/// The maximum lifetime of an index snapshot file, after which it
/// should be considered expired and deleted.
const REPO_SNAPSHOT_SHELF_LIFE_DURATION: Duration =
    Duration::from_secs(60 * 60 * 24 * REPO_SNAPSHOT_SHELF_LIFE_DAYS);

/// Subdirectory inside the app's statedirectory that holds snapshot files.
const REPO_SNAPSHOT_SUBDIR_NAME: &str = "codebase_index_snapshots";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotStorage {
    dir: PathBuf,
}

impl SnapshotStorage {
    /// Construct snapshot storage using the app's default secure snapshot directory.
    pub fn app_default() -> Option<Self> {
        snapshot_dir().map(|dir| Self { dir })
    }

    /// Construct snapshot storage rooted at the supplied directory, creating it if needed.
    pub fn from_dir(dir: PathBuf) -> Option<Self> {
        if !dir.is_dir() {
            std::fs::create_dir_all(&dir).ok()?;
        }
        Some(Self { dir })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
    #[cfg(feature = "local_fs")]
    pub(super) fn is_app_default(&self) -> bool {
        self.dir == default_snapshot_dir_path()
    }

    pub(super) fn has_snapshot(&self, repo_path: &Path) -> bool {
        self.snapshot_path(repo_path).is_file()
    }

    pub(super) fn snapshot_path(&self, repo_path: &Path) -> PathBuf {
        snapshot_path(&self.dir, repo_path)
    }
}

/// Splits a list of codebase indices into invalid and valid indices,
/// based on their last write date and whether they have a corresponding snapshot file.
pub(super) fn split_snapshot_metadata_by_validity(
    persisted_codebase_indices: Vec<WorkspaceMetadata>,
    snapshot_storage: Option<&SnapshotStorage>,
) -> (Vec<WorkspaceMetadata>, Vec<WorkspaceMetadata>) {
    let now = Utc::now();
    persisted_codebase_indices
        .into_iter()
        .partition(|index_metadata| {
            log::info!(
                "Discarding expired codebase index snapshot for {:?}",
                index_metadata.path
            );
            index_metadata.is_expired(now, REPO_SNAPSHOT_SHELF_LIFE_DAYS)
                || !snapshot_storage
                    .is_some_and(|storage| storage.has_snapshot(&index_metadata.path))
        })
}

/// Delete snapshot files that are missing metadata or have expired from the snapshot directory.
pub(super) fn clean_up_snapshot_files(
    snapshot_file_dir: &Path,
    persisted_codebase_indices: &[WorkspaceMetadata],
) {
    let expected_snapshot_filenames: HashSet<_> = persisted_codebase_indices
        .iter()
        .map(|index_metadata| snapshot_path(snapshot_file_dir, &index_metadata.path))
        .collect();

    if let Ok(fs_entries) = std::fs::read_dir(snapshot_file_dir) {
        let fs_now = std::time::SystemTime::now();
        for fs_entry in fs_entries.flatten() {
            let path = fs_entry.path();

            // Check if this is a regular file with a snapshot_ prefix
            if let Ok(fs_metadata) = fs_entry.metadata() {
                if fs_metadata.is_file() {
                    maybe_clean_up_snapshot_file(
                        &path,
                        &fs_metadata,
                        &fs_now,
                        &expected_snapshot_filenames,
                    );
                }
            }
        }
    }
}

fn maybe_clean_up_snapshot_file(
    path: &Path,
    fs_metadata: &std::fs::Metadata,
    fs_now: &std::time::SystemTime,
    expected_snapshot_filenames: &HashSet<PathBuf>,
) {
    if let Some(filename) = path.file_name() {
        if filename.to_string_lossy().starts_with("snapshot_") {
            let mut should_remove = false;

            // Check if file itself is expired
            if let Ok(modified_time) = fs_metadata.modified() {
                if let Ok(age) = fs_now.duration_since(modified_time) {
                    if age >= REPO_SNAPSHOT_SHELF_LIFE_DURATION {
                        should_remove = true;
                    }
                }
            }

            // Check if file is not in alive_files set
            if !expected_snapshot_filenames.contains(path) {
                should_remove = true;
            }

            // Remove file if either condition is true
            if should_remove {
                if let Err(e) = std::fs::remove_file(path) {
                    log::warn!("Failed to remove stale snapshot file {path:?}: {e}");
                }
            }
        }
    }
}

#[cfg(feature = "local_fs")]
pub(super) fn read_snapshot(
    store_client: Arc<dyn StoreClient>,
    snapshot_dir: &Path,
    repository: ModelHandle<Repository>,
    max_files_repo_limit: usize,
    embedding_generation_batch_size: usize,
    ctx: &mut ModelContext<CodebaseIndex>,
) -> anyhow::Result<CodebaseIndex> {
    let repo_path_buf = repository.as_ref(ctx).root_dir().to_local_path_lossy();
    let repo_path = repo_path_buf.as_path();
    let snapshot_path = snapshot_path(snapshot_dir, repo_path);
    let snapshot_bytes = std::fs::read(&snapshot_path)?;

    let result = CodebaseIndex::new_from_snapshot(
        repository,
        store_client.clone(),
        EmbeddingConfig::default(),
        snapshot_bytes,
        max_files_repo_limit,
        embedding_generation_batch_size,
        ctx,
    );

    // If rebuilding merkle tree from snapshot fails due to parsing error, delete the snapshot file.
    if let Err(CodebaseIndexError::SnapshotParsingFailed) = result {
        log::info!(
            "Deleting invalid snapshot {:?} for repo",
            snapshot_path.display(),
        );
        if let Err(e) = std::fs::remove_file(&snapshot_path) {
            log::warn!(
                "Failed to remove invalid snapshot file {:?}: {}",
                snapshot_path.display(),
                e
            );
        }
    }

    Ok(result?)
}

pub(super) fn has_snapshot(repo_path: &Path) -> bool {
    SnapshotStorage::app_default().is_some_and(|storage| storage.has_snapshot(repo_path))
}

/// Construct a directory to store index snapshots, if it doesn't already exist,
/// and return its path.
pub(super) fn snapshot_dir() -> Option<PathBuf> {
    #[cfg(not(feature = "local_fs"))]
    return None;

    #[cfg(feature = "local_fs")]
    {
        let snapshot_dir_path = default_snapshot_dir_path();

        if !snapshot_dir_path.is_dir() {
            std::fs::create_dir_all(&snapshot_dir_path).ok()?;
        }
        Some(snapshot_dir_path)
    }
}

#[cfg(feature = "local_fs")]
fn default_snapshot_dir_path() -> PathBuf {
    warp_core::paths::secure_state_dir()
        .unwrap_or_else(warp_core::paths::state_dir)
        .join(REPO_SNAPSHOT_SUBDIR_NAME)
}
/// Constructs a snapshot path given a base directory and the codebase index's root path.
pub(super) fn snapshot_path(snapshot_dir: &Path, repo_path: &Path) -> PathBuf {
    // Use a hash of the repo_path to create a unique filename
    let mut hasher = DefaultHasher::new();
    repo_path.hash(&mut hasher);
    let snapshot_file_name = format!("snapshot_{}", hasher.finish());
    snapshot_dir.join(snapshot_file_name)
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;

#[cfg(feature = "local_fs")]
pub(super) fn migrate_snapshots_to_secure_dir_if_needed() -> anyhow::Result<()> {
    // Only perform migration if a secure state directory is available.
    let Some(secure_base) = warp_core::paths::secure_state_dir() else {
        return Ok(());
    };

    let new_dir = secure_base.join(REPO_SNAPSHOT_SUBDIR_NAME);
    let old_dir = warp_core::paths::state_dir().join(REPO_SNAPSHOT_SUBDIR_NAME);

    if new_dir == old_dir {
        return Ok(());
    }

    if old_dir.exists() && !new_dir.exists() {
        if let Some(parent) = new_dir.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create application data directory")?;
        }
        std::fs::rename(&old_dir, &new_dir)
            .context("Failed to migrate codebase index snapshots")?;
        safe_info!(
            safe: ("Migrated codebase index snapshots into secure application container"),
            full: ("Migrated codebase index snapshots from `{}` to `{}`", old_dir.display(), new_dir.display())
        );
    }

    Ok(())
}
