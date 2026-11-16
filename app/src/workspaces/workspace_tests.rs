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

fn pack(credits: i32, price_usd_cents: i32, usage_cents: Option<i32>) -> AddonCreditsOption {
    AddonCreditsOption {
        credits,
        price_usd_cents,
        usage_cents,
    }
}

fn auto_reload_settings(
    selected_auto_reload_credit_denomination: Option<i32>,
    selected_auto_reload_usage_cents: Option<i32>,
) -> AddonCreditsSettings {
    AddonCreditsSettings {
        auto_reload_enabled: true,
        max_monthly_spend_cents: None,
        selected_auto_reload_credit_denomination,
        selected_auto_reload_usage_cents,
    }
}

#[test]
fn auto_reload_pack_matches_by_credit_denomination_for_credit_catalogs() {
    let options = [pack(1_000, 1_000, None), pack(2_000, 1_800, None)];

    let settings = auto_reload_settings(Some(2_000), None);
    assert_eq!(
        settings.selected_auto_reload_option_index(&options),
        Some(1)
    );

    let unlisted = auto_reload_settings(Some(5_000), None);
    assert_eq!(unlisted.selected_auto_reload_option_index(&options), None);

    let unconfigured = auto_reload_settings(None, None);
    assert_eq!(
        unconfigured.selected_auto_reload_option_index(&options),
        None
    );
}

#[test]
fn auto_reload_pack_matches_by_list_price_for_dollar_catalogs() {
    // The credit counts the packs are listed under have changed since auto-reload was
    // configured; the list price still identifies the pack.
    let options = [
        pack(1_200, 1_000, Some(1_000)),
        pack(2_400, 2_000, Some(2_000)),
    ];

    let settings = auto_reload_settings(Some(2_000), Some(2_000));
    assert_eq!(
        settings.selected_auto_reload_option_index(&options),
        Some(1)
    );

    // Settings from before the catalog switched to dollars fall back to the credit count.
    let credits_only = auto_reload_settings(Some(1_200), None);
    assert_eq!(
        credits_only.selected_auto_reload_option_index(&options),
        Some(0)
    );
}

#[test]
fn auto_reload_price_follows_the_matched_pack_with_premium() {
    let mut workspace = make_workspace(None);
    workspace.billing_metadata =
        billing_metadata_with_purchase_policy(Some(PurchaseAddOnCreditsPolicy {
            enabled: false,
            premium_enabled: true,
            price_premium_bps: 1_000,
        }));
    workspace.settings.addon_credits_settings = auto_reload_settings(Some(1_000), Some(2_000));
    let options = [
        pack(1_000, 1_000, Some(1_000)),
        pack(2_000, 2_000, Some(2_000)),
    ];

    assert_eq!(workspace.get_auto_reload_price_cents(&options), Some(2_200));
    assert_eq!(workspace.get_auto_reload_price_cents(&[]), None);
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
