use crate::error::UserFacingError;
use crate::object::CloudObjectEventEntrypoint;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct RemoveUserFromWorkspaceVariables {
    pub input: RemoveUserFromWorkspaceInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "RemoveUserFromWorkspaceVariables"
)]
pub struct RemoveUserFromWorkspace {
    #[arguments(input: $input, requestContext: $request_context)]
    pub remove_user_from_workspace: RemoveUserFromWorkspaceResult,
}
crate::client::define_operation! {
    remove_user_from_workspace(RemoveUserFromWorkspaceVariables) -> RemoveUserFromWorkspace;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RemoveUserFromWorkspaceOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RemoveUserFromWorkspaceResult {
    RemoveUserFromWorkspaceOutput(RemoveUserFromWorkspaceOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct RemoveUserFromWorkspaceInput {
    pub entrypoint: CloudObjectEventEntrypoint,
    pub user_uid: cynic::Id,
    pub workspace_uid: cynic::Id,
}
