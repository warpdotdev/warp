use crate::error::UserFacingError;
use crate::object_actions::{ActionType, ObjectActionHistory};
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;

/*
mutation RecordObjectAction($input: RecordObjectActionInput!, $requestContext: RequestContext!) {
  recordObjectAction(input: $input, requestContext: $requestContext) {
    ... on RecordObjectActionOutput {
      history {
        actions {
          ... on BundledActions {
            actionType
            count
            latestProcessedAtTimestamp
            latestTimestamp
            oldestTimestamp
          }
          ... on SingleAction {
            actionType
            processedAtTimestamp
            timestamp
          }
        }
        latestProcessedAtTimestamp
        latestTimestamp
        objectType
        uid
      }
      responseContext {
        serverVersion
      }
    }
    ... on UserFacingError {
      error {
        message
      }
      responseContext {
        serverVersion
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct RecordObjectActionVariables {
    pub input: RecordObjectActionInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "RecordObjectActionVariables"
)]
pub struct RecordObjectAction {
    #[arguments(input: $input, requestContext: $request_context)]
    pub record_object_action: RecordObjectActionResult,
}
crate::client::define_operation! {
    record_object_action(RecordObjectActionVariables) -> RecordObjectAction;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RecordObjectActionOutput {
    pub history: ObjectActionHistory,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RecordObjectActionResult {
    RecordObjectActionOutput(RecordObjectActionOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct RecordObjectActionInput {
    pub action: ActionType,
    pub json_data: Option<String>,
    pub timestamp: Time,
    pub uid: cynic::Id,
}
