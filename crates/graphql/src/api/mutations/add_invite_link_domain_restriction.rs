use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation addInviteLinkDomainRestriction($requestContext: RequestContext!, $input: AddInviteLinkDomainRestrictionInput!) {
  addInviteLinkDomainRestriction(requestContext: $requestContext, input:$input) {
    ... on AddInviteLinkDomainRestrictionOutput {
      success
      responseContext {
        serverVersion
      }
    }
    ... on UserFacingError {
      error {
        message
      }
      responseContext {
        serverVersion
      }
    }
  }
}
*/

#[derive(cynic::InputObject, Debug)]
pub struct AddInviteLinkDomainRestrictionInput {
    pub domain: String,
    pub team_uid: cynic::Id,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct AddInviteLinkDomainRestrictionVariables {
    pub input: AddInviteLinkDomainRestrictionInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "AddInviteLinkDomainRestrictionVariables"
)]
pub struct AddInviteLinkDomainRestriction {
    #[arguments(requestContext: $request_context, input: $input)]
    pub add_invite_link_domain_restriction: AddInviteLinkDomainRestrictionResult,
}
crate::client::define_operation! {
    add_invite_link_domain_restriction(AddInviteLinkDomainRestrictionVariables) -> AddInviteLinkDomainRestriction;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct AddInviteLinkDomainRestrictionOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum AddInviteLinkDomainRestrictionResult {
    AddInviteLinkDomainRestrictionOutput(AddInviteLinkDomainRestrictionOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
