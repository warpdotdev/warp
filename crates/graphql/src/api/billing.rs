use crate::{scalars::Time, schema, workspace::UgcCollectionEnablementSetting};

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct BillingMetadata {
    pub customer_type: CustomerType,
    pub delinquency_status: DelinquencyStatus,
    pub tier: Tier,
    pub service_agreements: Vec<ServiceAgreement>,
    pub ai_overages: Option<AiOverages>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AiOverages {
    pub current_monthly_request_cost_cents: i32,
    pub current_monthly_requests_used: i32,
    pub current_period_end: Time,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct BonusGrantsInfo {
    pub grants: Vec<BonusGrant>,
    pub spending_info: Option<BonusGrantSpendingInfo>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct BonusGrantSpendingInfo {
    pub current_month_credits_purchased: i32,
    pub current_month_period_end: Time,
    pub current_month_spend_cents: i32,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BonusGrantType {
    AmbientOnly,
    Any,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct BonusGrant {
    pub created_at: Time,
    pub cost_cents: i32,
    pub expiration: Option<Time>,
    pub grant_type: BonusGrantType,
    pub reason: String,
    pub user_facing_message: Option<String>,
    pub request_credits_granted: i32,
    pub request_credits_remaining: i32,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum AddonCreditAutoReloadStatus {
    Failed,
    Succeeded,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ServiceAgreement {
    pub addon_credit_auto_reload_status: Option<AddonCreditAutoReloadStatus>,
    pub current_period_end: Time,
    pub status: ServiceAgreementStatus,
    pub stripe_subscription_id: Option<String>,
    #[cynic(rename = "type")]
    pub type_: ServiceAgreementType,
    pub sunsetted_to_build_ts: Option<Time>,
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum ServiceAgreementStatus {
    Active,
    Canceled,
    PastDue,
    Unpaid,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Debug, PartialEq)]
pub enum ServiceAgreementType {
    Enterprise,
    Legacy,
    ProTrial,
    Prosumer,
    SelfServe,
    TeamTrial,
    Turbo,
    Business,
    Lightspeed,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct Tier {
    pub name: String,
    pub description: String,
    pub warp_ai_policy: Option<WarpAiPolicy>,
    pub team_size_policy: Option<TeamSizePolicy>,
    pub shared_notebooks_policy: Option<SharedNotebooksPolicy>,
    pub shared_workflows_policy: Option<SharedWorkflowsPolicy>,
    pub session_sharing_policy: Option<SessionSharingPolicy>,
    pub ai_autonomy_policy: Option<AiAutonomyPolicy>,
    pub telemetry_data_collection_policy: Option<TelemetryDataCollectionPolicy>,
    pub ugc_data_collection_policy: Option<UgcDataCollectionPolicy>,
    pub usage_based_pricing_policy: Option<UsageBasedPricingPolicy>,
    pub codebase_context_policy: Option<CodebaseContextPolicy>,
    pub byo_api_key_policy: Option<ByoApiKeyPolicy>,
    pub purchase_add_on_credits_policy: Option<PurchaseAddOnCreditsPolicy>,
    pub enterprise_pay_as_you_go_policy: Option<EnterprisePayAsYouGoPolicy>,
    pub enterprise_credits_auto_reload_policy: Option<EnterpriseCreditsAutoReloadPolicy>,
    pub multi_admin_policy: Option<MultiAdminPolicy>,
    pub ambient_agents_policy: Option<AmbientAgentsPolicy>,
    pub usage_visibility_policy: Option<UsageVisibilityPolicy>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct SessionSharingPolicy {
    pub enabled: bool,
    pub max_session_bytes_size: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AiAutonomyPolicy {
    pub enabled: bool,
    pub toggleable: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct SharedWorkflowsPolicy {
    pub is_unlimited: bool,
    pub limit: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct SharedNotebooksPolicy {
    pub is_unlimited: bool,
    pub limit: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct TeamSizePolicy {
    pub is_unlimited: bool,
    pub limit: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct WarpAiPolicy {
    pub limit: i32,
    pub is_code_suggestions_toggleable: bool,
    pub is_prompt_suggestions_toggleable: bool,
    pub is_next_command_enabled: bool,
    pub is_git_operations_ai_enabled: bool,
    pub is_voice_enabled: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct TelemetryDataCollectionPolicy {
    pub default: bool,
    pub toggleable: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct UgcDataCollectionPolicy {
    pub default_setting: UgcCollectionEnablementSetting,
    pub toggleable: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct UsageBasedPricingPolicy {
    pub toggleable: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct CodebaseContextPolicy {
    pub toggleable: bool,
    pub is_unlimited_indices: bool,
    pub max_indices: i32,
    pub max_files_per_repo: i32,
    pub embedding_generation_batch_size: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ByoApiKeyPolicy {
    pub enabled: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct PurchaseAddOnCreditsPolicy {
    pub enabled: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct EnterprisePayAsYouGoPolicy {
    pub enabled: bool,
    pub payg_cost_per_thousand_credits_cents: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct EnterpriseCreditsAutoReloadPolicy {
    pub enabled: bool,
    pub auto_reload_cost_cents: i32,
    pub auto_reload_credit_denomination: i32,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct MultiAdminPolicy {
    pub enabled: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AmbientAgentsPolicy {
    pub enabled: bool,
    pub toggleable: bool,
    pub max_concurrent_agents: i32,
    pub instance_shape: Option<InstanceShape>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct InstanceShape {
    pub vcpus: i32,
    pub memory_gb: i32,
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum CustomerType {
    Enterprise,
    Free,
    Legacy,
    ProTrial,
    Prosumer,
    SelfServe,
    TeamTrial,
    Turbo,
    Business,
    Lightspeed,
    Build,
    BuildMax,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum DelinquencyStatus {
    NoDelinquency,
    PastDue,
    TeamLimitExceeded,
    Unpaid,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct AddonCreditsOption {
    pub credits: i32,
    pub price_usd_cents: i32,
}

impl AddonCreditsOption {
    pub fn rate(&self) -> f32 {
        self.price_usd_cents as f32 / self.credits as f32
    }
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct PricingInfo {
    pub plans: Vec<PlanPricing>,
    pub overages: OveragesPricing,
    pub addon_credits_options: Vec<AddonCreditsOption>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct PlanPricing {
    pub plan: StripeSubscriptionPlan,
    pub monthly_plan_price_per_month_usd_cents: i32,
    pub yearly_plan_price_per_month_usd_cents: i32,
    pub request_limit: Option<i32>,
    pub codebase_limit: i32,
    pub codebase_context_file_limit: i32,
    pub max_team_size: Option<i32>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct OveragesPricing {
    pub price_per_request_usd_cents: i32,
}

#[derive(cynic::Enum, Clone, Debug, PartialEq)]
pub enum StripeSubscriptionPlan {
    Business,
    Lightspeed,
    Pro,
    Team,
    Turbo,
    Build,
    BuildBusiness,
    BuildMax,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct UsageVisibilityPolicy {
    pub admin_granularity: UsageVisibilityGranularity,
    pub max_prior_cycles: i32,
}

#[derive(cynic::Enum, Clone, Debug, PartialEq, Eq)]
pub enum UsageVisibilityGranularity {
    OwnOnly,
    TeamAggregate,
    PerUserTotals,
    FullBreakdown,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct BillingCycleUsageHistory {
    pub current_period_start: Time,
    pub current_period_end: Time,
    pub summaries: Vec<BillingCycleUsageSummary>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct BillingCycleUsageSummary {
    pub period_start: Time,
    pub period_end: Time,
    pub entries: Vec<UsageEntry>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct UsageEntry {
    pub subject_type: AiCreditsUsageAndCostSubjectType,
    pub subject_uid: Option<String>,
    pub subject_display_name: Option<String>,
    pub cost_type: AiCreditsUsageAndCostType,
    pub usage_bucket: AiCreditsUsageBucket,
    pub usage_source: AiCreditsUsageSource,
    pub credits_used: i32,
    pub cost_cents: i32,
}

#[derive(cynic::Enum, Clone, Debug, PartialEq, Eq)]
#[cynic(graphql_type = "AICreditsUsageAndCostSubjectType")]
pub enum AiCreditsUsageAndCostSubjectType {
    Team,
    User,
    ServiceAccount,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Debug, PartialEq, Eq)]
#[cynic(graphql_type = "AICreditsUsageAndCostType")]
pub enum AiCreditsUsageAndCostType {
    BaseLimit,
    BonusGrant,
    Payg,
    AmbientBonusGrant,
    Aggregate,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Debug, PartialEq, Eq)]
#[cynic(graphql_type = "AICreditsUsageBucket")]
pub enum AiCreditsUsageBucket {
    Ai,
    Compute,
    Platform,
    SuggestedCodeDiffs,
    Voice,
    Aggregate,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Clone, Debug, PartialEq, Eq)]
#[cynic(graphql_type = "AICreditsUsageSource")]
pub enum AiCreditsUsageSource {
    Local,
    Cloud,
    Aggregate,
    #[cynic(fallback)]
    Other(String),
}
