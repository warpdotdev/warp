use super::error::UserFacingError;
use super::object::ObjectMetadata;
use super::object_permissions::ObjectPermissions;
use super::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct Notebook {
    pub data: String,
    pub title: String,
    pub ai_document_id: Option<String>,
    pub metadata: ObjectMetadata,
    pub permissions: ObjectPermissions,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateNotebookEditAccessInput {
    pub uid: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateNotebookEditAccessOutput {
    pub accepted: bool,
    pub metadata: ObjectMetadata,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateNotebookEditAccessResult {
    UpdateNotebookEditAccessOutput(UpdateNotebookEditAccessOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
