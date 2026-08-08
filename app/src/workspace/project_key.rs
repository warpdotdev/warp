//! Stable project identity for the Projects × Tasks sidebar.
//!
//! A [`ProjectKey`] collapses every git worktree of the same repository into a
//! single project by keying on the repo's shared (common) `.git` directory, so
//! parallel-agent worktrees show up as *tasks under one project* rather than as
//! separate projects. This reuses the existing [`repo_metadata`] detection —
//! there is no new git plumbing.

use std::path::Path;

#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::AppContext;
#[cfg(feature = "local_fs")]
use warpui::SingletonEntity as _;

/// Separates a remote project's host id from its path in the storage encoding.
///
/// ASCII Unit Separator, deliberately not `:` or `/`: a host id is an opaque
/// server-issued string and a Windows-encoded path carries its own `:`, so
/// neither of the readable candidates can be split unambiguously. No path and
/// no host id can contain a control character, so this one always can be.
const REMOTE_FIELD_SEPARATOR: char = '\u{1f}';

/// A stable identity for a project in the session sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProjectKey {
    /// A local git repository, keyed by its shared (common) `.git` directory so
    /// that every worktree of the repo resolves to the same project.
    LocalGit(StandardizedPath),
    /// A local, non-git directory keyed by its standardized path.
    LocalDir(StandardizedPath),
    /// A remote repository/host.
    Remote(RemotePath),
}

impl ProjectKey {
    /// Derives the project key for a tab's focused session path.
    ///
    /// Local paths consult [`DetectedRepositories`]: a watched git repo yields
    /// its `common_git_dir()` (which unifies worktrees); otherwise this falls
    /// back to the detected repo root, else the path itself, as a
    /// [`ProjectKey::LocalDir`]. Returns `None` only when a local path cannot be
    /// standardized.
    pub fn for_path(path: &LocalOrRemotePath, ctx: &AppContext) -> Option<Self> {
        match path {
            LocalOrRemotePath::Remote(remote) => Some(Self::Remote(remote.clone())),
            LocalOrRemotePath::Local(local) => Self::for_local_path(local, ctx),
        }
    }

    #[cfg(feature = "local_fs")]
    fn for_local_path(local: &Path, ctx: &AppContext) -> Option<Self> {
        let detected = DetectedRepositories::as_ref(ctx);
        // Prefer the shared common `.git` dir so a repo's worktrees unify into
        // one project rather than appearing as separate per-checkout projects.
        if let Some(repo) = detected.get_local_watched_repo_for_path(local, ctx)
            && let Ok(common) = StandardizedPath::try_from_local(&repo.as_ref(ctx).common_git_dir())
        {
            return Some(Self::LocalGit(common));
        }
        // Fall back to the detected repo (working-tree) root, else the path
        // itself, as a non-git directory. Detection may not have completed yet;
        // the projection recomputes on repo-change events and upgrades this to
        // `LocalGit` once the repo is known. `root` is bound to a local so the
        // `&Path` borrowed from it via `to_local_path` stays valid.
        let root = detected.get_root_for_path(&LocalOrRemotePath::Local(local.to_path_buf()));
        let dir = root
            .as_ref()
            .and_then(LocalOrRemotePath::to_local_path)
            .unwrap_or(local);
        StandardizedPath::try_from_local(dir)
            .ok()
            .map(Self::LocalDir)
    }

    #[cfg(not(feature = "local_fs"))]
    fn for_local_path(local: &Path, _ctx: &AppContext) -> Option<Self> {
        StandardizedPath::try_from_local(local)
            .ok()
            .map(Self::LocalDir)
    }

    /// Encodes this key as the single canonical string used to persist
    /// per-project state in settings (see
    /// [`ProjectPriorities`](super::project_priorities::ProjectPriorities)).
    ///
    /// Tagged by variant, because two projects can share a path string while
    /// being different kinds of project, and a settings key must never
    /// collide. The payload is the path's own standardized string form, which
    /// round-trips losslessly through [`StandardizedPath::try_new`].
    ///
    /// Persisting the *key* rather than a raw cwd is what makes every worktree
    /// of a repo share one entry: [`Self::LocalGit`] already carries the
    /// repo's shared common `.git` dir, so the encoding inherits that
    /// unification for free.
    pub fn to_storage_key(&self) -> String {
        match self {
            Self::LocalGit(common_git_dir) => format!("git:{common_git_dir}"),
            Self::LocalDir(path) => format!("dir:{path}"),
            Self::Remote(remote) => {
                let host = &remote.host_id;
                let path = &remote.path;
                format!("remote:{host}{REMOTE_FIELD_SEPARATOR}{path}")
            }
        }
    }

    /// Parses a key produced by [`Self::to_storage_key`].
    ///
    /// Returns `None` for anything unrecognised — a settings file written by a
    /// newer version, or hand-edited into nonsense — so one bad entry is
    /// skipped rather than poisoning the whole list.
    pub fn from_storage_key(encoded: &str) -> Option<Self> {
        let (tag, payload) = encoded.split_once(':')?;
        match tag {
            "git" => StandardizedPath::try_new(payload).ok().map(Self::LocalGit),
            "dir" => StandardizedPath::try_new(payload).ok().map(Self::LocalDir),
            "remote" => {
                let (host, path) = payload.split_once(REMOTE_FIELD_SEPARATOR)?;
                StandardizedPath::try_new(path)
                    .ok()
                    .map(|path| Self::Remote(RemotePath::new(HostId::new(host.to_owned()), path)))
            }
            _ => None,
        }
    }

    /// A human-readable project label: the repository or directory folder name.
    pub fn display_name(&self) -> String {
        match self {
            // `common_git_dir` is `<repo>/.git`; the project name is `<repo>`.
            Self::LocalGit(common_git_dir) => common_git_dir
                .to_local_path()
                .as_deref()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "repo".to_owned()),
            Self::LocalDir(path) => path
                .to_local_path()
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "project".to_owned()),
            Self::Remote(remote) => Path::new(&remote.path.to_string())
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "remote".to_owned()),
        }
    }
}

#[cfg(test)]
#[path = "project_key_tests.rs"]
mod tests;
