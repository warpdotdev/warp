use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::remote_path::RemotePath;
use crate::standardized_path::StandardizedPath;

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
    /// Returns `true` if this is a `Local` location.
    pub fn is_local(&self) -> bool {
        matches!(self, LocalOrRemotePath::Local(_))
    }

    /// Returns `true` if this is a `Remote` location.
    pub fn is_remote(&self) -> bool {
        matches!(self, LocalOrRemotePath::Remote(_))
    }

    /// Returns the standardized path component of the location, regardless of where it lives.
    pub fn path_component(&self) -> StandardizedPath {
        match self {
            LocalOrRemotePath::Local(path) => StandardizedPath::from_local_absolute_unchecked(path),
            LocalOrRemotePath::Remote(remote) => remote.path.clone(),
        }
    }

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

    /// Returns a displayable path string.
    pub fn display_path(&self) -> String {
        match self {
            LocalOrRemotePath::Local(path) => path.to_string_lossy().to_string(),
            LocalOrRemotePath::Remote(remote) => {
                format!("{}", remote.path)
            }
        }
    }

    /// Returns this location's parent, preserving remote host identity.
    pub fn parent(&self) -> Option<LocalOrRemotePath> {
        match self {
            LocalOrRemotePath::Local(path) => path
                .parent()
                .map(|parent| LocalOrRemotePath::Local(parent.to_path_buf())),
            LocalOrRemotePath::Remote(remote) => remote.path.parent().map(|parent| {
                LocalOrRemotePath::Remote(RemotePath::new(remote.host_id.clone(), parent))
            }),
        }
    }

    /// Returns the file name component, regardless of where the path lives.
    pub fn file_name(&self) -> Option<&str> {
        match self {
            LocalOrRemotePath::Local(path) => path.file_name().and_then(|name| name.to_str()),
            LocalOrRemotePath::Remote(remote) => remote.path.file_name(),
        }
    }

    /// Returns whether this location starts with `base`.
    ///
    /// Remote locations only compare as ancestors when they are on the same
    /// host. This prevents `/repo` on one host from matching `/repo` on another.
    pub fn starts_with(&self, base: &LocalOrRemotePath) -> bool {
        match (self, base) {
            (LocalOrRemotePath::Local(path), LocalOrRemotePath::Local(base)) => {
                path.starts_with(base)
            }
            (LocalOrRemotePath::Remote(path), LocalOrRemotePath::Remote(base)) => {
                path.host_id == base.host_id && path.path.starts_with(&base.path)
            }
            _ => false,
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

    /// Joins a (typically repo-relative) segment onto this location, preserving
    /// the host.
    ///
    /// Accepts a `&str` rather than a `&Path` so that no caller is forced to
    /// construct a local-filesystem path type when working with paths that
    /// may originate from a remote host.
    ///
    /// For `Local`, this delegates to `PathBuf::join` and yields a new local
    /// path. For `Remote`, the host id is carried through and only the
    /// path component is extended.
    ///
    /// Note: if `segment` is itself absolute, the standard `Path::join`
    /// replacement semantics apply (the joined result is `segment`), so
    /// callers that already hold an absolute path from a wire decode will
    /// get back the absolute path unchanged — modulo host preservation on
    /// the remote side.
    pub fn join(&self, segment: &str) -> LocalOrRemotePath {
        match self {
            LocalOrRemotePath::Local(path) => LocalOrRemotePath::Local(path.join(segment)),
            LocalOrRemotePath::Remote(remote) => {
                let joined = remote.path.join(segment);
                LocalOrRemotePath::Remote(RemotePath::new(remote.host_id.clone(), joined))
            }
        }
    }

    /// If `file` shares this location's host and starts with this location's
    /// path, returns the relative remainder as a `String`. Returns `None`
    /// when the hosts differ or when `file` is not under this location.
    ///
    /// Returns a `String` (rather than `PathBuf`) so that callers do not
    /// implicitly assume the relative remainder lives on the local
    /// filesystem — remote paths may use a different encoding than the host
    /// the client is running on.
    ///
    /// Use this when you want to compute a repo-relative path from an
    /// absolute file path without silently dropping the host id (as
    /// `path_component().strip_prefix(...)` would).
    pub fn strip_repo_prefix(&self, file: &LocalOrRemotePath) -> Option<String> {
        match (self, file) {
            (LocalOrRemotePath::Local(repo), LocalOrRemotePath::Local(f)) => f
                .strip_prefix(repo)
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
            (LocalOrRemotePath::Remote(repo), LocalOrRemotePath::Remote(f))
                if repo.host_id == f.host_id =>
            {
                f.path.strip_prefix(&repo.path).map(str::to_owned)
            }
            _ => None,
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

    fn try_from(location: LocalOrRemotePath) -> Result<Self, Self::Error> {
        match location {
            LocalOrRemotePath::Local(path) => Ok(path),
            LocalOrRemotePath::Remote(remote) => Err(remote),
        }
    }
}

impl TryFrom<&LocalOrRemotePath> for PathBuf {
    type Error = ();

    fn try_from(location: &LocalOrRemotePath) -> Result<Self, Self::Error> {
        match location {
            LocalOrRemotePath::Local(path) => Ok(path.clone()),
            LocalOrRemotePath::Remote(_) => Err(()),
        }
    }
}
#[cfg(test)]
#[path = "local_or_remote_path_tests.rs"]
mod tests;
