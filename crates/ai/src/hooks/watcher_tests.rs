#![cfg(feature = "local_fs")]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use instant::Instant;
use tempfile::TempDir;
use warpui_core::r#async::Timer;
use warpui_core::{App, ModelHandle};
use watcher::BulkFilesystemWatcherEvent;

use super::*;
use crate::hooks::model::{
    DiagnosticSeverity, HookBoundary, HookExecutionHost, HookSourceKind, HookSurface,
};
use crate::hooks::registry::{HookRegistry, HookRegistryEvent, RegistrySnapshot};
use crate::hooks::sources::PROJECT_CURSOR_HOOKS_PATH;
use crate::hooks::test_util::StaticTrust;
use crate::hooks::trust::{CommandExecutionPosture, StaticTrustPolicy, TrustDecision, TrustPolicy};

const VALID: &str =
    r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./hook.sh" } ] } }"#;
const VALID_TWO: &str = r#"{ "version": 1, "hooks": { "beforeShellExecution": [
    { "command": "./hook.sh" }, { "command": "./other.sh" } ] } }"#;

#[test]
fn the_watch_matrix_watches_cloud_prepared_roots_only_in_command_line_mode() {
    for surface in [HookSurface::OzRun, HookSurface::CliRun] {
        assert!(watch_matrix(surface, true), "{surface:?}");
        assert!(watch_matrix(surface, false), "{surface:?}");
    }
    for surface in [HookSurface::Gui, HookSurface::Tui, HookSurface::LocalChild] {
        assert!(!watch_matrix(surface, true), "{surface:?}");
        assert!(watch_matrix(surface, false), "{surface:?}");
    }
}

/// A canonical temp root. The hook file is written and removed by the tests themselves.
struct Root {
    _dir: TempDir,
    path: PathBuf,
}

impl Root {
    fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let path = dunce::canonicalize(dir.path()).expect("canonical");
        Self { _dir: dir, path }
    }

    fn with_hooks(hooks_json: &str) -> Self {
        let root = Self::new();
        root.write(hooks_json);
        root
    }

    fn hooks_file(&self) -> PathBuf {
        self.path.join(PROJECT_CURSOR_HOOKS_PATH)
    }

    fn write(&self, hooks_json: &str) {
        fs::create_dir_all(self.path.join(".cursor")).expect("mkdir");
        fs::write(self.hooks_file(), hooks_json).expect("write");
    }

    fn remove(&self) {
        fs::remove_file(self.hooks_file()).expect("remove");
    }
}

fn added(path: PathBuf) -> BulkFilesystemWatcherEvent {
    BulkFilesystemWatcherEvent {
        added: HashSet::from([path]),
        ..Default::default()
    }
}

fn modified(path: PathBuf) -> BulkFilesystemWatcherEvent {
    BulkFilesystemWatcherEvent {
        modified: HashSet::from([path]),
        ..Default::default()
    }
}

fn deleted(path: PathBuf) -> BulkFilesystemWatcherEvent {
    BulkFilesystemWatcherEvent {
        deleted: HashSet::from([path]),
        ..Default::default()
    }
}

struct Harness {
    registry: ModelHandle<HookRegistry>,
    watcher: ModelHandle<HookConfigWatcher>,
    /// How many `SnapshotChanged` events the registry emitted.
    changes: Arc<AtomicUsize>,
}

impl Harness {
    fn new(app: &mut App, trust: Arc<dyn TrustPolicy>, surface: HookSurface) -> Self {
        Self::build(app, trust, surface, false)
    }

    /// A harness whose watcher listens to the real file system.
    fn with_real_watcher(app: &mut App) -> Self {
        Self::build(
            app,
            Arc::new(StaticTrust(TrustDecision::Activate)),
            HookSurface::CliRun,
            true,
        )
    }

    fn build(
        app: &mut App,
        trust: Arc<dyn TrustPolicy>,
        surface: HookSurface,
        real_filesystem_events: bool,
    ) -> Self {
        let registry = app.add_model(|_| HookRegistry::new());
        let changes = Arc::new(AtomicUsize::new(0));
        let counter = changes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&registry, move |_, event, _| {
                let HookRegistryEvent::SnapshotChanged { .. } = event;
                counter.fetch_add(1, Ordering::SeqCst);
            });
        });
        let watcher = app.add_model(|ctx| {
            if real_filesystem_events {
                HookConfigWatcher::new(registry.clone(), trust, surface, ctx)
            } else {
                HookConfigWatcher::new_without_filesystem_events(
                    registry.clone(),
                    trust,
                    surface,
                    ctx,
                )
            }
        });
        Self {
            registry,
            watcher,
            changes,
        }
    }

    fn activating(app: &mut App) -> Self {
        Self::new(
            app,
            Arc::new(StaticTrust(TrustDecision::Activate)),
            HookSurface::CliRun,
        )
    }

    fn add_root(&self, app: &mut App, root: &Path) {
        self.watcher
            .update(app, |watcher, ctx| watcher.add_root(root, ctx));
    }

    fn event(&self, app: &mut App, event: BulkFilesystemWatcherEvent) {
        self.watcher
            .update(app, |watcher, ctx| watcher.handle_fs_event(&event, ctx));
    }

    fn snapshot(&self, app: &App) -> Arc<RegistrySnapshot> {
        self.registry.read(app, |registry, _| registry.snapshot())
    }

    fn revision(&self, app: &App) -> u64 {
        self.snapshot(app).revision
    }

    fn tool_pre_bindings_under(&self, app: &App, root: &Path) -> usize {
        self.snapshot(app)
            .bindings_for(
                HookBoundary::ToolPre,
                Some(&root.join("src")),
                HookExecutionHost::Local,
            )
            .len()
    }

    /// Yields to the app until the revision moves past `from`, or `timeout` passes.
    async fn wait_for_revision_past(&self, app: &mut App, from: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.revision(app) > from {
                return true;
            }
            Timer::after(Duration::from_millis(25)).await;
        }
        false
    }
}

#[test]
fn adding_a_root_loads_its_hook_file_scoped_to_that_root() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &root.path);

        let snapshot = harness.snapshot(&app);
        assert_eq!(
            snapshot.revision, 2,
            "the root is announced, then its file loads"
        );
        assert_eq!(snapshot.bindings.len(), 1);
        let binding = &snapshot.bindings[0];
        assert_eq!(
            binding.binding.id.source.kind,
            HookSourceKind::ProjectCursorFile
        );
        assert_eq!(binding.working_directory, root.path);
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 1);
        assert_eq!(snapshot.protected_paths, vec![root.hooks_file()]);
        assert_eq!(snapshot.project_roots, vec![root.path.clone()]);
        assert_eq!(harness.changes.load(Ordering::SeqCst), 2);
        assert_eq!(
            harness
                .watcher
                .read(&app, |watcher, _| watcher.roots().count()),
            1
        );
    });
}

#[test]
fn adding_the_same_root_twice_loads_it_once() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &root.path);
        harness.add_root(&mut app, &root.path);
        assert_eq!(harness.revision(&app), 2);
    });
}

// Criterion 10: create, modify, and delete each produce one registry update.
#[test]
fn create_modify_and_delete_each_update_the_registry_once() {
    App::test((), |mut app| async move {
        let root = Root::new();
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &root.path);
        assert_eq!(
            harness.revision(&app),
            1,
            "the root is a known project; there is no file to load"
        );
        assert_eq!(
            harness.snapshot(&app).project_roots,
            vec![root.path.clone()]
        );

        root.write(VALID);
        harness.event(&mut app, added(root.hooks_file()));
        assert_eq!(harness.revision(&app), 2);
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 1);

        root.write(VALID_TWO);
        harness.event(&mut app, modified(root.hooks_file()));
        assert_eq!(harness.revision(&app), 3);
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 2);

        root.remove();
        harness.event(&mut app, deleted(root.hooks_file()));
        assert_eq!(harness.revision(&app), 4);
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 0);
        assert!(harness.snapshot(&app).protected_paths.is_empty());
        assert_eq!(
            harness.snapshot(&app).project_roots,
            vec![root.path.clone()],
            "the directory is still a project without its file"
        );
        assert_eq!(harness.changes.load(Ordering::SeqCst), 4);
    });
}

#[test]
fn one_event_that_touches_the_file_several_ways_loads_it_once() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &root.path);

        // An atomic replacement shows up as a delete plus an add of the same path.
        root.write(VALID_TWO);
        harness.event(
            &mut app,
            BulkFilesystemWatcherEvent {
                added: HashSet::from([root.hooks_file()]),
                modified: HashSet::from([root.hooks_file()]),
                deleted: HashSet::from([root.hooks_file()]),
                moved: HashMap::new(),
            },
        );
        assert_eq!(harness.revision(&app), 3);
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 2);
    });
}

#[test]
fn an_event_for_an_unrelated_path_does_not_reload() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &root.path);
        harness.event(&mut app, modified(root.path.join("src/main.rs")));
        harness.event(&mut app, modified(root.path.join(".cursor/rules.md")));
        assert_eq!(harness.revision(&app), 2);
    });
}

#[test]
fn removing_the_cursor_directory_withdraws_the_source() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &root.path);
        fs::remove_dir_all(root.path.join(".cursor")).expect("remove .cursor");
        harness.event(&mut app, deleted(root.path.join(".cursor")));
        assert_eq!(harness.revision(&app), 3);
        assert!(harness.snapshot(&app).is_empty());
    });
}

#[test]
fn a_rewrite_that_does_not_parse_keeps_the_previous_bindings() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &root.path);
        root.write("{ not json");
        harness.event(&mut app, modified(root.hooks_file()));
        let snapshot = harness.snapshot(&app);
        assert_eq!(snapshot.revision, 3);
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 1);
        assert!(
            snapshot
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        );
    });
}

#[test]
fn two_roots_are_loaded_and_scoped_independently() {
    App::test((), |mut app| async move {
        let first = Root::with_hooks(VALID);
        let second = Root::with_hooks(VALID_TWO);
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &first.path);
        harness.add_root(&mut app, &second.path);
        assert_eq!(harness.tool_pre_bindings_under(&app, &first.path), 1);
        assert_eq!(harness.tool_pre_bindings_under(&app, &second.path), 2);

        harness.event(&mut app, modified(second.hooks_file()));
        assert_eq!(harness.revision(&app), 5);
        assert_eq!(harness.tool_pre_bindings_under(&app, &first.path), 1);
    });
}

/// A nested repository is its own project. Without a hook file it has no project hooks, and the
/// enclosing project's hooks do not reach into it.
#[test]
fn a_nested_root_without_a_hook_file_is_its_own_project() {
    App::test((), |mut app| async move {
        let outer = Root::with_hooks(VALID);
        let inner = outer.path.join("vendor/inner");
        fs::create_dir_all(&inner).expect("mkdir");
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &outer.path);
        assert_eq!(harness.tool_pre_bindings_under(&app, &inner), 1);

        harness.add_root(&mut app, &inner);
        assert_eq!(harness.tool_pre_bindings_under(&app, &inner), 0);
        assert_eq!(harness.tool_pre_bindings_under(&app, &outer.path), 1);

        fs::create_dir_all(inner.join(".cursor")).expect("mkdir");
        fs::write(inner.join(PROJECT_CURSOR_HOOKS_PATH), VALID_TWO).expect("write");
        harness.event(&mut app, added(inner.join(PROJECT_CURSOR_HOOKS_PATH)));
        assert_eq!(harness.tool_pre_bindings_under(&app, &inner), 2);
        assert_eq!(harness.tool_pre_bindings_under(&app, &outer.path), 1);

        harness
            .watcher
            .update(&mut app, |watcher, ctx| watcher.remove_root(&inner, ctx));
        assert_eq!(
            harness.tool_pre_bindings_under(&app, &inner),
            1,
            "once the inner directory is no longer a project, the outer one's hooks apply"
        );
    });
}

#[test]
fn removing_a_root_withdraws_its_source() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &root.path);
        harness.watcher.update(&mut app, |watcher, ctx| {
            watcher.remove_root(&root.path, ctx)
        });
        assert!(harness.snapshot(&app).is_empty());
        assert!(harness.snapshot(&app).project_roots.is_empty());
        assert_eq!(
            harness
                .watcher
                .read(&app, |watcher, _| watcher.roots().count()),
            0
        );
    });
}

/// The static policy keeps a project file inert until the run's posture is known, and then
/// activates it only for a headless run that auto-approves commands.
#[test]
fn trust_is_decided_again_from_the_posture_when_the_run_starts() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::new(&mut app, Arc::new(StaticTrustPolicy), HookSurface::CliRun);
        harness.add_root(&mut app, &root.path);
        assert!(harness.snapshot(&app).is_empty(), "the posture is unknown");
        assert!(
            harness
                .snapshot(&app)
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("not known yet"))
        );

        harness.watcher.update(&mut app, |watcher, ctx| {
            watcher.set_trust_inputs(
                HookSurface::CliRun,
                CommandExecutionPosture::AutoApproves,
                ctx,
            )
        });
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 1);
        assert_eq!(
            harness.watcher.read(&app, |watcher, _| watcher.posture()),
            CommandExecutionPosture::AutoApproves
        );

        harness.watcher.update(&mut app, |watcher, ctx| {
            watcher.set_trust_inputs(
                HookSurface::CliRun,
                CommandExecutionPosture::RequiresApproval,
                ctx,
            )
        });
        assert!(harness.snapshot(&app).is_empty());

        harness.watcher.update(&mut app, |watcher, ctx| {
            watcher.set_trust_inputs(HookSurface::Gui, CommandExecutionPosture::AutoApproves, ctx)
        });
        assert!(
            harness.snapshot(&app).is_empty(),
            "an interactive surface never activates a project file"
        );
    });
}

#[test]
fn a_root_added_after_the_posture_is_known_uses_it() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::new(&mut app, Arc::new(StaticTrustPolicy), HookSurface::OzRun);
        harness.watcher.update(&mut app, |watcher, ctx| {
            watcher.set_trust_inputs(
                HookSurface::OzRun,
                CommandExecutionPosture::AutoApproves,
                ctx,
            )
        });
        harness.add_root(&mut app, &root.path);
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 1);
    });
}

#[test]
fn a_change_after_the_run_is_pinned_is_reported_and_not_applied() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::activating(&mut app);
        harness.add_root(&mut app, &root.path);
        HookRegistry::change(&harness.registry, &mut app, |registry| {
            registry.pin(CommandExecutionPosture::AutoApproves)
        });

        root.write(VALID_TWO);
        harness.event(&mut app, modified(root.hooks_file()));
        let snapshot = harness.snapshot(&app);
        assert_eq!(snapshot.revision, 4);
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 1);
        assert!(
            snapshot
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("after the run started"))
        );

        let late = Root::with_hooks(VALID);
        harness.add_root(&mut app, &late.path);
        assert_eq!(harness.tool_pre_bindings_under(&app, &late.path), 0);
        let snapshot = harness.snapshot(&app);
        let mut expected = vec![root.hooks_file(), late.hooks_file()];
        expected.sort();
        assert_eq!(
            snapshot.protected_paths, expected,
            "a file found late is still protected"
        );
        assert_eq!(snapshot.project_roots, vec![root.path.clone()]);
    });
}

/// Through the real file system watcher: a burst of writes loads the file once, and a file the
/// filter does not report changes nothing.
#[test]
fn rapid_writes_seen_by_the_real_watcher_load_the_file_once() {
    App::test((), |mut app| async move {
        let root = Root::with_hooks(VALID);
        let harness = Harness::with_real_watcher(&mut app);
        harness.add_root(&mut app, &root.path);
        let loaded = harness.revision(&app);
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 1);

        // The platform watch registers on a background thread; give it a moment to attach.
        Timer::after(Duration::from_millis(500)).await;

        for _ in 0..5 {
            root.write(VALID_TWO);
            Timer::after(Duration::from_millis(10)).await;
        }
        assert!(
            harness
                .wait_for_revision_past(&mut app, loaded, Duration::from_secs(15))
                .await,
            "the file change never reached the watcher"
        );
        // Two debounce windows later nothing else has arrived.
        Timer::after(HOOK_CONFIG_DEBOUNCE * 2).await;
        assert_eq!(harness.revision(&app), loaded + 1, "the burst loaded once");
        assert_eq!(harness.tool_pre_bindings_under(&app, &root.path), 2);

        fs::write(root.path.join("notes.txt"), "unrelated").expect("write");
        fs::create_dir_all(root.path.join("src")).expect("mkdir");
        fs::write(root.path.join("src/main.rs"), "fn main() {}").expect("write");
        Timer::after(HOOK_CONFIG_DEBOUNCE * 2).await;
        assert_eq!(
            harness.revision(&app),
            loaded + 1,
            "files the filter does not report change nothing"
        );
    });
}
