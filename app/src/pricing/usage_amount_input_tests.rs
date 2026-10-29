use super::*;

#[test]
fn whole_dollar_usage_obeys_server_range() {
    let range = AddonUsagePurchaseRange {
        min_usage_cents: 1000,
        max_usage_cents: 100000,
        step_usage_cents: 100,
    };
    assert_eq!(parse_usage_amount("10", &range), Some(1000));
    assert_eq!(parse_usage_amount("67", &range), Some(6700));
    assert_eq!(parse_usage_amount("1000", &range), Some(100000));
    assert_eq!(parse_usage_amount("9", &range), None);
    assert_eq!(parse_usage_amount("1001", &range), None);
    assert_eq!(parse_usage_amount("10.5", &range), None);
    assert_eq!(parse_usage_amount("", &range), None);
    assert_eq!(parse_usage_amount("abc", &range), None);
    assert_eq!(parse_usage_amount("9999999999999999999", &range), None);
}

#[test]
fn custom_usage_price_does_not_change_usage() {
    let amount = AddonPurchaseAmount::UsageCents(6700);
    assert_eq!(amount.price_usd_cents_with_premium(2000), 8040);
    assert_eq!(amount.label(UsageDisplayUnit::Dollars), "$67 of usage");
}
