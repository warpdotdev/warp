//! Tests for spawn-config retention and reconnect installation lookup.

use std::collections::HashMap;

use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::{App, ModelHandle};

use super::super::KnownServerFacts;
use super::{RetainedSpawnConfig, SpawnProvenance};
use crate::ai::mcp::{
    JsonTemplate, TemplatableMCPServer, TemplatableMCPServerInstallation,
    TemplatableMCPServerManager,
};
use crate::auth::AuthStateProvider;

fn setup_app(app: &mut App) -> ModelHandle<TemplatableMCPServerManager> {
    app.add_singleton_model(|_| {
        settings::PublicPreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
    app.add_singleton_model(|_| {
        settings::PrivatePreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
    let global_resources = crate::GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| {
        crate::GlobalResourceHandlesProvider::new(global_resources.clone())
    });
    app.add_singleton_model(|_| TemplatableMCPServerManager::default())
}

fn test_installation(name: &str) -> TemplatableMCPServerInstallation {
    let server = TemplatableMCPServer {
        uuid: Uuid::new_v4(),
        name: name.to_string(),
        description: None,
        template: JsonTemplate {
            json: format!(r#"{{"{name}": {{"command": "echo", "args": []}}}}"#),
            variables: Vec::new(),
        },
        version: 0,
        gallery_data: None,
    };
    TemplatableMCPServerInstallation::new(Uuid::new_v4(), server, HashMap::new())
}

#[test]
fn respawnable_config_returns_retained_config_for_ephemeral_servers() {
    let _flag = FeatureFlag::McpSelfHeal.override_enabled(true);
    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let installation = test_installation("cli-server");
        let uuid = installation.uuid();

        manager.update(&mut app, |manager, ctx| {
            manager.spawn_configs.insert(
                uuid,
                RetainedSpawnConfig {
                    installation: installation.clone(),
                    provenance: SpawnProvenance::CliEphemeral,
                },
            );

            let (found, provenance) = manager
                .respawnable_config(uuid, ctx)
                .expect("retained ephemeral config should be respawnable");
            assert_eq!(found.uuid(), uuid);
            assert!(matches!(provenance, SpawnProvenance::CliEphemeral));
        });
    });
}

#[test]
fn respawnable_config_requires_local_install_when_flag_disabled() {
    let _flag = FeatureFlag::McpSelfHeal.override_enabled(false);
    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let ephemeral = test_installation("cli-server");
        let ephemeral_uuid = ephemeral.uuid();
        let local = test_installation("local-server");
        let local_uuid = local.uuid();

        manager.update(&mut app, |manager, ctx| {
            manager.spawn_configs.insert(
                ephemeral_uuid,
                RetainedSpawnConfig {
                    installation: ephemeral.clone(),
                    provenance: SpawnProvenance::CliEphemeral,
                },
            );
            manager
                .locally_installed_servers
                .insert(local_uuid, local.clone());

            // With the flag off, retained ephemeral configs are ignored,
            // matching pre-self-heal behavior.
            let err = manager
                .respawnable_config(ephemeral_uuid, ctx)
                .expect_err("ephemeral servers should not reconnect with the flag off");
            assert_eq!(err, "Installation not found");

            let (found, provenance) = manager
                .respawnable_config(local_uuid, ctx)
                .expect("locally installed servers should still reconnect");
            assert_eq!(found.uuid(), local_uuid);
            assert!(matches!(provenance, SpawnProvenance::LocallyInstalled));
        });
    });
}

#[test]
fn respawnable_config_prefers_live_locally_installed_config() {
    let _flag = FeatureFlag::McpSelfHeal.override_enabled(true);
    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let stale = test_installation("stale-server");
        let uuid = stale.uuid();
        let edited = TemplatableMCPServerInstallation::new(
            uuid,
            TemplatableMCPServer {
                name: "edited-server".to_string(),
                ..stale.templatable_mcp_server().clone()
            },
            HashMap::new(),
        );

        manager.update(&mut app, |manager, ctx| {
            manager.spawn_configs.insert(
                uuid,
                RetainedSpawnConfig {
                    installation: stale.clone(),
                    provenance: SpawnProvenance::LocallyInstalled,
                },
            );
            manager
                .locally_installed_servers
                .insert(uuid, edited.clone());

            let (found, _) = manager
                .respawnable_config(uuid, ctx)
                .expect("locally installed config should be respawnable");
            assert_eq!(found.templatable_mcp_server().name, "edited-server");
        });
    });
}

#[test]
fn respawnable_config_falls_back_to_locally_installed_without_retained_entry() {
    let _flag = FeatureFlag::McpSelfHeal.override_enabled(true);
    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let local = test_installation("local-server");
        let uuid = local.uuid();

        manager.update(&mut app, |manager, ctx| {
            manager
                .locally_installed_servers
                .insert(uuid, local.clone());

            let (found, provenance) = manager
                .respawnable_config(uuid, ctx)
                .expect("locally installed servers should reconnect without a retained entry");
            assert_eq!(found.uuid(), uuid);
            assert!(matches!(provenance, SpawnProvenance::LocallyInstalled));

            let err = manager
                .respawnable_config(Uuid::new_v4(), ctx)
                .expect_err("unknown servers cannot reconnect");
            assert_eq!(err, "Installation not found");
        });
    });
}

#[test]
fn shutdown_clears_retained_config_and_fails_reconnect_waiters() {
    let _flag = FeatureFlag::McpSelfHeal.override_enabled(true);
    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let installation = test_installation("cli-server");
        let uuid = installation.uuid();
        let (tx, mut rx) = tokio::sync::oneshot::channel();

        manager.update(&mut app, |manager, ctx| {
            manager.spawn_configs.insert(
                uuid,
                RetainedSpawnConfig {
                    installation: installation.clone(),
                    provenance: SpawnProvenance::CliEphemeral,
                },
            );
            manager.pending_reconnections.insert(uuid, vec![tx]);

            manager.shutdown_server(uuid, ctx);

            let err = manager
                .respawnable_config(uuid, ctx)
                .expect_err("an explicitly stopped server must not be respawnable");
            assert_eq!(err, "Installation not found");
        });

        let waiter_result = rx
            .try_recv()
            .expect("shutdown should notify pending reconnect waiters");
        assert_eq!(waiter_result.unwrap_err(), "Server was shut down");
    });
}

fn known_facts(name: &str, tool_names: &[&str]) -> KnownServerFacts {
    KnownServerFacts {
        name: name.to_string(),
        tools: tool_names
            .iter()
            .map(|tool| {
                rmcp::model::Tool::new(
                    tool.to_string(),
                    "test tool",
                    rmcp::model::JsonObject::new(),
                )
            })
            .collect(),
        resources: Vec::new(),
    }
}

#[test]
fn known_server_facts_keep_tools_visible_during_reconnect_windows() {
    let _flag = FeatureFlag::McpSelfHeal.override_enabled(true);
    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let installation = test_installation("cli-server");
        let uuid = installation.uuid();

        manager.update(&mut app, |manager, _ctx| {
            manager
                .known_servers
                .insert(uuid, known_facts("cli-server", &["do_thing"]));

            // Not eligible without a retained spawn config (e.g. deleted).
            assert!(manager.tools_for_server(uuid).is_empty());

            manager.spawn_configs.insert(
                uuid,
                RetainedSpawnConfig {
                    installation: installation.clone(),
                    provenance: SpawnProvenance::CliEphemeral,
                },
            );

            let tools = manager.tools_for_server(uuid);
            assert_eq!(tools.len(), 1);
            assert_eq!(tools[0].name, "do_thing");
            assert_eq!(manager.tools().count(), 1);
            assert_eq!(
                manager.server_from_tool("do_thing".to_string()),
                Some(&uuid)
            );
        });
    });
}

#[test]
fn known_server_facts_are_inert_with_the_flag_disabled() {
    let _flag = FeatureFlag::McpSelfHeal.override_enabled(false);
    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let installation = test_installation("cli-server");
        let uuid = installation.uuid();

        manager.update(&mut app, |manager, _ctx| {
            manager
                .known_servers
                .insert(uuid, known_facts("cli-server", &["do_thing"]));
            manager.spawn_configs.insert(
                uuid,
                RetainedSpawnConfig {
                    installation: installation.clone(),
                    provenance: SpawnProvenance::CliEphemeral,
                },
            );

            assert!(manager.tools_for_server(uuid).is_empty());
            assert_eq!(manager.tools().count(), 0);
            assert_eq!(manager.server_from_tool("do_thing".to_string()), None);
        });
    });
}

#[test]
fn repeated_reconnect_failures_trip_the_circuit_breaker() {
    let _flag = FeatureFlag::McpSelfHeal.override_enabled(true);
    App::test((), |mut app| async move {
        let manager = setup_app(&mut app);
        let installation = test_installation("flaky-server");
        let uuid = installation.uuid();
        let (tx, mut rx) = tokio::sync::oneshot::channel();

        manager.update(&mut app, |manager, ctx| {
            manager
                .known_servers
                .insert(uuid, known_facts("flaky-server", &[]));
            manager.record_reconnect_failure(uuid);
            manager.record_reconnect_failure(uuid);

            let backoff = manager.reconnect_backoff.get(&uuid).expect("backoff entry");
            assert_eq!(backoff.consecutive_failures, 2);
            assert!(backoff.blocked_until.expect("blocked") > std::time::Instant::now());

            manager.reconnect_server(uuid, tx, ctx);
            // Blocked: no reconnection was started.
            assert!(!manager.pending_reconnections.contains_key(&uuid));
        });

        let result = rx.try_recv().expect("breaker answers immediately");
        let err = result.expect_err("breaker refuses the reconnect");
        assert!(err.contains("flaky-server"), "unexpected error: {err}");
    });
}

#[test]
fn builtin_reconnect_without_credentials_fails() {
    let _flag = FeatureFlag::McpSelfHeal.override_enabled(true);
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        let manager = setup_app(&mut app);
        let installation = test_installation("warp-factory");
        let uuid = installation.uuid();

        manager.update(&mut app, |manager, ctx| {
            manager.spawn_configs.insert(
                uuid,
                RetainedSpawnConfig {
                    installation: installation.clone(),
                    provenance: SpawnProvenance::Builtin,
                },
            );

            let err = manager
                .respawnable_config(uuid, ctx)
                .expect_err("builtin reconnect requires a usable bearer token");
            assert!(err.contains("bearer token"), "unexpected error: {err}");
        });
    });
}
