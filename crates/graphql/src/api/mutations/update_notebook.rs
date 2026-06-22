use crate::error::UserFacingError;
use crate::notebook::Notebook;
use crate::object::ObjectUpdateSuccess;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateNotebookVariables {
    pub input: UpdateNotebookInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateNotebookOutput {
    pub response_context: ResponseContext,
    pub update: NotebookUpdate,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "UpdateNotebookVariables")]
pub struct UpdateNotebook {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_notebook: UpdateNotebookResult,
}
crate::client::define_operation! {
    update_notebook(UpdateNotebookVariables) -> UpdateNotebook;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct NotebookUpdateRejected {
    pub conflicting_notebook: Notebook,
    pub revision_ts: Time,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum NotebookUpdate {
    NotebookUpdateRejected(NotebookUpdateRejected),
    ObjectUpdateSuccess(ObjectUpdateSuccess),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum UpdateNotebookResult {
    UpdateNotebookOutput(UpdateNotebookOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateNotebookInput {
    pub data: Option<String>,
    pub revision_ts: Option<Time>,
    pub title: Option<String>,
    pub uid: cynic::Id,
}
