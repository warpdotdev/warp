use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_lite::io::AsyncReadExt as _;
use warp_errors::{ErrorExt, register_error};

/// Reads `path` into a `Vec<u8>`, rejecting files above `max_bytes`.
///
/// `async_fs::read` reserves the file's entire on-disk size up front via
/// `Vec::with_capacity`, with no upper bound. A pathologically large or sparse file can
/// therefore balloon the process's memory footprint by tens of GiB in a single allocation
/// before a single byte is read.
///
/// The cap is enforced by reading at most `max_bytes + 1` bytes from a single open handle
/// (via [`AsyncReadExt::take`]), not by trusting a `stat`-reported length: the file's on-disk
/// size can change (or the path can be atomically replaced) between a `stat` and a later,
/// separate open-by-path read, and a FIFO, character device, or `/proc`-style file reports an
/// unrelated (often zero) length from `stat` regardless of how much data it actually yields.
/// `metadata` is still queried, but only to pre-size the buffer — never to decide whether to
/// reject. See APP-4801.
pub async fn read_capped(path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
    let file = async_fs::File::open(path).await?;
    let read_limit = max_bytes.saturating_add(1);
    // Best-effort sizing hint only; capped so an inflated or unrelated `stat` length (e.g. a
    // sparse file, or a device/FIFO reporting an unrelated length) can never itself cause an
    // over-cap reservation.
    let size_hint = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let mut buffer = Vec::with_capacity(size_hint.min(read_limit) as usize);
    file.take(read_limit).read_to_end(&mut buffer).await?;
    if buffer.len() as u64 > max_bytes {
        return Err(io::Error::other(format!(
            "file is too large to load into memory: at least {} bytes read (limit {max_bytes} bytes)",
            buffer.len()
        )));
    }
    Ok(buffer)
}

/// String counterpart of [`read_capped`]; see its doc comment for the rationale.
pub async fn read_to_string_capped(path: &Path, max_bytes: u64) -> io::Result<String> {
    let bytes = read_capped(path, max_bytes).await?;
    String::from_utf8(bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

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
    /// A non-IO failure with a self-describing message (e.g. content could
    /// not be derived for the write).
    #[error("{0}")]
    Other(String),
}

impl ErrorExt for FileSaveError {
    fn is_actionable(&self) -> bool {
        match self {
            FileSaveError::NoFilePath(_) | FileSaveError::Other(_) => true,
            FileSaveError::IOError { .. } | FileSaveError::RemoteError(_) => false,
        }
    }
}
register_error!(FileSaveError);

#[derive(thiserror::Error, Debug)]
pub enum FileLoadError {
    #[error("File does not exist")]
    DoesNotExist,
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

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
