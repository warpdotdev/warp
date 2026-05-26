//! Persistence utilities for cloud objects.

mod cloud_objects;

pub use cloud_objects::{decode_guests, decode_link_sharing, encode_guests, encode_link_sharing};
use diesel::SqliteConnection;
use diesel::result::Error;
use persistence::model::{NewObjectMetadata, NewObjectPermissions, ObjectMetadata};
use persistence::schema;
use warp_core::features::FeatureFlag;

use crate::cloud_object::{
    CloudObjectMetadata, CloudObjectPermissions, ObjectIdType, ObjectType, Owner,
};
use crate::ids::SyncId;

/// The sqlite id of a cloud object.
pub type CloudObjectId = i32;

/// When upserting a cloud object, this callback is used to create the cloud
/// object itself. It returns the id of the created cloud object.
/// Note: the supplied conn has already started a transaction.
pub type CreateCloudObjectFn =
    Box<dyn FnOnce(&mut SqliteConnection) -> Result<CloudObjectId, Error>>;

/// When upserting a cloud object, this callback is used to update the cloud
/// object. It takes the id of the cloud object to update as a parameter.
/// The supplied conn has already started a transaction.
pub type UpdateCloudObjectFn =
    Box<dyn FnOnce(&mut SqliteConnection, CloudObjectId) -> Result<(), Error>>;

pub fn upsert_cloud_object(
    conn: &mut SqliteConnection,
    cloud_object_type: ObjectType,
    sync_id: SyncId,
    cloud_object_metadata: CloudObjectMetadata,
    cloud_object_permissions: CloudObjectPermissions,
    create_object_fn: CreateCloudObjectFn,
    update_object_fn: UpdateCloudObjectFn,
) -> Result<(), Error> {
    use diesel::prelude::*;
    use schema::object_metadata::dsl::{
        client_id, current_editor, folder_id, is_pending, last_editor_uid,
        metadata_last_updated_ts, object_metadata, revision_ts, server_id, trashed_ts,
    };
    use schema::object_permissions::dsl::{
        anyone_with_link_access_level, anyone_with_link_source, object_guests, object_metadata_id,
        object_permissions, permissions_last_updated_at, subject_id, subject_type, subject_uid,
    };

    let (subject_type_value, subject_id_value, subject_uid_value) =
        match cloud_object_permissions.owner {
            Owner::User { user_uid } => ("USER", Some(user_uid.to_string()), user_uid.to_string()),
            Owner::Team { team_uid } => ("TEAM", None, team_uid.to_string()),
        };
    let permissions_ts = cloud_object_permissions
        .permissions_last_updated_ts
        .map(|ts| ts.timestamp_micros());
    let guests = if FeatureFlag::SharedWithMe.is_enabled() {
        match encode_guests(&cloud_object_permissions.guests) {
            Ok(guests) => Some(guests),
            Err(err) => {
                log::warn!("Unable to encode guests: {err:#}");
                None
            }
        }
    } else {
        None
    };
    let (anyone_with_link_access_level_value, anyone_with_link_source_value) =
        if FeatureFlag::SharedWithMe.is_enabled() {
            match cloud_object_permissions
                .anyone_with_link
                .as_ref()
                .map(encode_link_sharing)
            {
                Some(Ok((access_level, source))) => (Some(access_level), source),
                Some(Err(err)) => {
                    log::warn!("Unable to encode link-sharing setting: {err:#}");
                    (None, None)
                }
                None => (None, None),
            }
        } else {
            (None, None)
        };

    let revision = cloud_object_metadata
        .revision
        .as_ref()
        .map(|r| r.timestamp_micros());
    let has_pending_content_changes = cloud_object_metadata.has_pending_content_changes();

    // Filter to find metadata row.
    // The diesel types for `filter`s are dependent on the columns being filtered
    // so while the `hashed_sync_id` will only match one of `client_id` and `server_id`,
    // we filter on both here for ergonomics.
    let hashed_sync_id = sync_id.sqlite_uid_hash(cloud_object_type.into());
    let metadata_filter = object_metadata
        .filter(client_id.eq(Some(hashed_sync_id.as_str())))
        .or_filter(server_id.eq(Some(hashed_sync_id.as_str())));
    let metadata: Option<ObjectMetadata> = metadata_filter.first(conn).ok();

    match metadata {
        Some(metadata) => {
            // The object already exists in sqlite so update the object.
            update_object_fn(conn, metadata.shareable_object_id)?;

            let metadata_last_updated_at = cloud_object_metadata
                .metadata_last_updated_ts
                .map(|ts| ts.timestamp_micros());

            let trashed_timestamp = cloud_object_metadata
                .trashed_ts
                .map(|ts| ts.timestamp_micros());

            let folder_id_str = cloud_object_metadata
                .folder_id
                .map(|folder_sync_id| folder_sync_id.sqlite_uid_hash(ObjectIdType::Folder));

            // Update the metadata. Note: this is holistic write of all the metadata based on the current state of the in-memory object.
            // TODO: we need to update author_id as well.
            diesel::update(metadata_filter)
                .set((
                    revision_ts.eq(revision),
                    is_pending.eq(has_pending_content_changes),
                    last_editor_uid.eq(cloud_object_metadata.last_editor_uid),
                ))
                .execute(conn)?;

            if !cloud_object_metadata
                .pending_changes_statuses
                .has_pending_metadata_change
            {
                diesel::update(metadata_filter)
                    .set((
                        metadata_last_updated_ts.eq(metadata_last_updated_at),
                        trashed_ts.eq(trashed_timestamp),
                        folder_id.eq(folder_id_str),
                        current_editor.eq(cloud_object_metadata.current_editor_uid),
                    ))
                    .execute(conn)?;
            }

            // Update the permissions.
            if !cloud_object_metadata
                .pending_changes_statuses
                .has_pending_permissions_change
            {
                let permissions_filter =
                    object_permissions.filter(object_metadata_id.eq(metadata.id));
                diesel::update(permissions_filter)
                    .set((
                        subject_type.eq(subject_type_value),
                        subject_id.eq(subject_id_value),
                        subject_uid.eq(subject_uid_value),
                        permissions_last_updated_at.eq(permissions_ts),
                        object_guests.eq(guests),
                        anyone_with_link_access_level.eq(anyone_with_link_access_level_value),
                        anyone_with_link_source.eq(anyone_with_link_source_value),
                    ))
                    .execute(conn)?;
            }
        }
        None => {
            // The object doesn't exist in sqlite so create the object.
            let object_id = create_object_fn(conn)?;

            // Create the metadata.
            let mut new_object_metadata = NewObjectMetadata {
                object_type: cloud_object_type.sqlite_object_type_as_str().to_string(),
                revision_ts: revision,
                shareable_object_id: object_id,
                is_pending: has_pending_content_changes,
                retry_count: 0,

                // TODO: we need to deserialize this from graphql.
                author_id: None,

                // One of these is set below.
                client_id: None,
                server_id: None,

                metadata_last_updated_ts: cloud_object_metadata
                    .metadata_last_updated_ts
                    .map(|ts| ts.timestamp_micros()),

                trashed_ts: cloud_object_metadata
                    .trashed_ts
                    .map(|ts| ts.timestamp_micros()),

                folder_id: cloud_object_metadata
                    .folder_id
                    .map(|sync_id| sync_id.sqlite_uid_hash(ObjectIdType::Folder)),

                // When we insert an object, mark whether it's a welcome object. This
                // field won't ever be updated and this is the only pathway for it to be set.
                is_welcome_object: cloud_object_metadata.is_welcome_object,

                creator_uid: cloud_object_metadata.creator_uid,
                last_editor_uid: cloud_object_metadata.last_editor_uid,
                current_editor: cloud_object_metadata.current_editor_uid,
            };

            // There are two distinct cases:
            // - If the client created this object, the clientId will be set. There is another model event to set the server id.
            // - Otherwise, the server notified the client about this object so only the serverId will be set.
            match sync_id {
                SyncId::ClientId(_) => {
                    new_object_metadata.client_id = Some(hashed_sync_id);
                }
                SyncId::ServerId(_) => {
                    new_object_metadata.server_id = Some(hashed_sync_id);
                }
            }
            diesel::insert_into(schema::object_metadata::dsl::object_metadata)
                .values(new_object_metadata)
                .execute(conn)?;

            // Retrieve the ID of the row that was just inserted. We need to
            // do it this way because sqlite doesn't support RETURNING.
            let metadata_id: i32 = schema::object_metadata::dsl::object_metadata
                .select(schema::object_metadata::dsl::id)
                .order(schema::object_metadata::dsl::id.desc())
                .first(conn)?;

            // Create the permissions.
            let new_object_permissions = NewObjectPermissions {
                object_metadata_id: metadata_id,
                subject_type: subject_type_value.to_owned(),
                subject_id: subject_id_value,
                subject_uid: subject_uid_value,
                permissions_last_updated_at: permissions_ts,
                object_guests: guests,
                anyone_with_link_access_level: anyone_with_link_access_level_value,
                anyone_with_link_source: anyone_with_link_source_value,
            };
            diesel::insert_into(schema::object_permissions::dsl::object_permissions)
                .values(new_object_permissions)
                .execute(conn)?;
        }
    }

    Ok(())
}
