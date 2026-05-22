use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct IssueTaskIdentityTokenVariables {
    pub input: IssueTaskIdentityTokenInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "IssueTaskIdentityTokenVariables"
)]
pub struct IssueTaskIdentityToken {
    #[arguments(input: $input, requestContext: $request_context)]
    pub issue_task_identity_token: IssueTaskIdentityTokenResult,
}

crate::client::define_operation! {
    issue_task_identity_token(IssueTaskIdentityTokenVariables) -> IssueTaskIdentityToken;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum IssueTaskIdentityTokenResult {
    IssueTaskIdentityTokenOutput(IssueTaskIdentityTokenOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct IssueTaskIdentityTokenOutput {
    pub token: String,
    pub expires_at: Time,
    pub issuer: String,
    pub response_context: ResponseContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct IssueTaskIdentityTokenInput {
    pub audience: String,
    pub requested_duration_seconds: i32,
    pub subject_template: Option<Vec<String>>,
}
