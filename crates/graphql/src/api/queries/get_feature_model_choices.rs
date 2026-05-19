use crate::{request_context::RequestContext, schema};

#[derive(cynic::QueryVariables, Debug)]
pub struct GetFeatureModelChoicesVariables {
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "GetFeatureModelChoicesVariables"
)]
pub struct GetFeatureModelChoices {
    #[arguments(requestContext: $request_context)]
    pub user: UserResult,
}
crate::client::define_operation! {
    get_feature_model_choices(GetFeatureModelChoicesVariables) -> GetFeatureModelChoices;
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
    pub workspaces: Vec<Workspace>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Workspace {
    pub feature_model_choice: FeatureModelChoice,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct FeatureModelChoice {
    pub agent_mode: AvailableLlms,
    pub planning: AvailableLlms,
    pub coding: AvailableLlms,
    pub cli_agent: AvailableLlms,
    pub computer_use_agent: AvailableLlms,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct AvailableLlms {
    pub default_id: String,
    pub choices: Vec<LlmInfo>,
    pub preferred_codex_model_id: Option<String>,
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum DisableReason {
    AdminDisabled,
    OutOfRequests,
    ProviderOutage,
    RequiresUpgrade,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum LlmModelHost {
    AwsBedrock,
    CustomEndpoint,
    DirectApi,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RoutingHostConfig {
    pub enabled: bool,
    pub model_routing_host: LlmModelHost,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct LlmContextWindow {
    pub is_configurable: bool,
    pub min: crate::scalars::Uint32,
    pub max: crate::scalars::Uint32,
    pub default: crate::scalars::Uint32,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct LlmInfo {
    pub display_name: String,
    pub base_model_name: String,
    pub id: String,
    pub reasoning_level: Option<String>,
    pub usage_metadata: LlmUsageMetadata,
    pub description: Option<String>,
    pub disable_reason: Option<DisableReason>,
    pub vision_supported: bool,
    pub spec: Option<LlmSpec>,
    pub provider: LlmProvider,
    pub host_configs: Vec<RoutingHostConfig>,
    pub pricing: LlmPricing,
    pub context_window: LlmContextWindow,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct LlmPricing {
    pub discount_percentage: Option<f64>,
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum LlmProvider {
    Openai,
    Anthropic,
    Google,
    Xai,
    Unknown,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::QueryFragment, Debug)]
pub struct LlmSpec {
    pub cost: f64,
    pub quality: f64,
    pub speed: f64,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct LlmUsageMetadata {
    pub credit_multiplier: Option<f64>,
    pub request_multiplier: i32,
}
