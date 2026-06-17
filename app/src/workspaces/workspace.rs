use std::cmp::Ordering;
use std::path::PathBuf;

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use warp_graphql::billing::{AddonCreditAutoReloadStatus, ServiceAgreement, ServiceAgreementType};
pub use warp_graphql::billing::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource,
};

use super::team::{MembershipRole, Team};
use crate::ai::execution_profiles::{
    ActionPermission, ComputerUsePermission, WriteToPtyPermission,
};
use crate::ai::llms::LLMModelHost;
use crate::auth::UserUid;
use crate::server::ids::ServerId;
use crate::settings::AgentModeCommandExecutionPredicate;

#[derive(Clone, Copy, Hash, Debug, PartialEq, Eq)]
pub struct WorkspaceUid(ServerId);
impl From<String> for WorkspaceUid {
    fn from(uid: String) -> Self {
        WorkspaceUid(ServerId::from_string_lossy(uid))
    }
}
impl From<WorkspaceUid> for String {
    fn from(workspace_uid: WorkspaceUid) -> String {
        workspace_uid.0.to_string()
    }
}
impl From<ServerId> for WorkspaceUid {
    fn from(uid: ServerId) -> Self {
        WorkspaceUid(uid)
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub uid: WorkspaceUid,
    pub name: String,
    pub stripe_customer_id: Option<String>,
    pub teams: Vec<Team>,
    pub billing_metadata: BillingMetadata,
    pub bonus_grants_purchased_this_month: BonusGrantsPurchased,
    pub billing_cycle_usage: Option<BillingCycleUsageData>,
    pub has_billing_history: bool,
    pub settings: WorkspaceSettings,
    pub invite_code: Option<WorkspaceInviteCode>,
    pub invite_link_domain_restrictions: Vec<InviteLinkDomainRestriction>,
    pub pending_email_invites: Vec<EmailInvite>,
    // If the team is eligible for discovery, then show toggle for setting discoverability to the team's admin
    pub is_eligible_for_discovery: bool,
    pub members: Vec<WorkspaceMember>,
    pub total_requests_used_since_last_refresh: i32,
}

impl Workspace {
    pub fn from_local_cache(uid: WorkspaceUid, name: String, teams: Option<Vec<Team>>) -> Self {
        // Derive the workspace billing metadata from the first team's cached billing
        // metadata, if available. This ensures the workspace-level billing info is
        // consistent with team-level data loaded from the cache.
        let billing_metadata = teams
            .as_ref()
            .and_then(|t| t.first())
            .map(|team| team.billing_metadata.clone())
            .unwrap_or_default();
        Self {
            uid,
            name,
            stripe_customer_id: Default::default(),
            teams: teams.unwrap_or_default(),
            billing_metadata,
            bonus_grants_purchased_this_month: Default::default(),
            billing_cycle_usage: None,
            has_billing_history: false,
            settings: Default::default(), // TODO: persistence wrapper instead of default
            invite_code: Default::default(),
            invite_link_domain_restrictions: Default::default(),
            pending_email_invites: Default::default(),
            is_eligible_for_discovery: false,
            members: Default::default(),
            total_requests_used_since_last_refresh: 0,
        }
    }

    fn get_member_by_email(&self, email: &str) -> Option<&WorkspaceMember> {
        self.members.iter().find(|member| member.email == email)
    }

    pub fn is_workspace_admin(&self, user_email: &str) -> bool {
        self.get_member_by_email(user_email)
            .is_some_and(|member| member.role.is_admin_or_owner())
    }

    pub fn resolve_usage_visibility(&self, is_admin: bool) -> UsageVisibility {
        let Some(policy) = self.billing_metadata.tier.usage_visibility_policy else {
            return UsageVisibility::default();
        };
        UsageVisibility {
            granularity: if is_admin {
                policy.admin_granularity
            } else {
                UsageVisibilityGranularity::OwnOnly
            },
            max_prior_cycles: policy.max_prior_cycles,
        }
    }

    pub fn can_be_deleted(&self, current_user_email: &str) -> bool {
        // Current user needs to be an admin and be the only user remaining
        self.is_workspace_admin(current_user_email)
            && self.members.len() == 1
            && self
                .members
                .first()
                .is_some_and(|m| m.email == current_user_email)
    }

    pub fn is_custom_llm_enabled(&self) -> bool {
        self.settings.llm_settings.enabled
    }

    pub fn are_overages_toggleable(&self) -> bool {
        self.billing_metadata
            .tier
            .usage_based_pricing_policy
            .is_some_and(|policy| policy.toggleable)
    }

    pub fn are_overages_enabled(&self) -> bool {
        self.settings.usage_based_pricing_settings.enabled
    }

    pub fn are_overages_remaining(&self) -> bool {
        if self.settings.usage_based_pricing_settings.enabled {
            if let Some(max_spend_cents) = self
                .settings
                .usage_based_pricing_settings
                .max_monthly_spend_cents
            {
                if let Some(ai_overages) = &self.billing_metadata.ai_overages {
                    return ai_overages.current_monthly_request_cost_cents < max_spend_cents as i32;
                } else {
                    // If they have the setting enabled but no overages usage so far,
                    // that means they have no database entry, so they have overages remaining.
                    return true;
                }
            }
        }

        false
    }

    pub fn is_byo_api_key_enabled(&self) -> bool {
        self.billing_metadata.is_byo_api_key_enabled()
    }

    /// Returns true if the workspace has reached or exceeded its monthly addon credits spend limit.
    pub fn is_at_addon_credits_monthly_limit(&self) -> bool {
        if let Some(limit) = self.settings.addon_credits_settings.max_monthly_spend_cents {
            self.bonus_grants_purchased_this_month.cents_spent >= limit
        } else {
            false
        }
    }

    /// Returns true if purchasing addon credits at the given price would reach or exceed the monthly limit.
    pub fn would_addon_purchase_reach_limit(&self, price_cents: i32) -> bool {
        if let Some(limit) = self.settings.addon_credits_settings.max_monthly_spend_cents {
            self.bonus_grants_purchased_this_month.cents_spent + price_cents > limit
        } else {
            false
        }
    }

    /// Returns the price in cents for the selected auto-reload credit denomination.
    /// Returns None if auto-reload is not configured or if the denomination can't be found in pricing options.
    pub fn get_auto_reload_price_cents(
        &self,
        addon_credits_options: &[warp_graphql::billing::AddonCreditsOption],
    ) -> Option<i32> {
        let selected_credits = self
            .settings
            .addon_credits_settings
            .selected_auto_reload_credit_denomination?;

        addon_credits_options
            .iter()
            .find(|option| option.credits == selected_credits)
            .map(|option| option.price_usd_cents)
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceInviteCode {
    pub code: String,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct WorkspaceMember {
    pub uid: UserUid,
    pub email: String,
    pub role: MembershipRole,
    pub usage_info: WorkspaceMemberUsageInfo,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct WorkspaceMemberUsageInfo {
    pub is_unlimited: bool,
    pub request_limit: i32,
    pub requests_used_since_last_refresh: i32,
    pub is_request_limit_prorated: bool,
}

impl PartialOrd for WorkspaceMember {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for WorkspaceMember {
    fn cmp(&self, other: &Self) -> Ordering {
        self.email.cmp(&other.email)
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct EmailInvite {
    pub invitee_email: String,
    pub expired: bool,
}

impl PartialOrd for EmailInvite {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EmailInvite {
    fn cmp(&self, other: &Self) -> Ordering {
        self.invitee_email.cmp(&other.invitee_email)
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct InviteLinkDomainRestriction {
    pub uid: ServerId,
    pub domain: String,
}

impl PartialOrd for InviteLinkDomainRestriction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for InviteLinkDomainRestriction {
    fn cmp(&self, other: &Self) -> Ordering {
        self.domain.cmp(&other.domain)
    }
}

/// This enum is the rust representation of `CustomerType` from the GraphQL Schema.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum CustomerType {
    #[default]
    Free,
    Turbo,
    SelfServe,
    Prosumer,
    Legacy,
    Enterprise,
    Business,
    Lightspeed,
    Build,
    BuildMax,
    Unknown,
}

impl CustomerType {
    pub fn to_display_string(self) -> String {
        match self {
            CustomerType::Free => "Free".to_string(),
            CustomerType::Turbo => "Turbo".to_string(),
            CustomerType::SelfServe => "Team".to_string(),
            CustomerType::Prosumer => "Pro".to_string(),
            CustomerType::Legacy => "Early adopter".to_string(),
            CustomerType::Enterprise => "Enterprise".to_string(),
            CustomerType::Business => "Business".to_string(),
            CustomerType::Lightspeed => "Lightspeed".to_string(),
            CustomerType::Build => "Build".to_string(),
            CustomerType::BuildMax => "Max".to_string(),
            CustomerType::Unknown => "".to_string(),
        }
    }
}

/// This enum is the rust representation of `DelinquencyStatus` from the GraphQL Schema.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum DelinquencyStatus {
    #[default]
    NoDelinquency,
    PastDue,
    Unpaid,
    TeamLimitExceeded,
    Unknown,
}

/// Rust representation of feature policies from the GraphQL Schema.
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct WarpAiPolicy {
    pub limit: i64,
    pub is_code_suggestions_toggleable: bool,
    pub is_prompt_suggestions_toggleable: bool,
    pub is_next_command_enabled: bool,
    pub is_git_operations_ai_enabled: bool,
    pub is_voice_enabled: bool,
}
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct WorkspaceSizePolicy {
    pub is_unlimited: bool,
    pub limit: i64,
}
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct SharedNotebooksPolicy {
    pub is_unlimited: bool,
    pub limit: i64,
}
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct SharedWorkflowsPolicy {
    pub is_unlimited: bool,
    pub limit: i64,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct SessionSharingPolicy {
    pub is_enabled: bool,
    pub max_session_size: u64,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct AIAutonomyPolicy {
    pub is_enabled: bool,
    pub toggleable: bool,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct TelemetryDataCollectionPolicy {
    pub default: bool,
    pub toggleable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UgcDataCollectionPolicy {
    pub default_setting: UgcCollectionEnablementSetting,
    pub toggleable: bool,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct UsageBasedPricingPolicy {
    pub toggleable: bool,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct CodebaseContextPolicy {
    pub toggleable: bool,
    pub index_limit: Option<u32>,
    pub max_files_per_repo: u32,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct ByoApiKeyPolicy {
    pub enabled: bool,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct PurchaseAddOnCreditsPolicy {
    pub enabled: bool,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct EnterprisePayAsYouGoPolicy {
    pub enabled: bool,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct EnterpriseCreditsAutoReloadPolicy {
    pub enabled: bool,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct MultiAdminPolicy {
    pub enabled: bool,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct AmbientAgentsPolicy {
    pub max_concurrent_agents: i32,
    pub instance_shape: Option<InstanceShape>,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub struct InstanceShape {
    pub vcpus: i32,
    pub memory_gb: i32,
}

/// Granularity at which a viewer can see AI usage across their team.
/// Non-admins always collapse to `OwnOnly` regardless of tier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageVisibilityGranularity {
    #[default]
    OwnOnly,
    TeamAggregate,
    PerUserTotals,
    FullBreakdown,
}

/// Number of prior billing cycles a viewer can scroll back through, in
/// addition to the always-visible current cycle. Plan-wide; applies to
/// admins and non-admins alike.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaxPriorCycles {
    #[default]
    None,
    /// Current cycle plus `n` prior cycles (`n >= 1`).
    Limited(u32),
    Unlimited,
}

/// Rust representation of the `UsageVisibilityPolicy` tier policy from the
/// GraphQL schema.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct UsageVisibilityPolicy {
    pub admin_granularity: UsageVisibilityGranularity,
    pub max_prior_cycles: MaxPriorCycles,
}

/// Effective per-viewer visibility, after combining the tier's
/// `UsageVisibilityPolicy` with the viewer's admin status. Non-admins always
/// collapse to `granularity == OwnOnly`; `max_prior_cycles` is plan-wide and
/// applies to admins and non-admins alike. Built by
/// [`Workspace::resolve_usage_visibility`].
#[derive(Clone, Copy, Debug, Default)]
pub struct UsageVisibility {
    pub granularity: UsageVisibilityGranularity,
    pub max_prior_cycles: MaxPriorCycles,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum HostEnablementSetting {
    Enforce,
    #[default]
    RespectUserSetting,
}

/// This struct is the rust representation of `Tier` from the GraphQL Schema.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tier {
    pub name: String,
    pub description: String,
    pub warp_ai_policy: Option<WarpAiPolicy>,
    pub workspace_size_policy: Option<WorkspaceSizePolicy>,
    pub shared_notebooks_policy: Option<SharedNotebooksPolicy>,
    pub shared_workflows_policy: Option<SharedWorkflowsPolicy>,
    pub session_sharing_policy: Option<SessionSharingPolicy>,
    pub ai_autonomy_policy: Option<AIAutonomyPolicy>,
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

/// This struct is the rust representation of `BillingMetadata` from the GraphQL Schema.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BillingMetadata {
    pub tier: Tier,
    pub customer_type: CustomerType,
    pub delinquency_status: DelinquencyStatus,
    #[serde(skip)]
    pub service_agreements: Vec<ServiceAgreement>,
    #[serde(skip)]
    pub ai_overages: Option<AiOverages>,
}

#[derive(Clone, Debug, Default)]
pub struct BonusGrantsPurchased {
    pub total_credits_purchased: i32,
    pub cents_spent: i32,
}

#[derive(Clone, Debug)]
pub struct AiOverages {
    pub current_monthly_request_cost_cents: i32,
    pub current_monthly_requests_used: i32,
    pub current_period_end: chrono::DateTime<chrono::Utc>,
}

/// A single redacted usage entry from `Workspace.billingCycleUsageHistory`.
///
/// The shape of this entry depends on the viewer's resolved `UsageVisibility`:
/// * `OwnOnly` viewers receive only their own entries with real `cost_type` /
///   `usage_bucket` / `usage_source` values.
/// * `TeamAggregate` viewers receive exactly one synthetic `TEAM` row per cycle
///   carrying `Aggregate` sentinels for all three categorical fields.
/// * `PerUserTotals` viewers receive one row per user / service account per
///   cycle, also with `Aggregate` sentinels on the categorical fields.
/// * `FullBreakdown` viewers receive every real row, one per
///   `(subject, cost_type, bucket, source)` tuple. Categorical fields always
///   carry real values — the server does **not** synthesize an aggregate team
///   total at this granularity. Compute team-wide sums client-side if needed.
#[derive(Clone, Debug)]
pub struct BillingCycleUsageEntry {
    pub subject_type: AiCreditsUsageAndCostSubjectType,
    pub subject_uid: Option<String>,
    pub subject_display_name: Option<String>,
    pub cost_type: AiCreditsUsageAndCostType,
    pub usage_bucket: AiCreditsUsageBucket,
    pub usage_source: AiCreditsUsageSource,
    pub credits_used: i32,
    pub cost_cents: i32,
}

/// Per-cycle bucket of redacted usage entries with explicit period bounds.
/// `period_end` is exclusive (e.g. a summary covering May 2026 has
/// `period_end = 2026-06-01T00:00:00Z`).
#[derive(Clone, Debug)]
pub struct BillingCycleUsageSummary {
    pub period_start: chrono::DateTime<chrono::Utc>,
    pub period_end: chrono::DateTime<chrono::Utc>,
    pub entries: Vec<BillingCycleUsageEntry>,
}

/// The full per-cycle usage history for a workspace, as redacted by the
/// server's `USAGE_VISIBILITY` policy. `current_period_start` /
/// `current_period_end` mark the cycle that's currently active; older
/// summaries cover prior cycles and the number of them retained is governed
/// by the policy's `max_prior_cycles`.
#[derive(Clone, Debug)]
pub struct BillingCycleUsageData {
    pub current_period_start: chrono::DateTime<chrono::Utc>,
    pub current_period_end: chrono::DateTime<chrono::Utc>,
    pub summaries: Vec<BillingCycleUsageSummary>,
}

impl BillingMetadata {
    /// Returns whether the current tier has a usage-based pricing policy that can be toggled.
    pub fn is_usage_based_pricing_toggleable(&self) -> bool {
        self.tier
            .usage_based_pricing_policy
            .as_ref()
            .is_some_and(|policy| policy.toggleable)
    }

    /**
     * Returns whether customer can upgrade to the Build plan based on their current tier.
     */
    pub fn can_upgrade_to_build_plan(&self) -> bool {
        match self.customer_type {
            CustomerType::Unknown
            | CustomerType::Business
            | CustomerType::Enterprise
            | CustomerType::Build
            | CustomerType::BuildMax => false,
            CustomerType::Free
            | CustomerType::Legacy
            | CustomerType::Prosumer
            | CustomerType::Turbo
            | CustomerType::SelfServe
            | CustomerType::Lightspeed => true,
        }
    }

    /**
     * Returns whether customer can upgrade to the Build Max plan based on their current tier.
     * Users on Build can upgrade to Build Max.
     */
    pub fn can_upgrade_to_build_max_plan(&self) -> bool {
        self.can_upgrade_to_build_plan() || self.customer_type == CustomerType::Build
    }

    /**
     * Returns whether customer can upgrade to a higher tier based on their current tier.
     */
    pub fn can_upgrade_to_higher_tier_plan(&self) -> bool {
        self.can_upgrade_to_build_plan()
    }

    pub fn is_stripe_paid_plan(customer_type: CustomerType) -> bool {
        match customer_type {
            CustomerType::Turbo
            | CustomerType::SelfServe
            | CustomerType::Prosumer
            | CustomerType::Business
            | CustomerType::Lightspeed
            | CustomerType::Build
            | CustomerType::BuildMax => true,
            CustomerType::Free
            | CustomerType::Enterprise
            | CustomerType::Legacy
            | CustomerType::Unknown => false,
        }
    }

    pub fn is_user_on_paid_plan(&self) -> bool {
        match self.customer_type {
            CustomerType::Turbo
            | CustomerType::SelfServe
            | CustomerType::Prosumer
            | CustomerType::Business
            | CustomerType::Lightspeed
            | CustomerType::Enterprise
            | CustomerType::Legacy
            | CustomerType::Build
            | CustomerType::BuildMax => true,
            CustomerType::Free | CustomerType::Unknown => false,
        }
    }

    pub fn is_on_stripe_paid_plan(&self) -> bool {
        BillingMetadata::is_stripe_paid_plan(self.customer_type)
    }

    pub fn is_on_build_plan(&self) -> bool {
        self.customer_type == CustomerType::Build
    }

    pub fn is_on_build_max_plan(&self) -> bool {
        self.customer_type == CustomerType::BuildMax
    }

    pub fn is_on_build_business_plan(&self) -> bool {
        self.customer_type == CustomerType::Business
            && matches!(
                self.service_agreements.first().map(|sa| &sa.type_),
                Some(ServiceAgreementType::SelfServe)
            )
    }

    pub fn is_on_legacy_business_plan(&self) -> bool {
        self.customer_type == CustomerType::Business && !self.is_on_build_business_plan()
    }

    pub fn is_enterprise_plan(&self) -> bool {
        self.customer_type == CustomerType::Enterprise
    }

    pub fn is_free_plan(&self) -> bool {
        self.customer_type == CustomerType::Free
    }

    pub fn is_on_legacy_paid_plan(&self) -> bool {
        match self.customer_type {
            CustomerType::Prosumer
            | CustomerType::Turbo
            | CustomerType::Lightspeed
            | CustomerType::SelfServe => true,
            CustomerType::Business => self.is_on_legacy_business_plan(),
            CustomerType::Free
            | CustomerType::Legacy
            | CustomerType::Enterprise
            | CustomerType::Build
            | CustomerType::BuildMax
            | CustomerType::Unknown => false,
        }
    }

    pub fn is_delinquent_due_to_payment_issue(&self) -> bool {
        self.delinquency_status == DelinquencyStatus::PastDue
            || self.delinquency_status == DelinquencyStatus::Unpaid
    }

    // Whether the enterprise customer is our Stable Warp Enterprise team (internal team of Warpers).
    pub fn is_warp_plan(&self) -> bool {
        self.tier.name == "Warp Plan"
    }

    pub fn has_active_subscription(&self) -> bool {
        if let Some(newest_service_agreement) = self.service_agreements.first() {
            let not_expired = Utc::now() < newest_service_agreement.current_period_end.utc();
            let not_delinquent = !self.is_delinquent_due_to_payment_issue();
            not_expired && not_delinquent
        } else {
            false
        }
    }

    pub fn is_byo_api_key_enabled(&self) -> bool {
        self.tier
            .byo_api_key_policy
            .is_some_and(|policy| policy.enabled)
    }

    pub fn has_overages_used(&self) -> bool {
        self.ai_overages
            .as_ref()
            .is_some_and(|ai_overages| ai_overages.current_monthly_requests_used > 0)
    }

    pub fn has_failed_addon_credit_auto_reload_status(&self) -> bool {
        self.service_agreements
            .first()
            .and_then(|sa| sa.addon_credit_auto_reload_status)
            .is_some_and(|status| matches!(status, AddonCreditAutoReloadStatus::Failed))
    }

    pub fn is_enterprise_pay_as_you_go_enabled(&self) -> bool {
        self.customer_type == CustomerType::Enterprise
            && self
                .tier
                .enterprise_pay_as_you_go_policy
                .is_some_and(|policy| policy.enabled)
    }

    pub fn is_enterprise_auto_reload_enabled(&self) -> bool {
        self.customer_type == CustomerType::Enterprise
            && self
                .tier
                .enterprise_credits_auto_reload_policy
                .is_some_and(|policy| policy.enabled)
    }

    pub fn is_purchase_add_on_credits_policy_enabled(&self) -> bool {
        self.tier
            .purchase_add_on_credits_policy
            .is_some_and(|policy| policy.enabled)
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LlmHostSettings {
    pub enabled: bool,
    pub enablement_setting: HostEnablementSetting,
    /// Full resource name of the GCP workload identity provider that Gemini Enterprise
    /// (GEAP) credential minting exchanges Warp OIDC JWTs against. Only populated on the
    /// `GeminiEnterprise` host entry; `None` for other hosts and for workspace caches
    /// written before this field existed.
    #[serde(default)]
    pub gcp_audience: Option<String>,
    /// Email of the GCP service account that Gemini Enterprise credential minting
    /// impersonates after the STS exchange. `None` (or empty) means the federated token
    /// is used directly.
    #[serde(default)]
    pub gcp_sa_email: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LlmSettings {
    pub enabled: bool,
    #[serde(default)]
    pub host_configs: std::collections::HashMap<LLMModelHost, LlmHostSettings>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TelemetrySettings {
    pub force_enabled: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum UgcCollectionEnablementSetting {
    Disable,
    Enable,
    #[default]
    RespectUserSetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UgcCollectionSettings {
    pub setting: UgcCollectionEnablementSetting,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum AdminEnablementSetting {
    Disable,
    Enable,
    #[default]
    RespectUserSetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CloudConversationStorageSettings {
    pub setting: AdminEnablementSetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AiPermissionsSettings {
    pub allow_ai_in_remote_sessions: bool,
    #[serde(with = "serde_regex")]
    pub remote_session_regex_list: Vec<Regex>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AiAutonomySettings {
    pub apply_code_diffs_setting: Option<ActionPermission>,
    pub read_files_setting: Option<ActionPermission>,
    pub read_files_allowlist: Option<Vec<PathBuf>>,
    pub execute_commands_setting: Option<ActionPermission>,
    pub execute_commands_allowlist: Option<Vec<AgentModeCommandExecutionPredicate>>,
    pub execute_commands_denylist: Option<Vec<AgentModeCommandExecutionPredicate>>,
    pub write_to_pty_setting: Option<WriteToPtyPermission>,
    pub computer_use_setting: Option<ComputerUsePermission>,
}

impl AiAutonomySettings {
    pub fn has_any_overrides(&self) -> bool {
        self.apply_code_diffs_setting.is_some()
            || self.read_files_setting.is_some()
            || self.read_files_allowlist.is_some()
            || self.execute_commands_setting.is_some()
            || self.execute_commands_allowlist.is_some()
            || self.execute_commands_denylist.is_some()
            || self.write_to_pty_setting.is_some()
            || self.computer_use_setting.is_some()
    }

    pub fn has_override_for_code_diffs(&self) -> bool {
        self.apply_code_diffs_setting.is_some()
    }

    pub fn has_override_for_read_files(&self) -> bool {
        self.read_files_setting.is_some()
    }

    pub fn has_override_for_read_files_allowlist(&self) -> bool {
        self.read_files_allowlist.is_some()
    }

    pub fn has_override_for_execute_commands(&self) -> bool {
        self.execute_commands_setting.is_some()
    }

    pub fn has_override_for_execute_commands_allowlist(&self) -> bool {
        self.execute_commands_allowlist.is_some()
    }

    pub fn has_override_for_execute_commands_denylist(&self) -> bool {
        self.execute_commands_denylist.is_some()
    }

    pub fn has_override_for_write_to_pty(&self) -> bool {
        self.write_to_pty_setting.is_some()
    }

    pub fn has_override_for_computer_use(&self) -> bool {
        self.computer_use_setting.is_some()
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LinkSharingSettings {
    pub anyone_with_link_sharing_enabled: bool,
    pub direct_link_sharing_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnterpriseSecretRegex {
    pub pattern: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SecretRedactionSettings {
    pub enabled: bool,
    pub regexes: Vec<EnterpriseSecretRegex>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageBasedPricingSettings {
    pub enabled: bool,
    pub max_monthly_spend_cents: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AddonCreditsSettings {
    pub auto_reload_enabled: bool,
    pub max_monthly_spend_cents: Option<i32>,
    pub selected_auto_reload_credit_denomination: Option<i32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CodebaseContextSettings {
    pub setting: AdminEnablementSetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SandboxedAgentSettings {
    pub execute_commands_denylist: Option<Vec<AgentModeCommandExecutionPredicate>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceSettings {
    pub llm_settings: LlmSettings,
    pub telemetry_settings: TelemetrySettings,
    pub ugc_collection_settings: UgcCollectionSettings,
    pub cloud_conversation_storage_settings: CloudConversationStorageSettings,
    pub link_sharing_settings: LinkSharingSettings,
    pub secret_redaction_settings: SecretRedactionSettings,
    pub ai_permissions_settings: AiPermissionsSettings,
    pub ai_autonomy_settings: AiAutonomySettings,
    pub is_invite_link_enabled: bool,
    pub is_discoverable: bool,
    pub usage_based_pricing_settings: UsageBasedPricingSettings,
    pub addon_credits_settings: AddonCreditsSettings,
    pub codebase_context_settings: CodebaseContextSettings,
    pub sandboxed_agent_settings: Option<SandboxedAgentSettings>,
    /// The team-level agent attribution setting. When `Enable` or `Disable`, the
    /// user toggle is locked. When `RespectUserSetting` (or absent), the user can choose.
    #[serde(default)]
    pub enable_warp_attribution: AdminEnablementSetting,
    #[serde(default)]
    pub default_host_slug: Option<String>,
}
