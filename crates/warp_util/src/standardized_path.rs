//! A normalized, platform-aware path type that does not require filesystem I/O.
//!
//! [`StandardizedPath`] wraps [`TypedPathBuf`] and guarantees that the inner path is always
//! absolute and normalized (`.` and `..` segments removed, separators collapsed). Unlike
//! [`CanonicalizedPath`](repo_metadata::CanonicalizedPath), construction does **not** resolve
//! symlinks or verify existence on disk.

use std::path::{Path, PathBuf};
use std::{fmt, io};

use serde::{Deserialize, Serialize};
use typed_path::{PathType, TypedPath, TypedPathBuf};

/// Error returned when a path cannot be converted into a [`StandardizedPath`].
#[derive(Debug, thiserror::Error)]
pub enum InvalidPathError {
    #[error("path is not absolute: {0}")]
    NotAbsolute(String),
    #[error("path contains invalid UTF-8")]
    InvalidUtf8,
}

/// A normalized, platform-aware path that does not require the file to exist.
///
/// Unlike `CanonicalizedPath`, construction does NOT perform filesystem I/O.
/// Normalization removes `.` and `..` segments and collapses separators, but
/// does not resolve symlinks or verify existence.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct StandardizedPath(TypedPathBuf);

impl StandardizedPath {
    // ── Construction APIs ─────────────────────────────────────────────

    /// Create from a string, inferring Unix vs Windows encoding.
    /// Normalizes the path (removes `.`/`..`, collapses separators).
    /// Returns an error if the path is not absolute.
    pub fn try_new(path: &str) -> Result<Self, InvalidPathError> {
        let typed = TypedPathBuf::from(path);
        let normalized = typed.normalize();
        if !normalized.is_absolute() {
            return Err(InvalidPathError::NotAbsolute(path.to_owned()));
        }
        Ok(Self(normalized))
    }

    /// Create with an explicit path type (Unix or Windows).
    /// Returns an error if the path is not absolute.
    pub fn try_with_encoding(path: &str, path_type: PathType) -> Result<Self, InvalidPathError> {
        let typed = TypedPathBuf::new(path_type);
        let typed = typed.join(path);
        let normalized = typed.normalize();
        if !normalized.is_absolute() {
            return Err(InvalidPathError::NotAbsolute(path.to_owned()));
        }
        Ok(Self(normalized))
    }

    /// Create from a local `std::path::Path`, inferring encoding from
    /// the compile target. Normalizes but does NOT canonicalize.
    /// Returns an error if the path is not absolute.
    pub fn try_from_local(path: &Path) -> Result<Self, InvalidPathError> {
        let path_str = path.to_str().ok_or(InvalidPathError::InvalidUtf8)?;
        let typed = local_typed_path_buf(path_str);
        let normalized = typed.normalize();
        if !normalized.is_absolute() {
            return Err(InvalidPathError::NotAbsolute(path_str.to_owned()));
        }
        Ok(Self(normalized))
    }

    /// Create from a local `std::path::Path` that is **known** to be absolute.
    ///
    /// # Panics
    /// Panics (debug-only) if the path is not absolute or contains invalid
    /// UTF-8. In release builds the path is accepted as-is to avoid
    /// to penalize hot paths.
    pub fn from_local_absolute_unchecked(path: &Path) -> Self {
        debug_assert!(
            path.is_absolute(),
            "from_local_absolute called with non-absolute path: {}",
            path.display()
        );
        debug_assert!(
            path.to_str().is_some(),
            "from_local_absolute called with non-UTF-8 path: {}",
            path.display()
        );
        let path_str = path.to_str().unwrap_or_default();
        let typed = local_typed_path_buf(path_str);
        Self(typed.normalize())
    }

    /// Create from a local path with full canonicalization (resolves
    /// symlinks, verifies existence). This is the I/O-performing
    /// equivalent of `CanonicalizedPath::try_from`.
    /// Use at shell boundaries when receiving paths from the OS.
    pub fn from_local_canonicalized(path: &Path) -> io::Result<Self> {
        let canonical = dunce::canonicalize(path)?;
        // dunce::simplified strips the UNC prefix when safe.
        let simplified = dunce::simplified(&canonical);
        let path_str = simplified.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "canonicalized path is not valid UTF-8",
            )
        })?;
        let typed = local_typed_path_buf(path_str);
        // Canonical paths are already normalized, but normalize anyway for consistency.
        Ok(Self(typed.normalize()))
    }

    // ── Query APIs ───────────────────────────────────────────────────

    /// Returns the underlying `TypedPath`.
    pub fn as_typed_path(&self) -> TypedPath<'_> {
        self.0.to_path()
    }

    /// Returns the string representation of the path.
    pub fn as_str(&self) -> &str {
        self.0.to_str().unwrap_or_default()
    }

    /// Returns the file name component, if any.
    pub fn file_name(&self) -> Option<&str> {
        self.0.file_name().and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Returns the extension, if any.
    pub fn extension(&self) -> Option<&str> {
        self.0.extension().and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Returns the parent path, if any.
    pub fn parent(&self) -> Option<StandardizedPath> {
        self.0.parent().map(|p| StandardizedPath(p.to_path_buf()))
    }

    /// Whether this path starts with the given prefix.
    pub fn starts_with(&self, base: &StandardizedPath) -> bool {
        self.0.starts_with(&base.0)
    }

    /// Whether this path ends with the given suffix (component-aware).
    ///
    /// The suffix can be a relative path string (e.g. `.agents/skills`).
    /// Matching is done at the component level, so `/repo/myskills` does
    /// **not** match the suffix `skills`.
    pub fn ends_with(&self, suffix: &str) -> bool {
        self.0.ends_with(suffix)
    }

    /// Strip a prefix from this path, returning the relative remainder.
    pub fn strip_prefix(&self, base: &StandardizedPath) -> Option<&str> {
        let self_str = self.as_str();
        let base_str = base.as_str();
        self_str.strip_prefix(base_str).map(|remainder| {
            // Remove leading separator if present.
            remainder
                .strip_prefix('/')
                .or_else(|| remainder.strip_prefix('\\'))
                .unwrap_or(remainder)
        })
    }

    /// Join a relative segment onto this path.
    pub fn join(&self, segment: &str) -> StandardizedPath {
        StandardizedPath(self.0.join(segment).normalize())
    }

    /// Whether the path uses Unix encoding.
    pub fn is_unix(&self) -> bool {
        self.0.to_path().is_unix()
    }

    /// Whether the path uses Windows encoding.
    pub fn is_windows(&self) -> bool {
        self.0.to_path().is_windows()
    }

    /// Sets the file name component of this path, analogous to
    /// [`PathBuf::set_file_name`].
    pub fn set_file_name(&mut self, name: &str) {
        self.0.set_file_name(name);
        self.0 = self.0.normalize();
    }

    /// Returns an iterator over the ancestors of this path, starting with
    /// the path itself and ending at the root.
    pub fn ancestors(&self) -> impl Iterator<Item = StandardizedPath> {
        let mut current = Some(self.clone());
        std::iter::from_fn(move || {
            let path = current.take()?;
            current = path.parent();
            Some(path)
        })
    }

    // ── Conversion APIs ──────────────────────────────────────────────

    /// Convert to a local `PathBuf` if the encoding matches the current OS.
    /// Returns `None` for a Unix-encoded path on Windows or vice versa.
    pub fn to_local_path(&self) -> Option<PathBuf> {
        if encoding_matches_local(&self.0) {
            Some(PathBuf::from(self.as_str()))
        } else {
            None
        }
    }

    /// Converts this path to a local [`PathBuf`] by re-encoding its components
    /// for the current OS.
    ///
    /// **Use this only when the path is known to originate from the local
    /// filesystem** (e.g. from [`LocalRepoMetadataModel`], [`Repository`],
    /// [`DetectedRepositories`], or any path that was constructed via
    /// [`from_local_canonicalized`](Self::from_local_canonicalized) /
    /// [`try_from_local`](Self::try_from_local)). For those paths the
    /// encoding already matches and the conversion is lossless.
    ///
    /// If the path was constructed with a foreign encoding (e.g. a
    /// Windows-encoded remote path on a macOS host), the conversion is lossy:
    /// platform-specific prefixes like `C:` are dropped and separators are
    /// translated. Prefer [`to_local_path`](Self::to_local_path) when the
    /// encoding match is not guaranteed and you need to handle the mismatch
    /// explicitly.
    ///
    /// This function is generally something you shouldn't use. We are using this
    /// as a stop gap to avoid `unwrap` as we migrate from PathBuf to StandardizedPath.
    pub fn to_local_path_lossy(&self) -> PathBuf {
        let local = if cfg!(windows) {
            self.0.with_windows_encoding()
        } else {
            self.0.with_unix_encoding()
        };
        PathBuf::from(local.to_str().unwrap_or_default())
    }
}

impl fmt::Display for StandardizedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Serialize for StandardizedPath {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StandardizedPath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::try_new(&s).map_err(serde::de::Error::custom)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Construct a `TypedPathBuf` using the local platform's encoding.
///
/// On Unix targets the path is always treated as Unix-encoded; on Windows
/// targets it is always treated as Windows-encoded. This avoids ambiguity
/// from the heuristic-based `TypedPathBuf::from` inference.
fn local_typed_path_buf(path_str: &str) -> TypedPathBuf {
    if cfg!(windows) {
        typed_path::WindowsPathBuf::from(path_str).to_typed_path_buf()
    } else {
        typed_path::UnixPathBuf::from(path_str).to_typed_path_buf()
    }
}

/// Returns true if the `TypedPathBuf` encoding matches the compilation target.
fn encoding_matches_local(typed: &TypedPathBuf) -> bool {
    let path = typed.to_path();
    if cfg!(windows) {
        path.is_windows()
    } else {
        path.is_unix()
    }
}

#[cfg(test)]
#[path = "standardized_path_tests.rs"]
mod tests;
