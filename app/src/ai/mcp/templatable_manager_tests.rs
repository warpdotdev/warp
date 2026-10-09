use futures_util::stream::AbortHandle;
use mcp::oauth::CallbackResult;
use url::Url;
use uuid::Uuid;
use warpui::App;

use super::{SpawnedServerInfo, TemplatableMCPServerManager};
use crate::ai::mcp::builtin;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

#[test]
fn oauth_callback_preserves_decoded_issuer() {
    let mut manager = TemplatableMCPServerManager::default();
    let installation_uuid = Uuid::new_v4();
    let (abort_handle, _) = AbortHandle::new_pair();
    let (oauth_result_tx, oauth_result_rx) = async_channel::unbounded();
    manager.spawned_servers.insert(
        installation_uuid,
        SpawnedServerInfo {
            abort_handle,
            oauth_result_tx,
        },
    );
    manager
        .pending_oauth_csrf
        .insert("test-state".to_string(), installation_uuid);
    let callback_url = Url::parse(
        "warpdev://mcp/oauth2callback?code=test-code&state=test-state&iss=https%3A%2F%2Fmcp.linear.app",
    )
    .unwrap();

    manager.handle_oauth_callback(&callback_url).unwrap();

    match oauth_result_rx.try_recv().unwrap() {
        CallbackResult::Success {
            code,
            csrf_token,
            issuer,
        } => {
            assert_eq!(code, "test-code");
            assert_eq!(csrf_token, "test-state");
            assert_eq!(issuer.as_deref(), Some("https://mcp.linear.app"));
        }
        CallbackResult::Error { error } => panic!("unexpected callback error: {error:?}"),
    }
}

#[test]
fn reconnectable_installation_falls_back_to_ephemeral_state() {
    let mut manager = TemplatableMCPServerManager::default();
    let installation = builtin::factory_mcp_installation("ephemeral-token", &[]);
    let installation_uuid = installation.uuid();

    manager
        .reconnectable_ephemeral_installations
        .insert(installation_uuid, installation);

    let resolved = manager
        .reconnectable_installation(installation_uuid)
        .expect("ephemeral installation should be reconnectable");
    assert!(resolved.template_json().contains("ephemeral-token"));
}

#[test]
fn reconnectable_installation_prefers_persisted_state() {
    let mut manager = TemplatableMCPServerManager::default();
    let installation_uuid = builtin::FACTORY_MCP_INSTALLATION_UUID;
    manager.reconnectable_ephemeral_installations.insert(
        installation_uuid,
        builtin::factory_mcp_installation("ephemeral-token", &[]),
    );
    manager.locally_installed_servers.insert(
        installation_uuid,
        builtin::factory_mcp_installation("persisted-token", &[]),
    );
    let resolved = manager
        .reconnectable_installation(installation_uuid)
        .expect("persisted installation should be reconnectable");
    assert!(resolved.template_json().contains("persisted-token"));
}

#[test]
fn reconnect_waiter_joins_pending_spawn() {
    let mut manager = TemplatableMCPServerManager::default();
    let installation_uuid = Uuid::new_v4();
    let (abort_handle, _) = AbortHandle::new_pair();
    let (oauth_result_tx, _) = async_channel::unbounded();
    manager.spawned_servers.insert(
        installation_uuid,
        SpawnedServerInfo {
            abort_handle,
            oauth_result_tx,
        },
    );
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();

    assert!(!manager.register_reconnect_waiter(installation_uuid, result_tx));
    assert!(manager.spawned_servers.contains_key(&installation_uuid));

    manager.notify_reconnect_waiters(installation_uuid, Err("spawn failed".to_string()));
    let error = result_rx
        .blocking_recv()
        .expect("waiter should be notified")
        .expect_err("spawn should fail");
    assert_eq!(error, "spawn failed");
}

#[test]
fn reconnect_completion_notifies_all_waiters() {
    let mut manager = TemplatableMCPServerManager::default();
    let installation_uuid = Uuid::new_v4();
    let (first_tx, first_rx) = tokio::sync::oneshot::channel();
    let (second_tx, second_rx) = tokio::sync::oneshot::channel();

    assert!(manager.register_reconnect_waiter(installation_uuid, first_tx));
    assert!(!manager.register_reconnect_waiter(installation_uuid, second_tx));

    manager.notify_reconnect_waiters(installation_uuid, Err("connection failed".to_string()));

    let first_error = first_rx
        .blocking_recv()
        .expect("first waiter should be notified")
        .expect_err("connection should fail");
    let second_error = second_rx
        .blocking_recv()
        .expect("second waiter should be notified")
        .expect_err("connection should fail");
    assert_eq!(first_error, "connection failed");
    assert_eq!(second_error, "connection failed");
    assert!(
        !manager
            .pending_reconnections
            .contains_key(&installation_uuid)
    );
}

#[test]
fn shutdown_ends_ephemeral_reconnect_lifecycle() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let global_resources = GlobalResourceHandles::mock(&mut app);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));

        let installation = builtin::factory_mcp_installation("ephemeral-token", &[]);
        let installation_uuid = installation.uuid();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let manager = app.add_model(|_| {
            let mut manager = TemplatableMCPServerManager::default();
            manager
                .reconnectable_ephemeral_installations
                .insert(installation_uuid, installation);
            manager.register_reconnect_waiter(installation_uuid, result_tx);
            manager
        });

        manager.update(&mut app, |manager, ctx| {
            manager.shutdown_server(installation_uuid, ctx);
        });

        manager.read(&app, |manager, _| {
            assert!(
                !manager
                    .reconnectable_ephemeral_installations
                    .contains_key(&installation_uuid)
            );
            assert!(
                !manager
                    .pending_reconnections
                    .contains_key(&installation_uuid)
            );
        });
        let error = result_rx
            .await
            .expect("waiter should be notified")
            .expect_err("shutdown should end the reconnect");
        assert_eq!(error, "Server shut down");
    });
}
