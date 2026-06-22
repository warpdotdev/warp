use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation DeleteInviteLinkDomainRestriction($input:DeleteInviteLinkDomainRestrictionInput!, $request_context:RequestContext!) {
  deleteInviteLinkDomainRestriction(
    input:$input,
    requestContext: $request_context
  ) {
    __typename
    ... on DeleteInviteLinkDomainRestrictionOutput {
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

#[derive(cynic::QueryVariables, Debug)]
pub struct DeleteInviteLinkDomainRestrictionVariables {
    pub input: DeleteInviteLinkDomainRestrictionInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "DeleteInviteLinkDomainRestrictionVariables"
)]
pub struct DeleteInviteLinkDomainRestriction {
    #[arguments(input: $input, requestContext: $request_context)]
    pub delete_invite_link_domain_restriction: DeleteInviteLinkDomainRestrictionResult,
}
crate::client::define_operation! {
    delete_invite_link_domain_restriction(DeleteInviteLinkDomainRestrictionVariables) -> DeleteInviteLinkDomainRestriction;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct DeleteInviteLinkDomainRestrictionOutput {
    pub success: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum DeleteInviteLinkDomainRestrictionResult {
    DeleteInviteLinkDomainRestrictionOutput(DeleteInviteLinkDomainRestrictionOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct DeleteInviteLinkDomainRestrictionInput {
    pub team_uid: cynic::Id,
    pub uid: cynic::Id,
}
