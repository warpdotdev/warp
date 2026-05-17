use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, ObjectType, ServerObjectModel},
    ids::FolderId,
};

/// The model for a `CloudFolder`.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudFolderModel {
    pub name: String,
    // TODO: since this is local only state, we should consider only surfacing it as part of the
    // CloudViewModel. Right now, every server folder uses CloudFolderModel, which means it
    // hardcodes a value of `false` for this property since it can't know what the local state is.
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

/// `CloudFolder` is a folder retrieved from the server.
pub type CloudFolder = GenericCloudObject<FolderId, CloudFolderModel>;
pub type ServerFolder = GenericServerObject<FolderId, CloudFolderModel>;
