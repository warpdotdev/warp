mod assertion;

use std::future::Future;
use std::pin::Pin;

pub use assertion::*;
use futures::FutureExt;
use futures::future::join_all;
use itertools::Itertools;
use warpui::{App, SingletonEntity};

use crate::cloud_object::Space;
use crate::cloud_object::model::persistence::CloudModel;
use crate::server::cloud_objects::update_manager::UpdateManager;

/// Clears the cloud model of all non-welcome objects in the user's personal space.
/// Returns a future that resolves when the cloud model is cleared.
pub fn clear_cloud_model(app: &mut App) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    let object_ids_to_delete = CloudModel::handle(app).read(app, |cloud_model, ctx| {
        cloud_model
            .active_non_welcome_cloud_objects_in_space(Space::Personal, ctx)
            .map(|object| object.cloud_object_type_and_id())
            .collect_vec()
    });

    let mut futures = Vec::new();
    for object_id in object_ids_to_delete {
        UpdateManager::handle(app).update(app, |update_manager, ctx| {
            update_manager.delete_object_by_user(object_id, ctx);
            if let Some(future_id) = update_manager.spawned_futures().last() {
                let future = ctx.await_spawned_future(*future_id);
                futures.push(future);
            }
        });
    }

    Box::pin(join_all(futures).map(|_| ()))
}
