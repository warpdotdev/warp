use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct DeleteObjectVariables {
    pub input: DeleteObjectInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "DeleteObjectVariables")]
pub struct DeleteObject {
    #[arguments(input: $input, requestContext: $request_context)]
    pub delete_object: DeleteObjectResult,
}
crate::client::define_operation! {
    delete_object(DeleteObjectVariables) -> DeleteObject;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct DeleteObjectOutput {
    pub deleted_uids: Vec<cynic::Id>,
    pub response_context: ResponseContext,
    pub success: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum DeleteObjectResult {
    DeleteObjectOutput(DeleteObjectOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct DeleteObjectInput {
    pub uid: cynic::Id,
}
