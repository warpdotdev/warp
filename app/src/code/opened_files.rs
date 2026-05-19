//! Module containing the definition of [`OpenedFilesModel`],
//! which tracks files that have been opened, organized by repository.

use std::{collections::HashMap, path::PathBuf};

use instant::Instant;
use warpui::{Entity, ModelContext, SingletonEntity};

#[derive(Default, Clone)]
pub struct OpenedFilesInRepo(HashMap<PathBuf, Instant>);

impl OpenedFilesInRepo {
    pub fn get(&self, file_path: &PathBuf) -> Option<&Instant> {
        self.0.get(file_path)
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &Instant)> {
        self.0.iter()
    }
}

/// Model that tracks files that have been opened, organized by repository.
/// Maps repository paths to files and when they were last opened.
#[derive(Default)]
pub struct OpenedFilesModel {
    opened_files: HashMap<PathBuf, OpenedFilesInRepo>,
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
    pub fn opened_files_for_repo(&self, repo_path: &PathBuf) -> Option<&OpenedFilesInRepo> {
        self.opened_files.get(repo_path)
    }

    /// Record that a file has been opened in a repository. If the `file_path` is not within the `repo_path`,
    /// then the file is not recorded.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn file_opened(
        &mut self,
        repo_path: PathBuf,
        file_path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) {
        let opened_at = Instant::now();

        // Convert absolute file path to relative path from repo root
        let Ok(relative_file_path) = file_path.strip_prefix(&repo_path) else {
            return;
        };

        self.opened_files
            .entry(repo_path.clone())
            .or_default()
            .0
            .insert(relative_file_path.into(), opened_at);

        ctx.notify();
    }
}
