use super::*;
use crate::server::ids::ServerId;

// `ServerId::from_string_lossy` requires exactly 22 characters.
const TEST_WORKSPACE_UID: &str = "workspace_uid123456789";

#[test]
fn ftue_account_classes_have_stable_telemetry_labels() {
    assert_eq!(FtueAccountClass::Paid.as_str(), "paid");
    assert_eq!(FtueAccountClass::FreeIcp.as_str(), "free_icp");
    assert_eq!(FtueAccountClass::FreeStandard.as_str(), "free_standard");
}
fn make_workspace(policy: Option<UsageVisibilityPolicy>) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        ServerId::from_string_lossy(TEST_WORKSPACE_UID).into(),
        "Test Workspace".to_string(),
        None,
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

fn billing_metadata_with_purchase_policy(
    purchase_policy: Option<PurchaseAddOnCreditsPolicy>,
) -> BillingMetadata {
    let mut billing_metadata = BillingMetadata::default();
    billing_metadata.tier.purchase_add_on_credits_policy = purchase_policy;
    billing_metadata
}

#[test]
fn purchase_policy_disabled_without_policy() {
    let billing_metadata = billing_metadata_with_purchase_policy(None);

    assert!(!billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(!billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 0);
}

#[test]
fn purchase_policy_standard_plan_has_no_premium() {
    let billing_metadata =
        billing_metadata_with_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
            enabled: true,
            premium_enabled: false,
            price_premium_bps: 0,
        }));

    assert!(billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(!billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 0);
}

#[test]
fn purchase_policy_premium_plan_enables_surcharged_purchasing() {
    let billing_metadata =
        billing_metadata_with_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
            enabled: false,
            premium_enabled: true,
            price_premium_bps: 1000,
        }));

    assert!(billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 1000);
}

#[test]
fn purchase_policy_fully_disabled_plan_remains_disabled() {
    let billing_metadata =
        billing_metadata_with_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
            enabled: false,
            premium_enabled: false,
            price_premium_bps: 1000,
        }));

    assert!(!billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(!billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 0);
}

#[test]
fn purchase_policy_standard_purchasing_wins_over_premium() {
    // Standard (list price) purchasing takes precedence if the server ever
    // sends both flags; no surcharge should be displayed or applied.
    let billing_metadata =
        billing_metadata_with_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
            enabled: true,
            premium_enabled: true,
            price_premium_bps: 1000,
        }));

    assert!(billing_metadata.is_purchase_add_on_credits_policy_enabled());
    assert!(!billing_metadata.is_premium_addon_credits_purchase());
    assert_eq!(billing_metadata.addon_credits_price_premium_bps(), 0);
}
