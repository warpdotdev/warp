use std::collections::HashMap;

use cloud_objects::UserUid;
use cloud_objects::cloud_object::{
    CloudObjectMetadata, CloudObjectPermissions, CloudObjectStatuses, CloudObjectSyncStatus,
    GENERIC_STRING_OBJECT_PREFIX, GenericStringObjectFormat, NumInFlightRequests, ObjectIdType,
    ObjectType, Owner, Revision, RevisionAndLastEditor, ServerCreationInfo,
};
use cloud_objects::ids::{ClientId, FolderId, HashableId, SyncId, ToServerId};
use diesel::result::Error;
use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection};
use persistence::model::{
    GenericStringObject as PersistedGenericStringObject, NewGenericStringObject, NewObjectMetadata,
    NewObjectPermissions, ObjectMetadata, ObjectPermissions,
};
use persistence::schema;
use warp_core::features::FeatureFlag;
use warp_graphql::scalars::time::ServerTimestamp;

use crate::{decode_guests, decode_link_sharing, encode_guests, encode_link_sharing};

/// The SQLite id of a cloud object.
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

/// When delete a cloud object, this callback is used to delete the cloud
/// object. It takes the id of the cloud object to delete as a parameter.
/// The supplied conn has already started a transaction.
pub type DeleteCloudObjectFn =
    Box<dyn FnOnce(&mut SqliteConnection, CloudObjectId) -> Result<(), Error>>;

/// Generic string object data prepared for persistence.
pub struct GenericStringObjectPersistenceData {
    pub id: SyncId,
    pub format: GenericStringObjectFormat,
    pub metadata: CloudObjectMetadata,
    pub permissions: CloudObjectPermissions,
    pub data: String,
}

/// A generic string object row loaded from SQLite.
pub struct GenericStringObjectRow {
    pub id: CloudObjectId,
    pub data: String,
}

/// Cloud-object metadata and permissions loaded from SQLite for reconstructing typed objects.
pub struct CloudObjectReadContext {
    metadata_by_id: HashMap<(CloudObjectId, String), ObjectMetadata>,
    permissions_by_id: HashMap<CloudObjectId, ObjectPermissions>,
    current_user_id: Option<UserUid>,
}

impl CloudObjectReadContext {
    pub fn metadata_for_object(
        &self,
        shareable_object_id: CloudObjectId,
        object_type: ObjectType,
    ) -> Option<&ObjectMetadata> {
        self.metadata_by_id
            .get(&(shareable_object_id, metadata_object_type_key(object_type)))
    }

    pub fn permissions_for_metadata(
        &self,
        metadata: &ObjectMetadata,
    ) -> Option<CloudObjectPermissions> {
        let permissions = self.permissions_by_id.get(&metadata.id)?;
        to_cloud_object_permissions(permissions, self.current_user_id)
    }
}

pub fn load_cloud_object_read_context(
    conn: &mut SqliteConnection,
    current_user_id: Option<UserUid>,
) -> Result<CloudObjectReadContext, Error> {
    let object_metadata =
        schema::object_metadata::dsl::object_metadata.load::<ObjectMetadata>(conn)?;
    let object_permissions =
        schema::object_permissions::dsl::object_permissions.load::<ObjectPermissions>(conn)?;

    // Cache metadata and permissions by id so that we aren't doing an n^2 lookups for each object type.
    let metadata_by_id = object_metadata
        .into_iter()
        .map(|metadata| {
            (
                (metadata.shareable_object_id, metadata_key(&metadata)),
                metadata,
            )
        })
        .collect::<HashMap<_, _>>();
    // Shareable object ids aren't unique across object types, so the object type needs to be
    // part of the hashmap key.  For generic objects, they are all in the same table,
    // so it's safe to use the generic prefix as part of the key.
    let permissions_by_id = object_permissions
        .into_iter()
        .map(|permissions| (permissions.object_metadata_id, permissions))
        .collect::<HashMap<_, _>>();

    Ok(CloudObjectReadContext {
        metadata_by_id,
        permissions_by_id,
        current_user_id,
    })
}

pub fn metadata_object_type_key(object_type: ObjectType) -> String {
    match object_type {
        ObjectType::GenericStringObject(_) => GENERIC_STRING_OBJECT_PREFIX.to_owned(),
        ObjectType::Notebook | ObjectType::Workflow | ObjectType::Folder => {
            object_type.sqlite_object_type_as_str().to_string()
        }
    }
}

fn metadata_key(metadata: &ObjectMetadata) -> String {
    if metadata
        .object_type
        .starts_with(GENERIC_STRING_OBJECT_PREFIX)
    {
        GENERIC_STRING_OBJECT_PREFIX.to_owned()
    } else {
        metadata.object_type.to_owned()
    }
}

pub fn upsert_cloud_object(
    conn: &mut SqliteConnection,
    cloud_object_type: ObjectType,
    sync_id: SyncId,
    cloud_object_metadata: CloudObjectMetadata,
    cloud_object_permissions: CloudObjectPermissions,
    create_object_fn: CreateCloudObjectFn,
    update_object_fn: UpdateCloudObjectFn,
) -> Result<(), Error> {
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

    let hashed_sync_id = sync_id.sqlite_uid_hash(cloud_object_type.into());
    // Filter to find metadata row.
    // The diesel types for `filter`s are dependent on the columns being filtered
    // so while the `hashed_sync_id` will only match one of `client_id` and `server_id`,
    // we filter on both here for ergonomics.
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

            if !cloud_object_metadata
                .pending_changes_statuses
                .has_pending_permissions_change
            {
                // Update the permissions.
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

/// Helper function to delete a cloud object identified by `sync_id`. If a valid object metadata row
/// for the object is found, `delete_object_fn` is called to delete the actual object.
pub fn delete_cloud_object(
    conn: &mut SqliteConnection,
    sync_id: SyncId,
    object_id_type: ObjectIdType,
    delete_object_fn: DeleteCloudObjectFn,
) -> Result<(), Error> {
    use schema::object_metadata::dsl::*;

    let hashed_sync_id = sync_id.sqlite_uid_hash(object_id_type);
    // Filter to find metadata row.
    // The diesel types for `filter`s are dependent on the columns being filtered
    // so while the `hashed_sync_id` will only match one of `client_id` and `server_id`,
    // we filter on both here for ergonomics.
    let metadata_filter = object_metadata
        .filter(client_id.eq(Some(hashed_sync_id.as_str())))
        .or_filter(server_id.eq(Some(hashed_sync_id.as_str())));

    let metadata: ObjectMetadata = metadata_filter.first(conn)?;
    let object_id = metadata.shareable_object_id;
    diesel::delete(object_metadata.filter(id.eq(metadata.id))).execute(conn)?;
    diesel::delete(
        schema::object_permissions::dsl::object_permissions
            .filter(schema::object_permissions::object_metadata_id.eq(metadata.id)),
    )
    .execute(conn)?;
    diesel::delete(
        schema::object_actions::dsl::object_actions
            .filter(schema::object_actions::hashed_object_id.eq(hashed_sync_id)),
    )
    .execute(conn)?;
    delete_object_fn(conn, object_id)?;
    Ok(())
}

pub fn upsert_generic_string_objects(
    conn: &mut SqliteConnection,
    cloud_generic_string_objects: Vec<GenericStringObjectPersistenceData>,
) -> Result<(), Error> {
    use schema::generic_string_objects::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        for object in cloud_generic_string_objects {
            let create_data = object.data.clone();
            let update_data = object.data;
            upsert_cloud_object(
                conn,
                ObjectType::GenericStringObject(object.format),
                object.id,
                object.metadata,
                object.permissions,
                Box::new(move |conn| {
                    let new_object = NewGenericStringObject { data: &create_data };
                    diesel::insert_into(
                        schema::generic_string_objects::dsl::generic_string_objects,
                    )
                    .values(new_object)
                    .execute(conn)?;
                    let object_id: i32 =
                        schema::generic_string_objects::dsl::generic_string_objects
                            .select(schema::generic_string_objects::columns::id)
                            .order(schema::generic_string_objects::columns::id.desc())
                            .first(conn)?;
                    Ok(object_id)
                }),
                Box::new(move |conn, object_id| {
                    diesel::update(
                        generic_string_objects
                            .filter(schema::generic_string_objects::dsl::id.eq(object_id)),
                    )
                    .set((data.eq(update_data),))
                    .execute(conn)?;
                    Ok(())
                }),
            )?
        }
        Ok(())
    })
}

pub fn read_generic_string_object_rows(
    conn: &mut SqliteConnection,
) -> Result<Vec<GenericStringObjectRow>, Error> {
    Ok(schema::generic_string_objects::dsl::generic_string_objects
        .load::<PersistedGenericStringObject>(conn)?
        .into_iter()
        .map(|object| GenericStringObjectRow {
            id: object.id,
            data: object.data,
        })
        .collect())
}

pub fn delete_generic_string_object(
    conn: &mut SqliteConnection,
    generic_string_object_id: CloudObjectId,
) -> Result<(), Error> {
    use schema::generic_string_objects::dsl::*;
    diesel::delete(generic_string_objects.filter(id.eq(generic_string_object_id))).execute(conn)?;
    Ok(())
}

/// Mark a shareable object as no longer having pending changes.
pub fn mark_object_as_synced(
    conn: &mut SqliteConnection,
    hashed_sqlite_id: String,
    new_revision_and_editor: RevisionAndLastEditor,
    new_metadata_ts: Option<ServerTimestamp>,
) -> Result<(), Error> {
    use schema::object_metadata::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(object_metadata.filter(server_id.eq(Some(hashed_sqlite_id.as_str()))))
            .set(is_pending.eq(false))
            .execute(conn)?;
        diesel::update(object_metadata.filter(server_id.eq(Some(hashed_sqlite_id.clone()))))
            .set((
                revision_ts.eq(new_revision_and_editor.revision.timestamp_micros()),
                last_editor_uid.eq(new_revision_and_editor.last_editor_uid),
            ))
            .execute(conn)?;

        if let Some(metadata_ts) = new_metadata_ts {
            diesel::update(object_metadata.filter(server_id.eq(Some(hashed_sqlite_id))))
                .set((metadata_last_updated_ts.eq(metadata_ts.timestamp_micros()),))
                .execute(conn)?;
        }
        Ok(())
    })
}

pub fn increment_retry_count(
    conn: &mut SqliteConnection,
    server_id_string: String,
) -> Result<(), Error> {
    use schema::object_metadata::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(object_metadata.filter(server_id.eq(Some(server_id_string))))
            .set(retry_count.eq(retry_count + 1))
            .execute(conn)?;
        Ok(())
    })
}

pub fn update_object_after_server_creation(
    conn: &mut SqliteConnection,
    client_id_string: String,
    server_creation_info: ServerCreationInfo,
) -> Result<(), Error> {
    use schema::commands::dsl::*;
    use schema::object_metadata::dsl::*;

    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(object_metadata.filter(client_id.eq(Some(client_id_string.clone()))))
            .set((
                server_id.eq(Some(
                    server_creation_info
                        .server_id_and_type
                        .sqlite_type_and_uid_hash(),
                )),
                creator_uid.eq(server_creation_info.creator_uid),
            ))
            .execute(conn)?;

        diesel::update(commands.filter(cloud_workflow_id.eq(Some(client_id_string))))
            .set(
                cloud_workflow_id.eq(Some(
                    server_creation_info
                        .server_id_and_type
                        .sqlite_type_and_uid_hash(),
                )),
            )
            .execute(conn)?;

        Ok(())
    })
}

/// SQLite endpoint for the ObjectMetadataUpdated RTC message that updates the metadata ts and other
/// metadata like current team_id of the object.
pub fn update_object_metadata(
    conn: &mut SqliteConnection,
    hashed_id: String,
    metadata: CloudObjectMetadata,
) -> Result<(), Error> {
    use schema::object_metadata::dsl::*;
    let metadata_last_updated_at = metadata
        .metadata_last_updated_ts
        .map(|ts| ts.timestamp_micros());

    let trashed_timestamp = metadata.trashed_ts.map(|ts| ts.timestamp_micros());
    let folder_id_str = metadata
        .folder_id
        .map(|folder_sync_id| folder_sync_id.sqlite_uid_hash(ObjectIdType::Folder));

    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(object_metadata.filter(server_id.eq(Some(hashed_id.as_str()))))
            .set((
                metadata_last_updated_ts.eq(metadata_last_updated_at),
                trashed_ts.eq(trashed_timestamp),
                folder_id.eq(folder_id_str),
                current_editor.eq(metadata.current_editor_uid),
            ))
            .execute(conn)?;

        Ok(())
    })
}

pub fn id_from_metadata<K: HashableId + ToServerId>(metadata: &ObjectMetadata) -> Option<SyncId> {
    match (&metadata.server_id, &metadata.client_id) {
        (Some(server_id), _) => {
            K::from_hash(server_id).map(|id| SyncId::ServerId(id.to_server_id()))
        }
        (None, Some(client_id)) => ClientId::from_hash(client_id).map(SyncId::ClientId),
        _ => None,
    }
}

pub fn to_cloud_object_metadata(metadata: &ObjectMetadata) -> CloudObjectMetadata {
    CloudObjectMetadata {
        current_editor_uid: metadata.current_editor.clone(),
        metadata_last_updated_ts: metadata
            .metadata_last_updated_ts
            .and_then(|epoch| ServerTimestamp::from_unix_timestamp_micros(epoch).ok()),
        revision: metadata
            .revision_ts
            .and_then(|epoch| Revision::from_unix_timestamp_micros(epoch).ok()),
        pending_changes_statuses: CloudObjectStatuses {
            pending_delete: false,
            content_sync_status: if metadata.is_pending {
                CloudObjectSyncStatus::InFlight(NumInFlightRequests(1))
            } else {
                CloudObjectSyncStatus::NoLocalChanges
            },
            has_pending_metadata_change: false,
            has_pending_permissions_change: false,
            pending_untrash: false,
        },
        trashed_ts: metadata
            .trashed_ts
            .and_then(|epoch| ServerTimestamp::from_unix_timestamp_micros(epoch).ok()),
        folder_id: metadata.folder_id.as_ref().and_then(|folder_id_str| {
            // First, attempt to convert the string into a server id.
            let as_server_id =
                FolderId::from_hash(folder_id_str).map(|id| SyncId::ServerId(id.into()));
            if as_server_id.is_none() {
                // If the string cannot be converted to server id, it may be a client id.
                ClientId::from_hash(folder_id_str).map(SyncId::ClientId)
            } else {
                as_server_id
            }
        }),
        is_welcome_object: metadata.is_welcome_object,
        creator_uid: metadata.creator_uid.clone(),
        last_editor_uid: metadata.last_editor_uid.clone(),
        last_task_run_ts: None,
    }
}

pub fn to_cloud_object_permissions(
    permissions: &ObjectPermissions,
    default_user_id: Option<UserUid>,
) -> Option<CloudObjectPermissions> {
    let owner = owner_for_permissions(permissions, default_user_id)?;
    let permissions_last_updated_ts = permissions
        .permissions_last_updated_at
        .and_then(|ts| ServerTimestamp::from_unix_timestamp_micros(ts).ok());

    // If deserializing guests fails, default to None and wait for an eventual refresh.
    let guests = if FeatureFlag::SharedWithMe.is_enabled() {
        permissions
            .object_guests
            .as_deref()
            .and_then(|guests| decode_guests(guests).ok())
            .unwrap_or_default()
    } else {
        Default::default()
    };

    // If deserializing link sharing fails, default to None and wait for an
    // eventual refresh.
    let anyone_with_link = if FeatureFlag::SharedWithMe.is_enabled() {
        permissions
            .anyone_with_link_access_level
            .as_deref()
            .and_then(|access_level| {
                decode_link_sharing(access_level, permissions.anyone_with_link_source.as_deref())
                    .ok()
            })
    } else {
        None
    };

    Some(CloudObjectPermissions {
        owner,
        permissions_last_updated_ts,
        guests,
        anyone_with_link,
    })
}

fn owner_for_permissions(
    permissions: &ObjectPermissions,
    default_user_id: Option<UserUid>,
) -> Option<Owner> {
    match permissions.subject_type.as_str() {
        "USER" => {
            let user_uid = permissions
                .subject_id
                .as_deref()
                .map(UserUid::new)
                .or(default_user_id)?;
            Some(Owner::User { user_uid })
        }
        "TEAM" => Some(Owner::Team {
            team_uid: cloud_objects::ids::ServerId::from_string_lossy(&permissions.subject_uid),
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "objects_tests.rs"]
mod tests;
