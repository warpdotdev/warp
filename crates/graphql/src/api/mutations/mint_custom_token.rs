use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct MintCustomTokenVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "MintCustomTokenVariables")]
pub struct MintCustomToken {
    #[arguments(requestContext: $request_context)]
    pub mint_custom_token: MintCustomTokenResult,
}
crate::client::define_operation! {
    mint_custom_token(MintCustomTokenVariables) -> MintCustomToken;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct MintCustomTokenOutput {
    pub custom_token: String,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum MintCustomTokenResult {
    MintCustomTokenOutput(MintCustomTokenOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
