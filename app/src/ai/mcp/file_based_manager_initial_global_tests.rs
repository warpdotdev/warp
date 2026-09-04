use std::path::PathBuf;

use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity as _};
use watcher::HomeDirectoryWatcher;

use super::{FileBasedMCPManager, MCPProvider};
use crate::ai::mcp::{FileMCPWatcher, FileMCPWatcherEvent, ParsedTemplatableMCPServerResult};
use crate::auth::AuthStateProvider;
use crate::settings::{AISettings, FocusedTerminalInfo};
use crate::warp_managed_paths_watcher::{WarpManagedPathsWatcher, warp_managed_mcp_config_path};
use crate::workspaces::user_workspaces::UserWorkspaces;

fn setup_app(app: &mut App) -> warpui::ModelHandle<FileBasedMCPManager> {
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
    app.add_singleton_model(|_| FileMCPWatcher::new_inert());
    app.add_singleton_model(AISettings::new_with_defaults);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(FocusedTerminalInfo::new);
    app.add_singleton_model(FileBasedMCPManager::new)
}

fn parse_mcp_json(json: &str) -> Vec<ParsedTemplatableMCPServerResult> {
    ParsedTemplatableMCPServerResult::from_user_json(json).unwrap_or_default()
}

fn set_file_based_mcp_enabled(app: &mut App, enabled: bool) {
    AISettings::handle(app).update(app, |settings, ctx| {
        settings
            .file_based_mcp_enabled
            .load_value(enabled, true, ctx)
            .expect("load_value should succeed in tests");
    });
}

/// Before the watcher's completion event arrives, `initial_global_scan_result` must report
/// `Pending` (i.e. `None`), and the frozen wait set must only include UUIDs auto-started
/// from a global-scoped parse — not from an ordinary project or cloud-environment scan.
#[test]
fn initial_global_scan_result_pending_until_watcher_signals_completion() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    let Some(warp_mcp_config_path) = warp_managed_mcp_config_path() else {
        return;
    };
    let global_root = warp_mcp_config_path.root_path;
    let project_root = PathBuf::from("/tmp/warp-test-initial-scan-project");
    let global_parsed = parse_mcp_json(r#"{"global-warp": {"command": "npx", "args": ["warp"]}}"#);
    let project_parsed =
        parse_mcp_json(r#"{"proj-warp": {"command": "npx", "args": ["proj-warp"]}}"#);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);

        manager.update(&mut app, |m, _| {
            assert_eq!(
                m.initial_global_scan_result(),
                None,
                "scan must be pending before any events arrive"
            );
        });

        manager.update(&mut app, |m, ctx| {
            // Global Warp parse: contributes its auto-started UUID when the scan later
            // freezes.
            m.handle_watcher_event(
                &FileMCPWatcherEvent::ConfigParsed {
                    config_path: global_root.join(".mcp.json"),
                    root_path: global_root.clone(),
                    provider: MCPProvider::Warp,
                    servers: global_parsed,
                },
                ctx,
            );
            // A project-scoped parse: even though these servers never auto-start, this
            // also must not be counted toward the initial scan.
            m.handle_watcher_event(
                &FileMCPWatcherEvent::ConfigParsed {
                    config_path: project_root.join(".warp/.mcp.json"),
                    root_path: project_root.clone(),
                    provider: MCPProvider::Warp,
                    servers: project_parsed,
                },
                ctx,
            );
        });

        manager.update(&mut app, |m, _| {
            assert_eq!(
                m.initial_global_scan_result(),
                None,
                "scan must remain pending until the watcher signals completion"
            );
        });

        let spawned_uuid = manager.update(&mut app, |m, _| {
            let servers = m.file_based_servers();
            assert_eq!(servers.len(), 2, "both installations should be tracked");
            m.global_warp_servers()
                .into_iter()
                .map(|s| s.uuid())
                .next()
                .expect("the global Warp server should be tracked")
        });

        manager.update(&mut app, |m, ctx| {
            m.handle_watcher_event(&FileMCPWatcherEvent::InitialGlobalMcpScanComplete, ctx);
        });

        manager.update(&mut app, |m, _| {
            assert_eq!(
                m.initial_global_scan_result(),
                Some(vec![spawned_uuid]),
                "only the global-scoped auto-started UUID should be in the wait set"
            );
        });
    });
}

#[test]
fn initial_global_scan_keeps_auto_started_uuid_across_reparse_before_completion() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    let Some(warp_mcp_config_path) = warp_managed_mcp_config_path() else {
        return;
    };
    let global_root = warp_mcp_config_path.root_path;
    let json = r#"{"global-warp": {"command": "npx", "args": ["warp"]}}"#;

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        manager.update(&mut app, |m, ctx| {
            m.handle_watcher_event(
                &FileMCPWatcherEvent::ConfigParsed {
                    config_path: global_root.join(".mcp.json"),
                    root_path: global_root.clone(),
                    provider: MCPProvider::Warp,
                    servers: parse_mcp_json(json),
                },
                ctx,
            );
        });
        let spawned_uuid = manager.update(&mut app, |m, _| {
            m.global_warp_servers()
                .into_iter()
                .map(|s| s.uuid())
                .next()
                .expect("the global Warp server should have been auto-started")
        });
        manager.update(&mut app, |m, ctx| {
            m.handle_watcher_event(
                &FileMCPWatcherEvent::ConfigParsed {
                    config_path: global_root.join(".mcp.json"),
                    root_path: global_root.clone(),
                    provider: MCPProvider::Warp,
                    servers: parse_mcp_json(json),
                },
                ctx,
            );
            m.handle_watcher_event(&FileMCPWatcherEvent::InitialGlobalMcpScanComplete, ctx);
        });
        manager.update(&mut app, |m, _| {
            assert_eq!(
                m.initial_global_scan_result(),
                Some(vec![spawned_uuid]),
                "a reparse that starts no new servers must not drop the first auto-started UUID"
            );
        });
    });
}

#[test]
fn initial_global_scan_includes_servers_enabled_before_completion() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    let Some(home_dir) = dirs::home_dir() else {
        return;
    };
    let json = r#"{"global-claude": {"command": "npx", "args": ["claude"]}}"#;

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        set_file_based_mcp_enabled(&mut app, false);
        manager.update(&mut app, |m, ctx| {
            m.handle_watcher_event(
                &FileMCPWatcherEvent::ConfigParsed {
                    config_path: home_dir.join(".claude.json"),
                    root_path: home_dir.clone(),
                    provider: MCPProvider::Claude,
                    servers: parse_mcp_json(json),
                },
                ctx,
            );
        });
        let spawned_uuid = manager.update(&mut app, |m, _| {
            assert_eq!(m.initial_global_scan_result(), None);
            m.file_based_servers()
                .into_iter()
                .map(|s| s.uuid())
                .next()
                .expect("the global third-party server should be tracked")
        });
        set_file_based_mcp_enabled(&mut app, true);
        manager.update(&mut app, |m, ctx| {
            m.handle_watcher_event(&FileMCPWatcherEvent::InitialGlobalMcpScanComplete, ctx);
        });
        manager.update(&mut app, |m, _| {
            assert_eq!(
                m.initial_global_scan_result(),
                Some(vec![spawned_uuid]),
                "enabling file-based MCP during the pending scan must include the spawned server"
            );
        });
    });
}

/// A consumer that queries `initial_global_scan_result` after the scan has already completed
/// must receive the cached snapshot immediately so a late subscriber still observes the frozen set.
#[test]
fn initial_global_scan_result_returns_cached_snapshot_after_completion() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        manager.update(&mut app, |m, ctx| {
            m.handle_watcher_event(&FileMCPWatcherEvent::InitialGlobalMcpScanComplete, ctx);
        });

        // A late consumer, analogous to one constructed long after startup.
        manager.read(&app, |m, _| {
            assert_eq!(m.initial_global_scan_result(), Some(Vec::new()));
        });
    });
}

/// A scan with no configured or eligible global servers must resolve to an immediate, empty
/// result rather than staying pending.
#[test]
fn initial_global_scan_with_no_sources_resolves_to_empty_result() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        manager.update(&mut app, |m, _| {
            assert_eq!(m.initial_global_scan_result(), None);
        });
        manager.update(&mut app, |m, ctx| {
            m.handle_watcher_event(&FileMCPWatcherEvent::InitialGlobalMcpScanComplete, ctx);
        });
        manager.update(&mut app, |m, _| {
            assert_eq!(
                m.initial_global_scan_result(),
                Some(Vec::new()),
                "no sources and no auto-started servers should still settle to an empty result"
            );
        });
    });
}

/// When the `FileBasedMcp` feature flag is disabled, the whole pipeline is inert (the manager
/// never subscribes to the watcher), so the scan must settle to an immediate empty result at
/// construction time rather than leaving a waiter pending forever.
#[test]
fn initial_global_scan_settles_immediately_when_feature_disabled() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(false);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        manager.read(&app, |m, _| {
            assert_eq!(
                m.initial_global_scan_result(),
                Some(Vec::new()),
                "disabled feature flag must not leave the first-turn wait pending forever"
            );
        });
    });
}
