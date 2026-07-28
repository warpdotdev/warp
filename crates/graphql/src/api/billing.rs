use crate::scalars::Time;
use crate::schema;
use crate::workspace::UgcCollectionEnablementSetting;

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
    pub byo_endpoint_policy: Option<ByoEndpointPolicy>,
    pub managed_byok_byoe_policy: Option<ManagedByokByoePolicy>,
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
pub struct ByoEndpointPolicy {
    pub enabled: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ManagedByokByoePolicy {
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
    /// Legacy price field kept for backward compatibility with old servers.
    pub price_usd_cents: i32,
    /// Nullable: old servers that have not yet deployed the price-breakdown fields
    /// return null here. Fall back to `price_usd_cents` when None.
    pub base_price_usd_cents: Option<i32>,
    /// Nullable: see `base_price_usd_cents`.
    pub markup_usd_cents: Option<i32>,
    /// Nullable: see `base_price_usd_cents`.
    pub total_price_usd_cents: Option<i32>,
}

impl AddonCreditsOption {
    /// Base price in cents, falling back to the legacy field for old servers.
    pub fn effective_base_price_cents(&self) -> i32 {
        self.base_price_usd_cents.unwrap_or(self.price_usd_cents)
    }

    /// Rate in cents per credit using the base price (for volume-discount badge calculations).
    pub fn rate(&self) -> f32 {
        self.effective_base_price_cents() as f32 / self.credits as f32
    }

    /// Whether this option has a Free-plan markup applied.
    pub fn has_markup(&self) -> bool {
        self.markup_usd_cents.is_some_and(|m| m > 0)
    }

    /// The total price in cents that the user will actually be charged.
    /// Falls back to the legacy `price_usd_cents` for old servers that have not
    /// yet deployed `totalPriceUsdCents`. Use for spend-limit checks and
    /// confirmation displays.
    pub fn total_price_cents(&self) -> i32 {
        self.total_price_usd_cents.unwrap_or(self.price_usd_cents)
    }

    /// Returns false when the server has returned internally inconsistent or
    /// otherwise unsafe price data (negative total, total < base, etc.).
    /// Purchase must be disabled and the amount must not be displayed when false.
    ///
    /// A partially-populated breakdown (some of the three nullable fields present,
    /// others null) is also treated as invalid, since a correct new server always
    /// sends all three fields together. The all-null case is the old-server fallback
    /// and uses `price_usd_cents`, which is always valid if positive.
    pub fn is_price_valid(&self) -> bool {
        // If any breakdown field is present, all three must be: a partial breakdown
        // from a new server is a malformed response, not a legacy-fallback case.
        let has_any = self.base_price_usd_cents.is_some()
            || self.markup_usd_cents.is_some()
            || self.total_price_usd_cents.is_some();
        let has_all = self.base_price_usd_cents.is_some()
            && self.markup_usd_cents.is_some()
            && self.total_price_usd_cents.is_some();
        if has_any && !has_all {
            return false;
        }
        let total = self.total_price_cents();
        let base = self.effective_base_price_cents();
        total > 0 && base > 0 && total >= base
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
