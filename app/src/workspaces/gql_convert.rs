use std::path::PathBuf;

use anyhow::{anyhow, bail};
use regex::Regex;
use warp_graphql::billing::{
    AiAutonomyPolicy as GqlAiAutonomyPolicy, AmbientAgentsPolicy as GqlAmbientAgentsPolicy,
    BillingCycleUsageHistory as GqlBillingCycleUsageHistory, BillingMetadata as GqlBillingMetadata,
    BonusGrant as GqlBonusGrant, ByoApiKeyPolicy as GqlByoApiKeyPolicy,
    CodebaseContextPolicy as GqlCodebaseContextPolicy, CustomerType as GqlCustomerType,
    DelinquencyStatus as GqlDelinquencyStatus,
    EnterpriseCreditsAutoReloadPolicy as GqlEnterpriseCreditsAutoReloadPolicy,
    EnterprisePayAsYouGoPolicy as GqlEnterprisePayAsYouGoPolicy, InstanceShape as GqlInstanceShape,
    MultiAdminPolicy as GqlMultiAdminPolicy,
    PurchaseAddOnCreditsPolicy as GqlPurchaseAddOnCreditsPolicy, ServiceAgreementType,
    SessionSharingPolicy as GqlSessionSharingPolicy,
    SharedNotebooksPolicy as GqlSharedNotebooksPolicy,
    SharedWorkflowsPolicy as GqlSharedWorkflowsPolicy, StripeSubscriptionPlan,
    TeamSizePolicy as GqlTeamSizePolicy,
    TelemetryDataCollectionPolicy as GqlTelemetryDataCollectionPolicy, Tier as GqlTier,
    UgcDataCollectionPolicy as GqlUgcDataCollectionPolicy,
    UsageBasedPricingPolicy as GqlUsageBasedPricingPolicy,
    UsageVisibilityGranularity as GqlUsageVisibilityGranularity,
    UsageVisibilityPolicy as GqlUsageVisibilityPolicy, WarpAiPolicy as GqlWarpAiPolicy,
};
use warp_graphql::object::CloudObjectWithDescendants;
use warp_graphql::queries::get_conversation_usage as gql_usage;
use warp_graphql::queries::get_workspaces_metadata_for_user::User as GqlUser;
use warp_graphql::subscriptions::get_warp_drive_updates::WarpDriveUpdate;
use warp_graphql::user::{DiscoverableTeamData as GqlDiscoverableTeamData, PublicUserProfile};
use warp_graphql::workspace::{
    AddonCreditsSettings as GqlAddonCreditsSettings,
    AdminEnablementSetting as GqlAdminEnablementSetting, AiAutonomyValue as GqlAiAutonomyValue,
    AiPermissionsSettings as GqlAiPermissionsSettings,
    ComputerUseAutonomyValue as GqlComputerUseAutonomyValue, EmailInvite as GqlEmailInvite,
    HostEnablementSetting as GqlHostEnablementSetting,
    InviteLinkDomainRestriction as GqlInviteLinkDomainRestriction,
    MembershipRole as GqlMembershipRole, Team as GqlTeam, TeamMember as GqlTeamMember,
    UgcCollectionEnablementSetting as GqlUgcCollectionEnablementSetting, Workspace as GqlWorkspace,
    WorkspaceMember as GqlWorkspaceMember, WorkspaceMemberUsageInfo as GqlWorkspaceMemberUsageInfo,
    WorkspaceSettings as GqlWorkspaceSettings,
    WriteToPtyAutonomyValue as GqlWriteToPtyAutonomyValue,
};

use super::team::{DiscoverableTeam, MembershipRole, Team, TeamMember};
use super::user_profiles::UserProfileWithUID;
use super::user_workspaces::WorkspacesMetadataResponse;
use super::workspace::{
    AIAutonomyPolicy, AddonCreditsSettings, AdminEnablementSetting, AiAutonomySettings,
    AiPermissionsSettings, AmbientAgentsPolicy, BillingCycleUsageData, BillingCycleUsageEntry,
    BillingCycleUsageSummary, BillingMetadata, CloudConversationStorageSettings,
    CodebaseContextSettings, CustomerType, DelinquencyStatus, EmailInvite, EnterpriseSecretRegex,
    HostEnablementSetting, InstanceShape, InviteLinkDomainRestriction, LinkSharingSettings,
    LlmSettings, MaxPriorCycles, SandboxedAgentSettings, SecretRedactionSettings,
    SessionSharingPolicy, SharedNotebooksPolicy, SharedWorkflowsPolicy,
    TelemetryDataCollectionPolicy, TelemetrySettings, Tier, UgcCollectionEnablementSetting,
    UgcCollectionSettings, UgcDataCollectionPolicy, UsageBasedPricingPolicy,
    UsageVisibilityGranularity, UsageVisibilityPolicy, WarpAiPolicy, Workspace,
    WorkspaceInviteCode, WorkspaceMember, WorkspaceMemberUsageInfo, WorkspaceSettings,
    WorkspaceSizePolicy,
};
use crate::ai::blocklist::usage::conversation_usage_view::ConversationUsageInfo;
use crate::ai::execution_profiles::{
    ActionPermission, ComputerUsePermission, WriteToPtyPermission,
};
use crate::ai::{BonusGrant, BonusGrantScope};
use crate::auth::UserUid;
use crate::cloud_object::{
    ServerAIExecutionProfile, ServerAIFact, ServerAmbientAgentEnvironment, ServerCloudObject,
    ServerEnvVarCollection, ServerFolder, ServerMCPServer, ServerNotebook, ServerPreference,
    ServerScheduledAmbientAgent, ServerTemplatableMCPServer, ServerWorkflow, ServerWorkflowEnum,
    TryFromGql as _,
};
use crate::server::cloud_objects::listener::ObjectUpdateMessage;
use crate::server::experiments::ServerExperiment;
use crate::server::ids::ServerId;
use crate::settings::AgentModeCommandExecutionPredicate;
use crate::workspaces::workspace::{
    AiOverages, BonusGrantsPurchased, ByoApiKeyPolicy, CodebaseContextPolicy,
    EnterpriseCreditsAutoReloadPolicy, EnterprisePayAsYouGoPolicy, MultiAdminPolicy,
    PurchaseAddOnCreditsPolicy, UsageBasedPricingSettings,
};
use crate::{convert_to_server_experiment, report_error};

pub const PLACEHOLDER_WORKSPACE_UID: &str = "NOT_A_REAL_WORKSPACE_UID";

impl From<GqlTeamMember> for TeamMember {
    fn from(gql_team_member: GqlTeamMember) -> TeamMember {
        Self {
            uid: UserUid::new(&gql_team_member.uid.into_inner()),
            email: gql_team_member.email,
            role: gql_team_member.role.into(),
        }
    }
}

impl From<GqlMembershipRole> for MembershipRole {
    fn from(role: GqlMembershipRole) -> Self {
        match role {
            GqlMembershipRole::Owner => MembershipRole::Owner,
            GqlMembershipRole::Admin => MembershipRole::Admin,
            GqlMembershipRole::User => MembershipRole::User,
            GqlMembershipRole::Unknown => {
                report_error!(anyhow!(
                    "Invalid MembershipRole from server; treating as User"
                ));
                MembershipRole::User
            }
        }
    }
}

impl From<MembershipRole> for GqlMembershipRole {
    fn from(role: MembershipRole) -> Self {
        match role {
            MembershipRole::Owner => GqlMembershipRole::Owner,
            MembershipRole::Admin => GqlMembershipRole::Admin,
            MembershipRole::User => GqlMembershipRole::User,
        }
    }
}

impl From<GqlWorkspaceMemberUsageInfo> for WorkspaceMemberUsageInfo {
    fn from(
        gql_workspace_member_usage_info: GqlWorkspaceMemberUsageInfo,
    ) -> WorkspaceMemberUsageInfo {
        Self {
            request_limit: gql_workspace_member_usage_info.request_limit,
            requests_used_since_last_refresh: gql_workspace_member_usage_info
                .requests_used_since_last_refresh,
            is_unlimited: gql_workspace_member_usage_info.is_unlimited,
            is_request_limit_prorated: gql_workspace_member_usage_info.is_request_limit_prorated,
        }
    }
}

impl From<GqlWorkspaceMember> for WorkspaceMember {
    fn from(gql_workspace_member: GqlWorkspaceMember) -> WorkspaceMember {
        Self {
            uid: UserUid::new(&gql_workspace_member.uid.into_inner()),
            email: gql_workspace_member.email,
            role: gql_workspace_member.role.into(),
            usage_info: gql_workspace_member.usage_info.into(),
        }
    }
}

impl From<GqlEmailInvite> for EmailInvite {
    fn from(gql_email_invite: GqlEmailInvite) -> EmailInvite {
        Self {
            invitee_email: gql_email_invite.email,
            expired: gql_email_invite.expired,
        }
    }
}

impl From<GqlInviteLinkDomainRestriction> for InviteLinkDomainRestriction {
    fn from(
        gql_invite_link_domain_restriction: GqlInviteLinkDomainRestriction,
    ) -> InviteLinkDomainRestriction {
        InviteLinkDomainRestriction {
            uid: ServerId::from_string_lossy(gql_invite_link_domain_restriction.uid.inner()),
            domain: gql_invite_link_domain_restriction.domain,
        }
    }
}

impl From<GqlWarpAiPolicy> for WarpAiPolicy {
    fn from(gql_warp_ai_policy: GqlWarpAiPolicy) -> WarpAiPolicy {
        Self {
            limit: i64::from(gql_warp_ai_policy.limit),
            is_code_suggestions_toggleable: gql_warp_ai_policy.is_code_suggestions_toggleable,
            is_prompt_suggestions_toggleable: gql_warp_ai_policy.is_prompt_suggestions_toggleable,
            is_next_command_enabled: gql_warp_ai_policy.is_next_command_enabled,
            is_git_operations_ai_enabled: gql_warp_ai_policy.is_git_operations_ai_enabled,
            is_voice_enabled: gql_warp_ai_policy.is_voice_enabled,
        }
    }
}

impl From<GqlTeamSizePolicy> for WorkspaceSizePolicy {
    fn from(gql_workspace_size_policy: GqlTeamSizePolicy) -> WorkspaceSizePolicy {
        Self {
            is_unlimited: gql_workspace_size_policy.is_unlimited,
            limit: i64::from(gql_workspace_size_policy.limit),
        }
    }
}

impl From<GqlSharedNotebooksPolicy> for SharedNotebooksPolicy {
    fn from(gql_shared_notebooks_policy: GqlSharedNotebooksPolicy) -> SharedNotebooksPolicy {
        Self {
            is_unlimited: gql_shared_notebooks_policy.is_unlimited,
            limit: i64::from(gql_shared_notebooks_policy.limit),
        }
    }
}

impl From<GqlSharedWorkflowsPolicy> for SharedWorkflowsPolicy {
    fn from(gql_shared_workflows_policy: GqlSharedWorkflowsPolicy) -> SharedWorkflowsPolicy {
        Self {
            is_unlimited: gql_shared_workflows_policy.is_unlimited,
            limit: i64::from(gql_shared_workflows_policy.limit),
        }
    }
}

impl From<GqlSessionSharingPolicy> for SessionSharingPolicy {
    fn from(gql_session_sharing_policy: GqlSessionSharingPolicy) -> SessionSharingPolicy {
        Self {
            is_enabled: gql_session_sharing_policy.enabled,
            max_session_size: u64::try_from(gql_session_sharing_policy.max_session_bytes_size)
                .unwrap_or_default(),
        }
    }
}

impl From<GqlAiAutonomyPolicy> for AIAutonomyPolicy {
    fn from(gql_ai_autonomy_policy: GqlAiAutonomyPolicy) -> AIAutonomyPolicy {
        Self {
            is_enabled: gql_ai_autonomy_policy.enabled,
            toggleable: gql_ai_autonomy_policy.toggleable,
        }
    }
}

impl From<GqlUgcCollectionEnablementSetting> for UgcCollectionEnablementSetting {
    fn from(
        gql_ugc_collection_enablement_setting: GqlUgcCollectionEnablementSetting,
    ) -> UgcCollectionEnablementSetting {
        match gql_ugc_collection_enablement_setting {
            GqlUgcCollectionEnablementSetting::Disable => UgcCollectionEnablementSetting::Disable,
            GqlUgcCollectionEnablementSetting::Enable => UgcCollectionEnablementSetting::Enable,
            GqlUgcCollectionEnablementSetting::RespectUserSetting => {
                UgcCollectionEnablementSetting::RespectUserSetting
            }
            GqlUgcCollectionEnablementSetting::Other(value) => {
                report_error!(anyhow!(
                    "Invalid UgcCollectionEnablementSetting '{value}'. Make sure to update client GraphQL types!"
                ));
                UgcCollectionEnablementSetting::RespectUserSetting
            }
        }
    }
}

impl From<&gql_usage::ConversationUsage> for ConversationUsageInfo {
    fn from(gql: &gql_usage::ConversationUsage) -> Self {
        let persistence::model::ConversationUsageMetadata {
            credits_spent,
            token_usage: models,
            tool_usage_metadata: tool,
            context_window_usage,
            ..
        } = (&gql.usage_metadata).into();
        ConversationUsageInfo {
            credits_spent,
            credits_spent_for_last_block: None,
            tool_calls: tool.total_tool_calls(),
            models,
            context_window_usage,
            files_changed: tool.apply_file_diff_stats.files_changed,
            lines_added: tool.apply_file_diff_stats.lines_added,
            lines_removed: tool.apply_file_diff_stats.lines_removed,
            commands_executed: tool.run_command_stats.commands_executed,
        }
    }
}

impl From<GqlAdminEnablementSetting> for AdminEnablementSetting {
    fn from(gql_admin_enablement_setting: GqlAdminEnablementSetting) -> AdminEnablementSetting {
        match gql_admin_enablement_setting {
            GqlAdminEnablementSetting::Disable => AdminEnablementSetting::Disable,
            GqlAdminEnablementSetting::Enable => AdminEnablementSetting::Enable,
            GqlAdminEnablementSetting::RespectUserSetting => {
                AdminEnablementSetting::RespectUserSetting
            }
            GqlAdminEnablementSetting::Other(value) => {
                report_error!(anyhow!(
                    "Invalid AdminEnablementSetting '{value}'. Make sure to update client GraphQL types!"
                ));
                AdminEnablementSetting::RespectUserSetting
            }
        }
    }
}

impl From<GqlHostEnablementSetting> for HostEnablementSetting {
    fn from(gql_host_enablement_setting: GqlHostEnablementSetting) -> HostEnablementSetting {
        match gql_host_enablement_setting {
            GqlHostEnablementSetting::Enforce => HostEnablementSetting::Enforce,
            GqlHostEnablementSetting::RespectUserSetting => {
                HostEnablementSetting::RespectUserSetting
            }
            GqlHostEnablementSetting::Other(value) => {
                report_error!(anyhow!(
                    "Invalid HostEnablementSetting '{value}'. Make sure to update client GraphQL types!"
                ));
                HostEnablementSetting::RespectUserSetting
            }
        }
    }
}
impl From<&GqlAiPermissionsSettings> for AiPermissionsSettings {
    fn from(gql_ai_permissions_settings: &GqlAiPermissionsSettings) -> AiPermissionsSettings {
        Self {
            allow_ai_in_remote_sessions: gql_ai_permissions_settings.allow_ai_in_remote_sessions,
            remote_session_regex_list: gql_ai_permissions_settings
                .remote_session_regex_list
                .iter()
                .filter_map(|r| {
                    let regex = Regex::new(r);
                    match regex {
                        Ok(regex) => Some(regex),
                        Err(_) => {
                            log::error!("Invalid regex pattern for remote session detection: {r}");
                            None
                        }
                    }
                })
                .collect(),
        }
    }
}

impl From<GqlUgcDataCollectionPolicy> for UgcDataCollectionPolicy {
    fn from(gql_ugc_data_collection_policy: GqlUgcDataCollectionPolicy) -> UgcDataCollectionPolicy {
        Self {
            default_setting: UgcCollectionEnablementSetting::from(
                gql_ugc_data_collection_policy.default_setting,
            ),
            toggleable: gql_ugc_data_collection_policy.toggleable,
        }
    }
}

impl From<GqlTelemetryDataCollectionPolicy> for TelemetryDataCollectionPolicy {
    fn from(
        gql_telemetry_data_collection_policy: GqlTelemetryDataCollectionPolicy,
    ) -> TelemetryDataCollectionPolicy {
        Self {
            default: gql_telemetry_data_collection_policy.default,
            toggleable: gql_telemetry_data_collection_policy.toggleable,
        }
    }
}

impl From<GqlUsageBasedPricingPolicy> for UsageBasedPricingPolicy {
    fn from(gql_usage_based_pricing_policy: GqlUsageBasedPricingPolicy) -> UsageBasedPricingPolicy {
        Self {
            toggleable: gql_usage_based_pricing_policy.toggleable,
        }
    }
}

impl From<GqlAddonCreditsSettings> for AddonCreditsSettings {
    fn from(gql_settings: GqlAddonCreditsSettings) -> AddonCreditsSettings {
        Self {
            auto_reload_enabled: gql_settings.auto_reload_enabled,
            max_monthly_spend_cents: gql_settings.max_monthly_spend_cents,
            selected_auto_reload_credit_denomination: gql_settings
                .selected_auto_reload_credit_denomination,
        }
    }
}

impl From<GqlCodebaseContextPolicy> for CodebaseContextPolicy {
    fn from(gql_codebase_context_policy: GqlCodebaseContextPolicy) -> CodebaseContextPolicy {
        Self {
            toggleable: gql_codebase_context_policy.toggleable,
            index_limit: if gql_codebase_context_policy.is_unlimited_indices {
                None
            } else {
                Some(gql_codebase_context_policy.max_indices as u32)
            },
            max_files_per_repo: gql_codebase_context_policy.max_files_per_repo as u32,
        }
    }
}

impl From<GqlByoApiKeyPolicy> for ByoApiKeyPolicy {
    fn from(gql_byo_api_key_policy: GqlByoApiKeyPolicy) -> ByoApiKeyPolicy {
        Self {
            enabled: gql_byo_api_key_policy.enabled,
        }
    }
}

impl From<GqlPurchaseAddOnCreditsPolicy> for PurchaseAddOnCreditsPolicy {
    fn from(
        gql_purchase_add_on_credits_policy: GqlPurchaseAddOnCreditsPolicy,
    ) -> PurchaseAddOnCreditsPolicy {
        Self {
            enabled: gql_purchase_add_on_credits_policy.enabled,
        }
    }
}

impl From<GqlEnterprisePayAsYouGoPolicy> for EnterprisePayAsYouGoPolicy {
    fn from(gql_policy: GqlEnterprisePayAsYouGoPolicy) -> EnterprisePayAsYouGoPolicy {
        Self {
            enabled: gql_policy.enabled,
        }
    }
}

impl From<GqlEnterpriseCreditsAutoReloadPolicy> for EnterpriseCreditsAutoReloadPolicy {
    fn from(gql_policy: GqlEnterpriseCreditsAutoReloadPolicy) -> EnterpriseCreditsAutoReloadPolicy {
        Self {
            enabled: gql_policy.enabled,
        }
    }
}

impl From<GqlMultiAdminPolicy> for MultiAdminPolicy {
    fn from(gql_policy: GqlMultiAdminPolicy) -> MultiAdminPolicy {
        Self {
            enabled: gql_policy.enabled,
        }
    }
}

impl From<GqlInstanceShape> for InstanceShape {
    fn from(gql_instance_shape: GqlInstanceShape) -> InstanceShape {
        Self {
            vcpus: gql_instance_shape.vcpus,
            memory_gb: gql_instance_shape.memory_gb,
        }
    }
}

impl From<GqlAmbientAgentsPolicy> for AmbientAgentsPolicy {
    fn from(gql_policy: GqlAmbientAgentsPolicy) -> AmbientAgentsPolicy {
        Self {
            max_concurrent_agents: gql_policy.max_concurrent_agents,
            instance_shape: gql_policy.instance_shape.map(From::from),
        }
    }
}

impl From<GqlUsageVisibilityGranularity> for UsageVisibilityGranularity {
    fn from(gql_granularity: GqlUsageVisibilityGranularity) -> UsageVisibilityGranularity {
        match gql_granularity {
            GqlUsageVisibilityGranularity::OwnOnly => UsageVisibilityGranularity::OwnOnly,
            GqlUsageVisibilityGranularity::TeamAggregate => {
                UsageVisibilityGranularity::TeamAggregate
            }
            GqlUsageVisibilityGranularity::PerUserTotals => {
                UsageVisibilityGranularity::PerUserTotals
            }
            GqlUsageVisibilityGranularity::FullBreakdown => {
                UsageVisibilityGranularity::FullBreakdown
            }
            GqlUsageVisibilityGranularity::Other(value) => {
                report_error!(anyhow!(
                    "Invalid UsageVisibilityGranularity '{value}'. Make sure to update client GraphQL types!"
                ));
                // Fail closed to the most restrictive granularity.
                UsageVisibilityGranularity::OwnOnly
            }
        }
    }
}

fn from_gql_max_prior_cycles(value: i32) -> MaxPriorCycles {
    match value {
        0 => MaxPriorCycles::None,
        n if n > 0 => MaxPriorCycles::Limited(n as u32),
        -1 => MaxPriorCycles::Unlimited,
        other => {
            report_error!(anyhow!(
                "Unexpected maxPriorCycles value '{other}' from server; treating as unlimited"
            ));
            MaxPriorCycles::None
        }
    }
}

impl From<GqlUsageVisibilityPolicy> for UsageVisibilityPolicy {
    fn from(gql_policy: GqlUsageVisibilityPolicy) -> UsageVisibilityPolicy {
        Self {
            admin_granularity: gql_policy.admin_granularity.into(),
            max_prior_cycles: from_gql_max_prior_cycles(gql_policy.max_prior_cycles),
        }
    }
}

fn convert_billing_cycle_usage(history: GqlBillingCycleUsageHistory) -> BillingCycleUsageData {
    BillingCycleUsageData {
        current_period_start: history.current_period_start.utc(),
        current_period_end: history.current_period_end.utc(),
        summaries: history
            .summaries
            .into_iter()
            .map(|summary| BillingCycleUsageSummary {
                period_start: summary.period_start.utc(),
                period_end: summary.period_end.utc(),
                entries: summary
                    .entries
                    .into_iter()
                    .map(|entry| BillingCycleUsageEntry {
                        subject_type: entry.subject_type,
                        subject_uid: entry.subject_uid,
                        subject_display_name: entry.subject_display_name,
                        cost_type: entry.cost_type,
                        usage_bucket: entry.usage_bucket,
                        usage_source: entry.usage_source,
                        credits_used: entry.credits_used,
                        cost_cents: entry.cost_cents,
                    })
                    .collect(),
            })
            .collect(),
    }
}

impl From<GqlTier> for Tier {
    fn from(gql_tier: GqlTier) -> Tier {
        Self {
            name: gql_tier.name,
            description: gql_tier.description,
            warp_ai_policy: gql_tier.warp_ai_policy.map(From::from),
            workspace_size_policy: gql_tier.team_size_policy.map(From::from),
            shared_notebooks_policy: gql_tier.shared_notebooks_policy.map(From::from),
            shared_workflows_policy: gql_tier.shared_workflows_policy.map(From::from),
            session_sharing_policy: gql_tier.session_sharing_policy.map(From::from),
            ai_autonomy_policy: gql_tier.ai_autonomy_policy.map(From::from),
            telemetry_data_collection_policy: gql_tier
                .telemetry_data_collection_policy
                .map(From::from),
            ugc_data_collection_policy: gql_tier.ugc_data_collection_policy.map(From::from),
            usage_based_pricing_policy: gql_tier.usage_based_pricing_policy.map(From::from),
            codebase_context_policy: gql_tier.codebase_context_policy.map(From::from),
            byo_api_key_policy: gql_tier.byo_api_key_policy.map(From::from),
            purchase_add_on_credits_policy: gql_tier.purchase_add_on_credits_policy.map(From::from),
            enterprise_pay_as_you_go_policy: gql_tier
                .enterprise_pay_as_you_go_policy
                .map(From::from),
            enterprise_credits_auto_reload_policy: gql_tier
                .enterprise_credits_auto_reload_policy
                .map(From::from),
            multi_admin_policy: gql_tier.multi_admin_policy.map(From::from),
            ambient_agents_policy: gql_tier.ambient_agents_policy.map(From::from),
            usage_visibility_policy: gql_tier.usage_visibility_policy.map(From::from),
        }
    }
}

impl From<GqlCustomerType> for CustomerType {
    fn from(gql_customer_type: GqlCustomerType) -> CustomerType {
        match gql_customer_type {
            GqlCustomerType::Free => CustomerType::Free,
            GqlCustomerType::Turbo => CustomerType::Turbo,
            GqlCustomerType::SelfServe => CustomerType::SelfServe,
            GqlCustomerType::Prosumer => CustomerType::Prosumer,
            GqlCustomerType::Legacy => CustomerType::Legacy,
            GqlCustomerType::Enterprise => CustomerType::Enterprise,
            GqlCustomerType::Business => CustomerType::Business,
            GqlCustomerType::Lightspeed => CustomerType::Lightspeed,
            GqlCustomerType::Build => CustomerType::Build,
            GqlCustomerType::BuildMax => CustomerType::BuildMax,
            GqlCustomerType::ProTrial | GqlCustomerType::TeamTrial | GqlCustomerType::Other(_) => {
                CustomerType::Unknown
            }
        }
    }
}

impl From<GqlDelinquencyStatus> for DelinquencyStatus {
    fn from(gql_delinquency_status: GqlDelinquencyStatus) -> DelinquencyStatus {
        match gql_delinquency_status {
            GqlDelinquencyStatus::NoDelinquency => DelinquencyStatus::NoDelinquency,
            GqlDelinquencyStatus::PastDue => DelinquencyStatus::PastDue,
            GqlDelinquencyStatus::Unpaid => DelinquencyStatus::Unpaid,
            GqlDelinquencyStatus::TeamLimitExceeded => DelinquencyStatus::TeamLimitExceeded,
            GqlDelinquencyStatus::Other(_) => DelinquencyStatus::Unknown,
        }
    }
}

impl BonusGrant {
    pub fn from_gql_bonus_grant(bonus_grant: GqlBonusGrant, scope: BonusGrantScope) -> Self {
        Self {
            created_at: bonus_grant.created_at.utc(),
            cost_cents: bonus_grant.cost_cents,
            expiration: bonus_grant.expiration.map(|exp| exp.utc()),
            grant_type: bonus_grant.grant_type,
            reason: bonus_grant.reason,
            user_facing_message: bonus_grant.user_facing_message,
            request_credits_granted: bonus_grant.request_credits_granted,
            request_credits_remaining: bonus_grant.request_credits_remaining,
            scope,
        }
    }
}

impl From<GqlBillingMetadata> for BillingMetadata {
    fn from(gql_billing_metadata: GqlBillingMetadata) -> BillingMetadata {
        Self {
            tier: gql_billing_metadata.tier.into(),
            customer_type: gql_billing_metadata.customer_type.into(),
            delinquency_status: gql_billing_metadata.delinquency_status.into(),
            service_agreements: gql_billing_metadata.service_agreements,
            ai_overages: gql_billing_metadata.ai_overages.map(|overages| AiOverages {
                current_monthly_request_cost_cents: overages.current_monthly_request_cost_cents,
                current_monthly_requests_used: overages.current_monthly_requests_used,
                current_period_end: overages.current_period_end.utc(),
            }),
        }
    }
}

impl TryFrom<&BillingMetadata> for StripeSubscriptionPlan {
    type Error = ();

    fn try_from(billing_metadata: &BillingMetadata) -> Result<Self, Self::Error> {
        match billing_metadata.customer_type {
            CustomerType::Turbo => Ok(StripeSubscriptionPlan::Turbo),
            CustomerType::SelfServe => Ok(StripeSubscriptionPlan::Team),
            CustomerType::Prosumer => Ok(StripeSubscriptionPlan::Pro),
            CustomerType::Business => {
                // Check if this is a legacy Business Plan, or a new Build Business plan based on service agreement type
                // See: https://github.com/warpdotdev/warp-server/pull/6828#discussion_r2496242091
                match billing_metadata
                    .service_agreements
                    .first()
                    .map(|sa| sa.type_.clone())
                {
                    Some(ServiceAgreementType::SelfServe) => {
                        Ok(StripeSubscriptionPlan::BuildBusiness)
                    }
                    _ => Ok(StripeSubscriptionPlan::Business),
                }
            }
            CustomerType::Lightspeed => Ok(StripeSubscriptionPlan::Lightspeed),
            CustomerType::Build => Ok(StripeSubscriptionPlan::Build),
            CustomerType::BuildMax => Ok(StripeSubscriptionPlan::BuildMax),
            // legacy customer types we don't support anymore, or customer types that don't get billed via stripe
            CustomerType::Free
            | CustomerType::Legacy
            | CustomerType::Enterprise
            | CustomerType::Unknown => Err(()),
        }
    }
}

fn convert_gql_ai_autonomy_value_to_action_permission(
    gql_ai_autonomy_value: GqlAiAutonomyValue,
) -> Option<ActionPermission> {
    match gql_ai_autonomy_value {
        GqlAiAutonomyValue::AgentDecides => Some(ActionPermission::AgentDecides),
        GqlAiAutonomyValue::AlwaysAllow => Some(ActionPermission::AlwaysAllow),
        GqlAiAutonomyValue::AlwaysAsk => Some(ActionPermission::AlwaysAsk),
        GqlAiAutonomyValue::RespectUserSetting => None,
        GqlAiAutonomyValue::Other(value) => {
            report_error!(anyhow!(
                "Invalid AiAutonomyValue '{value}'. Make sure to update client GraphQL types!"
            ));
            None
        }
    }
}

fn convert_gql_write_to_pty_autonomy_value_to_write_to_pty_permission(
    gql_write_to_pty_autonomy_value: GqlWriteToPtyAutonomyValue,
) -> Option<WriteToPtyPermission> {
    match gql_write_to_pty_autonomy_value {
        GqlWriteToPtyAutonomyValue::AlwaysAllow => Some(WriteToPtyPermission::AlwaysAllow),
        GqlWriteToPtyAutonomyValue::AlwaysAsk => Some(WriteToPtyPermission::AlwaysAsk),
        GqlWriteToPtyAutonomyValue::AskOnFirstWrite => Some(WriteToPtyPermission::AskOnFirstWrite),
        GqlWriteToPtyAutonomyValue::RespectUserSetting => None,
        GqlWriteToPtyAutonomyValue::Other(value) => {
            report_error!(anyhow!(
                "Invalid WriteToPtyAutonomyValue '{value}'. Make sure to update client GraphQL types!"
            ));
            None
        }
    }
}

fn convert_gql_computer_use_autonomy_value_to_computer_use_permission(
    gql_computer_use_autonomy_value: GqlComputerUseAutonomyValue,
) -> Option<ComputerUsePermission> {
    match gql_computer_use_autonomy_value {
        GqlComputerUseAutonomyValue::Never => Some(ComputerUsePermission::Never),
        GqlComputerUseAutonomyValue::AlwaysAsk => Some(ComputerUsePermission::AlwaysAsk),
        GqlComputerUseAutonomyValue::AlwaysAllow => Some(ComputerUsePermission::AlwaysAllow),
        GqlComputerUseAutonomyValue::RespectUserSetting => None,
        GqlComputerUseAutonomyValue::Other(value) => {
            report_error!(anyhow!(
                "Invalid ComputerUseAutonomyValue '{value}'. Make sure to update client GraphQL types!"
            ));
            None
        }
    }
}

trait ToAgentModeCommandExecutionPredicates {
    fn to_predicates(self) -> Vec<AgentModeCommandExecutionPredicate>;
}

impl ToAgentModeCommandExecutionPredicates for Vec<String> {
    fn to_predicates(self) -> Vec<AgentModeCommandExecutionPredicate> {
        self.into_iter()
            .filter_map(|pattern| {
                match AgentModeCommandExecutionPredicate::new_regex(&pattern) {
                    Ok(predicate) => Some(predicate),
                    Err(e) => {
                        report_error!(anyhow!(e).context(
                            "Couldn't parse GQL-provided command regex into AgentModeCommandExecutionPredicate"
                        ));
                        None
                    }
                }
            })
            .collect()
    }
}

trait ToPathBufs {
    fn to_path_bufs(self) -> Vec<PathBuf>;
}

impl ToPathBufs for Vec<String> {
    fn to_path_bufs(self) -> Vec<PathBuf> {
        self.into_iter().map(PathBuf::from).collect()
    }
}
impl From<warp_graphql::workspace::LlmModelHost> for crate::ai::llms::LLMModelHost {
    fn from(gql_host: warp_graphql::workspace::LlmModelHost) -> Self {
        use warp_graphql::workspace::LlmModelHost as GqlLlmModelHost;
        match gql_host {
            GqlLlmModelHost::DirectApi => Self::DirectApi,
            GqlLlmModelHost::AwsBedrock => Self::AwsBedrock,
            GqlLlmModelHost::CustomEndpoint => Self::CustomEndpoint,
            GqlLlmModelHost::Other(value) => {
                report_error!(anyhow!(
                    "Unknown LlmModelHost '{value}'. Make sure to update client GraphQL types!"
                ));
                Self::Unknown
            }
        }
    }
}

impl From<warp_graphql::workspace::LlmHostSettings> for super::workspace::LlmHostSettings {
    fn from(gql_settings: warp_graphql::workspace::LlmHostSettings) -> Self {
        Self {
            enabled: gql_settings.enabled,
            enablement_setting: gql_settings
                .enablement_setting
                .map(Into::into)
                .unwrap_or_default(),
        }
    }
}

impl From<warp_graphql::workspace::LlmSettings> for LlmSettings {
    fn from(gql_settings: warp_graphql::workspace::LlmSettings) -> Self {
        let mut host_configs = std::collections::HashMap::new();
        for entry in gql_settings.host_configs {
            let host: crate::ai::llms::LLMModelHost = entry.host.into();
            if host_configs
                .insert(host.clone(), entry.settings.into())
                .is_some()
            {
                log::warn!(
                    "Duplicate LLMModelHost entry for {:?}, using latest value",
                    host
                );
            }
        }
        Self {
            enabled: gql_settings.enabled,
            host_configs,
        }
    }
}

impl From<GqlWorkspaceSettings> for WorkspaceSettings {
    fn from(gql_workspace_settings: GqlWorkspaceSettings) -> WorkspaceSettings {
        Self {
            llm_settings: gql_workspace_settings.llm_settings.into(),
            telemetry_settings: TelemetrySettings {
                force_enabled: gql_workspace_settings.telemetry_settings.force_enabled,
            },
            ugc_collection_settings: UgcCollectionSettings {
                setting: UgcCollectionEnablementSetting::from(
                    gql_workspace_settings.ugc_collection_settings.setting,
                ),
            },
            cloud_conversation_storage_settings: CloudConversationStorageSettings {
                setting: gql_workspace_settings
                    .cloud_conversation_storage_settings
                    .setting
                    .into(),
            },
            ai_permissions_settings: AiPermissionsSettings {
                allow_ai_in_remote_sessions: gql_workspace_settings
                    .ai_permissions_settings
                    .allow_ai_in_remote_sessions,
                remote_session_regex_list: gql_workspace_settings
                    .ai_permissions_settings
                    .remote_session_regex_list
                    .iter()
                    .filter_map(|r| {
                        let regex = Regex::new(r);
                        match regex {
                            Ok(regex) => Some(regex),
                            Err(_) => {
                                log::error!(
                                    "Invalid regex pattern for remote session detection: {r}"
                                );
                                None
                            }
                        }
                    })
                    .collect(),
            },
            link_sharing_settings: LinkSharingSettings {
                anyone_with_link_sharing_enabled: gql_workspace_settings
                    .link_sharing_settings
                    .anyone_with_link_sharing_enabled,
                direct_link_sharing_enabled: gql_workspace_settings
                    .link_sharing_settings
                    .direct_link_sharing_enabled,
            },
            secret_redaction_settings: SecretRedactionSettings {
                enabled: gql_workspace_settings.secret_redaction_settings.enabled,
                regexes: gql_workspace_settings
                    .secret_redaction_settings
                    .regexes
                    .into_iter()
                    .map(|gql_regex| EnterpriseSecretRegex {
                        pattern: gql_regex.pattern,
                        name: gql_regex.name,
                    })
                    .collect(),
            },
            is_invite_link_enabled: gql_workspace_settings.is_invite_link_enabled,
            is_discoverable: gql_workspace_settings.is_discoverable,
            ai_autonomy_settings: AiAutonomySettings {
                apply_code_diffs_setting: gql_workspace_settings
                    .ai_autonomy_settings
                    .apply_code_diffs_setting
                    .and_then(convert_gql_ai_autonomy_value_to_action_permission),
                read_files_setting: gql_workspace_settings
                    .ai_autonomy_settings
                    .read_files_setting
                    .and_then(convert_gql_ai_autonomy_value_to_action_permission),
                read_files_allowlist: gql_workspace_settings
                    .ai_autonomy_settings
                    .read_files_allowlist
                    .map(|allowlist| allowlist.to_path_bufs()),
                execute_commands_setting: gql_workspace_settings
                    .ai_autonomy_settings
                    .execute_commands_setting
                    .and_then(convert_gql_ai_autonomy_value_to_action_permission),
                execute_commands_allowlist: gql_workspace_settings
                    .ai_autonomy_settings
                    .execute_commands_allowlist
                    .map(|allowlist| allowlist.to_predicates()),
                execute_commands_denylist: gql_workspace_settings
                    .ai_autonomy_settings
                    .execute_commands_denylist
                    .map(|denylist| denylist.to_predicates()),
                write_to_pty_setting: gql_workspace_settings
                    .ai_autonomy_settings
                    .write_to_pty_setting
                    .and_then(convert_gql_write_to_pty_autonomy_value_to_write_to_pty_permission),
                computer_use_setting: gql_workspace_settings
                    .ai_autonomy_settings
                    .computer_use_setting
                    .and_then(convert_gql_computer_use_autonomy_value_to_computer_use_permission),
            },
            usage_based_pricing_settings: UsageBasedPricingSettings {
                enabled: gql_workspace_settings.usage_based_pricing_settings.enabled,
                max_monthly_spend_cents: gql_workspace_settings
                    .usage_based_pricing_settings
                    .max_monthly_spend_cents
                    .and_then(|cents| {
                        if cents < 0 {
                            report_error!(anyhow!(
                                "Usage-based pricing has a negative max monthly spend of {} cents",
                                cents
                            ));
                            None
                        } else {
                            Some(cents as u32)
                        }
                    }),
            },
            addon_credits_settings: gql_workspace_settings.addon_credits_settings.into(),
            codebase_context_settings: CodebaseContextSettings {
                setting: gql_workspace_settings
                    .codebase_context_settings
                    .setting
                    .into(),
            },
            sandboxed_agent_settings: gql_workspace_settings.sandboxed_agent_settings.map(|s| {
                SandboxedAgentSettings {
                    execute_commands_denylist: s
                        .execute_commands_denylist
                        .map(|denylist| denylist.to_predicates()),
                }
            }),
            enable_warp_attribution: gql_workspace_settings
                .ambient_agent_settings
                .as_ref()
                .map(|s| s.enable_warp_attribution.clone().into())
                .unwrap_or_default(),
            default_host_slug: gql_workspace_settings
                .ambient_agent_settings
                .as_ref()
                .and_then(|s| s.default_host_slug.clone()),
        }
    }
}

impl Team {
    pub fn from_gql(gql_workspace: GqlWorkspace, gql_team: GqlTeam) -> Team {
        Self {
            // TEAM FIELDS
            // These fields will persist in the Team rust type even after we finish
            // rolling out workspaces.
            uid: ServerId::from_string_lossy(gql_team.uid.inner()),
            name: gql_team.name.clone(),
            members: gql_team
                .members
                .clone()
                .into_iter()
                .map(|gql_member| gql_member.into())
                .collect(),

            // WORKSPACE FIELDS
            // TODO(skambashi): The fields below are derived from the workspace. We should
            // remove these from the Team rust type and use the values in the parent
            // Workspace instead.
            invite_code: gql_workspace
                .invite_code
                .clone()
                .map(|code| WorkspaceInviteCode { code: code.clone() }),
            pending_email_invites: gql_workspace
                .pending_email_invites
                .clone()
                .into_iter()
                .map(|gql_email_invite| gql_email_invite.into())
                .collect(),
            invite_link_domain_restrictions: gql_workspace
                .invite_link_domain_restrictions
                .clone()
                .into_iter()
                .map(|gql_domain_restriction| gql_domain_restriction.into())
                .collect(),
            billing_metadata: gql_workspace.billing_metadata.clone().into(),
            stripe_customer_id: gql_workspace
                .stripe_customer_id
                .as_ref()
                .map(|id| id.clone().into_inner()),
            organization_settings: gql_workspace.settings.clone().into(),
            is_eligible_for_discovery: gql_workspace.is_eligible_for_discovery,
            has_billing_history: gql_workspace.has_billing_history,
        }
    }
}

impl From<GqlWorkspace> for Workspace {
    fn from(gql_workspace: GqlWorkspace) -> Workspace {
        Self {
            uid: ServerId::from_string_lossy(gql_workspace.uid.inner()).into(),
            name: gql_workspace.name.clone(),
            stripe_customer_id: gql_workspace
                .stripe_customer_id
                .as_ref()
                .map(|id| id.clone().into_inner()),
            teams: gql_workspace
                .teams
                .clone()
                .into_iter()
                .map(|gql_team| Team::from_gql(gql_workspace.clone(), gql_team))
                .collect(),
            billing_metadata: gql_workspace.billing_metadata.clone().into(),
            bonus_grants_purchased_this_month: gql_workspace
                .bonus_grants_info
                .spending_info
                .map(|info| BonusGrantsPurchased {
                    total_credits_purchased: info.current_month_credits_purchased,
                    cents_spent: info.current_month_spend_cents,
                })
                .unwrap_or_default(),
            billing_cycle_usage: gql_workspace
                .billing_cycle_usage_history
                .map(convert_billing_cycle_usage),
            has_billing_history: gql_workspace.has_billing_history,
            settings: gql_workspace.settings.clone().into(),
            invite_code: gql_workspace
                .invite_code
                .clone()
                .map(|code| WorkspaceInviteCode { code: code.clone() }),
            invite_link_domain_restrictions: gql_workspace
                .invite_link_domain_restrictions
                .clone()
                .into_iter()
                .map(|gql_domain_restriction| gql_domain_restriction.into())
                .collect(),
            pending_email_invites: gql_workspace
                .pending_email_invites
                .clone()
                .into_iter()
                .map(|gql_email_invite| gql_email_invite.into())
                .collect(),
            is_eligible_for_discovery: gql_workspace.is_eligible_for_discovery,
            members: gql_workspace
                .members
                .clone()
                .into_iter()
                .map(|gql_member| gql_member.into())
                .collect(),
            total_requests_used_since_last_refresh: gql_workspace
                .total_requests_used_since_last_refresh,
        }
    }
}

impl From<GqlUser> for WorkspacesMetadataResponse {
    fn from(gql_user: GqlUser) -> WorkspacesMetadataResponse {
        let feature_model_choices = gql_user
            .workspaces
            .first()
            .map(|gql_workspace| gql_workspace.feature_model_choice.clone());

        let workspaces: Vec<Workspace> = gql_user
            .workspaces
            .clone()
            .into_iter()
            .filter(|gql_workspace| {
                // TODO(skambashi): REV-717: Clean up this code once every user always has
                // a workspace, and the server no longer returns a placeholder workspace.
                gql_workspace.uid != PLACEHOLDER_WORKSPACE_UID.into()
            })
            .map(|gql_workspace| gql_workspace.into())
            .collect();

        let joinable_teams = gql_user
            .discoverable_teams
            .clone()
            .into_iter()
            .map(|gql_joinable_team| gql_joinable_team.into())
            .collect();

        let experiments = gql_user
            .experiments
            .and_then(|experiments| convert_to_server_experiment!(experiments));

        // TODO(skambashi) refactor to return back workspaces, and not teams
        WorkspacesMetadataResponse {
            workspaces,
            joinable_teams,
            experiments,
            feature_model_choices,
        }
    }
}

impl From<PublicUserProfile> for UserProfileWithUID {
    fn from(value: PublicUserProfile) -> Self {
        UserProfileWithUID {
            firebase_uid: UserUid::new(&value.uid),
            display_name: value.display_name,
            email: value.email.unwrap_or_default(),
            photo_url: value.photo_url.unwrap_or_default(),
        }
    }
}

impl TryFrom<WarpDriveUpdate> for ObjectUpdateMessage {
    type Error = anyhow::Error;

    fn try_from(value: WarpDriveUpdate) -> Result<Self, Self::Error> {
        match value {
            WarpDriveUpdate::ObjectActionOccurred(message) => {
                Ok(ObjectUpdateMessage::ObjectActionOccurred {
                    history: message.history.try_into()?,
                })
            }
            WarpDriveUpdate::ObjectContentUpdated(message) => {
                let server_object = message.object.try_into()?;
                let last_editor = message.last_editor.map(|e| e.into());
                Ok(ObjectUpdateMessage::ObjectContentChanged {
                    server_object: Box::new(server_object),
                    last_editor,
                })
            }
            WarpDriveUpdate::ObjectDeleted(message) => Ok(ObjectUpdateMessage::ObjectDeleted {
                object_uid: ServerId::from_string_lossy(message.object_uid.inner()),
            }),
            WarpDriveUpdate::ObjectMetadataUpdated(message) => {
                Ok(ObjectUpdateMessage::ObjectMetadataChanged {
                    metadata: message.metadata.try_into()?,
                })
            }
            WarpDriveUpdate::ObjectPermissionsUpdated(message) => {
                Ok(ObjectUpdateMessage::ObjectPermissionsChangedV2 {
                    object_uid: ServerId::from_string_lossy(message.object_uid.inner()),
                    user_profiles: message
                        .user_profiles
                        .into_iter()
                        .flatten()
                        .map(Into::into)
                        .collect(),
                    permissions: message.permissions.try_into()?,
                })
            }
            WarpDriveUpdate::TeamMembershipsChanged(_) => {
                Ok(ObjectUpdateMessage::TeamMembershipsChanged)
            }
            WarpDriveUpdate::AmbientTaskUpdated(message) => {
                Ok(ObjectUpdateMessage::AmbientTaskUpdated {
                    task_id: message.task_id.inner().to_string(),
                    timestamp: message.task_updated_ts.utc(),
                })
            }
            WarpDriveUpdate::Unknown => bail!("Unexpected WarpDriveUpdate variant"),
        }
    }
}

impl TryFrom<warp_graphql::object::CloudObject> for ServerCloudObject {
    type Error = anyhow::Error;

    fn try_from(value: warp_graphql::object::CloudObject) -> Result<Self, Self::Error> {
        match value {
            warp_graphql::object::CloudObject::AIConversation(_) => Err(anyhow::anyhow!(
                "AIConversation is not a supported object type for this operation"
            )),
            warp_graphql::object::CloudObject::Folder(folder) => Ok(ServerCloudObject::Folder(
                ServerFolder::try_from_gql(folder)?,
            )),
            warp_graphql::object::CloudObject::GenericStringObject(gso) => {
                match gso.format {
                    warp_graphql::generic_string_object::GenericStringObjectFormat::JsonEnvVarCollection => {
                        Ok(ServerCloudObject::EnvVarCollection(ServerEnvVarCollection::try_from_gql(gso)?))
                    }
                    warp_graphql::generic_string_object::GenericStringObjectFormat::JsonPreference => {
                        Ok(ServerCloudObject::Preference(ServerPreference::try_from_gql(gso)?))
                    }
                    warp_graphql::generic_string_object::GenericStringObjectFormat::JsonWorkflowEnum => {
                        Ok(ServerCloudObject::WorkflowEnum(ServerWorkflowEnum::try_from_gql(gso)?))
                    }
                    warp_graphql::generic_string_object::GenericStringObjectFormat::JsonAIFact => {
                        Ok(ServerCloudObject::AIFact(ServerAIFact::try_from_gql(gso)?))
                    }
                    warp_graphql::generic_string_object::GenericStringObjectFormat::JsonMCPServer => {
                        Ok(ServerCloudObject::MCPServer(ServerMCPServer::try_from_gql(gso)?))
                    }
                    warp_graphql::generic_string_object::GenericStringObjectFormat::JsonAIExecutionProfile => {
                        Ok(ServerCloudObject::AIExecutionProfile(ServerAIExecutionProfile::try_from_gql(gso)?))
                    }
                    warp_graphql::generic_string_object::GenericStringObjectFormat::JsonTemplatableMCPServer => {
                        Ok(ServerCloudObject::TemplatableMCPServer(ServerTemplatableMCPServer::try_from_gql(gso)?))
                    }
                    warp_graphql::generic_string_object::GenericStringObjectFormat::JsonCloudEnvironment => {
                        Ok(ServerCloudObject::AmbientAgentEnvironment(ServerAmbientAgentEnvironment::try_from_gql(gso)?))
                    }
                    warp_graphql::generic_string_object::GenericStringObjectFormat::JsonScheduledAmbientAgent => {
                        Ok(ServerCloudObject::ScheduledAmbientAgent(ServerScheduledAmbientAgent::try_from_gql(gso)?))
                    }
                }
            }
            warp_graphql::object::CloudObject::Notebook(notebook) => Ok(
                ServerCloudObject::Notebook(ServerNotebook::try_from_gql(notebook)?),
            ),
            warp_graphql::object::CloudObject::Workflow(workflow) => Ok(
                ServerCloudObject::Workflow(Box::new(ServerWorkflow::try_from_gql(workflow)?)),
            ),
            warp_graphql::object::CloudObject::Unknown => {
                Err(anyhow::anyhow!("Unable to convert cloud object type"))
            }
        }
    }
}

impl TryFrom<CloudObjectWithDescendants> for ServerCloudObject {
    type Error = anyhow::Error;

    fn try_from(value: CloudObjectWithDescendants) -> Result<Self, Self::Error> {
        match value {
            CloudObjectWithDescendants::AIConversation(_) => {
                Err(anyhow::anyhow!("AIConversation is not a supported object type for this operation"))
            }
            CloudObjectWithDescendants::FolderWithDescendants(fwd) => {
                Ok(ServerCloudObject::Folder(ServerFolder::try_from_gql(fwd.folder)?))
            }
            CloudObjectWithDescendants::GenericStringObject(gso) => match gso.format {
                warp_graphql::generic_string_object::GenericStringObjectFormat::JsonEnvVarCollection => {
                    Ok(ServerCloudObject::EnvVarCollection(ServerEnvVarCollection::try_from_gql(gso)?))
                }
                warp_graphql::generic_string_object::GenericStringObjectFormat::JsonPreference => {
                    Ok(ServerCloudObject::Preference(ServerPreference::try_from_gql(gso)?))
                }
                warp_graphql::generic_string_object::GenericStringObjectFormat::JsonWorkflowEnum => {
                    Ok(ServerCloudObject::WorkflowEnum(ServerWorkflowEnum::try_from_gql(gso)?))
                }
                warp_graphql::generic_string_object::GenericStringObjectFormat::JsonAIFact => {
                    Ok(ServerCloudObject::AIFact(ServerAIFact::try_from_gql(gso)?))
                }
                warp_graphql::generic_string_object::GenericStringObjectFormat::JsonMCPServer => {
                    Ok(ServerCloudObject::MCPServer(ServerMCPServer::try_from_gql(gso)?))
                }
                warp_graphql::generic_string_object::GenericStringObjectFormat::JsonAIExecutionProfile => {
                    Ok(ServerCloudObject::AIExecutionProfile(ServerAIExecutionProfile::try_from_gql(gso)?))
                }
                warp_graphql::generic_string_object::GenericStringObjectFormat::JsonTemplatableMCPServer => {
                    Ok(ServerCloudObject::TemplatableMCPServer(ServerTemplatableMCPServer::try_from_gql(gso)?))
                }
                warp_graphql::generic_string_object::GenericStringObjectFormat::JsonCloudEnvironment => {
                    Ok(ServerCloudObject::AmbientAgentEnvironment(ServerAmbientAgentEnvironment::try_from_gql(gso)?))
                }
                warp_graphql::generic_string_object::GenericStringObjectFormat::JsonScheduledAmbientAgent => {
                    Ok(ServerCloudObject::ScheduledAmbientAgent(ServerScheduledAmbientAgent::try_from_gql(gso)?))
                }
            }
            CloudObjectWithDescendants::Notebook(notebook) => Ok(ServerCloudObject::Notebook(ServerNotebook::try_from_gql(notebook)?)),
            CloudObjectWithDescendants::Workflow(workflow) => Ok(ServerCloudObject::Workflow(Box::new(ServerWorkflow::try_from_gql(workflow)?))),
            CloudObjectWithDescendants::Unknown => Err(anyhow::anyhow!("Unable to convert cloud object with descendants type")),
        }
    }
}

impl From<GqlDiscoverableTeamData> for DiscoverableTeam {
    fn from(gql_discoverable_team: GqlDiscoverableTeamData) -> DiscoverableTeam {
        Self {
            team_uid: gql_discoverable_team.team_uid.into_inner(),
            num_members: i64::from(gql_discoverable_team.num_members),
            name: gql_discoverable_team.name,
            team_accepting_invites: gql_discoverable_team.team_accepting_invites,
        }
    }
}
