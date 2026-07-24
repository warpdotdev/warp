use warpui::{App, SingletonEntity as _};

use super::*;
use crate::ai::cloud_environments::{AmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel};
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions};
use crate::server::ids::{ClientId, SyncId};

#[test]
fn environment_creation_refreshes_after_cloud_model_inserts_the_object() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CloudModel::mock);
        let catalog = app.add_singleton_model(CloudEnvironmentCatalog::new);
        let sync_id = SyncId::ClientId(ClientId::new());
        let environment = AmbientAgentEnvironment::new(
            "Created environment".to_owned(),
            None,
            Vec::new(),
            "ubuntu:latest".to_owned(),
            Vec::new(),
        );
        let object = CloudAmbientAgentEnvironment::new(
            sync_id,
            CloudAmbientAgentEnvironmentModel::new(environment),
            CloudObjectMetadata::mock(),
            CloudObjectPermissions::mock_personal(),
        );

        app.update(|ctx| {
            CloudModel::handle(ctx).update(ctx, |model, ctx| {
                model.create_object(sync_id, object, ctx);
            });
            assert!(
                catalog.as_ref(ctx).environments().is_empty(),
                "create event fires before CloudModel inserts the object"
            );
        });

        for _ in 0..10 {
            if catalog.read(&app, |catalog, _| !catalog.environments().is_empty()) {
                break;
            }
            futures_lite::future::yield_now().await;
        }
        assert_eq!(
            catalog.read(&app, |catalog, _| catalog.environments().to_vec()),
            vec![CloudEnvironment {
                id: sync_id,
                name: "Created environment".to_owned(),
            }]
        );
    });
}
