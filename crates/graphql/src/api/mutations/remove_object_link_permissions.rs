use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct RemoveObjectLinkPermissionsVariables {
    pub input: RemoveObjectLinkPermissionsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "RemoveObjectLinkPermissionsVariables"
)]
pub struct RemoveObjectLinkPermissions {
    #[arguments(input: $input, requestContext: $request_context)]
    pub remove_object_link_permissions: RemoveObjectLinkPermissionsResult,
}
crate::client::define_operation! {
    remove_object_link_permissions(RemoveObjectLinkPermissionsVariables) -> RemoveObjectLinkPermissions;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RemoveObjectLinkPermissionsOutput {
    pub response_context: ResponseContext,
    pub success: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RemoveObjectLinkPermissionsResult {
    RemoveObjectLinkPermissionsOutput(RemoveObjectLinkPermissionsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct RemoveObjectLinkPermissionsInput {
    pub uid: cynic::Id,
}
