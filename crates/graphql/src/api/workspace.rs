use super::billing::{BillingCycleUsageHistory, BillingMetadata, BonusGrantsInfo};
use crate::schema;

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct Workspace {
    pub uid: cynic::Id,
    pub name: String,
    pub stripe_customer_id: Option<cynic::Id>,
    pub members: Vec<WorkspaceMember>,
    pub teams: Vec<Team>,
    pub billing_metadata: BillingMetadata,
    pub bonus_grants_info: BonusGrantsInfo,
    pub billing_cycle_usage_history: Option<BillingCycleUsageHistory>,
    pub settings: WorkspaceSettings,
    pub has_billing_history: bool,
    pub invite_code: Option<String>,
    pub pending_email_invites: Vec<EmailInvite>,
    pub invite_link_domain_restrictions: Vec<InviteLinkDomainRestriction>,
    pub is_eligible_for_discovery: bool,
    pub feature_model_choice: FeatureModelChoice,
    pub total_requests_used_since_last_refresh: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct FeatureModelChoice {
    pub agent_mode: AvailableLlms,
    pub planning: AvailableLlms,
    pub coding: AvailableLlms,
    pub cli_agent: AvailableLlms,
    pub computer_use_agent: AvailableLlms,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AvailableLlms {
    pub default_id: String,
    pub choices: Vec<LlmInfo>,
    pub preferred_codex_model_id: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct RoutingHostConfig {
    pub enabled: bool,
    pub model_routing_host: LlmModelHost,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct LlmContextWindow {
    pub is_configurable: bool,
    pub min: crate::scalars::Uint32,
    pub max: crate::scalars::Uint32,
    pub default: crate::scalars::Uint32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
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

#[derive(cynic::QueryFragment, Debug, Clone)]
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

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct LlmSpec {
    pub cost: f64,
    pub quality: f64,
    pub speed: f64,
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

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct LlmUsageMetadata {
    pub credit_multiplier: Option<f64>,
    pub request_multiplier: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct WorkspaceMember {
    pub uid: cynic::Id,
    pub email: String,
    pub role: MembershipRole,
    pub usage_info: WorkspaceMemberUsageInfo,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct WorkspaceMemberUsageInfo {
    pub is_unlimited: bool,
    pub request_limit: i32,
    pub requests_used_since_last_refresh: i32,
    pub is_request_limit_prorated: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct CodebaseContextSettings {
    pub enabled: bool,
    pub setting: AdminEnablementSetting,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AmbientAgentSettings {
    pub enable_warp_attribution: AdminEnablementSetting,
    pub default_host_slug: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct WorkspaceSettings {
    pub is_discoverable: bool,
    pub is_invite_link_enabled: bool,
    pub llm_settings: LlmSettings,
    pub telemetry_settings: TelemetrySettings,
    pub ugc_collection_settings: UgcCollectionSettings,
    pub cloud_conversation_storage_settings: CloudConversationStorageSettings,
    pub ai_permissions_settings: AiPermissionsSettings,
    pub link_sharing_settings: LinkSharingSettings,
    pub secret_redaction_settings: SecretRedactionSettings,
    pub ai_autonomy_settings: AiAutonomySettings,
    pub usage_based_pricing_settings: UsageBasedPricingSettings,
    pub addon_credits_settings: AddonCreditsSettings,
    pub codebase_context_settings: CodebaseContextSettings,
    pub sandboxed_agent_settings: Option<SandboxedAgentSettings>,
    pub ambient_agent_settings: Option<AmbientAgentSettings>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct TelemetrySettings {
    pub force_enabled: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct UgcCollectionSettings {
    pub setting: UgcCollectionEnablementSetting,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct CloudConversationStorageSettings {
    pub setting: AdminEnablementSetting,
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum UgcCollectionEnablementSetting {
    Disable,
    Enable,
    RespectUserSetting,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AiPermissionsSettings {
    pub allow_ai_in_remote_sessions: bool,
    pub remote_session_regex_list: Vec<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct LinkSharingSettings {
    pub anyone_with_link_sharing_enabled: bool,
    pub direct_link_sharing_enabled: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct SecretRedactionRegex {
    pub name: Option<String>,
    pub pattern: String,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct SecretRedactionSettings {
    pub enabled: bool,
    pub regexes: Vec<SecretRedactionRegex>,
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum AdminEnablementSetting {
    Disable,
    Enable,
    RespectUserSetting,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum AiAutonomyValue {
    AgentDecides,
    AlwaysAllow,
    AlwaysAsk,
    RespectUserSetting,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum WriteToPtyAutonomyValue {
    AlwaysAllow,
    AlwaysAsk,
    AskOnFirstWrite,
    RespectUserSetting,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum ComputerUseAutonomyValue {
    Never,
    AlwaysAsk,
    AlwaysAllow,
    RespectUserSetting,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AiAutonomySettings {
    pub apply_code_diffs_setting: Option<AiAutonomyValue>,
    pub read_files_setting: Option<AiAutonomyValue>,
    pub read_files_allowlist: Option<Vec<String>>,
    pub create_plans_setting: Option<AiAutonomyValue>,
    pub execute_commands_setting: Option<AiAutonomyValue>,
    pub execute_commands_allowlist: Option<Vec<String>>,
    pub execute_commands_denylist: Option<Vec<String>>,
    pub write_to_pty_setting: Option<WriteToPtyAutonomyValue>,
    pub computer_use_setting: Option<ComputerUseAutonomyValue>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct SandboxedAgentSettings {
    pub execute_commands_denylist: Option<Vec<String>>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct Team {
    pub uid: cynic::Id,
    pub name: String,
    pub members: Vec<TeamMember>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct TeamMember {
    pub uid: cynic::Id,
    pub email: String,
    pub role: MembershipRole,
}

#[derive(cynic::Enum, Clone, Debug, PartialEq, Eq, Copy)]
pub enum MembershipRole {
    Admin,
    Owner,
    User,
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum LlmModelHost {
    AwsBedrock,
    CustomEndpoint,
    DirectApi,
    GeminiEnterprise,
    #[cynic(fallback)]
    Other(String),
}
#[derive(cynic::Enum, Clone, Debug)]
pub enum HostEnablementSetting {
    Enforce,
    RespectUserSetting,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct LlmHostSettings {
    pub enabled: bool,
    pub opt_out_of_new_models: bool,
    pub enablement_setting: Option<HostEnablementSetting>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct LlmHostSettingsEntry {
    pub host: LlmModelHost,
    pub settings: LlmHostSettings,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct LlmSettings {
    pub enabled: bool,
    pub host_configs: Vec<LlmHostSettingsEntry>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct InviteLinkDomainRestriction {
    pub uid: cynic::Id,
    pub domain: String,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct EmailInvite {
    pub email: String,
    pub expired: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct UsageBasedPricingSettings {
    pub enabled: bool,
    pub max_monthly_spend_cents: Option<i32>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AddonCreditsSettings {
    pub auto_reload_enabled: bool,
    pub max_monthly_spend_cents: Option<i32>,
    pub selected_auto_reload_credit_denomination: Option<i32>,
}
