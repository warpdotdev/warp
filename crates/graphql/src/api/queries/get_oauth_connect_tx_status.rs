use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct GetOAuthConnectTxStatusInput {
    #[cynic(rename = "txId")]
    pub tx_id: cynic::Id,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct GetOAuthConnectTxStatusVariables {
    pub request_context: RequestContext,
    pub input: GetOAuthConnectTxStatusInput,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetOAuthConnectTxStatusVariables"
)]
pub struct GetOAuthConnectTxStatus {
    #[arguments(input: $input, requestContext: $request_context)]
    #[cynic(rename = "getOAuthConnectTxStatus")]
    pub get_oauth_connect_tx_status: GetOAuthConnectTxStatusResult,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GetOAuthConnectTxStatusOutput {
    pub __typename: String,
    pub status: OauthConnectTxStatus,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GetOAuthConnectTxStatusResult {
    GetOAuthConnectTxStatusOutput(GetOAuthConnectTxStatusOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
#[cynic(graphql_type = "OAuthConnectTxStatus")]
pub enum OauthConnectTxStatus {
    Completed,
    Expired,
    Failed,
    InProgress,
    Pending,
}

crate::client::define_operation! {
    get_oauth_connect_tx_status(GetOAuthConnectTxStatusVariables) -> GetOAuthConnectTxStatus;
}
