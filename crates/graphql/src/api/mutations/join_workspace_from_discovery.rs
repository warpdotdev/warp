use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct JoinWorkspaceFromDiscoveryVariables {
    pub input: JoinWorkspaceFromDiscoveryInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "JoinWorkspaceFromDiscoveryVariables"
)]
pub struct JoinWorkspaceFromDiscovery {
    #[arguments(input: $input, requestContext: $request_context)]
    pub join_workspace_from_discovery: JoinWorkspaceFromDiscoveryResult,
}
crate::client::define_operation! {
    join_workspace_from_discovery(JoinWorkspaceFromDiscoveryVariables) -> JoinWorkspaceFromDiscovery;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct JoinWorkspaceFromDiscoveryOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum JoinWorkspaceFromDiscoveryResult {
    JoinWorkspaceFromDiscoveryOutput(JoinWorkspaceFromDiscoveryOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct JoinWorkspaceFromDiscoveryInput {
    pub workspace_uid: cynic::Id,
}
