use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use warp_core::errors::{AnyhowErrorExt as _, ErrorExt};

use warp_core::sync_queue::{IsTransientError, SyncQueueTaskTrait};

use super::diff_state::{DiffMode, FileDiffAndContent, LocalDiffStateModel};
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub(crate) enum FileInvalidationErrorKind {
    #[error("git rejected repository ownership")]
    GitRejectedRepositoryOwnership,
    #[error("git is unavailable")]
    GitUnavailable,
    #[error("git lfs is unavailable")]
    GitLfsUnavailable,
    #[error("xcode license is not accepted")]
    XcodeLicenseNotAccepted,
    #[error("invalid empty pathspec")]
    InvalidEmptyPathspec,
    #[error("path is outside repository")]
    PathOutsideRepository,
    #[error("path is not a git repository")]
    NotGitRepository,
    #[error("repository is not a work tree")]
    NotWorkTree,
    #[error("repository path is not accessible")]
    RepositoryPathNotAccessible,
    #[error("path is not valid UTF-8")]
    NonUtf8Path,
    #[error("git revision is unavailable")]
    GitRevisionUnavailable,
    #[error("git head tree is invalid")]
    GitHeadTreeInvalid,
    #[error("git status output is invalid")]
    InvalidGitStatusOutput,
    #[error("repository path is invalid")]
    RepositoryPathInvalid,
    #[error("unknown file invalidation error")]
    Unknown,
}

impl FileInvalidationErrorKind {
    fn from_message(message: &str) -> Self {
        if message.contains("detected dubious ownership in repository") {
            Self::GitRejectedRepositoryOwnership
        } else if message.contains("No such file or directory")
            || message.contains("program not found")
            || message.contains("No developer tools were found")
        {
            Self::GitUnavailable
        } else if message.contains("git-lfs: command not found") {
            Self::GitLfsUnavailable
        } else if message.contains("Xcode license agreements") {
            Self::XcodeLicenseNotAccepted
        } else if message.contains("empty string is not a valid pathspec") {
            Self::InvalidEmptyPathspec
        } else if message.contains("outside repository") {
            Self::PathOutsideRepository
        } else if message.contains("not a git repository") {
            Self::NotGitRepository
        } else if message.contains("this operation must be run in a work tree") {
            Self::NotWorkTree
        } else if message.contains("Operation not permitted")
            || message.contains("Permission denied")
        {
            Self::RepositoryPathNotAccessible
        } else if message.contains("non-UTF-8 path") {
            Self::NonUtf8Path
        } else if message.contains("bad revision") || message.contains("unknown revision") {
            Self::GitRevisionUnavailable
        } else if message.contains("bad tree object HEAD") {
            Self::GitHeadTreeInvalid
        } else if message.contains("Invalid status code") {
            Self::InvalidGitStatusOutput
        } else if message.contains("os error 267") {
            Self::RepositoryPathInvalid
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub struct FileInvalidationError {
    pub(crate) kind: FileInvalidationErrorKind,
    error: anyhow::Error,
}

impl From<anyhow::Error> for FileInvalidationError {
    fn from(error: anyhow::Error) -> Self {
        let message = format!("{error:#}");
        let kind = FileInvalidationErrorKind::from_message(&message);
        Self { kind, error }
    }
}

impl ErrorExt for FileInvalidationError {
    fn is_actionable(&self) -> bool {
        match self.kind {
            FileInvalidationErrorKind::InvalidEmptyPathspec
            | FileInvalidationErrorKind::InvalidGitStatusOutput => true,
            FileInvalidationErrorKind::Unknown => self.error.is_actionable(),
            FileInvalidationErrorKind::GitRejectedRepositoryOwnership
            | FileInvalidationErrorKind::GitUnavailable
            | FileInvalidationErrorKind::GitLfsUnavailable
            | FileInvalidationErrorKind::XcodeLicenseNotAccepted
            | FileInvalidationErrorKind::PathOutsideRepository
            | FileInvalidationErrorKind::NotGitRepository
            | FileInvalidationErrorKind::NotWorkTree
            | FileInvalidationErrorKind::RepositoryPathNotAccessible
            | FileInvalidationErrorKind::NonUtf8Path
            | FileInvalidationErrorKind::GitRevisionUnavailable
            | FileInvalidationErrorKind::GitHeadTreeInvalid
            | FileInvalidationErrorKind::RepositoryPathInvalid => false,
        }
    }
}
warp_core::errors::register_error!(FileInvalidationError);

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
