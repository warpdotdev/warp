use crate::object::{CloudObject, ObjectMetadata};
use crate::object_permissions::ObjectPermissions;
use crate::schema;

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct Folder {
    pub name: String,
    pub metadata: ObjectMetadata,
    pub permissions: ObjectPermissions,
    #[cynic(rename = "isWarpPack")]
    pub is_warp_pack: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct FolderWithDescendants {
    pub descendants: Vec<CloudObject>,
    pub folder: Folder,
}
