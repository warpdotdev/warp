//! Repository metadata utilities for Warp.
//!
//! This crate provides utilities for managing repository metadata, including file trees,
//! gitignore processing, and filesystem watching capabilities.s
use std::{
    borrow::Borrow,
    path::{Path, PathBuf},
};

use thiserror::Error;
use warp_util::standardized_path::StandardizedPath;

#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;

/// Errors that can occur when working with repository metadata.
#[derive(Error, Debug)]
pub enum RepoMetadataError {
    #[error("Repository not found: {0}")]
    RepoNotFound(String),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("Path encoding does not match local OS: {0}")]
    PathEncodingMismatch(StandardizedPath),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Build tree error: {0}")]
    BuildTree(BuildTreeError),
    #[error("Failed to start watcher: {0}")]
    WatcherError(#[from] anyhow::Error),
}
// Re-export the modules
pub mod entry;
pub mod file_tree_store;
pub mod file_tree_update;
pub mod local_model;
pub mod remote_model;
pub mod repositories;
pub mod repository;
pub mod repository_identifier;
mod telemetry;
pub mod watcher;
pub mod wrapper_model;

pub use entry::{
    gitignores_for_directory, matches_gitignores, path_passes_filters, should_ignore_git_path,
    BuildTreeError, DirectoryEntry, Entry, FileId, FileMetadata,
};

// Re-export the local model's event under its original name for backward compatibility.
pub use local_model::RepositoryMetadataEvent;

pub use repository::Repository;
pub use watcher::{DirectoryWatcher, RepositoryUpdate, TargetFile};

#[cfg(not(target_family = "wasm"))]
pub fn is_in_repo(path: &str, app: &warpui::AppContext) -> bool {
    use crate::repositories::DetectedRepositories;
    use warp_util::local_or_remote_path::LocalOrRemotePath;

    DetectedRepositories::as_ref(app)
        .get_root_for_path(&LocalOrRemotePath::Local(std::path::PathBuf::from(path)))
        .is_some()
}

#[cfg(target_family = "wasm")]
pub fn is_in_repo(_path: &str, _app: &warpui::AppContext) -> bool {
    false
}
pub use file_tree_store::FileTreeEntry;

pub use local_model::{LocalRepoMetadataModel, RepoContent};

// New types.
pub use file_tree_update::RepoMetadataUpdate;
pub use remote_model::RemoteRepoMetadataModel;
pub use repository_identifier::{RemoteRepositoryIdentifier, RepositoryIdentifier};
pub use wrapper_model::{RepoMetadataEvent, RepoMetadataModel};

/// A wrapper around PathBuf that ensures the path is canonicalized.
/// This helps avoid issues with symbolic links, relative paths, and different path representations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanonicalizedPath(PathBuf);

impl std::fmt::Display for CanonicalizedPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.display().fmt(f)
    }
}

impl CanonicalizedPath {
    pub fn as_path_buf(&self) -> &PathBuf {
        &self.0
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// See [`PathBuf::pop`]
    pub fn pop(&mut self) -> bool {
        self.0.pop()
    }

    pub fn starts_with(&self, path: &CanonicalizedPath) -> bool {
        self.0.starts_with(&path.0)
    }
}

impl TryFrom<PathBuf> for CanonicalizedPath {
    type Error = std::io::Error;

    fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
        let canonical = dunce::canonicalize(&value)?;
        Ok(CanonicalizedPath(canonical))
    }
}

impl TryFrom<&Path> for CanonicalizedPath {
    type Error = std::io::Error;

    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        let canonical = dunce::canonicalize(value)?;
        Ok(CanonicalizedPath(canonical))
    }
}

impl TryFrom<&PathBuf> for CanonicalizedPath {
    type Error = std::io::Error;

    fn try_from(value: &PathBuf) -> Result<Self, Self::Error> {
        let canonical = dunce::canonicalize(value)?;
        Ok(CanonicalizedPath(canonical))
    }
}

impl TryFrom<&str> for CanonicalizedPath {
    type Error = std::io::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let path = PathBuf::from(value);
        let canonical = dunce::canonicalize(&path)?;
        Ok(CanonicalizedPath(canonical))
    }
}

impl From<CanonicalizedPath> for PathBuf {
    fn from(canonical: CanonicalizedPath) -> Self {
        canonical.0
    }
}

impl From<&CanonicalizedPath> for PathBuf {
    fn from(canonical: &CanonicalizedPath) -> Self {
        canonical.0.clone()
    }
}

// We can implement Borrow<Path> and Borrow<PathBuf> because CanonicalizedPath uses identical trait
// implementations to those types (particularly for Hash, Ord, and Eq). If the implementations of
// those traits differ, the Borrow implementations will no longer be sound.

impl Borrow<Path> for CanonicalizedPath {
    fn borrow(&self) -> &Path {
        self.0.borrow()
    }
}

impl Borrow<PathBuf> for CanonicalizedPath {
    fn borrow(&self) -> &PathBuf {
        &self.0
    }
}

impl From<CanonicalizedPath> for StandardizedPath {
    fn from(canonical: CanonicalizedPath) -> Self {
        // CanonicalizedPath is always absolute and local, so try_from_local will not fail.
        StandardizedPath::try_from_local(canonical.as_path())
            .expect("CanonicalizedPath is always absolute")
    }
}
