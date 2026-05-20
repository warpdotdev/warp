use dirs::home_dir;
use std::path::{Path, PathBuf};
#[cfg(not(target_family = "wasm"))]
use std::{fs, sync::Arc, time::Duration};

#[cfg(not(target_family = "wasm"))]
use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
use repo_metadata::RepositoryUpdate;
#[cfg(any(not(target_family = "wasm"), test))]
use repo_metadata::TargetFile;
#[cfg(not(target_family = "wasm"))]
use warpui::ModelHandle;
use warpui::{Entity, ModelContext, SingletonEntity};
#[cfg(not(target_family = "wasm"))]
use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};

/// Duration between filesystem watch events for the Warp managed paths watcher, in milliseconds.
#[cfg(not(target_family = "wasm"))]
const WARP_MANAGED_PATHS_WATCHER_DEBOUNCE_MILLI_SECS: u64 = 500;

pub(crate) fn warp_data_dir() -> PathBuf {
    warp_core::paths::data_dir()
}

#[cfg(target_family = "wasm")]
pub(crate) fn ensure_warp_watch_roots_exist() {}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn ensure_warp_watch_roots_exist() {
    let data_dir = warp_data_dir();
    if let Err(err) = fs::create_dir_all(&data_dir) {
        log::warn!(
            "Failed to create Warp data directory {}: {err}",
            data_dir.display()
        );
    }

    let config_local_dir = warp_core::paths::config_local_dir();
    if config_local_dir != data_dir {
        if let Err(err) = fs::create_dir_all(&config_local_dir) {
            log::warn!(
                "Failed to create Warp config directory {}: {err}",
                config_local_dir.display()
            );
        }
    }
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn warp_home_config_dir() -> Option<PathBuf> {
    warp_core::paths::warp_home_config_dir()
}

pub(crate) fn warp_home_skills_dir() -> Option<PathBuf> {
    warp_core::paths::warp_home_skills_dir()
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn warp_home_mcp_config_file_path() -> Option<PathBuf> {
    warp_core::paths::warp_home_mcp_config_file_path()
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WarpMcpConfigPath {
    pub(crate) root_path: PathBuf,
    pub(crate) config_path: PathBuf,
}

pub(crate) fn warp_managed_skill_dirs() -> Vec<PathBuf> {
    warp_home_skills_dir().into_iter().collect()
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn warp_managed_mcp_config_path() -> Option<WarpMcpConfigPath> {
    Some(WarpMcpConfigPath {
        root_path: home_dir()?,
        config_path: warp_home_mcp_config_file_path()?,
    })
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn repository_update_touches_path(update: &RepositoryUpdate, path: &Path) -> bool {
    repository_update_paths(update).any(|candidate| candidate == path)
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn repository_update_touches_prefix(update: &RepositoryUpdate, prefix: &Path) -> bool {
    repository_update_paths(update).any(|candidate| candidate.starts_with(prefix))
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn filter_repository_update_by_prefix(
    update: &RepositoryUpdate,
    prefix: &Path,
) -> Option<RepositoryUpdate> {
    filter_repository_update(update, |path| path.starts_with(prefix))
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
fn repository_update_paths(update: &RepositoryUpdate) -> impl Iterator<Item = &Path> {
    update
        .added
        .iter()
        .map(|target| target.path.as_path())
        .chain(update.modified.iter().map(|target| target.path.as_path()))
        .chain(update.deleted.iter().map(|target| target.path.as_path()))
        .chain(update.moved.iter().flat_map(|(to_target, from_target)| {
            [to_target.path.as_path(), from_target.path.as_path()]
        }))
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
fn filter_repository_update(
    update: &RepositoryUpdate,
    keep_path: impl Fn(&Path) -> bool,
) -> Option<RepositoryUpdate> {
    let mut filtered = RepositoryUpdate {
        commit_updated: update.commit_updated,
        index_lock_detected: update.index_lock_detected,
        remote_ref_updated: update.remote_ref_updated,
        ..Default::default()
    };

    for target in &update.added {
        if keep_path(&target.path) {
            filtered.added.insert(target.clone());
        }
    }

    for target in &update.modified {
        if keep_path(&target.path) {
            filtered.modified.insert(target.clone());
        }
    }

    for target in &update.deleted {
        if keep_path(&target.path) {
            filtered.deleted.insert(target.clone());
        }
    }

    for (to_target, from_target) in &update.moved {
        let keep_to = keep_path(&to_target.path);
        let keep_from = keep_path(&from_target.path);

        match (keep_to, keep_from) {
            (true, true) => {
                filtered
                    .moved
                    .insert(to_target.clone(), from_target.clone());
            }
            (true, false) => {
                filtered.added.insert(to_target.clone());
            }
            (false, true) => {
                filtered.deleted.insert(from_target.clone());
            }
            (false, false) => {}
        }
    }

    (!filtered.is_empty()).then_some(filtered)
}

#[cfg(not(target_family = "wasm"))]
fn filesystem_event_to_repository_update(event: &BulkFilesystemWatcherEvent) -> RepositoryUpdate {
    RepositoryUpdate {
        added: event
            .added
            .iter()
            .cloned()
            .map(|path| TargetFile::new(path, false))
            .collect(),
        modified: event
            .modified
            .iter()
            .cloned()
            .map(|path| TargetFile::new(path, false))
            .collect(),
        deleted: event
            .deleted
            .iter()
            .cloned()
            .map(|path| TargetFile::new(path, false))
            .collect(),
        moved: event
            .moved
            .iter()
            .map(|(to_path, from_path)| {
                (
                    TargetFile::new(to_path.clone(), false),
                    TargetFile::new(from_path.clone(), false),
                )
            })
            .collect(),
        commit_updated: false,
        index_lock_detected: false,
        remote_ref_updated: false,
    }
}

#[cfg(target_family = "wasm")]
#[allow(dead_code)]
pub(crate) enum WarpManagedPathsWatcherEvent {}

#[cfg(not(target_family = "wasm"))]
pub(crate) enum WarpManagedPathsWatcherEvent {
    FilesChanged(RepositoryUpdate),
}

#[cfg(not(target_family = "wasm"))]
pub(crate) struct WarpManagedPathsWatcher {
    _watcher: ModelHandle<BulkFilesystemWatcher>,
}

#[cfg(target_family = "wasm")]
pub(crate) struct WarpManagedPathsWatcher;

#[cfg(not(target_family = "wasm"))]
impl WarpManagedPathsWatcher {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        Self::new_internal(ctx, true)
    }

    #[cfg(test)]
    pub(crate) fn new_for_testing(ctx: &mut ModelContext<Self>) -> Self {
        Self::new_internal(ctx, false)
    }

    fn new_internal(ctx: &mut ModelContext<Self>, should_register_watcher: bool) -> Self {
        let watcher = if should_register_watcher {
            ctx.add_model(|ctx| {
                BulkFilesystemWatcher::new(
                    Duration::from_millis(WARP_MANAGED_PATHS_WATCHER_DEBOUNCE_MILLI_SECS),
                    ctx,
                )
            })
        } else {
            ctx.add_model(|_| BulkFilesystemWatcher::new_for_test())
        };
        ctx.subscribe_to_model(&watcher, Self::handle_fs_event);

        if should_register_watcher {
            let data_dir = warp_data_dir();
            let config_local_dir = warp_core::paths::config_local_dir();
            let should_register_config_local_dir = config_local_dir != data_dir;
            let worktrees_dir = data_dir.join("worktrees");
            Self::register_path(
                ctx,
                &watcher,
                data_dir.clone(),
                WatchFilter::with_filter(Arc::new(move |path| !path.starts_with(&worktrees_dir))),
                RecursiveMode::Recursive,
                "Warp data directory",
            );
            if should_register_config_local_dir {
                Self::register_path(
                    ctx,
                    &watcher,
                    config_local_dir.clone(),
                    WatchFilter::accept_all(),
                    RecursiveMode::Recursive,
                    "Warp config directory",
                );
            }
            if let Some(warp_home_skills_dir) = warp_home_skills_dir() {
                if warp_home_skills_dir.exists()
                    && !warp_home_skills_dir.starts_with(&data_dir)
                    && (!should_register_config_local_dir
                        || !warp_home_skills_dir.starts_with(&config_local_dir))
                {
                    Self::register_path(
                        ctx,
                        &watcher,
                        warp_home_skills_dir,
                        WatchFilter::accept_all(),
                        RecursiveMode::Recursive,
                        "Warp home skills directory",
                    );
                }
            }
            if let (Some(warp_home_config_dir), Some(warp_home_mcp_config_path)) =
                (warp_home_config_dir(), warp_home_mcp_config_file_path())
            {
                if warp_home_config_dir.exists()
                    && !warp_home_config_dir.starts_with(&data_dir)
                    && (!should_register_config_local_dir
                        || !warp_home_config_dir.starts_with(&config_local_dir))
                {
                    Self::register_path(
                        ctx,
                        &watcher,
                        warp_home_config_dir,
                        WatchFilter::with_filter(Arc::new(move |path| {
                            path == warp_home_mcp_config_path
                        })),
                        RecursiveMode::NonRecursive,
                        "Warp home MCP config directory",
                    );
                }
            }
        }

        Self { _watcher: watcher }
    }

    fn register_path(
        ctx: &mut ModelContext<Self>,
        watcher: &ModelHandle<BulkFilesystemWatcher>,
        directory_path: PathBuf,
        watch_filter: WatchFilter,
        recursive_mode: RecursiveMode,
        description: &'static str,
    ) {
        let registration_path = directory_path.clone();
        let registration = watcher.update(ctx, |watcher, _ctx| {
            watcher.register_path(&registration_path, watch_filter, recursive_mode)
        });

        ctx.spawn(registration, move |_, result, _ctx| {
            if let Err(err) = result {
                log::warn!(
                    "Failed to start watching {description} {}: {err}",
                    directory_path.display()
                );
            }
        });
    }

    fn handle_fs_event(
        &mut self,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let update = filesystem_event_to_repository_update(event);
        if !update.is_empty() {
            ctx.emit(WarpManagedPathsWatcherEvent::FilesChanged(update));
        }
    }
}

#[cfg(target_family = "wasm")]
impl WarpManagedPathsWatcher {
    pub(crate) fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self
    }

    #[cfg(test)]
    pub(crate) fn new_for_testing(_ctx: &mut ModelContext<Self>) -> Self {
        Self
    }
}

impl Entity for WarpManagedPathsWatcher {
    type Event = WarpManagedPathsWatcherEvent;
}

impl SingletonEntity for WarpManagedPathsWatcher {}

#[cfg(test)]
mod tests {
    use dirs::home_dir;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    use repo_metadata::{RepositoryUpdate, TargetFile};

    use super::{
        filter_repository_update_by_prefix, warp_home_mcp_config_file_path, warp_home_skills_dir,
        warp_managed_mcp_config_path, warp_managed_skill_dirs,
    };

    #[test]
    fn warp_managed_skill_dirs_contains_only_warp_home_path() {
        let dirs = warp_managed_skill_dirs();
        match warp_home_skills_dir() {
            Some(warp_home_skills_dir) => assert_eq!(dirs, vec![warp_home_skills_dir]),
            None => assert!(dirs.is_empty()),
        }
    }

    #[test]
    fn warp_managed_mcp_config_path_contains_only_warp_home_path() {
        match (
            home_dir(),
            warp_home_mcp_config_file_path(),
            warp_managed_mcp_config_path(),
        ) {
            (Some(home_dir), Some(warp_home_mcp_config_path), Some(path)) => {
                assert_eq!(path.root_path, home_dir);
                assert_eq!(path.config_path, warp_home_mcp_config_path);
            }
            (_, _, None) => {}
            _ => panic!("Expected Warp MCP path when home directory is available"),
        }
    }

    #[test]
    fn filter_repository_update_by_prefix_keeps_only_matching_paths() {
        let skills_dir = PathBuf::from("/tmp/.warp-local/skills");
        let other_dir = PathBuf::from("/tmp/.warp-local/worktrees/repo");
        let skill_file = skills_dir.join("deploy").join("SKILL.md");
        let other_file = other_dir.join("README.md");

        let update = RepositoryUpdate {
            added: HashSet::from([
                TargetFile::new(skill_file.clone(), false),
                TargetFile::new(other_file.clone(), false),
            ]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        let filtered =
            filter_repository_update_by_prefix(&update, &skills_dir).expect("expected update");

        assert!(filtered.contains_added_or_modified(&TargetFile::new(skill_file, false)));
        assert!(!filtered.contains_added_or_modified(&TargetFile::new(other_file, false)));
    }

    #[test]
    fn filter_repository_update_by_prefix_converts_cross_boundary_moves() {
        let skills_dir = PathBuf::from("/tmp/.warp-local/skills");
        let skill_file = skills_dir.join("deploy").join("SKILL.md");
        let ignored_file = PathBuf::from("/tmp/.warp-local/worktrees/repo/SKILL.md");

        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::from([(
                TargetFile::new(skill_file.clone(), false),
                TargetFile::new(ignored_file, false),
            )]),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        let filtered =
            filter_repository_update_by_prefix(&update, &skills_dir).expect("expected update");

        assert!(filtered.contains_added_or_modified(&TargetFile::new(skill_file, false)));
        assert!(filtered.moved.is_empty());
        assert!(filtered.deleted.is_empty());
    }
}
