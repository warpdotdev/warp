use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

/*
mutation updateOnboardingSurveyStatus($input: UpdateOnboardingSurveyStatusInput!, $clientContext: ClientContext!, $osContext: OSContext!) {
  updateOnboardingSurveyStatus(
    input: $input
    requestContext: {clientContext: $clientContext, osContext: $osContext}
  ) {
    ... on UpdateOnboardingSurveyStatusOutput {
      status

    }
    ... on UserFacingError {
      error {
        message
      }
    }
  }
}

*/
#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateOnboardingSurveyStatusVariables {
    pub input: UpdateOnboardingSurveyStatusInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateOnboardingSurveyStatusOutput {
    pub status: OnboardingSurveyStatus,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "UpdateOnboardingSurveyStatusVariables"
)]
pub struct UpdateOnboardingSurveyStatus {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_onboarding_survey_status: UpdateOnboardingSurveyStatusResult,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateOnboardingSurveyStatusResult {
    UpdateOnboardingSurveyStatusOutput(UpdateOnboardingSurveyStatusOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum AcquisitionChannelSurveyResponse {
    Friend,
    Internet,
    InTheWild,
    Teammate,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum OnboardingSurveyStatus {
    Completed,
    Shown,
    Skipped,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum RoleSurveyResponse {
    BackendEngineer,
    BusinessAnalyst,
    Data,
    DevopsSre,
    EngineeringManager,
    FrontendEngineer,
    FullstackEngineer,
    Marketer,
    MobileEngineer,
    Other,
    ProductDesigner,
    ProductManager,
    SalesBusinessDev,
    Student,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum UsagePlanSurveyResponse {
    AiCodeProduction,
    AiPersonalProjects,
    ExploringTool,
    Other,
    ReplaceTerminal,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateOnboardingSurveyStatusInput {
    pub responses: Option<SurveyResponsesInput>,
    pub status: OnboardingSurveyStatus,
}

#[derive(cynic::InputObject, Debug)]
pub struct SurveyResponsesInput {
    #[cynic(rename = "ACQUISITION_CHANNEL")]
    pub acquisition_channel: Option<AcquisitionChannelQuestionResponseInput>,
    #[cynic(rename = "ROLE")]
    pub role: Option<RoleQuestionResponseInput>,
    #[cynic(rename = "USAGE_PLAN")]
    pub usage_plan: Option<UsagePlanQuestionResponseInput>,
}

#[derive(cynic::InputObject, Debug)]
pub struct AcquisitionChannelQuestionResponseInput {
    pub answer: AcquisitionChannelSurveyResponse,
    pub details: Option<String>,
}

#[derive(cynic::InputObject, Debug)]
pub struct RoleQuestionResponseInput {
    pub answer: RoleSurveyResponse,
}

#[derive(cynic::InputObject, Debug)]
pub struct UsagePlanQuestionResponseInput {
    pub answer: UsagePlanSurveyResponse,
    pub details: Option<String>,
}
