//! Discovery of project hook files. The caller supplies the roots to watch; the watcher reads
//! `<root>/.cursor/hooks.json` at each one, reloads it when it changes, and keeps the registry in
//! step. It never decides what a project is and never watches the home directory.

use crate::hooks::model::HookSurface;

/// Whether a root discovered on `surface` gets a hook file watch. Roots prepared for a cloud
/// run are watched only in command-line mode; on interactive surfaces they are left alone, as
/// the file-based MCP watcher does in the TUI.
pub fn watch_matrix(surface: HookSurface, is_cloud_prepared_root: bool) -> bool {
    match surface {
        HookSurface::OzRun | HookSurface::CliRun => true,
        HookSurface::LocalChild | HookSurface::Gui | HookSurface::Tui => !is_cloud_prepared_root,
    }
}

#[cfg(feature = "local_fs")]
pub use native::{HOOK_CONFIG_DEBOUNCE, HookConfigWatcher};

#[cfg(feature = "local_fs")]
mod native {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
    use warpui_core::{Entity, ModelContext, ModelHandle};
    use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};

    use crate::hooks::model::{
        HookFormat, HookSourceId, HookSourceKind, HookSourceLocation, HookSurface,
    };
    use crate::hooks::registry::{HookRegistry, SourceUpdate};
    use crate::hooks::sources::PROJECT_CURSOR_HOOKS_PATH;
    use crate::hooks::trust::{CommandExecutionPosture, TrustPolicy};

    /// How long the file watcher waits after the last change before reporting a batch, so a
    /// burst of writes to one file loads it once.
    pub const HOOK_CONFIG_DEBOUNCE: Duration = Duration::from_millis(500);

    /// Watches the project hook file under each root it is given and loads it into the registry.
    ///
    /// Loads are synchronous: the file is small and changes rarely, and reading it on the spot
    /// means a root added before the run is pinned is in the registry before the pin happens.
    /// Trust decisions come from the policy, the process's surface, and the run's command
    /// execution posture; the posture is unknown until the run starts, which keeps project files
    /// inert until then.
    pub struct HookConfigWatcher {
        registry: ModelHandle<HookRegistry>,
        fs_watcher: ModelHandle<BulkFilesystemWatcher>,
        trust: Arc<dyn TrustPolicy>,
        surface: HookSurface,
        posture: CommandExecutionPosture,
        /// Watched roots, canonical, and the source each currently provides.
        roots: BTreeMap<PathBuf, Option<HookSourceId>>,
    }

    impl HookConfigWatcher {
        pub fn new(
            registry: ModelHandle<HookRegistry>,
            trust: Arc<dyn TrustPolicy>,
            surface: HookSurface,
            ctx: &mut ModelContext<Self>,
        ) -> Self {
            let fs_watcher =
                ctx.add_model(|ctx| BulkFilesystemWatcher::new(HOOK_CONFIG_DEBOUNCE, ctx));
            Self::with_fs_watcher(registry, fs_watcher, trust, surface, ctx)
        }

        /// A watcher whose file system events come only from [`Self::handle_fs_event`], so a
        /// test can drive it without waiting on the platform watcher.
        #[cfg(any(test, feature = "test-util"))]
        pub fn new_without_filesystem_events(
            registry: ModelHandle<HookRegistry>,
            trust: Arc<dyn TrustPolicy>,
            surface: HookSurface,
            ctx: &mut ModelContext<Self>,
        ) -> Self {
            let fs_watcher = ctx.add_model(|_| BulkFilesystemWatcher::new_for_test());
            Self::with_fs_watcher(registry, fs_watcher, trust, surface, ctx)
        }

        fn with_fs_watcher(
            registry: ModelHandle<HookRegistry>,
            fs_watcher: ModelHandle<BulkFilesystemWatcher>,
            trust: Arc<dyn TrustPolicy>,
            surface: HookSurface,
            ctx: &mut ModelContext<Self>,
        ) -> Self {
            ctx.subscribe_to_model(&fs_watcher, |me, _, event, ctx| {
                me.handle_fs_event(event, ctx);
            });
            Self {
                registry,
                fs_watcher,
                trust,
                surface,
                posture: CommandExecutionPosture::Unknown,
                roots: BTreeMap::new(),
            }
        }

        pub fn surface(&self) -> HookSurface {
            self.surface
        }

        pub fn posture(&self) -> CommandExecutionPosture {
            self.posture
        }

        /// The roots being watched.
        pub fn roots(&self) -> impl Iterator<Item = &Path> {
            self.roots.keys().map(PathBuf::as_path)
        }

        /// Starts watching `root`, makes it a project the registry knows, and loads its hook
        /// file now. A root that is already watched is left alone.
        pub fn add_root(&mut self, root: &Path, ctx: &mut ModelContext<Self>) {
            let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            if self.roots.contains_key(&root) {
                return;
            }
            self.roots.insert(root.clone(), None);
            self.fs_watcher.update(ctx, |fs_watcher, _| {
                std::mem::drop(fs_watcher.register_path(
                    &root,
                    hook_file_filter(&root),
                    RecursiveMode::Recursive,
                ));
            });
            HookRegistry::change(&self.registry, ctx, |registry| {
                registry.add_project_root(&root)
            });
            self.load_root(&root, ctx);
        }

        /// Stops watching `root` and withdraws it and its hook file from the registry.
        pub fn remove_root(&mut self, root: &Path, ctx: &mut ModelContext<Self>) {
            let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            let Some(source) = self.roots.remove(&root) else {
                return;
            };
            self.fs_watcher.update(ctx, |fs_watcher, _| {
                std::mem::drop(fs_watcher.unregister_path(&root));
            });
            if let Some(source) = source {
                HookRegistry::change(&self.registry, ctx, |registry| {
                    registry.remove_source_root(&source, &root)
                });
            }
            HookRegistry::change(&self.registry, ctx, |registry| {
                registry.remove_project_root(&root)
            });
        }

        /// Records the run's surface and command execution posture and decides trust again for
        /// every loaded root. Meant to run once, right before the registry is pinned.
        pub fn set_trust_inputs(
            &mut self,
            surface: HookSurface,
            posture: CommandExecutionPosture,
            ctx: &mut ModelContext<Self>,
        ) {
            self.surface = surface;
            self.posture = posture;
            let sources = self.roots.values().flatten().cloned().collect::<Vec<_>>();
            for source in sources {
                let trust = self.trust.may_activate(&source, self.surface, self.posture);
                HookRegistry::change(&self.registry, ctx, |registry| {
                    registry.set_trust(&source, trust)
                });
            }
        }

        /// Reloads every root whose hook file, or `.cursor` directory, the event touched. The
        /// current state on disk decides what happens, so an atomic replacement reported as a
        /// delete and an add loads once and never drops the source in between.
        pub fn handle_fs_event(
            &mut self,
            event: &BulkFilesystemWatcherEvent,
            ctx: &mut ModelContext<Self>,
        ) {
            let touched = self
                .roots
                .keys()
                .filter(|root| touches_hook_file(event, root))
                .cloned()
                .collect::<Vec<_>>();
            for root in touched {
                self.load_root(&root, ctx);
            }
        }

        fn load_root(&mut self, root: &Path, ctx: &mut ModelContext<Self>) {
            let path = root.join(PROJECT_CURSOR_HOOKS_PATH);
            let previous = self.roots.get(root).cloned().flatten();
            let current = dunce::canonicalize(&path)
                .ok()
                .map(|canonical| HookSourceId {
                    kind: HookSourceKind::ProjectCursorFile,
                    location: HookSourceLocation::CanonicalPath(canonical),
                });

            if let Some(previous) = &previous
                && current.as_ref() != Some(previous)
            {
                HookRegistry::change(&self.registry, ctx, |registry| {
                    registry.remove_source_root(previous, root)
                });
            }
            if let Some(source) = &current {
                let trust = self.trust.may_activate(source, self.surface, self.posture);
                let canonical = match &source.location {
                    HookSourceLocation::CanonicalPath(canonical) => canonical.clone(),
                    HookSourceLocation::Named(name) => PathBuf::from(name),
                };
                match fs::read(&canonical) {
                    Ok(bytes) => {
                        HookRegistry::change(&self.registry, ctx, |registry| {
                            registry.upsert_source(SourceUpdate {
                                source: source.clone(),
                                format: HookFormat::Cursor,
                                bytes: &bytes,
                                project_root: Some(root.to_path_buf()),
                                trust,
                            })
                        });
                    }
                    Err(error) => {
                        let message = format!("{}: {error}", path.display());
                        HookRegistry::change(&self.registry, ctx, |registry| {
                            registry.record_read_failure(source, &message)
                        });
                    }
                }
            }
            self.roots.insert(root.to_path_buf(), current);
        }
    }

    impl Entity for HookConfigWatcher {
        type Event = ();
    }

    /// Watches only the root itself and its `.cursor` directory, and reports only the hook file
    /// and that directory, so a large repository costs no more than a small one.
    fn hook_file_filter(root: &Path) -> WatchFilter {
        let cursor_dir = root.join(".cursor");
        let hooks_file = root.join(PROJECT_CURSOR_HOOKS_PATH);
        let root = root.to_path_buf();
        let watched_dirs = {
            let cursor_dir = cursor_dir.clone();
            move |path: &Path| path == root || path == cursor_dir
        };
        let reported_paths = move |path: &Path| path == cursor_dir || path == hooks_file;
        WatchFilter::with_filter(Arc::new(watched_dirs), Arc::new(reported_paths))
    }

    fn touches_hook_file(event: &BulkFilesystemWatcherEvent, root: &Path) -> bool {
        let cursor_dir = root.join(".cursor");
        let hooks_file = root.join(PROJECT_CURSOR_HOOKS_PATH);
        let is_target = |path: &PathBuf| *path == hooks_file || *path == cursor_dir;
        event.added_or_updated_iter().any(is_target)
            || event.deleted.iter().any(is_target)
            || event.moved.keys().any(is_target)
            || event.moved.values().any(is_target)
    }
}

#[cfg(test)]
#[path = "watcher_tests.rs"]
mod tests;
