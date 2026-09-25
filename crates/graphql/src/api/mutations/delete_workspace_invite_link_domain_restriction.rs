use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct DeleteWorkspaceInviteLinkDomainRestrictionInput {
    pub workspace_uid: cynic::Id,
    pub uid: cynic::Id,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct DeleteWorkspaceInviteLinkDomainRestrictionVariables {
    pub input: DeleteWorkspaceInviteLinkDomainRestrictionInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "DeleteWorkspaceInviteLinkDomainRestrictionVariables"
)]
pub struct DeleteWorkspaceInviteLinkDomainRestriction {
    #[arguments(requestContext: $request_context, input: $input)]
    pub delete_workspace_invite_link_domain_restriction:
        DeleteWorkspaceInviteLinkDomainRestrictionResult,
}
crate::client::define_operation! {
    delete_workspace_invite_link_domain_restriction(DeleteWorkspaceInviteLinkDomainRestrictionVariables) -> DeleteWorkspaceInviteLinkDomainRestriction;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct DeleteWorkspaceInviteLinkDomainRestrictionOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum DeleteWorkspaceInviteLinkDomainRestrictionResult {
    DeleteWorkspaceInviteLinkDomainRestrictionOutput(
        DeleteWorkspaceInviteLinkDomainRestrictionOutput,
    ),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
