use crate::error::UserFacingError;
use crate::folder::Folder;
use crate::object_permissions::Owner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateFolderVariables {
    pub input: CreateFolderInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "CreateFolderVariables")]
pub struct CreateFolder {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_folder: CreateFolderResult,
}
crate::client::define_operation! {
    create_folder(CreateFolderVariables) -> CreateFolder;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateFolderOutput {
    pub folder: Folder,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CreateFolderResult {
    CreateFolderOutput(CreateFolderOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct CreateFolderInput {
    pub initial_folder_id: Option<cynic::Id>,
    pub name: String,
    pub owner: Owner,
}
