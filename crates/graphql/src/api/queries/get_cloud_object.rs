use crate::error::UserFacingError;
use crate::object::CloudObjectWithDescendants;
use crate::object_actions::ObjectActionHistory;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetCloudObjectVariables {
    pub input: CloudObjectInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "GetCloudObjectVariables")]
pub struct GetCloudObject {
    #[arguments(input: $input, requestContext: $request_context)]
    pub cloud_object: CloudObjectResult,
}
crate::client::define_operation! {
    get_cloud_object(GetCloudObjectVariables) -> GetCloudObject;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CloudObjectOutput {
    pub object: CloudObjectWithDescendants,
    pub action_histories: Option<Vec<ObjectActionHistory>>,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CloudObjectResult {
    CloudObjectOutput(CloudObjectOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct CloudObjectInput {
    pub uid: cynic::Id,
}
