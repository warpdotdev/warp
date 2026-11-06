use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetIncludedUsagePoolsVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetIncludedUsagePoolsVariables"
)]
pub struct GetIncludedUsagePools {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}

crate::client::define_operation! {
    get_included_usage_pools(GetIncludedUsagePoolsVariables) -> GetIncludedUsagePools;
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
    pub included_usage_pool: Option<IncludedUsagePool>,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
pub struct IncludedUsagePool {
    pub monthly_credits: f64,
    pub used_credits: f64,
    pub monthly_cents: i32,
    pub granted_cents: f64,
    pub remaining_cents: f64,
    pub expires_at: Option<Time>,
}

#[cfg(test)]
#[path = "get_included_usage_pools_tests.rs"]
mod tests;
