use warp_graphql::billing::CustomUsageAmounts;

use super::parse_usage_dollars;

fn bounds() -> CustomUsageAmounts {
    CustomUsageAmounts {
        min_usage_cents: 1000,
        max_usage_cents: 1_000_000,
        step_usage_cents: 100,
    }
}

#[test]
fn custom_purchase_serializes_only_monetary_amount() {
    let input = warp_graphql::mutations::purchase_addon_credits::PurchaseAddonCreditsInput {
        credits: None,
        usage_cents: Some(25_000),
        team_uid: None,
    };

    let value = serde_json::to_value(input).unwrap();

    assert_eq!(value["usageCents"], 25_000);
    assert!(value.get("credits").is_none());
}

#[test]
fn custom_reload_omits_legacy_denomination() {
    let input = warp_graphql::mutations::update_workspace_settings::AddonCreditsSettingsInput {
        auto_reload_enabled: Some(true),
        max_monthly_spend_cents: None,
        selected_auto_reload_usage_cents: Some(25_000),
        selected_auto_reload_credit_denomination: None,
    };

    let value = serde_json::to_value(input).unwrap();

    assert_eq!(value["selectedAutoReloadUsageCents"], 25_000);
    assert!(value.get("selectedAutoReloadCreditDenomination").is_none());
}

#[test]
fn accepts_inclusive_whole_dollar_boundaries() {
    assert_eq!(parse_usage_dollars("10", bounds()), Some(1000));
    assert_eq!(parse_usage_dollars("10000", bounds()), Some(1_000_000));
}

#[test]
fn preserves_non_preset_dollar_amounts_exactly() {
    assert_eq!(parse_usage_dollars("137", bounds()), Some(13700));
}

#[test]
fn rejects_out_of_range_dollars() {
    assert_eq!(parse_usage_dollars("9", bounds()), None);
    assert_eq!(parse_usage_dollars("10001", bounds()), None);
    assert_eq!(parse_usage_dollars("0", bounds()), None);
}

#[test]
fn rejects_empty_fractional_and_malformed_input_without_rounding() {
    assert_eq!(parse_usage_dollars("", bounds()), None);
    assert_eq!(parse_usage_dollars("10.00", bounds()), None);
    assert_eq!(parse_usage_dollars("10.5", bounds()), None);
    assert_eq!(parse_usage_dollars("$10", bounds()), None);
    assert_eq!(parse_usage_dollars("-10", bounds()), None);
    assert_eq!(parse_usage_dollars("1e2", bounds()), None);
    assert_eq!(parse_usage_dollars(" 10", bounds()), None);
    assert_eq!(parse_usage_dollars("999999999999999999", bounds()), None);
}
