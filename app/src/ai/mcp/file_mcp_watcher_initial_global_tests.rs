use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use futures::stream::AbortHandle;
use warpui::App;

use super::{FileMCPWatcher, FileMCPWatcherEvent, InFlightParse, InitialGlobalScanCohort};
use crate::ai::mcp::MCPProvider;

/// Constructs a `FileMCPWatcher` singleton with an explicit initial-global-scan pending set,
/// bypassing the real home-directory scan in `FileMCPWatcher::new` so tests are deterministic
/// regardless of what actually exists on the test machine's filesystem.
fn setup_watcher_with_pending(
    app: &mut App,
    pending: HashSet<(PathBuf, MCPProvider)>,
) -> warpui::ModelHandle<FileMCPWatcher> {
    app.add_singleton_model(move |_ctx| FileMCPWatcher {
        file_mcp_tx: async_channel::unbounded().0,
        in_flight_parses: HashMap::new(),
        next_parse_generation: 0,
        home_provider_watchers: HashMap::new(),
        project_repo_watchers: HashSet::new(),
        cloud_env_pending: HashMap::new(),
        initial_global_scan: InitialGlobalScanCohort::from_pending(pending),
    })
}

/// Test-only collector model. A separate model is required to subscribe to `FileMCPWatcher`
/// events in tests: a model may not subscribe to its own events, so the assertions below
/// (`watch_initial_global_scan_completions`) subscribe from this standalone entity instead.
struct WatcherEventCollector;

impl warpui::Entity for WatcherEventCollector {
    type Event = ();
}

/// Subscribes to `FileMCPWatcher` and returns a future that resolves once
/// `InitialGlobalMcpScanComplete` has been observed `expected_count` times, using a shared
/// counter so callers can also assert an exact emission count after the fact.
fn watch_initial_global_scan_completions(
    app: &mut App,
    watcher: &warpui::ModelHandle<FileMCPWatcher>,
    expected_count: usize,
) -> futures::channel::oneshot::Receiver<()> {
    let (tx, rx) = futures::channel::oneshot::channel::<()>();
    let mut tx = Some(tx);
    let count = std::rc::Rc::new(std::cell::RefCell::new(0usize));
    let collector = app.add_model(|_| WatcherEventCollector);
    collector.update(app, |_, ctx| {
        ctx.subscribe_to_model(watcher, move |_, _, event, _| {
            if matches!(event, FileMCPWatcherEvent::InitialGlobalMcpScanComplete) {
                *count.borrow_mut() += 1;
                if *count.borrow() == expected_count
                    && let Some(sender) = tx.take()
                {
                    let _ = sender.send(());
                }
            }
        });
    });
    // Leak the collector so it (and its subscription) outlives this function; tests are
    // short-lived, so this is acceptable.
    std::mem::forget(collector);
    rx
}

/// The initial global scan must settle once every scheduled source has produced a terminal
/// parse outcome, whether that outcome is a valid parse, a missing file, or an invalid config.
#[test]
fn initial_global_scan_settles_after_parsed_missing_and_invalid_sources() {
    let dir = tempfile::tempdir().expect("temp dir should be created");
    let root = dir.path().to_path_buf();
    let parsed_path = root.join("parsed.json");
    std::fs::write(&parsed_path, r#"{"mcpServers":{"test":{"command":"npx"}}}"#).unwrap();
    let missing_path = root.join("missing.json");
    let invalid_path = root.join("invalid.json");
    std::fs::write(&invalid_path, "{invalid").unwrap();

    let pending = HashSet::from([
        (parsed_path.clone(), MCPProvider::Warp),
        (missing_path.clone(), MCPProvider::Claude),
        (invalid_path.clone(), MCPProvider::Codex),
    ]);

    App::test((), |mut app| async move {
        let watcher = setup_watcher_with_pending(&mut app, pending);
        let rx = watch_initial_global_scan_completions(&mut app, &watcher, 1);

        watcher.update(&mut app, |watcher, ctx| {
            watcher.update_servers_from_config_file(
                &parsed_path,
                root.clone(),
                MCPProvider::Warp,
                ctx,
            );
            watcher.update_servers_from_config_file(
                &missing_path,
                root.clone(),
                MCPProvider::Claude,
                ctx,
            );
            watcher.update_servers_from_config_file(
                &invalid_path,
                root.clone(),
                MCPProvider::Codex,
                ctx,
            );
        });

        rx.await
            .expect("initial global scan should settle after mixed terminal outcomes");
        watcher.read(&app, |watcher, _| {
            assert!(
                watcher.initial_global_scan.is_empty(),
                "pending set should be drained once every source settles"
            );
        });
    });
}

/// `InitialGlobalMcpScanComplete` must fire exactly once, even if settlement logic runs again
/// afterward (e.g. a later, unrelated parse completion).
#[test]
fn initial_global_scan_completion_event_fires_exactly_once() {
    let dir = tempfile::tempdir().expect("temp dir should be created");
    let root = dir.path().to_path_buf();
    let missing_path = root.join("missing.json");
    let pending = HashSet::from([(missing_path.clone(), MCPProvider::Warp)]);

    App::test((), |mut app| async move {
        let watcher = setup_watcher_with_pending(&mut app, pending);
        let rx = watch_initial_global_scan_completions(&mut app, &watcher, 1);

        watcher.update(&mut app, |watcher, ctx| {
            watcher.update_servers_from_config_file(
                &missing_path,
                root.clone(),
                MCPProvider::Warp,
                ctx,
            );
        });
        rx.await.expect("initial global scan should settle");

        // Driving the completion check again after settlement (as a later, unrelated parse
        // completion would) must not re-emit the event. There is no positive event to await
        // here (that's the point), so bound the wait instead of hanging forever.
        let second_rx = watch_initial_global_scan_completions(&mut app, &watcher, 1);
        watcher.update(&mut app, |watcher, ctx| {
            watcher.maybe_emit_initial_global_scan_complete(ctx);
        });
        use warpui::r#async::FutureExt as _;
        assert!(
            second_rx
                .with_timeout(std::time::Duration::from_millis(200))
                .await
                .is_err(),
            "a second subscriber should never observe a second completion event"
        );
    });
}

/// If an initial parse is aborted because a file update schedules a replacement (e.g. the file
/// changed while the initial parse was still in flight), the replacement's completion must
/// still settle the initial-scan obligation for that source.
#[test]
fn replaced_initial_parse_settles_via_replacement() {
    let dir = tempfile::tempdir().expect("temp dir should be created");
    let root = dir.path().to_path_buf();
    let config_path = root.join("config.json");
    std::fs::write(&config_path, r#"{"mcpServers":{"test":{"command":"npx"}}}"#).unwrap();
    let pending = HashSet::from([(config_path.clone(), MCPProvider::Warp)]);

    App::test((), |mut app| async move {
        let watcher = setup_watcher_with_pending(&mut app, pending);
        let rx = watch_initial_global_scan_completions(&mut app, &watcher, 1);

        watcher.update(&mut app, |watcher, ctx| {
            // Simulate an in-flight initial parse for this key that is about to be aborted.
            let (abort_handle, _registration) = AbortHandle::new_pair();
            watcher.in_flight_parses.insert(
                (config_path.clone(), MCPProvider::Warp),
                InFlightParse {
                    generation: 0,
                    abort_handle,
                },
            );

            // A file update schedules a replacement parse for the same key. This aborts the
            // simulated in-flight parse above and spawns a new one; the pending set must still
            // contain the key so the replacement's completion settles the scan.
            watcher.update_servers_from_config_file(
                &config_path,
                root.clone(),
                MCPProvider::Warp,
                ctx,
            );
            assert!(
                watcher
                    .initial_global_scan
                    .contains(&(config_path.clone(), MCPProvider::Warp)),
                "the obligation must transfer to the replacement, not be dropped on abort"
            );
        });

        rx.await
            .expect("the replacement parse should settle the initial scan");
    });
}

/// The core race behind a superseded parse's completion callback: its background future can
/// already be queued on the foreground executor before a replacement's `AbortHandle::abort()`
/// call takes effect (the framework applies `abort()` only the next time the aborted future is
/// polled). Unlike [`replaced_initial_parse_settles_via_replacement`] (which only parks an inert
/// `AbortHandle`, never invoking any callback logic), this drives the actual generation check a
/// stale callback runs through: it must not be able to reclaim the source once a replacement has
/// taken it over, and the replacement's own record must survive untouched.
#[test]
fn stale_completion_callback_cannot_reclaim_a_superseded_source() {
    let dir = tempfile::tempdir().expect("temp dir should be created");
    let root = dir.path().to_path_buf();
    let config_path = root.join("config.json");
    std::fs::write(&config_path, r#"{"mcpServers":{"test":{"command":"npx"}}}"#).unwrap();
    let key = (config_path.clone(), MCPProvider::Warp);
    let pending = HashSet::from([key.clone()]);

    App::test((), |mut app| async move {
        let watcher = setup_watcher_with_pending(&mut app, pending);
        let rx = watch_initial_global_scan_completions(&mut app, &watcher, 1);

        let stale_generation = watcher.update(&mut app, |watcher, ctx| {
            // Schedule the original parse ("A") for this source and capture the generation a
            // completion callback for it would have captured.
            watcher.update_servers_from_config_file(
                &config_path,
                root.clone(),
                MCPProvider::Warp,
                ctx,
            );
            let stale_generation = watcher.in_flight_parses[&key].generation;

            // Schedule a replacement ("B") for the same source before A's completion runs, as
            // a rapid file edit would. This is the moment A's callback could already be queued
            // on the foreground executor, ahead of the `abort()` inside this same call taking
            // effect on A's background future.
            watcher.update_servers_from_config_file(
                &config_path,
                root.clone(),
                MCPProvider::Warp,
                ctx,
            );
            let current_generation = watcher.in_flight_parses[&key].generation;
            assert_ne!(
                stale_generation, current_generation,
                "the replacement must claim a fresh generation, not reuse A's"
            );

            stale_generation
        });

        // A's now-stale completion callback fires (out of process order): it must not be able
        // to reclaim the source, since B's record currently owns it.
        let reclaimed = watcher.update(&mut app, |watcher, _ctx| {
            watcher.take_current_in_flight_parse(&key, stale_generation)
        });
        assert!(
            !reclaimed,
            "a stale callback must not be able to reclaim a source superseded by a replacement"
        );
        watcher.read(&app, |watcher, _| {
            assert!(
                watcher.in_flight_parses.contains_key(&key),
                "the stale callback must not have removed the replacement's own record"
            );
            assert!(
                watcher.initial_global_scan.contains(&key),
                "the stale callback must not have claimed or settled the cohort obligation"
            );
        });

        // The replacement (B) itself, once it actually completes, must still settle the scan.
        rx.await
            .expect("the replacement's own completion should settle the initial scan");
    });
}

/// Subscribes to `FileMCPWatcher` and returns a future that resolves when the first
/// `ConfigParsed` event for `provider` is observed.
fn watch_first_config_parsed(
    app: &mut App,
    watcher: &warpui::ModelHandle<FileMCPWatcher>,
    provider: MCPProvider,
) -> futures::channel::oneshot::Receiver<()> {
    let (tx, rx) = futures::channel::oneshot::channel();
    let mut tx = Some(tx);
    let collector = app.add_model(|_| WatcherEventCollector);
    collector.update(app, |_, ctx| {
        ctx.subscribe_to_model(watcher, move |_, _, event, _| {
            if let FileMCPWatcherEvent::ConfigParsed {
                provider: event_provider,
                ..
            } = event
                && *event_provider == provider
                && let Some(sender) = tx.take()
            {
                let _ = sender.send(());
            }
        });
    });
    // Leak the collector so it (and its subscription) outlives this function; tests are
    // short-lived, so this is acceptable.
    std::mem::forget(collector);
    rx
}

/// If the directory watcher's registration for a home-subdir provider fails
/// *asynchronously* -- after `start_watching` already queued its initial scan -- `stop_watching`
/// removes the subscription before that scan can find it, so no `on_scan` (and so no config
/// parse) ever arrives. Regression test for the resulting stall: `settle_stranded_subdir_configs`
/// (called from that failure handler) must parse the affected provider config directly,
/// settling any pending initial-scan obligation instead of leaving it to block until the
/// caller's timeout.
#[test]
fn registration_failure_settles_stranded_subdir_provider_directly() {
    let dir = tempfile::tempdir().expect("temp dir should be created");
    let home_dir = dir.path().to_path_buf();
    let codex_dir = home_dir.join(".codex");
    std::fs::create_dir_all(&codex_dir).expect("codex subdir should be created");
    let codex_config_path = codex_dir.join("config.toml");
    std::fs::write(
        &codex_config_path,
        "[mcp_servers.test-codex-server]\ncommand = \"npx\"\nargs = [\"-y\", \"test-server\"]\n",
    )
    .expect("codex config should be written");

    let pending = HashSet::from([(codex_config_path, MCPProvider::Codex)]);

    App::test((), |mut app| async move {
        let watcher = setup_watcher_with_pending(&mut app, pending);
        let rx = watch_initial_global_scan_completions(&mut app, &watcher, 1);
        let parsed_rx = watch_first_config_parsed(&mut app, &watcher, MCPProvider::Codex);

        watcher.update(&mut app, |watcher, ctx| {
            watcher.settle_stranded_subdir_configs(&codex_dir, home_dir.clone(), ctx);
        });

        rx.await
            .expect("the direct parse from the failure handler must settle the initial scan");
        parsed_rx
            .await
            .expect("the stranded source's direct parse must still emit ConfigParsed");
    });
}

/// The stranded-subdir fallback must not re-read a source that has already settled by the
/// time it runs (e.g. the watcher's queued scan delivered and completed before an async
/// registration failure was even reported): re-reading would be a second filesystem read and a
/// second `ConfigParsed` reconciliation for no benefit, violating the one-read-per-initial-
/// source invariant.
#[test]
fn settle_stranded_subdir_configs_skips_an_already_settled_source() {
    let dir = tempfile::tempdir().expect("temp dir should be created");
    let home_dir = dir.path().to_path_buf();
    let codex_dir = home_dir.join(".codex");
    std::fs::create_dir_all(&codex_dir).expect("codex subdir should be created");
    let codex_config_path = codex_dir.join("config.toml");
    std::fs::write(
        &codex_config_path,
        "[mcp_servers.test-codex-server]\ncommand = \"npx\"\nargs = [\"-y\", \"test-server\"]\n",
    )
    .expect("codex config should be written");

    let pending = HashSet::from([(codex_config_path.clone(), MCPProvider::Codex)]);

    App::test((), |mut app| async move {
        let watcher = setup_watcher_with_pending(&mut app, pending);
        let rx = watch_initial_global_scan_completions(&mut app, &watcher, 1);

        // Simulate the watcher's queued scan delivering and settling the source first -- the
        // non-stranded case: an ordinary parse for this source completes and drains the
        // cohort, exactly as the real `on_scan`-triggered path would.
        watcher.update(&mut app, |watcher, ctx| {
            watcher.update_servers_from_config_file(
                &codex_config_path,
                home_dir.clone(),
                MCPProvider::Codex,
                ctx,
            );
        });
        rx.await
            .expect("the delivered parse should settle the initial scan");

        // A second `ConfigParsed` for this source after settlement would mean the fallback
        // re-read it.
        let second_parse_rx = watch_first_config_parsed(&mut app, &watcher, MCPProvider::Codex);

        // The registration-failure fallback runs anyway (e.g. the async failure was reported
        // after the scan already settled the source); it must be a no-op now.
        watcher.update(&mut app, |watcher, ctx| {
            watcher.settle_stranded_subdir_configs(&codex_dir, home_dir.clone(), ctx);
        });

        use warpui::r#async::FutureExt as _;
        assert!(
            second_parse_rx
                .with_timeout(std::time::Duration::from_millis(200))
                .await
                .is_err(),
            "the fallback must not re-read an already-settled source"
        );
    });
}

/// A config removal with no replacement parse (e.g. the file was deleted) must still settle a
/// pending initial-scan source; otherwise the scan would hang forever.
#[test]
fn aborted_initial_parse_without_replacement_settles_scan() {
    let config_path = PathBuf::from("/tmp/removed-during-initial-scan.json");
    let pending = HashSet::from([(config_path.clone(), MCPProvider::Warp)]);

    App::test((), |mut app| async move {
        let watcher = setup_watcher_with_pending(&mut app, pending);
        let rx = watch_initial_global_scan_completions(&mut app, &watcher, 1);

        watcher.update(&mut app, |watcher, ctx| {
            let (abort_handle, _registration) = AbortHandle::new_pair();
            watcher.in_flight_parses.insert(
                (config_path.clone(), MCPProvider::Warp),
                InFlightParse {
                    generation: 0,
                    abort_handle,
                },
            );

            // The config was removed outright; no replacement parse follows.
            watcher.abort_config_parse_for_removal(&config_path, MCPProvider::Warp, ctx);
        });

        rx.await
            .expect("removal without a replacement must still settle the initial scan");
        watcher.read(&app, |watcher, _| {
            assert!(watcher.initial_global_scan.is_empty());
        });
    });
}
