use std::collections::HashMap;

use uuid::Uuid;
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warp_core::features::FeatureFlag;
use warpui::{App, Entity, ModelHandle};
use warpui_extras::secure_storage;

use super::*;
use crate::ai::mcp::builtin;
use crate::ai::mcp::templatable::{JsonTemplate, TemplatableMCPServer};

/// Registers the singletons `installation_uses_credential_store` and
/// `sync_builtin_servers` read, without any of the app-wide state (cloud
/// sync, telemetry, log files) that a real spawn would need.
fn setup_app(app: &mut App) {
    app.add_singleton_model(FileBasedMCPManager::new);
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.update(secure_storage::register_unavailable);
}

fn dummy_installation(uuid: Uuid, template_uuid: Uuid) -> TemplatableMCPServerInstallation {
    TemplatableMCPServerInstallation::new(
        uuid,
        TemplatableMCPServer {
            uuid: template_uuid,
            name: "test-server".to_string(),
            description: None,
            template: JsonTemplate {
                json: "{}".to_string(),
                variables: Vec::new(),
            },
            version: 0,
            gallery_data: None,
        },
        HashMap::new(),
    )
}

/// Test-only collector for `CredentialsChanged` events.
#[derive(Default)]
struct CredentialEvents {
    changed: Vec<Uuid>,
}

impl Entity for CredentialEvents {
    type Event = ();
}

fn subscribe_credential_events(
    app: &mut App,
    manager: &ModelHandle<TemplatableMCPServerManager>,
) -> ModelHandle<CredentialEvents> {
    let events = app.add_model(|_| CredentialEvents::default());
    events.update(app, |_, ctx| {
        ctx.subscribe_to_model(manager, |me, _, event, _| {
            if let TemplatableMCPServerManagerEvent::CredentialsChanged { uuid } = event {
                me.changed.push(*uuid);
            }
        });
    });
    events
}

/// This is the exact guard `spawn_server_impl` now checks before calling
/// `delete_credentials_from_secure_storage` on a failed spawn. The built-in
/// Factory MCP server is never inserted into `locally_installed_servers` and
/// is never file-based, so it must never be treated as having credentials to
/// clean up.
#[test]
fn ephemeral_installations_never_use_the_credential_store() {
    App::test((), |mut app| async move {
        setup_app(&mut app);
        let manager = app.add_model(|_| TemplatableMCPServerManager::default());

        manager.read(&app, |manager, ctx| {
            assert!(
                !manager.installation_uses_credential_store(
                    builtin::FACTORY_MCP_INSTALLATION_UUID,
                    ctx,
                )
            );
        });
    });
}

/// A locally-installed server's credential cleanup is unaffected by the new
/// guard: it's found via `locally_installed_servers`, so the guard passes and
/// `delete_credentials_from_secure_storage` still emits `CredentialsChanged`.
#[test]
fn locally_installed_servers_are_still_cleaned_up_on_failure() {
    App::test((), |mut app| async move {
        setup_app(&mut app);
        let manager = app.add_model(|_| TemplatableMCPServerManager::default());
        let events = subscribe_credential_events(&mut app, &manager);

        let installation_uuid = Uuid::new_v4();
        let template_uuid = Uuid::new_v4();
        manager.update(&mut app, |manager, _| {
            manager.locally_installed_servers.insert(
                installation_uuid,
                dummy_installation(installation_uuid, template_uuid),
            );
        });

        manager.read(&app, |manager, ctx| {
            assert!(manager.installation_uses_credential_store(installation_uuid, ctx));
        });

        manager.update(&mut app, |manager, ctx| {
            manager.delete_credentials_from_secure_storage(installation_uuid, ctx);
        });

        events.read(&app, |events, _| {
            assert_eq!(events.changed, vec![installation_uuid]);
        });
    });
}

/// While `builtin_server_forbidden` is set, `sync_builtin_servers` must treat
/// the built-in server as ineligible even though every other eligibility
/// condition (feature flag, execution mode, login state) holds - proving the
/// 403 backoff actually gates the respawn path.
#[test]
fn builtin_server_forbidden_clears_tracked_builtin_state() {
    App::test((), |mut app| async move {
        setup_app(&mut app);
        let _factory_mcp_override = FeatureFlag::FactoryMcp.override_enabled(true);
        let manager = app.add_model(|_| TemplatableMCPServerManager::default());

        manager.update(&mut app, |manager, _| {
            manager.builtin_server_forbidden = true;
            manager.builtin_server_token = Some("stale-token".to_string());
        });

        manager.update(&mut app, |manager, ctx| {
            manager.sync_builtin_servers(false, ctx);
        });

        manager.read(&app, |manager, _| {
            assert_eq!(manager.builtin_server_token, None);
        });
    });
}

/// Contrasts with the previous test: with `builtin_server_forbidden` unset,
/// the same eligibility conditions leave a pending spawn alone (the existing
/// `is_active && !force_respawn` short-circuit), so `builtin_server_token` is
/// untouched. Together, the two tests isolate the effect of the new flag from
/// every other eligibility condition.
#[test]
fn sync_builtin_servers_leaves_state_untouched_when_not_forbidden() {
    App::test((), |mut app| async move {
        setup_app(&mut app);
        let _factory_mcp_override = FeatureFlag::FactoryMcp.override_enabled(true);
        let manager = app.add_model(|_| TemplatableMCPServerManager::default());

        // Mark the built-in as already spawning, so `sync_builtin_servers`
        // takes the early-return path instead of attempting a real spawn.
        let (oauth_result_tx, _oauth_result_rx) = async_channel::unbounded();
        let (abort_handle, _registration) = futures_util::stream::AbortHandle::new_pair();
        manager.update(&mut app, |manager, _| {
            manager.builtin_server_forbidden = false;
            manager.builtin_server_token = Some("stale-token".to_string());
            manager.spawned_servers.insert(
                builtin::FACTORY_MCP_INSTALLATION_UUID,
                SpawnedServerInfo {
                    abort_handle,
                    oauth_result_tx,
                },
            );
        });

        manager.update(&mut app, |manager, ctx| {
            manager.sync_builtin_servers(false, ctx);
        });

        manager.read(&app, |manager, _| {
            assert_eq!(manager.builtin_server_token.as_deref(), Some("stale-token"));
        });
    });
}
