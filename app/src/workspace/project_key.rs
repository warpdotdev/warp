//! Project identity for automatic tab grouping.
//!
//! A tab's *project key* is the value its group is keyed by. This module is
//! pure: it takes a directory, whatever git resolution the caller already has
//! for that directory, and the non-git keys already in use in the window, and
//! returns a key, a display name and a color. It performs no I/O and touches no
//! workspace state, so every rule below is directly testable.

use std::path::{Path, PathBuf};

use warp_core::ui::theme::AnsiColorIdentifier;

use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;

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

    /// Rebuilds a key from the form a group persists it in. The stored value is
    /// the key's path verbatim, so this is the inverse of
    /// [`Self::to_storage_string`].
    pub fn from_path(path: PathBuf) -> Self {
        Self(path)
    }

    /// The form a group stores its key in. `TabGroup::project_key` is a
    /// `String` because that is what the group table persists.
    pub fn to_storage_string(&self) -> String {
        self.0.to_string_lossy().into_owned()
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
    qualified_name(key).unwrap_or(name)
}

/// The name qualified with the segment above it, so two repositories called
/// `api` read as `services/api` and `vendor/api`. `None` when the key has no
/// such segment to qualify with.
fn qualified_name(key: &ProjectKey) -> Option<String> {
    let name = base_name(key);
    let qualifier_source = if key.path().file_name().and_then(|n| n.to_str()) == Some(".git") {
        key.path().parent()
    } else {
        Some(key.path())
    };

    qualifier_source
        .and_then(|path| path.parent())
        .and_then(|parent| parent.file_name())
        .and_then(|segment| segment.to_str())
        .map(|segment| format!("{segment}/{name}"))
}

/// Whether `name` is one automatic grouping could have derived for `key`.
///
/// Derivation has exactly two outcomes — the bare project name and that name
/// qualified by one parent segment — so anything else was typed by the user.
/// Callers use this to re-qualify their own names without overwriting a rename.
pub fn is_derived_name(key: &ProjectKey, name: &str) -> bool {
    name == base_name(key) || qualified_name(key).as_deref() == Some(name)
}

/// The color a key derives: one project always reads the same, and two
/// projects usually read differently.
///
/// Keyed off the *key* rather than the display name, which changes when a
/// collision qualifies it (`api` becoming `vendor/api`) and would take the
/// color with it. Two checkouts of one repository share a key, so they also
/// share a color; two clones of the same upstream at different paths do not,
/// exactly as they do not share a group.
///
/// Two projects can land on the same color — the palette has six entries and
/// nothing spaces the hash out, so a second project collides with the first
/// about one time in six. Colliding is the deliberate trade: de-colliding
/// within a window would make a project's color depend on what else happens to
/// be open, which is the one property this is for.
pub fn derived_color(key: &ProjectKey) -> AnsiColorIdentifier {
    // The palette's length is part of what every stored group color was derived
    // from, so growing or shrinking it repaints all of them. Failing the build
    // here is the point: the change is legitimate, but it must be deliberate,
    // and the golden test beside this function will need new values.
    const _: () = assert!(TAB_COLOR_OPTIONS.len() == 6);

    let index = stable_hash(&key.to_storage_string()) % TAB_COLOR_OPTIONS.len() as u64;
    TAB_COLOR_OPTIONS[index as usize]
}

/// FNV-1a, 64-bit.
///
/// Spelled out rather than taken from [`std::hash`], whose `DefaultHasher` is
/// explicitly not guaranteed to be stable across releases, and whose `HashMap`
/// hasher is seeded randomly per process — either would repaint every group on
/// restart. `rustc-hash`, the workspace's other hasher, gives the same
/// no-stability-guarantee. The property wanted here is a value that never
/// changes, so the function is pinned here and by the golden test beside it.
fn stable_hash(value: &str) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
#[path = "project_key_tests.rs"]
mod tests;
