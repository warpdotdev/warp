use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::remote_path::RemotePath;

/// Uniquely identifies where a file lives — either on the local filesystem
/// or on a remote host. Used across both the buffer model and the
/// editor/view layers as the canonical file-identity type.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum LocalOrRemotePath {
    /// File on the local filesystem.
    Local(PathBuf),
    /// File on a remote host, identified by host + path.
    Remote(RemotePath),
}

impl LocalOrRemotePath {
    /// Returns the file name component for display (e.g. tab titles).
    pub fn display_name(&self) -> &str {
        match self {
            LocalOrRemotePath::Local(path) => path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default(),
            LocalOrRemotePath::Remote(remote) => remote.path.file_name().unwrap_or_default(),
        }
    }

    /// Returns the local path if this is a `Local` location, `None` for `Remote`.
    /// Callers that only work with local files (LSP, save-to-disk, reveal-in-finder)
    /// should use this to gate their behavior.
    pub fn to_local_path(&self) -> Option<&Path> {
        match self {
            LocalOrRemotePath::Local(path) => Some(path.as_path()),
            LocalOrRemotePath::Remote(_) => None,
        }
    }
}

impl From<PathBuf> for LocalOrRemotePath {
    fn from(path: PathBuf) -> Self {
        LocalOrRemotePath::Local(path)
    }
}

impl From<RemotePath> for LocalOrRemotePath {
    fn from(remote: RemotePath) -> Self {
        LocalOrRemotePath::Remote(remote)
    }
}

impl TryFrom<LocalOrRemotePath> for PathBuf {
    type Error = RemotePath;

    /// Extracts the local path, returning `Err(RemotePath)` for remote locations.
    fn try_from(value: LocalOrRemotePath) -> Result<Self, Self::Error> {
        match value {
            LocalOrRemotePath::Local(path) => Ok(path),
            LocalOrRemotePath::Remote(remote) => Err(remote),
        }
    }
}
