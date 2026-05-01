//! Singleton that watches the OS-side shell history files (`~/.zsh_history`,
//! `~/.bash_history`, fish, PSReadLine) for changes made by *other* terminals
//! and forwards modify events to subscribers.
//!
//! The existing [`watcher::HomeDirectoryWatcher`] only watches `$HOME`
//! non-recursively, which covers bash and zsh but not fish
//! (`~/.local/share/fish/fish_history`) or PSReadLine
//! (`~/.local/share/powershell/PSReadLine/...`). Rather than special-case those
//! paths inside the home watcher, this singleton wraps a dedicated
//! [`BulkFilesystemWatcher`] and exposes a simple `register_histfile` /
//! `unregister_histfile` API that the [`super::history::History`] model calls
//! per-session.
//!
//! See GH-3422.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};
use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};

/// Debounce duration for histfile watch events. Histfiles can be appended to
/// many times in quick succession (long pipelines, scripts) — debouncing keeps
/// the merge work cheap without losing observability.
const SHELL_HISTORY_WATCHER_DEBOUNCE_MS: u64 = 500;

/// Event emitted when one of the registered histfiles changes on disk.
///
/// Carries the underlying [`BulkFilesystemWatcherEvent`] so subscribers can
/// inspect exactly which paths were added / modified / deleted.
#[derive(Clone, Debug)]
pub enum ShellHistoryWatcherEvent {
    /// One or more registered histfile paths changed.
    HistfilesChanged(BulkFilesystemWatcherEvent),
}

/// Singleton that registers individual shell history files with an underlying
/// [`BulkFilesystemWatcher`] and re-emits filesystem events under a typed
/// [`ShellHistoryWatcherEvent`].
pub struct ShellHistoryWatcher {
    watcher: ModelHandle<BulkFilesystemWatcher>,
    /// Reference count per registered path: same histfile may be opened by
    /// multiple sessions, and we only `unregister_path` once the last one
    /// drops.
    refcounts: HashMap<PathBuf, usize>,
}

impl ShellHistoryWatcher {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let watcher = ctx.add_model(|ctx| {
            BulkFilesystemWatcher::new(
                Duration::from_millis(SHELL_HISTORY_WATCHER_DEBOUNCE_MS),
                ctx,
            )
        });
        ctx.subscribe_to_model(&watcher, Self::handle_fs_event);

        Self {
            watcher,
            refcounts: HashMap::new(),
        }
    }

    /// Test-only constructor that uses the stub filesystem watcher (no
    /// background thread).
    pub fn new_for_test(ctx: &mut ModelContext<Self>) -> Self {
        let watcher = ctx.add_model(|_| BulkFilesystemWatcher::new_for_test());
        Self {
            watcher,
            refcounts: HashMap::new(),
        }
    }

    /// Begin watching `path` for filesystem changes. Idempotent — calling it
    /// twice for the same path bumps an internal refcount; the path is only
    /// passed to the underlying watcher on the first call.
    pub fn register_histfile(&mut self, path: &Path, ctx: &mut ModelContext<Self>) {
        let entry = self.refcounts.entry(path.to_path_buf()).or_insert(0);
        *entry += 1;
        if *entry == 1 {
            let path = path.to_path_buf();
            self.watcher.update(ctx, |watcher, _ctx| {
                std::mem::drop(watcher.register_path(
                    &path,
                    WatchFilter::accept_all(),
                    RecursiveMode::NonRecursive,
                ));
            });
        }
    }

    /// Decrement the refcount for `path`. When it hits zero the path is
    /// passed to the underlying watcher's `unregister_path`.
    pub fn unregister_histfile(&mut self, path: &Path, ctx: &mut ModelContext<Self>) {
        let Some(count) = self.refcounts.get_mut(path) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.refcounts.remove(path);
            let path = path.to_path_buf();
            self.watcher.update(ctx, |watcher, _ctx| {
                std::mem::drop(watcher.unregister_path(&path));
            });
        }
    }

    fn handle_fs_event(
        &mut self,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.emit(ShellHistoryWatcherEvent::HistfilesChanged(event.clone()));
    }
}

impl Entity for ShellHistoryWatcher {
    type Event = ShellHistoryWatcherEvent;
}

impl SingletonEntity for ShellHistoryWatcher {}
