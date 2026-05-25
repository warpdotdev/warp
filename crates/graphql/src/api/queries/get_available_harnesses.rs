use crate::ai::AgentHarness;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetAvailableHarnessesVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetAvailableHarnessesVariables"
)]
pub struct GetAvailableHarnesses {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_available_harnesses(GetAvailableHarnessesVariables) -> GetAvailableHarnesses;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub available_harnesses: AvailableHarnesses,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct AvailableHarnesses {
    pub harnesses: Vec<HarnessInfo>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct HarnessInfo {
    pub harness: AgentHarness,
    pub display_name: String,
    pub enabled: bool,
    pub available_models: Vec<HarnessModel>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct HarnessModel {
    pub id: cynic::Id,
    pub display_name: String,
    pub reasoning_level: Option<String>,
}
