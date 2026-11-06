use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::rc::Rc;

use futures::stream::AbortHandle;
use repo_metadata::repositories::RepoDetectionSource;
use repo_metadata::{RepositoryUpdate, TargetFile};
use settings::SettingsMode;
use warpui::{App, Entity, ModelHandle};

use super::{
    FileMCPConfigDiagnosticKind, FileMCPConfigParseOutcome, FileMCPWatcher, FileMCPWatcherEvent,
    InFlightParse, PendingScan, config_change_flags, home_subdir_to_watch, parse_mcp_config_file,
    providers_in_scope, should_watch_repository, substitute_env_vars,
};
use crate::ai::mcp::MCPProvider;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn setup_watcher(
    app: &mut App,
    pending_scans: HashMap<PendingScan, HashSet<(PathBuf, MCPProvider)>>,
) -> ModelHandle<FileMCPWatcher> {
    app.add_singleton_model(|_| FileMCPWatcher {
        pending_scans,
        ..FileMCPWatcher::new_inert()
    })
}

struct WatcherEventCollector;

impl Entity for WatcherEventCollector {
    type Event = ();
}

/// Records every watcher event. The returned collector handle must outlive the assertions.
fn collect_watcher_events(
    app: &mut App,
    watcher: &ModelHandle<FileMCPWatcher>,
) -> (
    ModelHandle<WatcherEventCollector>,
    Rc<RefCell<Vec<CollectedEvent>>>,
) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let collector = app.add_model(|_| WatcherEventCollector);
    collector.update(app, {
        let events = events.clone();
        |_, ctx| {
            ctx.subscribe_to_model(watcher, move |_, _, event, _| {
                events.borrow_mut().push(CollectedEvent::from(event));
            });
        }
    });
    (collector, events)
}

#[derive(Debug, Eq, PartialEq)]
enum CollectedEvent {
    ConfigParsed(MCPProvider),
    ConfigRemoved(MCPProvider),
    ConfigError(MCPProvider),
    ScanComplete(PendingScan),
}

impl From<&FileMCPWatcherEvent> for CollectedEvent {
    fn from(event: &FileMCPWatcherEvent) -> Self {
        match event {
            FileMCPWatcherEvent::ConfigParsed { provider, .. } => Self::ConfigParsed(*provider),
            FileMCPWatcherEvent::ConfigRemoved { provider, .. } => Self::ConfigRemoved(*provider),
            FileMCPWatcherEvent::ConfigError { diagnostic } => {
                Self::ConfigError(diagnostic.provider)
            }
            FileMCPWatcherEvent::ScanComplete(scan) => Self::ScanComplete(scan.clone()),
        }
    }
}

fn cleanup_env_vars(vars: &[&str]) {
    for var in vars {
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { env::remove_var(var) };
    }
}

#[test]
fn abort_config_parse_cancels_and_removes_inflight_task() {
    let (file_mcp_tx, _file_mcp_rx) = async_channel::unbounded();
    let config_path = PathBuf::from("/tmp/.mcp.json");
    let key = (config_path.clone(), MCPProvider::Warp);
    let (abort_handle, _abort_registration) = AbortHandle::new_pair();
    let observed_handle = abort_handle.clone();
    let mut watcher = FileMCPWatcher {
        file_mcp_tx,
        in_flight_parses: HashMap::from([(
            key.clone(),
            InFlightParse {
                generation: 0,
                abort_handle,
            },
        )]),
        next_parse_generation: 1,
        home_provider_watchers: HashMap::new(),
        project_repo_watchers: HashSet::new(),
        pending_scans: HashMap::new(),
    };

    watcher.abort_config_parse(&config_path, MCPProvider::Warp);

    assert!(observed_handle.is_aborted());
    assert!(!watcher.in_flight_parses.contains_key(&key));
}

#[test]
fn repository_discovery_is_surface_aware() {
    assert!(should_watch_repository(
        RepoDetectionSource::TerminalNavigation,
        SettingsMode::Gui
    ));
    assert!(should_watch_repository(
        RepoDetectionSource::CloudEnvironmentPrep,
        SettingsMode::Gui
    ));
    assert!(!should_watch_repository(
        RepoDetectionSource::ProjectRulesIndexing,
        SettingsMode::Gui
    ));
    assert!(!should_watch_repository(
        RepoDetectionSource::CodeReviewInitialization,
        SettingsMode::Gui
    ));

    assert!(should_watch_repository(
        RepoDetectionSource::TerminalNavigation,
        SettingsMode::Tui
    ));
    assert!(!should_watch_repository(
        RepoDetectionSource::ProjectRulesIndexing,
        SettingsMode::Tui
    ));
    assert!(!should_watch_repository(
        RepoDetectionSource::CodeReviewInitialization,
        SettingsMode::Tui
    ));
    assert!(!should_watch_repository(
        RepoDetectionSource::CloudEnvironmentPrep,
        SettingsMode::Tui
    ));
}

#[test]
fn global_provider_initial_scans_cover_claude_codex_and_agents() {
    let home = PathBuf::from("/home/test");

    assert_eq!(home_subdir_to_watch(MCPProvider::Claude), None);
    assert_eq!(
        home.join(MCPProvider::Claude.home_config_path()),
        home.join(".claude.json")
    );

    for (provider, subdir, config) in [
        (MCPProvider::Codex, ".codex", ".codex/config.toml"),
        (MCPProvider::Agents, ".agents", ".agents/.mcp.json"),
    ] {
        assert_eq!(home_subdir_to_watch(provider), Some(PathBuf::from(subdir)));
        let discovered =
            providers_in_scope(home.clone(), home.join(subdir)).collect::<HashSet<_>>();
        assert!(
            discovered.contains(&(provider, home.join(config))),
            "{provider:?} config should be included in its home subdirectory scan"
        );
    }
}

#[test]
fn project_initial_scan_covers_each_supported_provider_config() {
    let repo = PathBuf::from("/work/repository");
    let discovered = providers_in_scope(repo.clone(), repo.clone()).collect::<HashSet<_>>();

    for provider in [
        MCPProvider::Warp,
        MCPProvider::Claude,
        MCPProvider::Codex,
        MCPProvider::Agents,
    ] {
        assert!(
            discovered.contains(&(provider, repo.join(provider.project_config_path()))),
            "{provider:?} project config should be included in a repository scan"
        );
    }
}

#[test]
fn incremental_updates_detect_each_supported_provider_config() {
    let repo = PathBuf::from("/work/repository");
    for provider in [
        MCPProvider::Warp,
        MCPProvider::Claude,
        MCPProvider::Codex,
        MCPProvider::Agents,
    ] {
        let config_path = repo.join(provider.project_config_path());
        let mut added = RepositoryUpdate::default();
        added
            .added
            .insert(TargetFile::new(config_path.clone(), false));
        assert_eq!(config_change_flags(&added, &config_path), (false, true));

        let mut deleted = RepositoryUpdate::default();
        deleted
            .deleted
            .insert(TargetFile::new(config_path.clone(), false));
        assert_eq!(config_change_flags(&deleted, &config_path), (true, false));
    }
}
#[test]
fn test_substitute_env_vars_success() {
    let test_vars = ["FOO", "BAZ", "REPEATED"];

    // Setup environment variables
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::set_var("FOO", "bar") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::set_var("BAZ", "qux") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::set_var("REPEATED", "value") };

    // Test 1: Single variable substitution
    let input = r#"{"key": "${FOO}"}"#;
    let result = substitute_env_vars(input).expect("Single variable substitution should succeed");
    assert_eq!(
        result, r#"{"key": "bar"}"#,
        "Single variable FOO should be replaced with 'bar'"
    );

    // Test 2: Multiple different variables
    let input = r#"{"key": "${FOO}", "other": "${BAZ}"}"#;
    let result = substitute_env_vars(input).expect("Multiple variable substitution should succeed");
    assert_eq!(
        result, r#"{"key": "bar", "other": "qux"}"#,
        "Multiple variables FOO and BAZ should be replaced"
    );

    // Test 3: Multiple occurrences of same variable
    let input = r#"{"a": "${REPEATED}", "b": "${REPEATED}", "c": "prefix_${REPEATED}_suffix"}"#;
    let result = substitute_env_vars(input).expect("Repeated variable substitution should succeed");
    assert_eq!(
        result, r#"{"a": "value", "b": "value", "c": "prefix_value_suffix"}"#,
        "All occurrences of REPEATED should be replaced with 'value', including within context"
    );

    // Cleanup
    cleanup_env_vars(&test_vars);
}

#[test]
fn test_substitute_env_vars_missing_or_empty() {
    // Test 1: Missing variable
    // Ensure MISSING_VAR is not set
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::remove_var("MISSING_VAR") };

    let input = r#"{"key": "${MISSING_VAR}"}"#;
    let result = substitute_env_vars(input);
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Missing or empty environment variable: MISSING_VAR"),
        "Error message should mention MISSING_VAR, got: {err_msg}"
    );

    // Test 2: Empty variable
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::set_var("EMPTY_VAR", "") };

    let input = r#"{"key": "${EMPTY_VAR}"}"#;
    let result = substitute_env_vars(input);
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Missing or empty environment variable: EMPTY_VAR"),
        "Error message should mention EMPTY_VAR, got: {err_msg}"
    );

    // Cleanup
    cleanup_env_vars(&["EMPTY_VAR"]);
}

#[tokio::test]
async fn parse_outcomes_distinguish_missing_invalid_and_valid_configs() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join(".mcp.json");

    assert!(matches!(
        parse_mcp_config_file(&path, MCPProvider::Warp).await,
        FileMCPConfigParseOutcome::Missing
    ));

    std::fs::write(&path, "{invalid").expect("invalid config should be written");
    match parse_mcp_config_file(&path, MCPProvider::Warp).await {
        FileMCPConfigParseOutcome::Error(diagnostic) => {
            assert_eq!(diagnostic.kind, FileMCPConfigDiagnosticKind::Parse);
        }
        _ => panic!("invalid JSON should produce a parse diagnostic"),
    }

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("WARP_MCP_TEST_MISSING") };
    std::fs::write(
        &path,
        r#"{"mcpServers":{"test":{"command":"${WARP_MCP_TEST_MISSING}"}}}"#,
    )
    .expect("missing-env config should be written");
    match parse_mcp_config_file(&path, MCPProvider::Warp).await {
        FileMCPConfigParseOutcome::Error(diagnostic) => {
            assert_eq!(
                diagnostic.kind,
                FileMCPConfigDiagnosticKind::MissingEnvironmentVariable
            );
        }
        _ => panic!("missing env should produce a diagnostic"),
    }

    std::fs::write(
        &path,
        r#"{"mcpServers":{"test":{"command":"test-command"}}}"#,
    )
    .expect("valid config should be written");
    match parse_mcp_config_file(&path, MCPProvider::Warp).await {
        FileMCPConfigParseOutcome::Parsed(servers) => assert_eq!(servers.len(), 1),
        _ => panic!("valid config should produce one server"),
    }
}

#[test]
fn pending_scan_completes_once_every_owed_source_settles() {
    let repo = PathBuf::from("/work/repository");
    let claude = (repo.join(".mcp.json"), MCPProvider::Claude);
    let codex = (repo.join(".codex/config.toml"), MCPProvider::Codex);
    let scan = PendingScan::CloudEnvRepo(repo.clone());

    App::test((), |mut app| async move {
        let watcher = setup_watcher(
            &mut app,
            HashMap::from([(scan.clone(), HashSet::from([claude.clone(), codex.clone()]))]),
        );
        let (_collector, events) = collect_watcher_events(&mut app, &watcher);

        // Settling the same source repeatedly must not stand in for the sources still owed.
        for _ in 0..2 {
            watcher.update(&mut app, |watcher, ctx| {
                watcher.settle_pending_source(&claude.0, claude.1, ctx);
            });
            assert!(events.borrow().is_empty());
        }

        watcher.update(&mut app, |watcher, ctx| {
            watcher.remove_config(codex.0.clone(), repo.clone(), codex.1, ctx);
        });
        assert_eq!(
            *events.borrow(),
            vec![
                CollectedEvent::ConfigRemoved(MCPProvider::Codex),
                CollectedEvent::ScanComplete(scan),
            ]
        );
        watcher.read(&app, |watcher, _| assert!(watcher.pending_scans.is_empty()));
    });
}

#[test]
fn scan_completes_after_the_current_parse_of_a_reparsed_source() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let root = directory.path().to_path_buf();
    let config_path = root.join("config.json");
    std::fs::write(
        &config_path,
        r#"{"mcpServers":{"test":{"command":"test-command"}}}"#,
    )
    .expect("valid config should be written");
    let source = (config_path.clone(), MCPProvider::Warp);

    App::test((), |mut app| async move {
        let watcher = setup_watcher(
            &mut app,
            HashMap::from([(PendingScan::InitialGlobal, HashSet::from([source]))]),
        );
        let (_collector, events) = collect_watcher_events(&mut app, &watcher);
        let (tx, rx) = futures::channel::oneshot::channel();
        let mut tx = Some(tx);
        let completion = app.add_model(|_| WatcherEventCollector);
        completion.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&watcher, move |_, _, event, _| {
                if matches!(event, FileMCPWatcherEvent::ScanComplete(_))
                    && let Some(tx) = tx.take()
                {
                    let _ = tx.send(());
                }
            });
        });

        watcher.update(&mut app, |watcher, ctx| {
            for _ in 0..2 {
                watcher.update_servers_from_config_file(
                    &config_path,
                    root.clone(),
                    MCPProvider::Warp,
                    ctx,
                );
            }
        });
        rx.await.expect("scan should complete");

        assert_eq!(
            *events.borrow(),
            vec![
                CollectedEvent::ConfigParsed(MCPProvider::Warp),
                CollectedEvent::ScanComplete(PendingScan::InitialGlobal),
            ],
            "only the current parse may report a result or settle the scan"
        );
        watcher.read(&app, |watcher, _| {
            assert!(watcher.pending_scans.is_empty());
            assert!(watcher.in_flight_parses.is_empty());
        });
    });
}

#[test]
fn stale_completion_callback_cannot_reclaim_a_superseded_source() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let root = directory.path().to_path_buf();
    let config_path = root.join("config.json");
    std::fs::write(
        &config_path,
        r#"{"mcpServers":{"test":{"command":"test-command"}}}"#,
    )
    .expect("valid config should be written");
    let key = (config_path.clone(), MCPProvider::Warp);

    App::test((), |mut app| async move {
        let watcher = setup_watcher(&mut app, HashMap::new());
        let stale_generation = watcher.update(&mut app, |watcher, ctx| {
            watcher.update_servers_from_config_file(
                &config_path,
                root.clone(),
                MCPProvider::Warp,
                ctx,
            );
            let stale_generation = watcher.in_flight_parses[&key].generation;
            watcher.update_servers_from_config_file(
                &config_path,
                root.clone(),
                MCPProvider::Warp,
                ctx,
            );
            assert_ne!(stale_generation, watcher.in_flight_parses[&key].generation);
            stale_generation
        });

        let reclaimed = watcher.update(&mut app, |watcher, _| {
            watcher.take_current_in_flight_parse(&key, stale_generation)
        });
        assert!(!reclaimed);
        watcher.read(&app, |watcher, _| {
            assert!(watcher.in_flight_parses.contains_key(&key));
        });
    });
}

#[test]
#[serial_test::serial]
fn startup_scan_parses_existing_subdir_config_before_completion() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let codex_dir = home.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).expect("Codex directory should be created");
    std::fs::write(
        codex_dir.join("config.toml"),
        "[mcp_servers.test]\ncommand = \"test-command\"\n",
    )
    .expect("Codex config should be written");

    let previous_home = std::env::var_os("HOME");
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HOME", home.path()) };

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let watcher = app.add_singleton_model(FileMCPWatcher::new);
        let collector = app.add_model(|_| WatcherEventCollector);
        let (tx, rx) = futures::channel::oneshot::channel();
        let mut tx = Some(tx);
        let mut parsed_codex = false;
        collector.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&watcher, move |_, _, event, _| match event {
                FileMCPWatcherEvent::ConfigParsed { provider, .. }
                    if *provider == MCPProvider::Codex =>
                {
                    parsed_codex = true;
                }
                FileMCPWatcherEvent::ScanComplete(PendingScan::InitialGlobal) => {
                    if let Some(sender) = tx.take() {
                        let _ = sender.send(parsed_codex);
                    }
                }
                FileMCPWatcherEvent::ConfigParsed { .. }
                | FileMCPWatcherEvent::ConfigRemoved { .. }
                | FileMCPWatcherEvent::ConfigError { .. }
                | FileMCPWatcherEvent::ScanComplete(PendingScan::CloudEnvRepo(_)) => {}
            });
        });

        assert!(
            rx.await.expect("startup scan should complete"),
            "the Codex config must be parsed before startup scan completion"
        );
    });

    match previous_home {
        // TODO: Audit that the environment access only happens in single-threaded code.
        Some(home) => unsafe { std::env::set_var("HOME", home) },
        // TODO: Audit that the environment access only happens in single-threaded code.
        None => unsafe { std::env::remove_var("HOME") },
    }
}
