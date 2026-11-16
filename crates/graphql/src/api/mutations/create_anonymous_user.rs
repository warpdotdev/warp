use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateAnonymousUserVariables {
    pub input: CreateAnonymousUserInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct CreateAnonymousUserInput {
    pub anonymous_user_type: AnonymousUserType,
    pub expiration_type: AnonymousUserExpirationType,
    pub referral_code: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "CreateAnonymousUserVariables"
)]
pub struct CreateAnonymousUser {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_anonymous_user: CreateAnonymousUserResult,
}
crate::client::define_operation! {
    create_anonymous_user(CreateAnonymousUserVariables) -> CreateAnonymousUser;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateAnonymousUserOutput {
    pub expires_at: Option<Time>,
    pub anonymous_user_type: AnonymousUserType,
    pub firebase_uid: String,
    pub id_token: String,
    pub is_invite_valid: bool,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum CreateAnonymousUserResult {
    CreateAnonymousUserOutput(CreateAnonymousUserOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum AnonymousUserExpirationType {
    #[cynic(rename = "EXPIRATION_14_DAYS")]
    Expiration14Days,
    #[cynic(rename = "NO_EXPIRATION")]
    NoExpiration,
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum AnonymousUserType {
    NativeClientAnonymousUser,
    NativeClientAnonymousUserFeatureGated,
    WebClientAnonymousUser,
    #[cynic(fallback)]
    Other(String),
}
