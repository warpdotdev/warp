use super::get_feature_model_choices::FeatureModelChoice;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetAgentDriverModelChoicesVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetAgentDriverModelChoicesVariables"
)]
pub struct GetAgentDriverModelChoices {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}

crate::client::define_operation! {
    get_agent_driver_model_choices(GetAgentDriverModelChoicesVariables) -> GetAgentDriverModelChoices;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(Box<UserOutput>),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub agent_driver_model_choices: FeatureModelChoice,
}
