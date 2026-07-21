use super::*;
use crate::server::ids::ServerId;

// `ServerId::from_string_lossy` requires exactly 22 characters.
const TEST_WORKSPACE_UID: &str = "workspace_uid123456789";

fn make_billing_metadata(
    customer_type: CustomerType,
    disable_premium_models: Option<bool>,
) -> BillingMetadata {
    BillingMetadata {
        customer_type,
        tier: Tier {
            warp_ai_policy: disable_premium_models.map(|disable_premium_models| WarpAiPolicy {
                limit: 0,
                disable_premium_models,
                is_code_suggestions_toggleable: false,
                is_prompt_suggestions_toggleable: false,
                is_next_command_enabled: false,
                is_git_operations_ai_enabled: false,
                is_voice_enabled: false,
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn make_workspace(policy: Option<UsageVisibilityPolicy>) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        ServerId::from_string_lossy(TEST_WORKSPACE_UID).into(),
        "Test Workspace".to_string(),
        None,
    );
    workspace.billing_metadata.tier.usage_visibility_policy = policy;
    workspace
}

fn policy(
    granularity: UsageVisibilityGranularity,
    max_prior_cycles: MaxPriorCycles,
) -> UsageVisibilityPolicy {
    UsageVisibilityPolicy {
        admin_granularity: granularity,
        max_prior_cycles,
    }
}

#[test]
fn missing_policy_returns_defaults_for_admin_and_non_admin() {
    let workspace = make_workspace(None);

    let as_admin = workspace.resolve_usage_visibility(true);
    assert_eq!(as_admin.granularity, UsageVisibilityGranularity::OwnOnly);
    assert_eq!(as_admin.max_prior_cycles, MaxPriorCycles::None);

    let as_non_admin = workspace.resolve_usage_visibility(false);
    assert_eq!(
        as_non_admin.granularity,
        UsageVisibilityGranularity::OwnOnly
    );
    assert_eq!(as_non_admin.max_prior_cycles, MaxPriorCycles::None);
}

#[test]
fn non_admin_collapses_granularity_but_keeps_max_prior_cycles() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::FullBreakdown,
        MaxPriorCycles::Limited(11),
    )));

    let resolved = workspace.resolve_usage_visibility(false);

    assert_eq!(resolved.granularity, UsageVisibilityGranularity::OwnOnly);
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Limited(11));
}

#[test]
fn admin_inherits_tier_team_aggregate_granularity() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::TeamAggregate,
        MaxPriorCycles::Limited(11),
    )));

    let resolved = workspace.resolve_usage_visibility(true);

    assert_eq!(
        resolved.granularity,
        UsageVisibilityGranularity::TeamAggregate
    );
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Limited(11));
}

#[test]
fn admin_inherits_tier_per_user_totals_unlimited() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::PerUserTotals,
        MaxPriorCycles::Unlimited,
    )));

    let resolved = workspace.resolve_usage_visibility(true);

    assert_eq!(
        resolved.granularity,
        UsageVisibilityGranularity::PerUserTotals
    );
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Unlimited);
}

#[test]
fn paid_customer_types_always_classify_as_paid() {
    let paid_customer_types = [
        CustomerType::Turbo,
        CustomerType::SelfServe,
        CustomerType::Prosumer,
        CustomerType::Legacy,
        CustomerType::Enterprise,
        CustomerType::Business,
        CustomerType::Lightspeed,
        CustomerType::Build,
        CustomerType::BuildMax,
    ];

    for customer_type in paid_customer_types {
        for disable_premium_models in [None, Some(false), Some(true)] {
            let metadata = make_billing_metadata(customer_type, disable_premium_models);
            assert_eq!(
                metadata.ftue_account_class(),
                FtueAccountClass::Paid,
                "customer type {customer_type:?} should be paid"
            );
        }
    }
}

#[test]
fn free_account_with_premium_models_classifies_as_free_icp() {
    let metadata = make_billing_metadata(CustomerType::Free, Some(false));

    assert_eq!(metadata.ftue_account_class(), FtueAccountClass::FreeIcp);
}

#[test]
fn free_account_without_premium_models_classifies_as_free_standard() {
    let metadata = make_billing_metadata(CustomerType::Free, Some(true));

    assert_eq!(
        metadata.ftue_account_class(),
        FtueAccountClass::FreeStandard
    );
}

#[test]
fn missing_free_policy_classifies_as_free_standard() {
    let metadata = make_billing_metadata(CustomerType::Free, None);

    assert_eq!(
        metadata.ftue_account_class(),
        FtueAccountClass::FreeStandard
    );
}

#[test]
fn unknown_customer_type_uses_free_policy_classification() {
    let with_premium_models = make_billing_metadata(CustomerType::Unknown, Some(false));
    let without_premium_models = make_billing_metadata(CustomerType::Unknown, Some(true));
    let without_policy = make_billing_metadata(CustomerType::Unknown, None);

    assert_eq!(
        with_premium_models.ftue_account_class(),
        FtueAccountClass::FreeIcp
    );
    assert_eq!(
        without_premium_models.ftue_account_class(),
        FtueAccountClass::FreeStandard
    );
    assert_eq!(
        without_policy.ftue_account_class(),
        FtueAccountClass::FreeStandard
    );
}

#[test]
fn warp_ai_policy_deserializes_without_disable_premium_models() {
    let policy: WarpAiPolicy = serde_json::from_value(serde_json::json!({
        "limit": 60,
        "is_code_suggestions_toggleable": true,
        "is_prompt_suggestions_toggleable": true,
        "is_next_command_enabled": true,
        "is_git_operations_ai_enabled": true,
        "is_voice_enabled": true
    }))
    .expect("persisted policies without disable_premium_models should remain compatible");

    assert!(policy.disable_premium_models);
}

#[test]
fn admin_inherits_tier_full_breakdown_unlimited() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::FullBreakdown,
        MaxPriorCycles::Unlimited,
    )));

    let resolved = workspace.resolve_usage_visibility(true);

    assert_eq!(
        resolved.granularity,
        UsageVisibilityGranularity::FullBreakdown
    );
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Unlimited);
}
