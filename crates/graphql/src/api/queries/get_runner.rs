use crate::error::UserFacingError;
use crate::queries::get_runners::Runner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::InputObject, Debug, Clone)]
pub struct RunnerSelector {
    pub uid: Option<cynic::Id>,
    pub name: Option<String>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct GetRunnerVariables {
    pub request_context: RequestContext,
    pub selector: RunnerSelector,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GetRunnerOutput {
    pub runner: Runner,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum GetRunnerResult {
    GetRunnerOutput(GetRunnerOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "GetRunnerVariables")]
pub struct GetRunner {
    #[arguments(requestContext: $request_context, selector: $selector)]
    pub get_runner: GetRunnerResult,
}

crate::client::define_operation! {
    get_runner(GetRunnerVariables) -> GetRunner;
}
