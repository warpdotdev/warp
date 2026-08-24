use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use warp_cli::CliCommand;
use warp_cli::agent::Harness;
use warp_cli::artifact::{
    ArtifactCommand, DownloadArtifactArgs, GetArtifactArgs, UploadArtifactArgs,
};
use warp_cli::task::{MessageCommand, MessageSendArgs, MessageWatchArgs, TaskCommand};
use warp_core::telemetry::TelemetryEvent;
use warpui::{App, SingletonEntity as _};

use super::{
    AgentDriverRunner, CommandAuthentication, command_authentication, command_requires_auth,
    command_to_telemetry_event, reconcile_task_harness,
};
use crate::ai::agent_sdk::driver::{AgentDriverError, AgentDriverOptions};
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, CloudAmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel,
};
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions};
use crate::server::cloud_objects::test_utils::{
    create_update_manager_struct, initialize_app, mock_server_api,
};
use crate::server::cloud_objects::update_manager::InitialLoadResponse;
use crate::server::ids::{ServerId, SyncId};

const TASK_ID: &str = "00000000-0000-0000-0000-000000000001";

#[test]
fn logout_does_not_require_auth() {
    assert!(!command_requires_auth(&CliCommand::Logout));
}

#[test]
fn login_does_not_require_auth() {
    assert!(!command_requires_auth(&CliCommand::Login));
}

#[test]
fn pending_api_key_is_selected_for_command_authentication() {
    assert_eq!(
        command_authentication(Some("api-key".to_owned()), false),
        Some(CommandAuthentication::PendingApiKey("api-key".to_owned()))
    );
}

#[test]
fn pending_api_key_takes_precedence_over_persisted_auth() {
    assert_eq!(
        command_authentication(Some("api-key".to_owned()), true),
        Some(CommandAuthentication::PendingApiKey("api-key".to_owned()))
    );
}

#[test]
fn persisted_auth_is_refreshed_without_pending_api_key() {
    assert_eq!(
        command_authentication(None, true),
        Some(CommandAuthentication::RefreshUser)
    );
}

#[test]
fn logged_out_command_has_no_authentication_source() {
    assert_eq!(command_authentication(None, false), None);
}

#[test]
fn artifact_download_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Artifact(
        ArtifactCommand::Download(DownloadArtifactArgs {
            artifact_uid: "artifact-123".to_string(),
            out: None,
        },)
    )));
}

#[test]
fn run_message_send_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Run(
        TaskCommand::Message(MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),)
    )));
}

#[test]
fn artifact_get_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Artifact(
        ArtifactCommand::Get(GetArtifactArgs {
            artifact_uid: "artifact-123".to_string(),
        },)
    )));
}

#[test]
fn artifact_upload_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Artifact(
        ArtifactCommand::Upload(UploadArtifactArgs {
            path: "artifact.txt".into(),
            run_id: Some("run-123".to_string()),
            conversation_id: None,
            description: None,
        },)
    )));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_uses_canonical_harness_from_env() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("OZ_HARNESS", "  CLAUDE  ") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };

    assert_eq!(event.payload(), Some(json!({ "harness": "claude" })));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_supports_claude_code_alias() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("OZ_HARNESS", "CLAUDE_CODE") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };

    assert_eq!(event.payload(), Some(json!({ "harness": "claude" })));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_supports_opencode_harness() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("OZ_HARNESS", "opencode") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };

    assert_eq!(event.payload(), Some(json!({ "harness": "opencode" })));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_defaults_to_unknown_harness() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));

    assert_eq!(event.payload(), Some(json!({ "harness": "unknown" })));
}

#[test]
fn reconcile_task_harness_adopts_task_harness_when_cli_uses_default() {
    let mut selected_harness = Harness::Oz;
    let harness = reconcile_task_harness(TASK_ID, &mut selected_harness, Harness::Claude)
        .expect("default harness should adopt task harness");

    assert_eq!(selected_harness, Harness::Claude);
    assert_eq!(harness.harness(), Harness::Claude);
}

#[test]
fn reconcile_task_harness_allows_matching_explicit_harness() {
    let mut selected_harness = Harness::Claude;
    let harness = reconcile_task_harness(TASK_ID, &mut selected_harness, Harness::Claude)
        .expect("matching harness should succeed");

    assert_eq!(selected_harness, Harness::Claude);
    assert_eq!(harness.harness(), Harness::Claude);
}

#[test]
fn reconcile_task_harness_rejects_explicit_mismatch() {
    let mut selected_harness = Harness::Gemini;
    let err = reconcile_task_harness(TASK_ID, &mut selected_harness, Harness::Claude)
        .expect_err("mismatched harness should fail");

    assert_eq!(selected_harness, Harness::Gemini);
    assert!(err.to_string().contains("Task"));
    assert!(err.to_string().contains("--harness gemini"));
    assert!(err.to_string().contains("claude"));
}

#[test]
#[serial_test::serial]
fn run_message_watch_telemetry_defaults_to_unknown_harness() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Watch(MessageWatchArgs {
            run_id: "run-123".to_string(),
            since_sequence: 0,
        }),
    )));

    assert_eq!(event.payload(), Some(json!({ "harness": "unknown" })));
}

// ── `resolve_environment` regression matrix ──────────────────────────────────
//
// Classification must be driven by whether the *sync* was healthy, never by
// whether the catalog happens to be empty. Each cell below combines sync
// health (via `UpdateManager::mock_initial_load`) with catalog population
// (via a real `CloudModel` object, added independently of the id under
// test) to prove catalog size has no bearing on the outcome.

fn minimal_driver_options() -> AgentDriverOptions {
    AgentDriverOptions {
        working_dir: PathBuf::from("/tmp"),
        secrets: HashMap::new(),
        task_id: None,
        parent_run_id: None,
        should_share: false,
        idle_on_complete: None,
        idle_on_fail: None,
        resume: None,
        cloud_providers: Vec::new(),
        environment: None,
        additional_source_repos: Vec::new(),
        selected_harness: Harness::Oz,
        third_party_harness_model_config: None,
        snapshot_disabled: None,
        snapshot_upload_timeout: None,
        snapshot_script_timeout: None,
        checkpoint_interval: None,
        skip_initial_turn: false,
        strict_mcp_startup: false,
        mcp_startup_timeout: None,
    }
}

/// Seeds an environment into `CloudModel` with a server id unrelated to any id looked up in
/// these tests, so "catalog populated" cells have a real, unrelated row while the target id
/// remains genuinely absent.
fn seed_unrelated_environment(app: &mut App, server_id: i64) {
    let sync_id = SyncId::ServerId(ServerId::from(server_id));
    let environment = AmbientAgentEnvironment::new(
        "Unrelated Env".to_string(),
        None,
        vec![],
        "ubuntu:latest".to_string(),
        vec![],
    );
    let object = CloudAmbientAgentEnvironment::new(
        sync_id,
        CloudAmbientAgentEnvironmentModel::new(environment),
        CloudObjectMetadata::mock(),
        CloudObjectPermissions::mock_personal(),
    );
    CloudModel::handle(app).update(app, |cloud_model, _| {
        cloud_model.add_object(sync_id, object);
    });
}

/// Drives the real `AgentDriverRunner::resolve_environment` for a missing environment id,
/// through a live `ModelSpawner`, and returns its result.
async fn resolve_missing_environment(
    app: &mut App,
    missing_id: &str,
) -> Result<(), AgentDriverError> {
    let runner = app.add_singleton_model(|_| AgentDriverRunner);
    let (tx, rx) = futures::channel::oneshot::channel();
    let missing_id = missing_id.to_string();
    runner.update(app, |_, ctx| {
        let spawner = ctx.spawner();
        let mut driver_options = minimal_driver_options();
        ctx.spawn(
            async move {
                let result = AgentDriverRunner::resolve_environment(
                    &spawner,
                    Some(missing_id),
                    &mut driver_options,
                )
                .await;
                let _ = tx.send(result);
            },
            |_, _, _| {},
        );
    });
    rx.await.expect("resolve_environment task should complete")
}

#[test]
fn resolve_environment_is_not_found_when_sync_healthy_and_catalog_empty() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let update_manager_struct =
            create_update_manager_struct(&mut app, Arc::new(mock_server_api()));
        update_manager_struct
            .update_manager
            .update(&mut app, |update_manager, ctx| {
                update_manager.mock_initial_load(InitialLoadResponse::default(), ctx);
            });

        let missing_id = ServerId::from(999_001_i64).to_string();
        let result = resolve_missing_environment(&mut app, &missing_id).await;

        assert!(
            matches!(&result, Err(AgentDriverError::EnvironmentNotFound(id)) if id == &missing_id),
            "expected EnvironmentNotFound, got {result:?}"
        );
    });
}

#[test]
fn resolve_environment_is_catalog_unavailable_when_sync_unhealthy_and_catalog_empty() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let update_manager_struct =
            create_update_manager_struct(&mut app, Arc::new(mock_server_api()));
        update_manager_struct
            .update_manager
            .update(&mut app, |update_manager, ctx| {
                update_manager.mock_initial_load(
                    InitialLoadResponse {
                        had_errors: true,
                        ..Default::default()
                    },
                    ctx,
                );
            });

        let missing_id = ServerId::from(999_002_i64).to_string();
        let result = resolve_missing_environment(&mut app, &missing_id).await;

        assert!(
            matches!(&result, Err(AgentDriverError::EnvironmentCatalogUnavailable(id)) if id == &missing_id),
            "an empty catalog after an unhealthy sync must not be reported as a genuine not-found: {result:?}"
        );
    });
}

#[test]
fn resolve_environment_is_not_found_when_sync_healthy_despite_unrelated_catalog_entries() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        seed_unrelated_environment(&mut app, 999_010);
        let update_manager_struct =
            create_update_manager_struct(&mut app, Arc::new(mock_server_api()));
        update_manager_struct
            .update_manager
            .update(&mut app, |update_manager, ctx| {
                update_manager.mock_initial_load(InitialLoadResponse::default(), ctx);
            });

        let missing_id = ServerId::from(999_011_i64).to_string();
        let result = resolve_missing_environment(&mut app, &missing_id).await;

        assert!(
            matches!(&result, Err(AgentDriverError::EnvironmentNotFound(id)) if id == &missing_id),
            "a healthy sync must report a genuine not-found even though the catalog has other rows: {result:?}"
        );
    });
}

#[test]
fn resolve_environment_is_catalog_unavailable_when_sync_unhealthy_despite_populated_catalog() {
    // This is the exact shape of the original incident: a partial sync failure leaves stale or
    // unrelated rows in the catalog (non-empty), while the requested environment is silently
    // missing. Catalog size alone would (wrongly) call this a genuine not-found.
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        seed_unrelated_environment(&mut app, 999_020);
        let update_manager_struct =
            create_update_manager_struct(&mut app, Arc::new(mock_server_api()));
        update_manager_struct
            .update_manager
            .update(&mut app, |update_manager, ctx| {
                update_manager.mock_initial_load(
                    InitialLoadResponse {
                        had_errors: true,
                        ..Default::default()
                    },
                    ctx,
                );
            });

        let missing_id = ServerId::from(999_021_i64).to_string();
        let result = resolve_missing_environment(&mut app, &missing_id).await;

        assert!(
            matches!(&result, Err(AgentDriverError::EnvironmentCatalogUnavailable(id)) if id == &missing_id),
            "a non-empty catalog must not mask an unhealthy sync: {result:?}"
        );
    });
}
