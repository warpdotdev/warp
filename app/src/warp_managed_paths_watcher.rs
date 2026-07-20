use std::path::{Path, PathBuf};
#[cfg(not(target_family = "wasm"))]
use std::{fs, sync::Arc, time::Duration};

use dirs::home_dir;
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
    if config_local_dir != data_dir
        && let Err(err) = fs::create_dir_all(&config_local_dir)
    {
        log::warn!(
            "Failed to create Warp config directory {}: {err}",
            config_local_dir.display()
        );
    }

    // The TUI surface stores its settings in a separate config directory
    // (see `warp_core::paths::tui_config_local_dir`). Create it up front — only
    // for that surface — so the watcher can register it at startup and pick up
    // the first settings write.
    if settings::settings_mode() == settings::SettingsMode::Tui {
        let tui_config_local_dir = warp_core::paths::tui_config_local_dir();
        if let Err(err) = fs::create_dir_all(&tui_config_local_dir) {
            log::warn!(
                "Failed to create Warp TUI config directory {}: {err}",
                tui_config_local_dir.display()
            );
        }
    }
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn warp_home_skills_dir() -> Option<PathBuf> {
    warp_core::paths::warp_home_skills_dir()
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn warp_home_mcp_config_file_path() -> Option<PathBuf> {
    warp_core::paths::warp_home_mcp_config_file_path()
}
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn active_mcp_config_file_path() -> Option<PathBuf> {
    match settings::settings_mode() {
        settings::SettingsMode::Gui => warp_home_mcp_config_file_path(),
        settings::SettingsMode::Tui => Some(warp_core::paths::tui_mcp_config_file_path()),
    }
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WarpMcpConfigPath {
    pub(crate) root_path: PathBuf,
    pub(crate) config_path: PathBuf,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn warp_managed_skill_dirs() -> Vec<PathBuf> {
    warp_home_skills_dir().into_iter().collect()
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) fn warp_managed_mcp_config_path() -> Option<WarpMcpConfigPath> {
    Some(WarpMcpConfigPath {
        root_path: home_dir()?,
        config_path: active_mcp_config_file_path()?,
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

    #[cfg(any(test, all(feature = "tui", feature = "test-util")))]
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
            // Safe to use for both directory registration and event emission.
            // If this rejects `worktrees_dir`, every descendant should be rejected too,
            // so the recursive watcher never prunes an ancestor needed to reach an allowed path.
            let filter = Arc::new(move |path: &Path| !path.starts_with(&worktrees_dir));
            Self::register_path(
                ctx,
                &watcher,
                data_dir.clone(),
                WatchFilter::with_filter(filter.clone(), filter),
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
            // Watch the TUI settings directory for that surface. On macOS it's
            // a sibling `.warp_cli*` directory outside `config_local_dir`; on
            // other platforms it nests under `config_local_dir` and is already
            // covered by the recursive watch above (the `starts_with` guard
            // skips the redundant registration).
            if settings::settings_mode() == settings::SettingsMode::Tui {
                let tui_config_local_dir = warp_core::paths::tui_config_local_dir();
                if tui_config_local_dir.exists()
                    && !tui_config_local_dir.starts_with(&data_dir)
                    && (!should_register_config_local_dir
                        || !tui_config_local_dir.starts_with(&config_local_dir))
                {
                    Self::register_path(
                        ctx,
                        &watcher,
                        tui_config_local_dir,
                        WatchFilter::accept_all(),
                        RecursiveMode::Recursive,
                        "Warp TUI config directory",
                    );
                }
            }
            if let Some(warp_home_skills_dir) = warp_home_skills_dir()
                && warp_home_skills_dir.exists()
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
            if let Some(active_mcp_config_path) = active_mcp_config_file_path()
                && let Some(active_mcp_config_dir) =
                    active_mcp_config_path.parent().map(Path::to_path_buf)
                && active_mcp_config_dir.exists()
                && !active_mcp_config_dir.starts_with(&data_dir)
                && (!should_register_config_local_dir
                    || !active_mcp_config_dir.starts_with(&config_local_dir))
            {
                // Watch the config directory non-recursively,
                // and ignore events for files other than the MCP config file.
                let emit = Arc::new(move |path: &Path| path == active_mcp_config_path);
                Self::register_path(
                    ctx,
                    &watcher,
                    active_mcp_config_dir,
                    WatchFilter::with_filter(Arc::new(|_: &Path| true), emit),
                    RecursiveMode::NonRecursive,
                    "Warp MCP config directory",
                );
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
        _: ModelHandle<BulkFilesystemWatcher>,
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
#[path = "warp_managed_paths_watcher_tests.rs"]
mod tests;
