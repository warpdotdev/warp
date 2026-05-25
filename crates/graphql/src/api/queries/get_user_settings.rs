use crate::request_context::RequestContext;
use crate::schema;

/*
query GetUserSettings($requestContext: RequestContext!) {
  user(requestContext: $requestContext) {
    ... on UserOutput {
      user {
        settings {
          isCloudConversationStorageEnabled
          isCrashReportingEnabled
          isTelemetryEnabled
        }
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct GetUserSettingsVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserOutput {
    pub user: User,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub settings: Option<UserSettings>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserResult {
    UserOutput(UserOutput),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "GetUserSettingsVariables")]
pub struct GetUserSettings {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_user_settings(GetUserSettingsVariables) -> GetUserSettings;
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UserSettings {
    pub is_cloud_conversation_storage_enabled: bool,
    pub is_crash_reporting_enabled: bool,
    pub is_telemetry_enabled: bool,
}
