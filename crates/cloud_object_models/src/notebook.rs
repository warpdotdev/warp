use ai::document::AIDocumentId;
use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, ObjectType, ServerObjectModel},
    ids::{ServerId, SyncId},
};
use serde::{Deserialize, Serialize};

/// Serialized representation of a notebook for sync queue
/// The AIDocumentID and ConversationID are stored here to avoid polluting the
/// generic CreateObjectRequest type.
#[derive(Serialize, Deserialize)]
pub struct SerializedNotebook {
    pub data: String,
    pub ai_document_id: Option<String>,
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CloudNotebookModel {
    pub title: String,
    pub data: String,
    pub ai_document_id: Option<AIDocumentId>,
    /// This is the server-generated conversation token, not the client-side AIConversationId.
    pub conversation_id: Option<String>,
}

impl ServerObjectModel for CloudNotebookModel {
    fn object_type(&self) -> ObjectType {
        ObjectType::Notebook
    }
}

/// This is the notebook_id in the database associated with this notebook.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct NotebookId(ServerId);
cloud_objects::server_id_traits! { NotebookId, "Notebook" }

impl From<NotebookId> for SyncId {
    fn from(id: NotebookId) -> Self {
        Self::ServerId(id.into())
    }
}

/// `CloudNotebook` is a notebook retrieved from the server.
pub type CloudNotebook = GenericCloudObject<NotebookId, CloudNotebookModel>;
pub type ServerNotebook = GenericServerObject<NotebookId, CloudNotebookModel>;
