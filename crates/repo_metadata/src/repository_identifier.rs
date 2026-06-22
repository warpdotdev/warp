use std::path::{Path, PathBuf};

use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

/// Identifies a repository across local and remote environments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RepositoryIdentifier {
    /// A repository on the local filesystem, identified by its standardized path.
    Local(StandardizedPath),
    /// A repository on a remote server, identified by host + path.
    Remote(RemotePath),
}

/// Type alias preserved for backward compatibility.
pub type RemoteRepositoryIdentifier = RemotePath;

impl RepositoryIdentifier {
    /// Convenience constructor for a local repository identifier.
    pub fn local(path: StandardizedPath) -> Self {
        Self::Local(path)
    }

    /// Convenience constructor that creates a `Local` identifier from a
    /// `std::path::Path`. Returns `None` if the path is not absolute or
    /// contains non-UTF-8 characters.
    pub fn try_local(path: &Path) -> Option<Self> {
        StandardizedPath::try_from_local(path).ok().map(Self::Local)
    }

    /// Returns the local path if this is a `Local` variant.
    pub fn local_path(&self) -> Option<&StandardizedPath> {
        match self {
            Self::Local(path) => Some(path),
            Self::Remote(_) => None,
        }
    }

    /// Returns the local path as a `PathBuf` if this is a `Local` variant
    /// and the encoding matches the current OS.
    pub fn local_path_buf(&self) -> Option<PathBuf> {
        match self {
            Self::Local(path) => path.to_local_path(),
            Self::Remote(_) => None,
        }
    }

    /// Converts this identifier to a `LocalOrRemotePath`.
    ///
    /// Returns `None` only for `Local` identifiers whose `StandardizedPath`
    /// cannot be converted to a local `PathBuf` (cross-platform edge case).
    pub fn to_local_or_remote_path(&self) -> Option<LocalOrRemotePath> {
        match self {
            Self::Local(path) => path.to_local_path().map(LocalOrRemotePath::Local),
            Self::Remote(remote) => Some(LocalOrRemotePath::Remote(remote.clone())),
        }
    }
}

impl From<RemotePath> for RepositoryIdentifier {
    fn from(id: RemotePath) -> Self {
        Self::Remote(id)
    }
}
