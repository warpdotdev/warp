use crate::error::UserFacingError;
use crate::object_permissions::AccessLevel;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct SetObjectLinkPermissionsVariables {
    pub input: SetObjectLinkPermissionsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SetObjectLinkPermissionsOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "SetObjectLinkPermissionsVariables"
)]
pub struct SetObjectLinkPermissions {
    #[arguments(input: $input, requestContext: $request_context)]
    pub set_object_link_permissions: SetObjectLinkPermissionsResult,
}
crate::client::define_operation! {
    set_object_link_permissions(SetObjectLinkPermissionsVariables) -> SetObjectLinkPermissions;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SetObjectLinkPermissionsResult {
    SetObjectLinkPermissionsOutput(SetObjectLinkPermissionsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
#[derive(cynic::InputObject, Debug)]
pub struct SetObjectLinkPermissionsInput {
    pub access_level: AccessLevel,
    pub uid: cynic::Id,
}
