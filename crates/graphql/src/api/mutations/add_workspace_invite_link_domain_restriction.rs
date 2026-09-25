use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct AddWorkspaceInviteLinkDomainRestrictionInput {
    pub domain: String,
    pub workspace_uid: cynic::Id,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct AddWorkspaceInviteLinkDomainRestrictionVariables {
    pub input: AddWorkspaceInviteLinkDomainRestrictionInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "AddWorkspaceInviteLinkDomainRestrictionVariables"
)]
pub struct AddWorkspaceInviteLinkDomainRestriction {
    #[arguments(requestContext: $request_context, input: $input)]
    pub add_workspace_invite_link_domain_restriction: AddWorkspaceInviteLinkDomainRestrictionResult,
}
crate::client::define_operation! {
    add_workspace_invite_link_domain_restriction(AddWorkspaceInviteLinkDomainRestrictionVariables) -> AddWorkspaceInviteLinkDomainRestriction;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct AddWorkspaceInviteLinkDomainRestrictionOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum AddWorkspaceInviteLinkDomainRestrictionResult {
    AddWorkspaceInviteLinkDomainRestrictionOutput(AddWorkspaceInviteLinkDomainRestrictionOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
