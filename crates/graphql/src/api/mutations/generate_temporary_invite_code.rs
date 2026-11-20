use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables)]
pub struct GenerateTemporaryInviteCodeVariables {
    pub input: GenerateTemporaryInviteCodeInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject)]
pub struct GenerateTemporaryInviteCodeInput {
    pub workspace_uid: cynic::Id,
    pub team_uid: Option<cynic::Id>,
}

#[derive(cynic::QueryFragment)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "GenerateTemporaryInviteCodeVariables"
)]
pub struct GenerateTemporaryInviteCode {
    #[arguments(input: $input, requestContext: $request_context)]
    pub generate_temporary_invite_code: GenerateTemporaryInviteCodeResult,
}

crate::client::define_operation! {
    generate_temporary_invite_code(GenerateTemporaryInviteCodeVariables) -> GenerateTemporaryInviteCode;
}

#[derive(cynic::InlineFragments)]
pub enum GenerateTemporaryInviteCodeResult {
    GenerateTemporaryInviteCodeOutput(GenerateTemporaryInviteCodeOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment)]
pub struct GenerateTemporaryInviteCodeOutput {
    pub invite_uid: cynic::Id,
    pub invite_link: String,
    pub expires_at: Option<Time>,
}
