use super::{CloudEnvMcpScanServer, FileBasedMCPManager, FileBasedMCPManagerEvent, MCPProvider};
use crate::ai::mcp::FileMCPWatcher;
use crate::ai::mcp::ParsedTemplatableMCPServerResult;
use crate::auth::AuthStateProvider;
use crate::settings::{AISettings, FocusedTerminalInfo};
use crate::warp_managed_paths_watcher::{warp_managed_mcp_config_path, WarpManagedPathsWatcher};
use crate::workspaces::user_workspaces::UserWorkspaces;
use repo_metadata::{
    repositories::DetectedRepositories, watcher::DirectoryWatcher, RepoMetadataModel,
};
use settings::Setting as _;
use std::collections::HashSet;
use std::path::PathBuf;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::{App, Entity, ModelHandle, SingletonEntity as _};
use watcher::HomeDirectoryWatcher;

// Helper to initialize dependencies and return FileBasedMCPManager handle
fn setup_app(app: &mut App) -> warpui::ModelHandle<FileBasedMCPManager> {
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
    app.add_singleton_model(FileMCPWatcher::new);
    app.add_singleton_model(AISettings::new_with_defaults);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(FocusedTerminalInfo::new);
    app.add_singleton_model(FileBasedMCPManager::new)
}

/// Parses an MCP JSON string directly into server results, bypassing file I/O.
/// Used in tests to exercise `apply_parsed_servers` without needing files on disk.
fn parse_mcp_json(json: &str) -> Vec<ParsedTemplatableMCPServerResult> {
    ParsedTemplatableMCPServerResult::from_user_json(json).unwrap_or_default()
}

/// Test-only event collector for `FileBasedMCPManagerEvent`s.
#[derive(Default)]
struct ManagerEvents {
    spawned_uuids: Vec<Uuid>,
    despawned_uuids: Vec<Uuid>,
    scan_completions: Vec<ScanCompletion>,
}

#[derive(Clone, Debug)]
struct ScanCompletion {
    repo_path: PathBuf,
    detected_servers: Vec<CloudEnvMcpScanServer>,
    wait_server_uuids: Vec<Uuid>,
}

impl Entity for ManagerEvents {
    type Event = ();
}

/// Subscribe a fresh `ManagerEvents` collector to the given manager handle.
fn subscribe_events(
    app: &mut App,
    manager: &ModelHandle<FileBasedMCPManager>,
) -> ModelHandle<ManagerEvents> {
    let events = app.add_model(|_| ManagerEvents::default());
    events.update(app, |_, ctx| {
        ctx.subscribe_to_model(manager, |me, event, _| match event {
            FileBasedMCPManagerEvent::SpawnServers { installations } => me
                .spawned_uuids
                .extend(installations.iter().map(|i| i.uuid())),
            FileBasedMCPManagerEvent::DespawnServers { installation_uuids } => {
                me.despawned_uuids
                    .extend(installation_uuids.iter().copied());
            }
            FileBasedMCPManagerEvent::PurgeCredentials { .. } => {}
            FileBasedMCPManagerEvent::CloudEnvMcpScanComplete {
                repo_path,
                detected_servers,
                wait_server_uuids,
            } => {
                me.scan_completions.push(ScanCompletion {
                    repo_path: repo_path.clone(),
                    detected_servers: detected_servers.clone(),
                    wait_server_uuids: wait_server_uuids.clone(),
                });
            }
        });
    });
    events
}

/// Set the `file_based_mcp_enabled` toggle without going through preferences.
fn set_file_based_mcp_enabled(app: &mut App, enabled: bool) {
    AISettings::handle(app).update(app, |settings, ctx| {
        settings
            .file_based_mcp_enabled
            .load_value(enabled, true, ctx)
            .expect("load_value should succeed in tests");
    });
}

#[test]
fn test_update_file_based_servers_spawns_new_servers() {
    let repo_path = PathBuf::from("/tmp/test-repo");
    let parsed = parse_mcp_json(
        r#"{"test-server": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-example"]}}"#,
    );

    App::test((), |mut app| async move {
        let manager_handle = setup_app(&mut app);

        manager_handle.update(&mut app, |manager, ctx| {
            // Initially, no servers should exist
            assert_eq!(
                manager.file_based_servers.len(),
                0,
                "Should start with no servers"
            );

            // Apply the pre-parsed servers directly
            manager.apply_parsed_servers(repo_path.clone(), MCPProvider::Claude, parsed, ctx);

            // Verify a new server was added to the data structures
            assert_eq!(
                manager.file_based_servers.len(),
                1,
                "Should have one server in file_based_servers"
            );

            // Verify the repo is mapped to the server
            let server_hashes = manager
                .file_based_servers_by_root
                .get(&repo_path)
                .and_then(|m| m.get(&MCPProvider::Claude))
                .unwrap();
            assert_eq!(server_hashes.len(), 1, "Repo should reference one server");

            // Verify the server hash is consistent
            let hash = *server_hashes.iter().next().unwrap();
            assert!(
                manager.file_based_servers.contains_key(&hash),
                "Server hash should exist in file_based_servers"
            );
        });
    });
}

#[test]
fn test_update_file_based_servers_adds_reference_to_existing_server() {
    let json = r#"{"shared-server": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-example"]}}"#;
    let repo1 = PathBuf::from("/tmp/test-repo-1");
    let repo2 = PathBuf::from("/tmp/test-repo-2");
    let parsed1 = parse_mcp_json(json);
    let parsed2 = parse_mcp_json(json);

    App::test((), |mut app| async move {
        let manager_handle = setup_app(&mut app);

        manager_handle.update(&mut app, |manager, ctx| {
            // First repo adds the server
            manager.apply_parsed_servers(repo1.clone(), MCPProvider::Claude, parsed1, ctx);

            assert_eq!(
                manager.file_based_servers.len(),
                1,
                "Should have one server after first scan"
            );
            let first_hash = *manager.file_based_servers.keys().next().unwrap();
            let first_installation_uuid = manager
                .file_based_servers
                .get(&first_hash)
                .unwrap()
                .uuid();

            // Second repo with same config should reuse the server
            manager.apply_parsed_servers(repo2.clone(), MCPProvider::Claude, parsed2, ctx);

            // Should still have only one server (same hash)
            assert_eq!(
                manager.file_based_servers.len(),
                1,
                "Should still have only one server (shared)"
            );

            // Both repos should reference the same server
            let hash1 = manager
                .file_based_servers_by_root
                .get(&repo1)
                .and_then(|m| m.get(&MCPProvider::Claude))
                .and_then(|s| s.iter().copied().next())
                .unwrap();

            let hash2 = manager
                .file_based_servers_by_root
                .get(&repo2)
                .and_then(|m| m.get(&MCPProvider::Claude))
                .and_then(|s| s.iter().copied().next())
                .unwrap();

            assert_eq!(
                hash1, hash2,
                "Both repos should reference the same server hash"
            );

            // Verify the installation at the hash hasn't changed
            let installation_uuid_after_rescan = manager
                .file_based_servers
                .get(&hash1)
                .unwrap()
                .uuid();
            assert_eq!(
                first_installation_uuid,
                installation_uuid_after_rescan,
                "Installation UUID must remain stable when reusing a hash - OAuth flow depends on this mapping"
            );
        });
    });
}

#[test]
fn test_update_file_based_servers_removes_unreferenced_servers() {
    let repo_path = PathBuf::from("/tmp/test-repo");
    let parsed1 = parse_mcp_json(r#"{"server1": {"command": "npx", "args": ["-y", "server1"]}}"#);
    let parsed2 = parse_mcp_json(r#"{"server2": {"command": "npx", "args": ["-y", "server2"]}}"#);

    App::test((), |mut app| async move {
        let manager_handle = setup_app(&mut app);

        manager_handle.update(&mut app, |manager, ctx| {
            manager.apply_parsed_servers(repo_path.clone(), MCPProvider::Claude, parsed1, ctx);

            assert_eq!(
                manager.file_based_servers.len(),
                1,
                "Should have one server after first scan"
            );
            let first_hash = *manager.file_based_servers.keys().next().unwrap();

            // Second scan: different server (old one should be removed)
            manager.apply_parsed_servers(repo_path.clone(), MCPProvider::Claude, parsed2, ctx);

            // Should still have one server, but it's a different one
            assert_eq!(
                manager.file_based_servers.len(),
                1,
                "Should still have one server after second scan"
            );

            let second_hash = *manager.file_based_servers.keys().next().unwrap();
            assert_ne!(
                first_hash, second_hash,
                "The server hash should be different (new server)"
            );

            // Old server should be gone
            assert!(
                !manager.file_based_servers.contains_key(&first_hash),
                "Old server should be removed"
            );

            // Repo should only reference the new server
            let server_hashes = manager
                .file_based_servers_by_root
                .get(&repo_path)
                .and_then(|m| m.get(&MCPProvider::Claude))
                .unwrap();
            assert_eq!(server_hashes.len(), 1, "Repo should reference one server");
            assert_eq!(
                *server_hashes.iter().next().unwrap(),
                second_hash,
                "Repo should reference the new server"
            );
        });
    });
}

/// A Warp-global installation detected from the managed `~/.warp*/.mcp.json`
/// watcher uses the home directory as its logical root and still always
/// auto-spawns.
#[test]
fn test_global_warp_server_from_managed_home_root_always_spawns() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    let Some(warp_mcp_config_path) = warp_managed_mcp_config_path() else {
        return;
    };
    let parsed = parse_mcp_json(r#"{"global-warp": {"command": "npx", "args": ["warp"]}}"#);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let events = subscribe_events(&mut app, &manager);

        // Toggle is off by default; the watcher-produced Warp root should still
        // be classified as the global Warp config and auto-spawn.
        manager.update(&mut app, |m, ctx| {
            m.apply_parsed_servers(
                warp_mcp_config_path.root_path.clone(),
                MCPProvider::Warp,
                parsed,
                ctx,
            );
        });

        events.update(&mut app, |e, _| {
            assert_eq!(
                e.spawned_uuids.len(),
                1,
                "Managed Warp MCP config should auto-spawn regardless of toggle"
            );
        });

        // Flipping the toggle must not despawn the global Warp server.
        set_file_based_mcp_enabled(&mut app, true);
        set_file_based_mcp_enabled(&mut app, false);

        events.update(&mut app, |e, _| {
            assert!(
                e.despawned_uuids.is_empty(),
                "Managed Warp MCP config should never be despawned by toggle changes, got: {:?}",
                e.despawned_uuids
            );
        });
    });
}
/// A globally-scoped non-Warp installation only auto-spawns when the toggle is on.
#[test]
fn test_global_non_warp_server_respects_toggle() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    let Some(home_dir) = dirs::home_dir() else {
        // Skip on platforms where a home dir isn't available (shouldn't happen on
        // our supported platforms, but guard to avoid false failures).
        return;
    };
    let parsed = parse_mcp_json(r#"{"global-claude": {"command": "npx", "args": ["claude"]}}"#);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let events = subscribe_events(&mut app, &manager);

        // Toggle is off by default: detection should NOT auto-spawn.
        manager.update(&mut app, |m, ctx| {
            m.apply_parsed_servers(home_dir.clone(), MCPProvider::Claude, parsed, ctx);
        });
        events.update(&mut app, |e, _| {
            assert!(
                e.spawned_uuids.is_empty(),
                "Global non-Warp server must not auto-spawn while toggle is off, got: {:?}",
                e.spawned_uuids
            );
        });

        let installation_uuid = manager.update(&mut app, |m, _| {
            let servers = m.file_based_servers();
            assert_eq!(servers.len(), 1);
            servers[0].uuid()
        });

        // Toggle on: the global non-Warp server should be spawned.
        set_file_based_mcp_enabled(&mut app, true);
        events.update(&mut app, |e, _| {
            assert_eq!(
                e.spawned_uuids,
                vec![installation_uuid],
                "Global non-Warp server should spawn when toggle flips on"
            );
        });

        // Toggle off: the global non-Warp server should be despawned.
        set_file_based_mcp_enabled(&mut app, false);
        events.update(&mut app, |e, _| {
            assert_eq!(
                e.despawned_uuids,
                vec![installation_uuid],
                "Global non-Warp server should despawn when toggle flips off"
            );
        });
    });
}

/// Project-scoped installations (both Warp and third-party) never auto-spawn on
/// detection, and the toggle must not spawn or despawn them either.
#[test]
fn test_project_scoped_servers_never_auto_spawn() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    let repo_path = PathBuf::from("/tmp/warp-test-repo");
    let claude_parsed =
        parse_mcp_json(r#"{"proj-claude": {"command": "npx", "args": ["proj-claude"]}}"#);
    let warp_parsed = parse_mcp_json(r#"{"proj-warp": {"command": "npx", "args": ["proj-warp"]}}"#);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let events = subscribe_events(&mut app, &manager);

        manager.update(&mut app, |m, ctx| {
            m.apply_parsed_servers(repo_path.clone(), MCPProvider::Claude, claude_parsed, ctx);
            m.apply_parsed_servers(repo_path.clone(), MCPProvider::Warp, warp_parsed, ctx);
        });

        // Neither detection should emit a spawn event.
        events.update(&mut app, |e, _| {
            assert!(
                e.spawned_uuids.is_empty(),
                "Project-scoped installations must not auto-spawn, got: {:?}",
                e.spawned_uuids
            );
        });

        let project_uuids: HashSet<Uuid> = manager.update(&mut app, |m, _| {
            m.file_based_servers().iter().map(|i| i.uuid()).collect()
        });
        assert_eq!(
            project_uuids.len(),
            2,
            "Both project-scoped installations should be tracked"
        );

        // Flipping the toggle must not spawn or despawn project-scoped servers.
        set_file_based_mcp_enabled(&mut app, true);
        set_file_based_mcp_enabled(&mut app, false);

        events.update(&mut app, |e, _| {
            assert!(
                e.spawned_uuids.is_empty(),
                "Toggle flip must not spawn project-scoped servers, got: {:?}",
                e.spawned_uuids
            );
            assert!(
                e.despawned_uuids.is_empty(),
                "Toggle flip must not despawn project-scoped servers, got: {:?}",
                e.despawned_uuids
            );
        });
    });
}

#[test]
fn test_project_scoped_cloud_scan_has_detected_servers_but_empty_wait_set() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    let repo_path = PathBuf::from("/tmp/warp-test-cloud-repo");
    let claude_parsed =
        parse_mcp_json(r#"{"proj-claude": {"command": "npx", "args": ["proj-claude"]}}"#);
    let warp_parsed = parse_mcp_json(r#"{"proj-warp": {"command": "npx", "args": ["proj-warp"]}}"#);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let events = subscribe_events(&mut app, &manager);

        manager.update(&mut app, |m, ctx| {
            m.apply_parsed_servers(repo_path.clone(), MCPProvider::Claude, claude_parsed, ctx);
            m.apply_parsed_servers(repo_path.clone(), MCPProvider::Warp, warp_parsed, ctx);
            m.handle_cloud_environment_scan_complete(&repo_path, ctx);
        });

        events.update(&mut app, |e, _| {
            assert_eq!(e.scan_completions.len(), 1);
            let scan = &e.scan_completions[0];
            assert_eq!(scan.repo_path, repo_path);
            assert_eq!(scan.detected_servers.len(), 2);
            assert!(
                scan.wait_server_uuids.is_empty(),
                "Project-scoped servers must not be included in the AgentDriver wait set, got: {:?}",
                scan.wait_server_uuids
            );
            assert!(
                scan.detected_servers
                    .iter()
                    .all(|server| !server.auto_start_eligible),
                "Project-scoped servers should not be auto-start eligible: {:?}",
                scan.detected_servers
            );
        });
    });
}

#[test]
fn test_auto_started_cloud_scan_uuids_are_in_wait_set() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    let Some(warp_mcp_config_path) = warp_managed_mcp_config_path() else {
        return;
    };
    let root_path = warp_mcp_config_path.root_path;
    let parsed = parse_mcp_json(r#"{"global-warp": {"command": "npx", "args": ["warp"]}}"#);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let events = subscribe_events(&mut app, &manager);

        manager.update(&mut app, |m, ctx| {
            m.apply_parsed_servers(root_path.clone(), MCPProvider::Warp, parsed, ctx);
            m.handle_cloud_environment_scan_complete(&root_path, ctx);
        });

        events.update(&mut app, |e, _| {
            assert_eq!(e.spawned_uuids.len(), 1);
            assert_eq!(e.scan_completions.len(), 1);
            let scan = &e.scan_completions[0];
            assert_eq!(scan.detected_servers.len(), 1);
            assert_eq!(scan.wait_server_uuids, e.spawned_uuids);
            assert!(
                scan.detected_servers[0].auto_start_eligible,
                "Global Warp server should be auto-start eligible"
            );
        });
    });
}

/// An installation referenced from both a global location and a project location
/// is considered global (and thus gated only by the toggle for non-Warp providers).
#[test]
fn test_server_referenced_from_both_global_and_project_is_global() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    let Some(home_dir) = dirs::home_dir() else {
        return;
    };
    let repo_path = PathBuf::from("/tmp/warp-test-repo-shared");
    let json = r#"{"shared-claude": {"command": "npx", "args": ["shared"]}}"#;
    let global_parsed = parse_mcp_json(json);
    let project_parsed = parse_mcp_json(json);

    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let events = subscribe_events(&mut app, &manager);

        // Register both a global and a project reference for the same server.
        manager.update(&mut app, |m, ctx| {
            m.apply_parsed_servers(home_dir.clone(), MCPProvider::Claude, global_parsed, ctx);
            m.apply_parsed_servers(repo_path.clone(), MCPProvider::Claude, project_parsed, ctx);
        });

        // Toggle starts off, so no auto-spawn yet.
        events.update(&mut app, |e, _| {
            assert!(
                e.spawned_uuids.is_empty(),
                "Server should not auto-spawn while toggle is off"
            );
        });

        let installation_uuid = manager.update(&mut app, |m, _| {
            let servers = m.file_based_servers();
            assert_eq!(
                servers.len(),
                1,
                "Both roots should reference the same server"
            );
            servers[0].uuid()
        });

        // Flipping the toggle on should spawn the server because it's also global.
        set_file_based_mcp_enabled(&mut app, true);
        events.update(&mut app, |e, _| {
            assert_eq!(
                e.spawned_uuids,
                vec![installation_uuid],
                "Global reference should make the server eligible for toggle-driven spawn"
            );
        });
    });
}

#[test]
fn test_update_file_based_servers_removes_server_only_when_no_refs() {
    let json = r#"{"shared-server": {"command": "npx", "args": ["-y", "shared"]}}"#;
    let repo1 = PathBuf::from("/tmp/test-repo-1");
    let repo2 = PathBuf::from("/tmp/test-repo-2");
    let parsed1 = parse_mcp_json(json);
    let parsed2 = parse_mcp_json(json);

    App::test((), |mut app| async move {
        let manager_handle = setup_app(&mut app);

        manager_handle.update(&mut app, |manager, ctx| {
            // Both repos add the same server
            manager.apply_parsed_servers(repo1.clone(), MCPProvider::Claude, parsed1, ctx);
            manager.apply_parsed_servers(repo2.clone(), MCPProvider::Claude, parsed2, ctx);

            assert_eq!(
                manager.file_based_servers.len(),
                1,
                "Should have one shared server"
            );
            let server_hash = *manager.file_based_servers.keys().next().unwrap();

            // repo1 removes the server (empty scan simulates deleted config file)
            manager.apply_parsed_servers(repo1.clone(), MCPProvider::Claude, vec![], ctx);

            // Server should still exist because repo2 still references it
            assert_eq!(
                manager.file_based_servers.len(),
                1,
                "Server should still exist (repo2 still references it)"
            );
            assert!(
                manager.file_based_servers.contains_key(&server_hash),
                "Server should still be in the map"
            );

            // Now repo2 removes the server too
            manager.apply_parsed_servers(repo2.clone(), MCPProvider::Claude, vec![], ctx);

            // Now the server should be removed (no more references)
            assert_eq!(
                manager.file_based_servers.len(),
                0,
                "Server should be removed (no more references)"
            );
            assert!(
                !manager.file_based_servers.contains_key(&server_hash),
                "Server should be completely removed"
            );
        });
    });
}
