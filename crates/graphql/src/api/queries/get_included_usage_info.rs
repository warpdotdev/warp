use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetIncludedUsageInfoVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetIncludedUsageInfoVariables"
)]
pub struct GetIncludedUsageInfo {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}

crate::client::define_operation! {
    get_included_usage_info(GetIncludedUsageInfoVariables) -> GetIncludedUsageInfo;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub workspaces: Vec<Workspace>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Workspace {
    pub uid: cynic::Id,
    pub included_usage_info: Option<WorkspaceIncludedUsageInfo>,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
pub struct WorkspaceIncludedUsageInfo {
    pub included_usage_credits: f64,
    pub usage_credits_used_since_last_refresh: f64,
    pub included_usage_cents: f64,
    pub usage_cents_used_since_last_refresh: f64,
    pub next_refresh_time: Time,
}

#[cfg(test)]
#[path = "get_included_usage_info_tests.rs"]
mod tests;
