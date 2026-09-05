use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use warp_errors::{ErrorExt, register_error};

#[derive(thiserror::Error, Debug)]
pub enum FileSaveError {
    #[error("No file path associated with file when saving file {0:?}")]
    NoFilePath(FileId),
    #[error("IO error when saving file.")]
    IOError {
        #[source]
        error: io::Error,
        path: PathBuf,
    },
    #[error("Remote file operation failed: {0}")]
    RemoteError(String),
    /// The first write of a file that did not exist when it was opened found something already at
    /// the path. Overwriting would discard whatever appeared there in the meantime.
    #[error("{} already exists", .path.display())]
    AlreadyExists { path: PathBuf },
    /// A non-IO failure with a self-describing message (e.g. content could
    /// not be derived for the write).
    #[error("{0}")]
    Other(String),
}

impl ErrorExt for FileSaveError {
    fn is_actionable(&self) -> bool {
        match self {
            FileSaveError::NoFilePath(_) | FileSaveError::Other(_) => true,
            FileSaveError::IOError { .. }
            | FileSaveError::RemoteError(_)
            | FileSaveError::AlreadyExists { .. } => false,
        }
    }
}
register_error!(FileSaveError);

#[derive(thiserror::Error, Debug)]
pub enum FileLoadError {
    /// Nothing exists at the path. Callers that can create the file — an editor opening a buffer,
    /// for instance — treat this as "not written yet" rather than as a failure.
    #[error("File does not exist")]
    DoesNotExist,
    /// Something is at the path but could not be read: no permission, a directory, a dangling
    /// symlink, or a device error. Always a real failure.
    #[error("IO error when loading file.")]
    IOError(#[from] io::Error),
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileId(usize);

impl FileId {
    /// Constructs a new globally-unique file ID.
    #[allow(clippy::new_without_default)]
    pub fn new() -> FileId {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        FileId(raw)
    }
}
