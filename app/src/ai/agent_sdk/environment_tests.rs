use std::sync::Arc;

use chrono::Utc;
use warp_graphql::object::{Space, SpaceType};
use warp_graphql::queries::get_runners::{Runner, RunnerArch, RunnerConfig, RunnerOs};
use warpui::{App, SingletonEntity};

use super::{EnvironmentCommandRunner, validate_default_runner};
use crate::ASSETS;
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, CloudAmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel,
};
use crate::auth::UserUid;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{
    CloudObjectMetadata, CloudObjectPermissions, Owner, Revision, RevisionAndLastEditor,
    UpdateCloudObjectResult,
};
use crate::server::cloud_objects::test_utils::{
    create_update_manager_struct, initialize_app, mock_server_api,
};
use crate::server::cloud_objects::update_manager::{
    ObjectOperation, ObjectOperationResult, OperationSuccessType, UpdateManagerEvent,
};
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::factory::MockFactoryClient;
use crate::server::sync_queue::SyncQueue;

fn runner(uid: &str, owner: Owner) -> Runner {
    let scope = match owner {
        Owner::Team { team_uid } => Space {
            uid: cynic::Id::new(team_uid.to_string()),
            type_: SpaceType::Team,
        },
        Owner::User { user_uid } => Space {
            uid: cynic::Id::new(user_uid.to_string()),
            type_: SpaceType::User,
        },
    };
    Runner {
        uid: cynic::Id::new(uid),
        config: RunnerConfig {
            name: "runner-name".to_string(),
            description: None,
            setup_commands: None,
            instance_shape: None,
            os: RunnerOs::Linux,
            arch: RunnerArch::X8664,
            mac: None,
            linux: None,
        },
        last_updated: chrono::DateTime::<Utc>::UNIX_EPOCH.into(),
        scope,
        creator: None,
        last_editor: None,
    }
}

#[tokio::test]
async fn default_runner_preflight_checks_environment_owner() {
    let team = Owner::Team {
        team_uid: ServerId::from(456),
    };
    let other_team = Owner::Team {
        team_uid: ServerId::from(789),
    };
    let personal = Owner::mock_current_user();
    let other_user = Owner::User {
        user_uid: UserUid::new("other-user"),
    };
    for (environment_owner, runner_owner, accepted) in [
        (team, team, true),
        (personal, personal, true),
        (team, other_team, false),
        (team, personal, false),
        (personal, team, false),
        (personal, other_user, false),
    ] {
        let mut factory = MockFactoryClient::new();
        factory
            .expect_get_runners()
            .withf(|sort_by, scope| sort_by.is_none() && scope.is_none())
            .once()
            .return_once(move |_, _| Ok(vec![runner("runner-uid", runner_owner)]));

        let result = validate_default_runner(&factory, "runner-uid", environment_owner).await;
        if accepted {
            result.unwrap();
        } else {
            assert!(result.unwrap_err().to_string().contains("same owner"));
        }
    }
}

#[tokio::test]
async fn default_runner_preflight_does_not_resolve_names() {
    let mut factory = MockFactoryClient::new();
    factory
        .expect_get_runners()
        .once()
        .return_once(|_, _| Ok(vec![runner("runner-uid", Owner::mock_current_user())]));

    let error = validate_default_runner(&factory, "runner-name", Owner::mock_current_user())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Runner 'runner-name' not found");
}

#[tokio::test]
async fn default_runner_preflight_propagates_lookup_errors() {
    let mut factory = MockFactoryClient::new();
    factory
        .expect_get_runners()
        .once()
        .return_once(|_, _| Err(anyhow::anyhow!("Runner lookup unavailable")));

    let error = validate_default_runner(&factory, "runner-uid", Owner::mock_current_user())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Runner lookup unavailable");
}

fn environment(server_id: ServerId) -> CloudAmbientAgentEnvironment {
    let model = serde_json::from_value::<AmbientAgentEnvironment>(serde_json::json!({
        "name": "environment",
        "description": "description",
        "docker_image": "ubuntu:latest",
        "code_forge": "GITHUB",
        "code_forges": ["GITHUB", "GITLAB"],
        "github_repos": [{"owner": "warpdotdev", "repo": "warp"}],
        "source_repos": [{"code_forge": "GITLAB", "owner": "group", "repo": "repo"}],
        "setup_commands": ["make setup"],
        "providers": {
            "gcp": {
                "project_number": "123",
                "workload_identity_federation_pool_id": "pool",
                "workload_identity_federation_provider_id": "provider",
                "service_account_email": "test@example.com"
            },
            "aws": {"role_arn": "arn:aws:iam::123:role/test"}
        },
        "secrets": [{"name": "TEST_SECRET"}],
        "default_runner_uid": "old-runner"
    }))
    .unwrap();
    let mut metadata = CloudObjectMetadata::mock();
    metadata.revision = Some(Revision::now());
    CloudAmbientAgentEnvironment::new(
        SyncId::ServerId(server_id),
        CloudAmbientAgentEnvironmentModel::new(model),
        metadata,
        CloudObjectPermissions::mock_personal(),
    )
}

#[test]
fn environment_update_serializes_default_runner_through_revision_aware_update() {
    for default_runner in [Some("new-runner"), None] {
        App::test(ASSETS, |mut app| async move {
            initialize_app(&mut app);
            let server_id = ServerId::from(123);
            let environment = environment(server_id);
            let mut expected = environment.model().string_model.clone();
            if let Some(uid) = default_runner {
                expected.default_runner_uid = Some(uid.to_string());
            } else {
                expected.name = "renamed".to_string();
            }
            let expected_json = serde_json::to_value(&expected).unwrap();
            let expected_revision = environment.metadata.revision;
            let mut server_api = mock_server_api();
            server_api
                .expect_update_generic_string_object()
                .withf(move |id, model, revision| {
                    *id == server_id.into()
                        && *revision == expected_revision
                        && serde_json::from_str::<serde_json::Value>(model.model_as_str()).unwrap()
                            == expected_json
                })
                .once()
                .return_once(|_, _, _| {
                    Ok(UpdateCloudObjectResult::Success {
                        revision_and_editor: RevisionAndLastEditor {
                            revision: Revision::now(),
                            last_editor_uid: None,
                        },
                    })
                });
            let updates = create_update_manager_struct(&mut app, Arc::new(server_api));
            CloudModel::handle(&app).update(&mut app, |model, _| {
                model.add_object(environment.id, environment.clone());
            });
            let command = app.add_singleton_model(|_| EnvironmentCommandRunner);
            command.update(&mut app, |_, ctx| {
                EnvironmentCommandRunner::update_environment_after_auth_check(
                    &environment,
                    server_id,
                    default_runner.is_none().then(|| "renamed".to_string()),
                    None,
                    false,
                    None,
                    default_runner.map(str::to_string),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    ctx,
                );
            });
            SyncQueue::handle(&app)
                .update(&mut app, |queue, ctx| {
                    ctx.await_spawned_future(queue.spawned_futures()[0])
                })
                .await;

            assert_eq!(
                CloudModel::handle(&app).read(&app, |model, _| {
                    model
                        .get_object_of_type::<_, CloudAmbientAgentEnvironmentModel>(&environment.id)
                        .unwrap()
                        .model()
                        .string_model
                        .clone()
                }),
                expected
            );
            assert!(updates.receiver.try_iter().count() > 0);
            assert!(app.termination_result().is_none());
        });
    }
}

#[test]
fn environment_update_reports_denied_operations() {
    App::test(ASSETS, |mut app| async move {
        initialize_app(&mut app);
        let updates = create_update_manager_struct(&mut app, Arc::new(mock_server_api()));
        SyncQueue::handle(&app).update(&mut app, |queue, _| queue.stop_dequeueing());
        let server_id = ServerId::from(123);
        let environment = environment(server_id);
        CloudModel::handle(&app).update(&mut app, |model, _| {
            model.add_object(environment.id, environment.clone());
        });
        let command = app.add_singleton_model(|_| EnvironmentCommandRunner);
        command.update(&mut app, |_, ctx| {
            EnvironmentCommandRunner::update_environment_after_auth_check(
                &environment,
                server_id,
                None,
                None,
                false,
                None,
                Some("new-runner".to_string()),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                ctx,
            );
        });
        updates.update_manager.update(&mut app, |_, ctx| {
            ctx.emit(UpdateManagerEvent::ObjectOperationComplete {
                result: ObjectOperationResult {
                    success_type: OperationSuccessType::Denied("Managed environment".to_string()),
                    operation: ObjectOperation::Update,
                    client_id: None,
                    server_id: Some(server_id),
                    num_objects: None,
                },
            });
        });

        let error = app.termination_result().unwrap().unwrap_err();
        assert_eq!(error.to_string(), "Failed to update environment");
    });
}
