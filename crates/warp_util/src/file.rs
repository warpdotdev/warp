use std::{
    io,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

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
}

/// Maximum file size (in bytes) that the editor will load into a buffer.
/// Files larger than this are rejected with `FileLoadError::FileTooLarge` to
/// prevent multi-gigabyte `SumTree<BufferText>` allocations, `StyledBufferBlock`
/// cloning, and unbounded tree-sitter parsing memory.
///
/// 20 MB is consistent with the limit used by `count_lines_if_text_file` in the
/// git utilities and is large enough for virtually all source files.
pub const MAX_EDITOR_FILE_SIZE: u64 = 20_000_000;

#[derive(thiserror::Error, Debug)]
pub enum FileLoadError {
    #[error("File does not exist")]
    DoesNotExist,
    #[error("File is too large to open in the editor ({size_bytes} bytes)")]
    FileTooLarge { size_bytes: u64 },
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
