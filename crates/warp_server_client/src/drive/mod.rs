pub mod sharing;

use crate::{
    cloud_object::{GenericStringObjectFormat, ObjectIdType, ObjectType},
    ids::{HashedSqliteId, ObjectUid, ServerId, SyncId},
};

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct OpenWarpDriveObjectSettings {
    /// The folder that should be focused in the Warp Drive when the object is opened.
    pub focused_folder_id: Option<ServerId>,
    /// The email of the user to invite to the object, if the object is being opened via the request access flow.
    pub invitee_email: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OpenWarpDriveObjectArgs {
    pub object_type: ObjectType,
    pub server_id: ServerId,
    pub settings: OpenWarpDriveObjectSettings,
}

/// Enum to use to pass down type and id between actions to avoid multiplying actions whenever we
/// need to pass the object id, etc.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum CloudObjectTypeAndId {
    Notebook(SyncId),
    Workflow(SyncId),
    Folder(SyncId),
    GenericStringObject {
        object_type: GenericStringObjectFormat,
        id: SyncId,
    },
}

impl CloudObjectTypeAndId {
    pub fn from_id_and_type(id: SyncId, object_type: ObjectType) -> Self {
        match object_type {
            ObjectType::Notebook => Self::Notebook(id),
            ObjectType::Workflow => Self::Workflow(id),
            ObjectType::Folder => Self::Folder(id),
            ObjectType::GenericStringObject(format) => Self::GenericStringObject {
                object_type: format,
                id,
            },
        }
    }

    pub fn uid(self) -> ObjectUid {
        match self {
            Self::Notebook(id) => id.uid(),
            Self::Workflow(id) => id.uid(),
            Self::Folder(id) => id.uid(),
            Self::GenericStringObject { id, .. } => id.uid(),
        }
    }

    pub fn sync_id(self) -> SyncId {
        match self {
            Self::Notebook(id)
            | Self::Workflow(id)
            | Self::Folder(id)
            | Self::GenericStringObject { id, .. } => id,
        }
    }

    pub fn sqlite_uid_hash(self) -> HashedSqliteId {
        match self {
            CloudObjectTypeAndId::Notebook(id) => id.sqlite_uid_hash(ObjectIdType::Notebook),
            CloudObjectTypeAndId::Workflow(id) => id.sqlite_uid_hash(ObjectIdType::Workflow),
            CloudObjectTypeAndId::Folder(id) => id.sqlite_uid_hash(ObjectIdType::Folder),
            CloudObjectTypeAndId::GenericStringObject { object_type: _, id } => {
                id.sqlite_uid_hash(ObjectIdType::GenericStringObject)
            }
        }
    }

    pub fn object_id_type(&self) -> ObjectIdType {
        match self {
            CloudObjectTypeAndId::Notebook(_) => ObjectIdType::Notebook,
            CloudObjectTypeAndId::Workflow(_) => ObjectIdType::Workflow,
            CloudObjectTypeAndId::GenericStringObject { .. } => ObjectIdType::GenericStringObject,
            CloudObjectTypeAndId::Folder(_) => ObjectIdType::Folder,
        }
    }

    pub fn object_type(&self) -> ObjectType {
        match self {
            CloudObjectTypeAndId::Notebook(_) => ObjectType::Notebook,
            CloudObjectTypeAndId::Workflow(_) => ObjectType::Workflow,
            CloudObjectTypeAndId::Folder(_) => ObjectType::Folder,
            CloudObjectTypeAndId::GenericStringObject { object_type, .. } => {
                ObjectType::GenericStringObject(*object_type)
            }
        }
    }

    pub fn as_folder_id(self) -> Option<SyncId> {
        match self {
            CloudObjectTypeAndId::Notebook(_) => None,
            CloudObjectTypeAndId::Workflow(_) => None,
            CloudObjectTypeAndId::GenericStringObject { .. } => None,
            CloudObjectTypeAndId::Folder(f) => Some(f),
        }
    }

    pub fn as_notebook_id(self) -> Option<SyncId> {
        match self {
            CloudObjectTypeAndId::Notebook(id) => Some(id),
            _ => None,
        }
    }

    pub fn as_generic_string_object_id(self) -> Option<SyncId> {
        match self {
            CloudObjectTypeAndId::GenericStringObject { object_type: _, id } => Some(id),
            _ => None,
        }
    }

    pub fn has_server_id(self) -> bool {
        matches!(
            self,
            CloudObjectTypeAndId::Notebook(SyncId::ServerId(_))
                | CloudObjectTypeAndId::Workflow(SyncId::ServerId(_))
                | CloudObjectTypeAndId::Folder(SyncId::ServerId(_))
                | CloudObjectTypeAndId::GenericStringObject {
                    id: SyncId::ServerId(_),
                    ..
                }
        )
    }

    pub fn server_id(self) -> Option<ServerId> {
        match self {
            CloudObjectTypeAndId::Notebook(SyncId::ServerId(notebook_id)) => Some(notebook_id),
            CloudObjectTypeAndId::Workflow(SyncId::ServerId(workflow_id)) => Some(workflow_id),
            CloudObjectTypeAndId::Folder(SyncId::ServerId(folder_id)) => Some(folder_id),
            CloudObjectTypeAndId::GenericStringObject {
                id: SyncId::ServerId(json_object_id),
                ..
            } => Some(json_object_id),
            _ => None,
        }
    }

    pub fn drive_row_position_id(self) -> String {
        format!("WarpDriveRow_{}", self.uid())
    }

    pub fn from_generic_string_object(object_type: GenericStringObjectFormat, id: SyncId) -> Self {
        Self::GenericStringObject { object_type, id }
    }
}
