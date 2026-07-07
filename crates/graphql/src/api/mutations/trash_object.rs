use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct TrashObjectVariables {
    pub input: TrashObjectInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TrashObjectOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "TrashObjectVariables")]
pub struct TrashObject {
    #[arguments(input: $input, requestContext: $request_context)]
    pub trash_object: TrashObjectResult,
}
crate::client::define_operation! {
    trash_object(TrashObjectVariables) -> TrashObject;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum TrashObjectResult {
    TrashObjectOutput(TrashObjectOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct TrashObjectInput {
    pub uid: cynic::Id,
}
