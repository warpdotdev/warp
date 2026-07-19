use std::path::{Path, PathBuf};

/// Whether a registered path is a git repository or a plain folder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepoEntryKind {
    Repo,
    Folder,
}

/// A registry entry identified by its canonical path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepoEntry {
    pub path: PathBuf,
    pub display_name: String,
    pub kind: RepoEntryKind,
}

impl RepoEntry {
    pub fn from_path(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = canonicalize_repo_path(path.as_ref())?;
        let display_name = display_name_for_path(&path);
        let kind = classify_entry_kind(&path);
        Ok(Self {
            path,
            display_name,
            kind,
        })
    }
}

/// Canonicalize `path` for registry identity (dedup trailing slash / symlink variants).
pub fn canonicalize_repo_path(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// Basename used as the primary row label.
pub fn display_name_for_path(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Classify as repo when `.git` exists as a file or directory (covers linked worktrees).
pub fn classify_entry_kind(path: &Path) -> RepoEntryKind {
    let git = path.join(".git");
    if git.exists() {
        RepoEntryKind::Repo
    } else {
        RepoEntryKind::Folder
    }
}

/// True when the path no longer exists on disk.
pub fn is_dead_path(path: &Path) -> bool {
    !path.exists()
}

#[cfg(test)]
#[path = "entry_tests.rs"]
mod tests;
