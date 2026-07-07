use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct LeaveObjectVariables {
    pub input: LeaveObjectInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "LeaveObjectVariables")]
pub struct LeaveObject {
    #[arguments(input: $input, requestContext: $request_context)]
    pub leave_object: LeaveObjectResult,
}
crate::client::define_operation! {
    leave_object(LeaveObjectVariables) -> LeaveObject;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct LeaveObjectOutput {
    pub object_uid: cynic::Id,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum LeaveObjectResult {
    LeaveObjectOutput(LeaveObjectOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct LeaveObjectInput {
    pub object_uid: cynic::Id,
}
