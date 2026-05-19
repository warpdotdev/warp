use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, ObjectType, ServerObjectModel},
    ids::FolderId,
};
#[cfg(not(target_family = "wasm"))]
pub mod persistence;

/// The model for a cloud folder.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudFolderModel {
    pub name: String,
    pub is_open: bool,
    pub is_warp_pack: bool,
}

impl CloudFolderModel {
    pub fn new(name: &str, is_warp_pack: bool) -> Self {
        Self {
            name: name.to_owned(),
            is_open: false,
            is_warp_pack,
        }
    }
}

impl ServerObjectModel for CloudFolderModel {
    fn object_type(&self) -> ObjectType {
        ObjectType::Folder
    }
}

pub type CloudFolder = GenericCloudObject<FolderId, CloudFolderModel>;
pub type ServerFolder = GenericServerObject<FolderId, CloudFolderModel>;
