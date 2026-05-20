use std::path::Path;
use std::{collections::HashSet, future::Future, path::PathBuf};

#[cfg(test)]
use virtual_fs::{Stub, VirtualFS};
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::{RemoteNavigationResult, RemotePath};
use warp_util::standardized_path::StandardizedPath;
#[cfg(test)]
use warpui::r#async::FutureId;
use warpui::{AppContext, Entity, ModelContext, ModelHandle};

use crate::DirectoryWatcher;
use crate::Repository;
use futures::future::{ready, Either};
use warpui::SingletonEntity;

/// Indicates why a repository detection event was emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoDetectionSource {
    /// User actively navigated to this repo in a terminal (via cd/pwd change).
    TerminalNavigation,
    /// Repo was detected during project rules indexing.
    ProjectRulesIndexing,
    /// Repo was detected for code review/diff state initialization.
    CodeReviewInitialization,
    /// Repo was cloned or discovered during cloud agent environment preparation.
    CloudEnvironmentPrep,
}

pub enum DetectedRepositoriesEvent {
    DetectedGitRepo {
        repository: ModelHandle<Repository>,
        source: RepoDetectionSource,
    },
}

/// Tracks the detected _git_ repositories during the lifetime of the application. This should be the canonical source of truth for repository information.
#[derive(Default)]
pub struct DetectedRepositories {
    repository_roots: HashSet<LocalOrRemotePath>,
    #[cfg(test)]
    /// List of spawned background tasks, for testing.
    spawned_futures: Vec<FutureId>,
}

impl DetectedRepositories {
    /// Detects the git repository root for the given working directory.
    ///
    /// For **local sessions**, pass `None` for `remote_detect` — this delegates
    /// to the local filesystem detection path.
    ///
    /// For **remote sessions**, pass `Some(future)` where the future resolves
    /// with `(RemotePath, is_git)` from the remote server. The future is
    /// typically obtained from `RemoteServerManager::navigate_to_directory`.
    /// When `is_git` is true, the result is `Some(LocalOrRemotePath::Remote(...))`.
    ///
    /// This design avoids a circular dependency between `repo_metadata` and
    /// `remote_server` — the caller in `app/` constructs the remote future
    /// and injects it here.
    pub fn detect_possible_git_repo<
        F: Future<Output = Option<RemoteNavigationResult>> + 'static,
    >(
        &mut self,
        active_directory: &str,
        source: RepoDetectionSource,
        remote_detect: Option<F>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Option<LocalOrRemotePath>> {
        match remote_detect {
            None => {
                // Local detection path.
                let fut = self.detect_possible_local_git_repo(active_directory, source, ctx);
                Either::Left(async move { fut.await.map(LocalOrRemotePath::Local) })
            }
            Some(remote_fut) => Either::Right(async move {
                match remote_fut.await {
                    Some(RemoteNavigationResult {
                        remote_path,
                        is_git: true,
                    }) => Some(LocalOrRemotePath::Remote(remote_path)),
                    _ => None,
                }
            }),
        }
    }

    /// Given the active directory pwd, kick off a background job to detect the git project root and emit an event
    /// to interested listeners.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn detect_possible_local_git_repo(
        &mut self,
        active_directory: &str,
        source: RepoDetectionSource,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Option<PathBuf>> {
        #[cfg(feature = "local_fs")]
        {
            use futures::channel::oneshot;

            let Ok(path) = StandardizedPath::from_local_canonicalized(Path::new(active_directory))
            else {
                return Either::Right(ready(None));
            };

            let local_key = path.to_local_path().map(LocalOrRemotePath::Local);
            if let Some(ref key) = local_key {
                if self.repository_roots.contains(key) {
                    if let Some(local_path) = path.to_local_path() {
                        if let Some(repository) = DirectoryWatcher::as_ref(ctx)
                            .get_watched_directory_for_path(&local_path)
                        {
                            ctx.emit(DetectedRepositoriesEvent::DetectedGitRepo {
                                repository: repository.clone(),
                                source,
                            });
                        }
                    }
                    return Either::Right(ready(path.to_local_path()));
                }
            }

            let local_path_for_search = path.to_local_path();
            let (tx, rx) = oneshot::channel::<Option<PathBuf>>();
            let spawned_handle = ctx.spawn(
                async move {
                    if let Some(local_path) = local_path_for_search {
                        find_git_repo(&local_path).await
                    } else {
                        None
                    }
                },
                move |me, res, ctx| {
                    if let Some(info) = res {
                        if let Some(repo_root_path) = info
                            .working_tree_path
                            .as_ref()
                            .and_then(|path| StandardizedPath::from_local_canonicalized(path).ok())
                        {
                            if let Some(local_path) = repo_root_path.to_local_path() {
                                me.repository_roots
                                    .insert(LocalOrRemotePath::Local(local_path));
                            }

                            let external_git_dir = StandardizedPath::from_local_canonicalized(
                                info.git_dir_path.as_path(),
                            )
                            .ok()
                            // Only treat as external if it's outside the working tree.
                            .filter(|p| !p.starts_with(&repo_root_path));

                            if let Some(repository) =
                                DirectoryWatcher::handle(ctx).update(ctx, |watcher, ctx| {
                                    watcher
                                        .add_directory_with_git_dir(
                                            repo_root_path,
                                            external_git_dir,
                                            ctx,
                                        )
                                        .ok()
                                })
                            {
                                let repo_path = repository.as_ref(ctx).root_dir().to_local_path();
                                ctx.emit(DetectedRepositoriesEvent::DetectedGitRepo {
                                    repository,
                                    source,
                                });
                                let _ = tx.send(repo_path);
                            } else {
                                let _ = tx.send(None);
                            }
                        } else {
                            // No working tree path; do not treat git_dir_path as a repository path.
                            let _ = tx.send(None);
                        }
                    } else {
                        let _ = tx.send(None);
                    }
                },
            );

            #[cfg(not(test))]
            let _ = spawned_handle;

            #[cfg(test)]
            self.spawned_futures.push(spawned_handle.future_id());

            Either::Left(async move { rx.await.unwrap_or(None) })
        }

        #[cfg(not(feature = "local_fs"))]
        {
            use futures::future::Ready;
            Either::<Ready<Option<PathBuf>>, Ready<Option<PathBuf>>>::Left(ready(None))
        }
    }

    #[cfg(test)]
    pub fn spawned_futures(&self) -> &[FutureId] {
        &self.spawned_futures
    }

    /// Given a local path, return its corresponding watched repository, if any.
    pub fn get_local_watched_repo_for_path(
        &self,
        path: &Path,
        ctx: &AppContext,
    ) -> Option<ModelHandle<Repository>> {
        let root = self.get_root_for_path(&LocalOrRemotePath::Local(path.to_path_buf()))?;
        let local_path = root.to_local_path()?;
        DirectoryWatcher::as_ref(ctx).get_watched_directory_for_path(local_path)
    }

    /// Given a local or remote path, return its corresponding repo root.
    /// This does not run the check against the actual file system.
    /// Instead it checks against our cached path to root mapping.
    pub fn get_root_for_path(&self, path: &LocalOrRemotePath) -> Option<LocalOrRemotePath> {
        match path {
            LocalOrRemotePath::Local(local_path) => {
                let std_path = StandardizedPath::from_local_canonicalized(local_path).ok()?;
                self.find_local_repository_root(&std_path)
            }
            LocalOrRemotePath::Remote(remote_path) => self.find_remote_repository_root(remote_path),
        }
    }

    /// Find the local repository that contains the given path, if any.
    fn find_local_repository_root(&self, path: &StandardizedPath) -> Option<LocalOrRemotePath> {
        for ancestor in path.ancestors() {
            if let Some(local_path) = ancestor.to_local_path() {
                let key = LocalOrRemotePath::Local(local_path);
                if self.repository_roots.contains(&key) {
                    return Some(key);
                }
            }
        }
        None
    }

    /// Find the remote repository that contains the given path, if any.
    fn find_remote_repository_root(&self, remote_path: &RemotePath) -> Option<LocalOrRemotePath> {
        for ancestor in remote_path.path.ancestors() {
            let candidate =
                LocalOrRemotePath::Remote(RemotePath::new(remote_path.host_id.clone(), ancestor));
            if self.repository_roots.contains(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// Register a remote repository root discovered via the remote server.
    pub fn register_remote_repo_root(&mut self, remote_path: RemotePath) {
        self.repository_roots
            .insert(LocalOrRemotePath::Remote(remote_path));
    }

    /// Remove all cached repository roots for a given remote host.
    /// Call on `HostDisconnected` to prevent stale entries.
    pub fn remove_roots_for_host(&mut self, host_id: &HostId) {
        self.repository_roots.retain(|entry| match entry {
            LocalOrRemotePath::Local(_) => true,
            LocalOrRemotePath::Remote(remote) => remote.host_id != *host_id,
        });
    }
}

impl Entity for DetectedRepositories {
    type Event = DetectedRepositoriesEvent;
}

impl SingletonEntity for DetectedRepositories {}

/// Test helpers: direct mutation of internal state.
#[cfg(any(test, feature = "test-util"))]
impl DetectedRepositories {
    /// Insert a local repository root path directly, bypassing git detection.
    pub fn insert_test_repo_root(&mut self, path: StandardizedPath) {
        if let Some(local_path) = path.to_local_path() {
            self.repository_roots
                .insert(LocalOrRemotePath::Local(local_path));
        }
    }
}

/// Information about a discovered Git repository.
#[cfg(feature = "local_fs")]
#[derive(Debug, Clone)]
struct GitRepoInfo {
    /// Path to the working tree, if present. None for bare repositories.
    working_tree_path: Option<PathBuf>,
    /// Path to the git directory (contains objects, refs, and index).
    /// We can watch the HEAD file for branch changes, but currently don't do so.
    git_dir_path: PathBuf,
}

/// Finds the Git repository containing the given path, if any.
///
/// Supports:
/// - A .git directory containing a HEAD file: parent directory is the working tree, .git is the git dir
/// - A <project>.git directory containing a HEAD file (bare repo): that directory is the git dir and there is no working tree
/// - A .git file containing "gitdir: <path>": working tree is the parent directory; git dir is the parsed path (resolved if relative)
///
/// Traverses up to the user's $HOME directory; if no repo is found by that point, returns `None`.
#[cfg(feature = "local_fs")]
async fn find_git_repo(path: &Path) -> Option<GitRepoInfo> {
    let home_dir = dirs::home_dir()?;
    let mut current = path.to_owned();

    loop {
        if current == home_dir {
            return None;
        }

        // First, check if the current directory is a bare git repository.
        if let Some(dir_name) = current.file_name().and_then(|s| s.to_str()) {
            if dir_name.ends_with(".git") && is_valid_git_dir(&current).await {
                return Some(GitRepoInfo {
                    working_tree_path: None,
                    git_dir_path: current.clone(),
                });
            }
        }

        // Check for a .git directory.
        let dot_git_path = current.join(".git");
        if let Ok(dot_git_type) = async_fs::symlink_metadata(&dot_git_path)
            .await
            .map(|m| m.file_type())
        {
            if dot_git_type.is_dir() {
                // A standard repository with a .git directory.
                if is_valid_git_dir(&dot_git_path).await {
                    return Some(GitRepoInfo {
                        working_tree_path: Some(current.clone()),
                        git_dir_path: dot_git_path,
                    });
                }
            } else if dot_git_type.is_file() {
                // A potential gitfile, used by worktrees and submodules.
                if let Ok(contents) = async_fs::read_to_string(&dot_git_path).await {
                    // Typical format: "gitdir: <path>\n"
                    if let Some(rest) = contents.trim().strip_prefix("gitdir:") {
                        let gitdir_path = PathBuf::from(rest.trim());
                        let resolved_gitdir = if gitdir_path.is_absolute() {
                            gitdir_path
                        } else {
                            current.join(gitdir_path)
                        };
                        if is_valid_git_dir(&resolved_gitdir).await {
                            return Some(GitRepoInfo {
                                working_tree_path: Some(current.clone()),
                                git_dir_path: resolved_gitdir,
                            });
                        }
                    }
                }
            }
        }

        if !current.pop() {
            return None;
        }
    }
}

/// Checks whether the given directory is a valid Git directory by verifying it contains a HEAD file.
#[cfg(feature = "local_fs")]
async fn is_valid_git_dir(dir: &Path) -> bool {
    async_fs::metadata(dir.join("HEAD"))
        .await
        .map(|m| m.is_file())
        .unwrap_or(false)
}

/// Helper function to stub a git repository in a VirtualFS with the given repository directory name.
#[cfg(test)]
pub(crate) fn stub_git_repository(vfs: &mut VirtualFS, repo_name: &str) {
    let objects_dir = format!("{repo_name}/.git/objects");
    vfs.mkdir(&objects_dir);

    let head_path = format!("{repo_name}/.git/HEAD");
    let config_path = format!("{repo_name}/.git/config");
    vfs.with_files(vec![
        Stub::FileWithContent(&head_path, "ref: refs/heads/main"),
        Stub::FileWithContent(&config_path, "[core]\n\trepositoryformatversion = 0"),
    ]);
}

#[cfg(test)]
#[path = "repositories_tests.rs"]
mod tests;
