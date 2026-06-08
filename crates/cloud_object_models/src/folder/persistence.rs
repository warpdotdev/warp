use cloud_object_persistence::{
    CloudObjectReadContext, id_from_metadata, to_cloud_object_metadata, upsert_cloud_object,
};
use cloud_objects::cloud_object::ObjectType;
use cloud_objects::ids::FolderId;
use diesel::result::Error;
use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection};
use persistence::model::{Folder, NewFolder};
use persistence::schema;

use super::{CloudFolder, CloudFolderModel};

pub fn upsert_folders(
    conn: &mut SqliteConnection,
    cloud_folders: Vec<CloudFolder>,
) -> Result<(), Error> {
    use schema::folders::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        for cloud_folder in cloud_folders {
            let folder_clone = cloud_folder.clone();
            let folder_name = cloud_folder.model().name.clone();
            let folder_is_open = cloud_folder.model().is_open;
            let folder_is_warp_pack = cloud_folder.model().is_warp_pack;
            upsert_cloud_object(
                conn,
                ObjectType::Folder,
                cloud_folder.id,
                cloud_folder.metadata,
                cloud_folder.permissions,
                Box::new(move |conn| {
                    let new_folder = NewFolder {
                        name: folder_name,
                        is_open: folder_is_open,
                        is_warp_pack: folder_is_warp_pack,
                    };
                    diesel::insert_into(schema::folders::dsl::folders)
                        .values(new_folder)
                        .execute(conn)?;
                    let folder_id: i32 = schema::folders::dsl::folders
                        .select(schema::folders::columns::id)
                        .order(schema::folders::columns::id.desc())
                        .first(conn)?;
                    Ok(folder_id)
                }),
                Box::new(move |conn, folder_id| {
                    diesel::update(folders.filter(schema::folders::dsl::id.eq(folder_id)))
                        .set((
                            name.eq(folder_clone.model().name.clone()),
                            is_open.eq(folder_clone.model().is_open),
                            is_warp_pack.eq(folder_clone.model().is_warp_pack),
                        ))
                        .execute(conn)?;
                    Ok(())
                }),
            )?
        }
        Ok(())
    })
}

pub fn read_folders(
    conn: &mut SqliteConnection,
    read_context: &CloudObjectReadContext,
) -> Result<Vec<CloudFolder>, Error> {
    Ok(schema::folders::dsl::folders
        .load::<Folder>(conn)?
        .into_iter()
        .filter_map(|folder| {
            let metadata = read_context.metadata_for_object(folder.id, ObjectType::Folder)?;
            let folder_id = id_from_metadata::<FolderId>(metadata)?;
            let cloud_object_permissions = read_context.permissions_for_metadata(metadata)?;
            Some(CloudFolder::new(
                folder_id,
                CloudFolderModel {
                    name: folder.name,
                    is_open: folder.is_open,
                    is_warp_pack: folder.is_warp_pack,
                },
                to_cloud_object_metadata(metadata),
                cloud_object_permissions,
            ))
        })
        .collect())
}

pub fn delete_folder(conn: &mut SqliteConnection, folder_id: i32) -> Result<(), Error> {
    use schema::folders::dsl::*;
    diesel::delete(folders.filter(id.eq(folder_id))).execute(conn)?;
    Ok(())
}
