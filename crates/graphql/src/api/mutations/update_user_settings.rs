use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation UpdateUserSettings($input: UpdateUserSettingsInput!, $requestContext: RequestContext!) {
  updateUserSettings(input: $input, requestContext: $requestContext) {
    ... on UpdateUserSettingsOutput {
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "UpdateUserSettingsVariables"
)]
pub struct UpdateUserSettings {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_user_settings: UpdateUserSettingsResult,
}

crate::client::define_operation! {
    update_user_settings(UpdateUserSettingsVariables) -> UpdateUserSettings;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateUserSettingsVariables {
    pub input: UpdateUserSettingsInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug, Default)]
pub struct UpdateUserSettingsInput {
    pub cloud_conversation_storage_enabled: Option<bool>,
    pub crash_reporting_enabled: Option<bool>,
    pub telemetry_enabled: Option<bool>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateUserSettingsOutput {
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateUserSettingsResult {
    UpdateUserSettingsOutput(UpdateUserSettingsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
