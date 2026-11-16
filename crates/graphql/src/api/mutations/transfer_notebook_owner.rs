use crate::error::UserFacingError;
use crate::object::ObjectMetadata;
use crate::object_permissions::Owner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct TransferNotebookOwnerVariables {
    pub input: TransferNotebookOwnerInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TransferNotebookOwnerOutput {
    pub metadata: ObjectMetadata,
    pub response_context: ResponseContext,
    pub success: bool,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "TransferNotebookOwnerVariables"
)]
pub struct TransferNotebookOwner {
    #[arguments(requestContext: $request_context, input: $input)]
    pub transfer_notebook_owner: TransferNotebookOwnerResult,
}
crate::client::define_operation! {
    transfer_notebook_owner(TransferNotebookOwnerVariables) -> TransferNotebookOwner;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum TransferNotebookOwnerResult {
    TransferNotebookOwnerOutput(TransferNotebookOwnerOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct TransferNotebookOwnerInput {
    pub owner: Owner,
    pub uid: cynic::Id,
}
