use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use warp_core::sync_queue::{IsTransientError, SyncQueueTaskTrait};

use super::diff_state::{DiffMode, FileDiffAndContent, LocalDiffStateModel};

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct FileInvalidationError(#[from] anyhow::Error);
impl FileInvalidationError {
    pub(crate) fn safe_message(&self) -> &'static str {
        let message = format!("{self:#}");

        if message.contains("detected dubious ownership in repository") {
            "git rejected repository ownership"
        } else if message.contains("No such file or directory")
            || message.contains("program not found")
            || message.contains("No developer tools were found")
        {
            "git is unavailable"
        } else if message.contains("git-lfs: command not found") {
            "git lfs is unavailable"
        } else if message.contains("Xcode license agreements") {
            "xcode license is not accepted"
        } else if message.contains("empty string is not a valid pathspec") {
            "invalid empty pathspec"
        } else if message.contains("outside repository") {
            "path is outside repository"
        } else if message.contains("not a git repository") {
            "path is not a git repository"
        } else if message.contains("this operation must be run in a work tree") {
            "repository is not a work tree"
        } else if message.contains("Operation not permitted")
            || message.contains("Permission denied")
        {
            "repository path is not accessible"
        } else if message.contains("non-UTF-8 path") {
            "path is not valid UTF-8"
        } else if message.contains("bad revision") || message.contains("unknown revision") {
            "git revision is unavailable"
        } else if message.contains("bad tree object HEAD") {
            "git head tree is invalid"
        } else if message.contains("Invalid status code") {
            "git status output is invalid"
        } else if message.contains("os error 267") {
            "repository path is invalid"
        } else {
            "unknown file invalidation error"
        }
    }
}

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
    /// The first element is the repo-relative path of the updated file.
    type Result = (String, Option<Arc<FileDiffAndContent>>);
    #[cfg(not(target_arch = "wasm32"))]
    type Fut = Pin<Box<dyn Future<Output = Result<Self::Result, Self::Error>> + Send>>;
    #[cfg(target_arch = "wasm32")]
    type Fut = Pin<Box<dyn Future<Output = Result<Self::Result, Self::Error>>>>;

    fn run(&mut self) -> Self::Fut {
        let repo_path = self.repo_path.clone();
        let file = self.file.clone();
        let mode = self.mode.clone();
        let merge_base = self.merge_base.clone();
        Box::pin(async move {
            // File invalidation runs local git commands against a local repo path,
            // so using LocalDiffStateModel directly is correct — remote repos use a
            // separate mechanism and never go through this queue.
            LocalDiffStateModel::retrieve_diff_state(
                &repo_path,
                &file,
                &mode,
                merge_base.as_deref(),
            )
            .await
            .map_err(FileInvalidationError::from)
        })
    }
}
