use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct SuggestCloudEnvironmentImageVariables {
    pub input: SuggestCloudEnvironmentImageInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "SuggestCloudEnvironmentImageVariables"
)]
pub struct SuggestCloudEnvironmentImage {
    #[arguments(input: $input, requestContext: $request_context)]
    #[cynic(rename = "suggestCloudEnvironmentImage")]
    pub suggest_cloud_environment_image: SuggestCloudEnvironmentImageResult,
}

crate::client::define_operation! {
    suggest_cloud_environment_image(SuggestCloudEnvironmentImageVariables) -> SuggestCloudEnvironmentImage;
}

#[derive(cynic::InputObject, Debug)]
pub struct SuggestCloudEnvironmentImageInput {
    pub repos: Vec<RepoInput>,
}

#[derive(cynic::InputObject, Debug)]
pub struct RepoInput {
    pub owner: String,
    pub repo: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SuggestCloudEnvironmentImageAuthRequiredOutput {
    pub auth_url: String,
    pub response_context: ResponseContext,
    #[cynic(rename = "txId")]
    pub tx_id: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SuggestCloudEnvironmentImageOutput {
    pub detected_languages: Vec<GithubReposLanguageStat>,
    pub image: String,
    pub needs_custom_image: bool,
    pub reason: String,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GithubReposLanguageStat {
    pub bytes: i32,
    pub language: String,
    pub percentage: f64,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum SuggestCloudEnvironmentImageResult {
    SuggestCloudEnvironmentImageAuthRequiredOutput(SuggestCloudEnvironmentImageAuthRequiredOutput),
    SuggestCloudEnvironmentImageOutput(SuggestCloudEnvironmentImageOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
