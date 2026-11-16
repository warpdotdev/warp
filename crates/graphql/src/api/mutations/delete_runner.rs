use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct DeleteRunnerVariables {
    pub input: DeleteRunnerInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct DeleteRunnerInput {
    pub uid: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct DeleteRunnerOutput {
    pub success: bool,
    pub deleted_uid: cynic::Id,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum DeleteRunnerResult {
    DeleteRunnerOutput(DeleteRunnerOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "DeleteRunnerVariables")]
pub struct DeleteRunner {
    #[arguments(input: $input, requestContext: $request_context)]
    pub delete_runner: DeleteRunnerResult,
}

crate::client::define_operation! {
    delete_runner(DeleteRunnerVariables) -> DeleteRunner;
}
