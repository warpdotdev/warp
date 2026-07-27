use std::cell::RefCell;
use std::collections::HashMap;

use chrono::{Duration, Utc};
use warpui::{App, SingletonEntity as _};

use super::*;
use crate::ai::cloud_environments::{AmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel};
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions};
use crate::server::ids::{ClientId, SyncId};
use crate::test_util::settings::initialize_settings_for_tests;

fn environment(sync_id: SyncId, name: &str) -> CloudAmbientAgentEnvironment {
    let environment = AmbientAgentEnvironment::new(
        name.to_owned(),
        None,
        Vec::new(),
        "ubuntu:latest".to_owned(),
        Vec::new(),
    );
    CloudAmbientAgentEnvironment::new(
        sync_id,
        CloudAmbientAgentEnvironmentModel::new(environment),
        CloudObjectMetadata::mock(),
        CloudObjectPermissions::mock_personal(),
    )
}

#[test]
fn environment_creation_refreshes_after_cloud_model_inserts_the_object() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CloudModel::mock);
        let catalog = app.add_singleton_model(CloudEnvironmentCatalog::new);
        let sync_id = SyncId::ClientId(ClientId::new());
        let object = environment(sync_id, "Created environment");
        let (refresh_tx, refresh_rx) = futures::channel::oneshot::channel();
        let refresh_tx = RefCell::new(Some(refresh_tx));

        app.update(|ctx| {
            ctx.subscribe_to_model(&catalog, move |_, _, _| {
                if let Some(tx) = refresh_tx.borrow_mut().take() {
                    let _ = tx.send(());
                }
            });
            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                model.create_object(sync_id, object, ctx);
            });
            assert!(
                catalog.as_ref(ctx).environments().is_empty(),
                "create event fires before CloudModel inserts the object"
            );
        });

        refresh_rx
            .await
            .expect("environment creation should refresh the catalog");
        assert_eq!(
            catalog.read(&app, |catalog, _| catalog.environments().to_vec()),
            vec![CloudEnvironment {
                id: sync_id,
                name: "Created environment".to_owned(),
            }]
        );
    });
}

#[test]
fn environment_timestamp_updates_refresh_recency_order() {
    App::test((), |mut app| async move {
        let cloud_model = app.add_singleton_model(CloudModel::mock);
        let older_id = SyncId::ClientId(ClientId::new());
        let newer_id = SyncId::ClientId(ClientId::new());
        app.update(|ctx| {
            cloud_model.update(ctx, |model, ctx| {
                model.create_object(older_id, environment(older_id, "Alpha"), ctx);
                model.create_object(newer_id, environment(newer_id, "Zulu"), ctx);
            });
        });
        let catalog = app.add_singleton_model(CloudEnvironmentCatalog::new);
        assert_eq!(
            catalog.read(&app, |catalog, _| {
                catalog
                    .environments()
                    .iter()
                    .map(|environment| environment.id)
                    .collect::<Vec<_>>()
            }),
            vec![older_id, newer_id]
        );

        let now = Utc::now();
        app.update(|ctx| {
            cloud_model.update(ctx, |model, ctx| {
                model.update_environment_last_task_run_timestamps(
                    HashMap::from([
                        (older_id.uid(), now - Duration::hours(1)),
                        (newer_id.uid(), now),
                    ]),
                    ctx,
                );
            });
        });

        assert_eq!(
            catalog.read(&app, |catalog, _| {
                catalog
                    .environments()
                    .iter()
                    .map(|environment| environment.id)
                    .collect::<Vec<_>>()
            }),
            vec![newer_id, older_id]
        );
    });
}

#[test]
fn default_resolution_preserves_each_gui_consumer_name_tie_breaker() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let cloud_model = app.add_singleton_model(CloudModel::mock);
        let lowercase_id = SyncId::ClientId(ClientId::new());
        let uppercase_id = SyncId::ClientId(ClientId::new());
        app.update(|ctx| {
            cloud_model.update(ctx, |model, ctx| {
                model.create_object(lowercase_id, environment(lowercase_id, "alpha"), ctx);
                model.create_object(uppercase_id, environment(uppercase_id, "Beta"), ctx);
            });
        });
        let catalog = app.add_singleton_model(CloudEnvironmentCatalog::new);

        catalog.read(&app, |catalog, ctx| {
            assert_eq!(catalog.default_environment_id(ctx), Some(lowercase_id));
            assert_eq!(
                catalog.orchestration_default_environment_id(ctx),
                Some(uppercase_id)
            );
        });
    });
}
