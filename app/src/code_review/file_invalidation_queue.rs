use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use warp_core::sync_queue::{IsTransientError, SyncQueueTaskTrait};

use super::diff_state::{DiffMode, DiffStateError, FileDiffAndContent, LocalDiffStateModel};

pub(crate) struct FileInvalidationTask {
    pub(crate) file: PathBuf,
    pub(crate) repo_path: PathBuf,
    pub(crate) mode: DiffMode,
    pub(crate) merge_base: Option<String>,
}

/// A [`FileInvalidationTask`] failure, paired with the repo-relative path of
/// the file it was invalidating. Unlike the success `Result`, a bare
/// [`DiffStateError`] doesn't identify which file failed; callers need that
/// to clear their own per-file dedup bookkeeping when a task completes.
///
/// This carries the exact `PathBuf` rather than a lossy `String`: the caller
/// removes this same value from its dedup set by equality, and
/// `retrieve_diff_state` rejects non-UTF-8 paths before ever producing a
/// success result, so a lossy `to_string_lossy()` round-trip here would
/// silently fail to match the original entry for such a path, stranding it
/// in the dedup set (and therefore never invalidating that file again) until
/// the next full reload.
#[derive(Debug)]
pub(crate) struct FileInvalidationError {
    pub(crate) path: PathBuf,
    pub(crate) error: DiffStateError,
}

impl fmt::Display for FileInvalidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for FileInvalidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl IsTransientError for FileInvalidationError {
    fn is_transient(&self) -> bool {
        self.error.is_transient()
    }
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
            let path = file.strip_prefix(&repo_path).unwrap_or(&file).to_path_buf();
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
            .map_err(|error| FileInvalidationError {
                path,
                error: DiffStateError::from(error),
            })
        })
    }
}

#[cfg(test)]
#[path = "file_invalidation_queue_tests.rs"]
mod tests;
