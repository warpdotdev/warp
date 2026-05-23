use crate::billing::BillingMetadata;
use crate::request_context::RequestContext;
use crate::schema;

/*
query GetAiOveragesForWorkspace($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        workspaces {
          billingMetadata {
            aiOverages {
              currentMonthlyRequestCostCents
              currentMonthlyRequestsUsed
              currentPeriodEnd
            }
          }
        }
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct GetAiOveragesForWorkspaceVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub workspaces: Vec<Workspace>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Workspace")]
pub struct Workspace {
    pub billing_metadata: BillingMetadata,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetAiOveragesForWorkspaceVariables"
)]
pub struct GetAiOveragesForWorkspace {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_ai_overages_for_workspace(GetAiOveragesForWorkspaceVariables) -> GetAiOveragesForWorkspace;
}
