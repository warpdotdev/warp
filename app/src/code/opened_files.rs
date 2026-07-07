//! Module containing the definition of [`OpenedFilesModel`],
//! which tracks files that have been opened, organized by repository.

use std::collections::HashMap;

use instant::Instant;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{Entity, ModelContext, SingletonEntity};

/// Tracks opened files within a single repository.
/// Keys are repo-relative file paths (e.g. `src/main.rs`).
#[derive(Default, Clone)]
pub struct OpenedFilesInRepo(HashMap<String, Instant>);

impl OpenedFilesInRepo {
    pub fn get(&self, relative_path: &str) -> Option<&Instant> {
        self.0.get(relative_path)
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Instant)> {
        self.0.iter()
    }
}

/// Model that tracks files that have been opened, organized by repository.
/// Maps repository root locations (local or remote) to their opened files.
#[derive(Default)]
pub struct OpenedFilesModel {
    opened_files: HashMap<LocalOrRemotePath, OpenedFilesInRepo>,
}

impl Entity for OpenedFilesModel {
    type Event = ();
}

impl SingletonEntity for OpenedFilesModel {}

impl OpenedFilesModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all opened files for a specific repository.
    pub fn opened_files_for_repo(
        &self,
        repo_root: &LocalOrRemotePath,
    ) -> Option<&OpenedFilesInRepo> {
        self.opened_files.get(repo_root)
    }

    /// Record that a file has been opened in a repository.
    ///
    /// `repo_root` is the repository root location (local or remote).
    /// `file_location` is the absolute file location. If it is not within
    /// `repo_root`, the file is not recorded.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn file_opened(
        &mut self,
        repo_root: LocalOrRemotePath,
        file_location: &LocalOrRemotePath,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(relative_path) = repo_root.strip_repo_prefix(file_location) else {
            return;
        };

        let opened_at = Instant::now();
        self.opened_files
            .entry(repo_root)
            .or_default()
            .0
            .insert(relative_path, opened_at);

        ctx.notify();
    }
}
