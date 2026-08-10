//! Project identity for automatic tab grouping.
//!
//! A tab's *project key* is the value its group is keyed by. This module is
//! pure: it takes a directory, whatever git resolution the caller already has
//! for that directory, and the non-git keys already in use in the window, and
//! returns a key and a display name. It performs no I/O and touches no
//! workspace state, so every rule below is directly testable.

use std::path::{Path, PathBuf};

/// What repository detection knows about a directory.
///
/// The distinction between [`Self::NotARepository`] and [`Self::Pending`]
/// matters: the first is a real answer that yields a directory key, the second
/// is the asynchronous-detection window, which must yield *no* key so the tab
/// is left where it is rather than being read as manually placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitResolution {
    /// The directory is not inside any known git repository.
    NotARepository,
    /// The directory is inside a repository with this shared git directory.
    /// Every worktree of one repository resolves to the same value.
    Resolved(PathBuf),
    /// Detection has not answered for this directory yet.
    Pending,
}

/// The identity a tab group is keyed by.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectKey(PathBuf);

impl ProjectKey {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// Everything the resolver needs, and nothing else.
pub struct ProjectKeyInput<'a> {
    /// The anchor pane's working directory, canonicalized. `None` for a tab
    /// with no terminal session at all.
    pub directory: Option<&'a Path>,
    /// What detection knows about `directory`.
    pub git: GitResolution,
    /// The non-git keys already in use in this window.
    pub existing_non_git_keys: &'a [ProjectKey],
    /// The user's home directory, if known.
    pub home_dir: Option<&'a Path>,
}

/// Resolve a tab's project key.
///
/// Returns `None` whenever identity cannot be established — no directory,
/// detection still pending, or a directory that would produce a group we
/// refuse to create. A `None` key means "leave this tab alone", never "this
/// tab was placed by the user".
pub fn resolve(input: &ProjectKeyInput<'_>) -> Option<ProjectKey> {
    let directory = input.directory?;

    match &input.git {
        // Detection has not answered; do not guess a directory key.
        GitResolution::Pending => None,
        GitResolution::Resolved(common_git_dir) => Some(ProjectKey(common_git_dir.to_path_buf())),
        GitResolution::NotARepository => resolve_non_git(directory, input),
    }
}

fn resolve_non_git(directory: &Path, input: &ProjectKeyInput<'_>) -> Option<ProjectKey> {
    // Never key on the home directory or the filesystem root: a group holding
    // "everything you have ever cd'd through" is not a project.
    if directory.parent().is_none() || input.home_dir == Some(directory) {
        return None;
    }

    // Joining an existing group beats creating one, so descending a directory
    // does not destroy one group and create another on every `cd`. With more
    // than one candidate the longest (most specific) prefix wins.
    let longest_prefix = input
        .existing_non_git_keys
        .iter()
        .filter(|key| directory.starts_with(key.path()))
        .max_by_key(|key| key.path().components().count());

    if let Some(key) = longest_prefix {
        return Some(key.clone());
    }

    // A directory *above* an existing key would produce a parent group that
    // swallows it. Yield no key instead.
    if input
        .existing_non_git_keys
        .iter()
        .any(|key| key.path().starts_with(directory))
    {
        return None;
    }

    Some(ProjectKey(directory.to_path_buf()))
}

/// The name a key derives, before any collision qualification.
///
/// This must be total — every key has to produce a non-empty name, or the
/// name-provenance comparison that protects a user's rename cannot work.
fn base_name(key: &ProjectKey) -> String {
    let path = key.path();
    let file_name = path.file_name().and_then(|name| name.to_str());

    match file_name {
        // A normal checkout's key is `<repo>/.git`; the repository's name is
        // the parent's.
        Some(".git") => path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or(".git")
            .to_string(),
        // A bare repository's key is `<repo>.git`.
        Some(name) => name.strip_suffix(".git").unwrap_or(name).to_string(),
        None => path.to_string_lossy().into_owned(),
    }
}

/// The name to display for `key`, qualified with its parent directory segment
/// when another key in the same window derives the same name.
pub fn display_name<'a>(
    key: &ProjectKey,
    others: impl IntoIterator<Item = &'a ProjectKey>,
) -> String {
    let name = base_name(key);
    let collides = others
        .into_iter()
        .any(|other| other != key && base_name(other) == name);

    if !collides {
        return name;
    }

    // Qualify with the segment above the name, so two repositories called
    // `api` read as `services/api` and `vendor/api`.
    let qualifier_source = if key.path().file_name().and_then(|n| n.to_str()) == Some(".git") {
        key.path().parent()
    } else {
        Some(key.path())
    };

    qualifier_source
        .and_then(|path| path.parent())
        .and_then(|parent| parent.file_name())
        .and_then(|segment| segment.to_str())
        .map_or(name.clone(), |segment| format!("{segment}/{name}"))
}

#[cfg(test)]
#[path = "project_key_tests.rs"]
mod tests;
