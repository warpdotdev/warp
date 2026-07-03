use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use warp_core::sync_queue::{IsTransientError, SyncQueueTaskTrait};

use super::diff_state::{DiffMode, DiffStateModel, FileDiffAndContent};

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct FileInvalidationError(#[from] anyhow::Error);

impl IsTransientError for FileInvalidationError {
    fn is_transient(&self) -> bool {
        true
    }
}

pub struct FileInvalidationTask {
    pub file: PathBuf,
    pub repo_path: PathBuf,
    pub mode: DiffMode,
    pub merge_base: Option<String>,
}

impl SyncQueueTaskTrait for FileInvalidationTask {
    type Error = FileInvalidationError;
    type Result = (PathBuf, Option<FileDiffAndContent>);
    type Fut = Pin<Box<dyn Future<Output = Result<Self::Result, Self::Error>> + Send>>;

    fn run(&mut self) -> Self::Fut {
        let repo_path = self.repo_path.clone();
        let file = self.file.clone();
        let mode = self.mode.clone();
        let merge_base = self.merge_base.clone();
        Box::pin(async move {
            DiffStateModel::retrieve_diff_state(&repo_path, &file, &mode, merge_base.as_deref())
                .await
                .map_err(FileInvalidationError::from)
        })
    }
}
