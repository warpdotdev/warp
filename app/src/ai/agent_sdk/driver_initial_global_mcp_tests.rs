use std::path::PathBuf;
use std::time::Duration;

use instant::Instant;
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity as _};

use super::{AgentDriver, AgentDriverError};
use crate::ai::mcp::file_based_manager::FileBasedMCPManager;
use crate::ai::mcp::{
    FileMCPWatcher, FileMCPWatcherEvent, MCPProvider, ParsedTemplatableMCPServerResult,
};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::warp_managed_paths_watcher::warp_managed_mcp_config_path;

fn setup_file_based_mcp_singletons(
    app: &mut App,
) -> (
    warpui::ModelHandle<FileMCPWatcher>,
    warpui::ModelHandle<FileBasedMCPManager>,
) {
    let watcher = app.add_singleton_model(|_| FileMCPWatcher::new_inert());
    let manager = app.add_singleton_model(FileBasedMCPManager::new);
    (watcher, manager)
}

fn emit_watcher_event(
    watcher: &warpui::ModelHandle<FileMCPWatcher>,
    app: &mut App,
    event: FileMCPWatcherEvent,
) {
    watcher.update(app, |_, ctx| {
        ctx.emit(event);
    });
}

fn simulate_completed_initial_global_scan_with_one_server(
    app: &mut App,
    watcher: &warpui::ModelHandle<FileMCPWatcher>,
    file_based_manager: &warpui::ModelHandle<FileBasedMCPManager>,
) -> Option<uuid::Uuid> {
    let warp_mcp_config_path = warp_managed_mcp_config_path()?;
    let parsed = ParsedTemplatableMCPServerResult::from_user_json(
        r#"{"global-warp": {"command": "npx", "args": ["warp"]}}"#,
    )
    .unwrap_or_default();
    emit_watcher_event(
        watcher,
        app,
        FileMCPWatcherEvent::ConfigParsed {
            config_path: warp_mcp_config_path.config_path.clone(),
            root_path: warp_mcp_config_path.root_path.clone(),
            provider: MCPProvider::Warp,
            servers: parsed,
        },
    );
    emit_watcher_event(
        watcher,
        app,
        FileMCPWatcherEvent::InitialGlobalMcpScanComplete,
    );
    Some(
        file_based_manager
            .read(app, |manager, _| {
                manager
                    .global_warp_servers()
                    .into_iter()
                    .map(|s| s.uuid())
                    .next()
            })
            .expect("the global Warp server should have been auto-started"),
    )
}

fn simulate_pending_initial_global_scan_with_one_server(
    app: &mut App,
    watcher: &warpui::ModelHandle<FileMCPWatcher>,
    file_based_manager: &warpui::ModelHandle<FileBasedMCPManager>,
) -> Option<uuid::Uuid> {
    let warp_mcp_config_path = warp_managed_mcp_config_path()?;
    let parsed = ParsedTemplatableMCPServerResult::from_user_json(
        r#"{"global-warp": {"command": "npx", "args": ["warp"]}}"#,
    )
    .unwrap_or_default();
    emit_watcher_event(
        watcher,
        app,
        FileMCPWatcherEvent::ConfigParsed {
            config_path: warp_mcp_config_path.config_path.clone(),
            root_path: warp_mcp_config_path.root_path.clone(),
            provider: MCPProvider::Warp,
            servers: parsed,
        },
    );
    Some(
        file_based_manager
            .read(app, |manager, _| {
                manager
                    .global_warp_servers()
                    .into_iter()
                    .map(|s| s.uuid())
                    .next()
            })
            .expect("the global Warp server should have been auto-started"),
    )
}

fn simulate_completed_initial_global_scan_with_one_server_and_paths(
    app: &mut App,
    watcher: &warpui::ModelHandle<FileMCPWatcher>,
    file_based_manager: &warpui::ModelHandle<FileBasedMCPManager>,
) -> Option<(uuid::Uuid, PathBuf, PathBuf)> {
    let warp_mcp_config_path = warp_managed_mcp_config_path()?;
    let installation_uuid =
        simulate_completed_initial_global_scan_with_one_server(app, watcher, file_based_manager)?;
    Some((
        installation_uuid,
        warp_mcp_config_path.config_path,
        warp_mcp_config_path.root_path,
    ))
}

fn add_test_driver(app: &mut App) -> warpui::ModelHandle<AgentDriver> {
    let terminal_view = add_window_with_terminal(app, None);
    app.add_model(|ctx| {
        let terminal_driver =
            super::terminal::TerminalDriver::create_from_existing_view(terminal_view, ctx);
        AgentDriver::new_for_test(std::env::temp_dir(), terminal_driver, ctx)
    })
}

#[test]
#[serial_test::serial]
fn initial_global_scan_wait_returns_cached_snapshot_immediately() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (watcher, file_based_manager) = setup_file_based_mcp_singletons(&mut app);
        let Some(installation_uuid) = simulate_completed_initial_global_scan_with_one_server(
            &mut app,
            &watcher,
            &file_based_manager,
        ) else {
            return;
        };

        let driver_handle = add_test_driver(&mut app);
        let wait_future = driver_handle.update(&mut app, |driver, ctx| {
            driver.wait_for_initial_global_file_based_mcp_scan(
                Instant::now() + Duration::from_secs(20),
                ctx,
            )
        });
        let wait_uuids = wait_future
            .await
            .expect("cached scan result should succeed");
        assert_eq!(
            wait_uuids,
            vec![installation_uuid],
            "the cached snapshot should be returned without waiting for a new event"
        );
    });
}

#[test]
#[serial_test::serial]
fn initial_global_scan_wait_unblocks_when_pending_scan_completes() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (watcher, file_based_manager) = setup_file_based_mcp_singletons(&mut app);
        let Some(installation_uuid) = simulate_pending_initial_global_scan_with_one_server(
            &mut app,
            &watcher,
            &file_based_manager,
        ) else {
            return;
        };

        let driver_handle = add_test_driver(&mut app);
        let wait_future = driver_handle.update(&mut app, |driver, ctx| {
            driver.wait_for_initial_global_file_based_mcp_scan(
                Instant::now() + Duration::from_secs(20),
                ctx,
            )
        });

        emit_watcher_event(
            &watcher,
            &mut app,
            FileMCPWatcherEvent::InitialGlobalMcpScanComplete,
        );

        assert_eq!(
            wait_future.await.expect("pending scan should complete"),
            vec![installation_uuid]
        );
    });
}

#[test]
#[serial_test::serial]
fn initial_global_mcp_readiness_wait_unblocks_on_running() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (watcher, file_based_manager) = setup_file_based_mcp_singletons(&mut app);
        let Some(installation_uuid) = simulate_completed_initial_global_scan_with_one_server(
            &mut app,
            &watcher,
            &file_based_manager,
        ) else {
            return;
        };

        let driver_handle = add_test_driver(&mut app);
        let (ready_tx, mut ready_rx) = futures::channel::oneshot::channel::<()>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait = driver.wait_for_file_based_mcps_running(
                vec![installation_uuid],
                Duration::from_secs(5),
                ctx,
            );
            ctx.spawn(wait, move |_, _result, _| {
                let _ = ready_tx.send(());
            });
        });

        assert!(
            ready_rx.try_recv().unwrap().is_none(),
            "must not resolve while the server is still starting"
        );

        crate::ai::mcp::TemplatableMCPServerManager::handle(&app).update(
            &mut app,
            |manager, ctx| {
                manager.change_server_state(
                    installation_uuid,
                    crate::ai::mcp::MCPServerState::Running,
                    ctx,
                );
            },
        );

        ready_rx
            .await
            .expect("readiness wait should resolve once the server reaches Running");
    });
}

#[test]
#[serial_test::serial]
fn initial_global_mcp_readiness_wait_unblocks_on_failed_to_start() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (watcher, file_based_manager) = setup_file_based_mcp_singletons(&mut app);
        let Some(installation_uuid) = simulate_completed_initial_global_scan_with_one_server(
            &mut app,
            &watcher,
            &file_based_manager,
        ) else {
            return;
        };

        let driver_handle = add_test_driver(&mut app);
        let (ready_tx, ready_rx) =
            futures::channel::oneshot::channel::<Result<(), AgentDriverError>>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait = driver.wait_for_file_based_mcps_running(
                vec![installation_uuid],
                Duration::from_secs(5),
                ctx,
            );
            ctx.spawn(wait, move |_, result, _| {
                let _ = ready_tx.send(result);
            });
        });

        crate::ai::mcp::TemplatableMCPServerManager::handle(&app).update(
            &mut app,
            |manager, ctx| {
                manager.change_server_state(
                    installation_uuid,
                    crate::ai::mcp::MCPServerState::FailedToStart,
                    ctx,
                );
            },
        );

        let result = ready_rx
            .await
            .expect("readiness wait should resolve once the server FailedToStart");
        assert!(
            matches!(result, Err(AgentDriverError::MCPStartupFailed { .. })),
            "FailedToStart must surface through the existing MCP startup-failed policy, got {result:?}"
        );
    });
}

#[test]
#[serial_test::serial]
fn initial_global_mcp_readiness_wait_settles_immediately_when_config_removed_before_subscribing() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (watcher, file_based_manager) = setup_file_based_mcp_singletons(&mut app);
        let Some((installation_uuid, config_path, root_path)) =
            simulate_completed_initial_global_scan_with_one_server_and_paths(
                &mut app,
                &watcher,
                &file_based_manager,
            )
        else {
            return;
        };

        emit_watcher_event(
            &watcher,
            &mut app,
            FileMCPWatcherEvent::ConfigRemoved {
                config_path,
                root_path,
                provider: MCPProvider::Warp,
            },
        );

        let driver_handle = add_test_driver(&mut app);
        use futures::FutureExt as _;
        let resolved_immediately = driver_handle
            .update(&mut app, |driver, ctx| {
                driver.wait_for_file_based_mcps_running(
                    vec![installation_uuid],
                    Duration::from_secs(5),
                    ctx,
                )
            })
            .now_or_never();

        assert!(
            resolved_immediately.is_some(),
            "a despawned installation must be settled up front, not subscribed and awaited"
        );
    });
}

#[test]
#[serial_test::serial]
fn initial_global_mcp_readiness_wait_settles_on_notrunning_after_subscribing() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (watcher, file_based_manager) = setup_file_based_mcp_singletons(&mut app);
        let Some((installation_uuid, config_path, root_path)) =
            simulate_completed_initial_global_scan_with_one_server_and_paths(
                &mut app,
                &watcher,
                &file_based_manager,
            )
        else {
            return;
        };

        crate::ai::mcp::TemplatableMCPServerManager::handle(&app).update(&mut app, |_, ctx| {
            let file_based_manager = FileBasedMCPManager::handle(ctx);
            ctx.subscribe_to_model(&file_based_manager, |me, _, event, ctx| {
                if let crate::ai::mcp::file_based_manager::FileBasedMCPManagerEvent::DespawnServers {
                    installation_uuids,
                } = event
                {
                    for uuid in installation_uuids {
                        me.shutdown_server(*uuid, ctx);
                    }
                }
            });
        });

        let driver_handle = add_test_driver(&mut app);
        let (ready_tx, ready_rx) = futures::channel::oneshot::channel::<()>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait = driver.wait_for_file_based_mcps_running(
                vec![installation_uuid],
                Duration::from_secs(5),
                ctx,
            );
            ctx.spawn(wait, move |_, _result, _| {
                let _ = ready_tx.send(());
            });
        });

        emit_watcher_event(
            &watcher,
            &mut app,
            FileMCPWatcherEvent::ConfigRemoved {
                config_path,
                root_path,
                provider: MCPProvider::Warp,
            },
        );

        use warpui::r#async::FutureExt as _;
        ready_rx
            .with_timeout(Duration::from_millis(500))
            .await
            .expect("the removal's NotRunning transition should settle the wait promptly")
            .expect("readiness wait task should not have been dropped");
    });
}

#[test]
#[serial_test::serial]
fn timed_out_wait_does_not_tear_down_a_later_waits_subscription() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (watcher, file_based_manager) = setup_file_based_mcp_singletons(&mut app);

        let root_path =
            std::env::temp_dir().join(format!("warp-test-mcp-root-{}", uuid::Uuid::new_v4()));
        let config_path = root_path.join(".mcp.json");
        let parsed = ParsedTemplatableMCPServerResult::from_user_json(
            r#"{"wait-a": {"command": "npx", "args": ["a"]}, "wait-b": {"command": "npx", "args": ["b"]}}"#,
        )
        .unwrap_or_default();
        emit_watcher_event(
            &watcher,
            &mut app,
            FileMCPWatcherEvent::ConfigParsed {
                config_path,
                root_path,
                provider: MCPProvider::Warp,
                servers: parsed,
            },
        );
        let (uuid_a, uuid_b) = file_based_manager.read(&app, |manager, _| {
            let uuid_for = |name: &str| {
                manager
                    .file_based_servers()
                    .into_iter()
                    .find(|installation| installation.templatable_mcp_server().name == name)
                    .map(|installation| installation.uuid())
                    .expect("the server should have been tracked")
            };
            (uuid_for("wait-a"), uuid_for("wait-b"))
        });

        let driver_handle = add_test_driver(&mut app);
        let (a_tx, a_rx) = futures::channel::oneshot::channel::<()>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait = driver.wait_for_file_based_mcps_running(
                vec![uuid_a],
                Duration::from_millis(50),
                ctx,
            );
            ctx.spawn(wait, move |_, _result, _| {
                let _ = a_tx.send(());
            });
        });
        a_rx.await.expect("wait A should resolve once it times out");

        let (b_tx, mut b_rx) = futures::channel::oneshot::channel::<()>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait =
                driver.wait_for_file_based_mcps_running(vec![uuid_b], Duration::from_secs(5), ctx);
            ctx.spawn(wait, move |_, _result, _| {
                let _ = b_tx.send(());
            });
        });
        assert!(
            b_rx.try_recv().unwrap().is_none(),
            "wait B must not resolve before its own uuid settles"
        );

        crate::ai::mcp::TemplatableMCPServerManager::handle(&app).update(
            &mut app,
            |manager, ctx| {
                manager.change_server_state(uuid_a, crate::ai::mcp::MCPServerState::Running, ctx);
            },
        );
        warpui::r#async::Timer::after(Duration::from_millis(50)).await;
        assert!(
            b_rx.try_recv().unwrap().is_none(),
            "A's late terminal event must not have torn down B's subscription"
        );

        crate::ai::mcp::TemplatableMCPServerManager::handle(&app).update(
            &mut app,
            |manager, ctx| {
                manager.change_server_state(uuid_b, crate::ai::mcp::MCPServerState::Running, ctx);
            },
        );
        use warpui::r#async::FutureExt as _;
        b_rx.with_timeout(Duration::from_millis(500))
            .await
            .expect(
                "wait B should resolve promptly via the event, not via its own internal timeout",
            )
            .expect("wait B's oneshot sender should not have been dropped");
    });
}

#[test]
#[serial_test::serial]
fn initial_global_scan_timeout_is_mcp_startup_failed() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_watcher, _manager) = setup_file_based_mcp_singletons(&mut app);
        let driver_handle = add_test_driver(&mut app);
        let result = driver_handle
            .update(&mut app, |driver, ctx| {
                driver.wait_for_initial_global_file_based_mcp_scan(Instant::now(), ctx)
            })
            .await;
        assert!(
            matches!(result, Err(AgentDriverError::MCPStartupFailed { .. })),
            "scan timeout must go through the existing MCP startup-failed policy, got {result:?}"
        );
    });
}
