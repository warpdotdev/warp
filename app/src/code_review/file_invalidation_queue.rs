use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use warp_core::sync_queue::SyncQueueTaskTrait;

use super::diff_state::{DiffMode, DiffStateError, FileDiffAndContent, LocalDiffStateModel};

pub(crate) struct FileInvalidationTask {
    pub(crate) file: PathBuf,
    pub(crate) repo_path: PathBuf,
    pub(crate) mode: DiffMode,
    pub(crate) merge_base: Option<String>,
}

impl SyncQueueTaskTrait for FileInvalidationTask {
    type Error = DiffStateError;
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
            .map_err(DiffStateError::from)
        })
    }
}
