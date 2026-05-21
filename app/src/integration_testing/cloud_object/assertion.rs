use warpui::{async_assert, integration::AssertionCallback};

use crate::{
    cloud_object::{model::persistence::CloudModel, CloudModelType, GenericCloudObject, Revision},
    server::ids::{HashableId, ServerId, SyncId, ToServerId},
};

/// Asserts metadata exists for the object with the given key and that the revision in that
/// metadata matches the given expected revision.
pub fn assert_metadata_revision<K, M>(id: &str, expected_revision: i64) -> AssertionCallback
where
    K: HashableId + ToServerId + std::fmt::Debug + Into<String> + Clone + 'static,
    M: CloudModelType<IdType = K, CloudObjectType = GenericCloudObject<K, M>> + 'static,
{
    let id = SyncId::ServerId(ServerId::try_from(id).expect("ID is invalid"));
    Box::new(move |app, _window_id| {
        let revision =
            app.get_singleton_model_handle::<CloudModel>()
                .read(app, |cloud_model, _| {
                    let object = cloud_model
                        .get_object_of_type::<K, M>(&id)
                        .expect("object should exist");
                    object
                        .metadata
                        .revision
                        .clone()
                        .expect("revision should exist")
                });
        async_assert!(
            revision
                == Revision::from_unix_timestamp_micros(expected_revision)
                    .expect("revision should parse"),
            "Expected revision to be:{expected_revision:?}\nBut got:\n{revision:?}"
        )
    })
}
