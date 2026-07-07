use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation SendReferralInviteEmails($input: SendReferralInviteEmailsInput!, $requestContext: RequestContext!) {
  sendReferralInviteEmails(input: $input, requestContext: $requestContext) {
    ... on SendReferralInviteEmailsOutput {
      responseContext {
        serverVersion
      }
      successfulEmails
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
pub struct SendReferralInviteEmailsVariables {
    pub input: SendReferralInviteEmailsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct SendReferralInviteEmailsInput {
    pub emails: Vec<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "SendReferralInviteEmailsVariables"
)]
pub struct SendReferralInviteEmails {
    #[arguments(input: $input, requestContext: $request_context)]
    pub send_referral_invite_emails: SendReferralInviteEmailsResult,
}
crate::client::define_operation! {
    send_referral_invite_emails(SendReferralInviteEmailsVariables) -> SendReferralInviteEmails;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SendReferralInviteEmailsOutput {
    pub successful_emails: Vec<String>,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SendReferralInviteEmailsResult {
    SendReferralInviteEmailsOutput(SendReferralInviteEmailsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
