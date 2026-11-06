use warp_graphql::billing::AddonCreditsOption;

use super::{
    PackAmount, addon_credits_description, format_price, larger_packs_cost_less_per_credit,
    pack_menu_label,
};
use crate::settings::UsageDisplayUnit;

fn credits_pack(credits: i32, price_usd_cents: i32) -> AddonCreditsOption {
    AddonCreditsOption {
        credits,
        price_usd_cents,
        usage_cents: None,
    }
}

fn usage_pack(usage_cents: i32, price_usd_cents: i32) -> AddonCreditsOption {
    AddonCreditsOption {
        credits: usage_cents,
        price_usd_cents,
        usage_cents: Some(usage_cents),
    }
}

#[test]
fn pack_amount_is_usage_only_when_shown_in_dollars_with_a_priced_pack() {
    assert_eq!(
        PackAmount::of(&usage_pack(1_000, 1_000), UsageDisplayUnit::Dollars),
        PackAmount::UsageCents(1_000)
    );
    // Shown in credits, a pack shows credits even when the catalog states usage.
    assert_eq!(
        PackAmount::of(&usage_pack(1_000, 1_000), UsageDisplayUnit::Credits),
        PackAmount::Credits(1_000)
    );
    // Shown in dollars, a pack falls back to credits when the catalog states no usage.
    assert_eq!(
        PackAmount::of(&credits_pack(1_000, 1_000), UsageDisplayUnit::Dollars),
        PackAmount::Credits(1_000)
    );
    assert_eq!(
        PackAmount::of(&credits_pack(1_000, 1_000), UsageDisplayUnit::Credits),
        PackAmount::Credits(1_000)
    );
    assert!(PackAmount::UsageCents(1_000).is_usage());
    assert!(!PackAmount::Credits(1_000).is_usage());
}

#[test]
fn credit_packs_keep_credit_labels() {
    assert_eq!(PackAmount::Credits(1_000).short_label(), "1,000");
    assert_eq!(PackAmount::Credits(1_000).label(), "1,000 credits");
    assert_eq!(PackAmount::Credits(1).label(), "1 credit");
}

#[test]
fn usage_packs_are_labelled_in_dollars() {
    assert_eq!(PackAmount::UsageCents(1_000).short_label(), "$10");
    assert_eq!(PackAmount::UsageCents(1_050).short_label(), "$10.50");
    assert_eq!(PackAmount::UsageCents(1_000).label(), "$10 of usage");
}

#[test]
fn price_drops_cents_only_for_whole_dollars() {
    assert_eq!(format_price(1_000), "$10");
    assert_eq!(format_price(1_001), "$10.01");
    assert_eq!(format_price(5), "$0.05");
}

#[test]
fn larger_packs_cost_less_per_credit_only_for_discounted_catalogs() {
    assert!(larger_packs_cost_less_per_credit(&[
        credits_pack(1_000, 1_000),
        credits_pack(2_500, 2_000),
    ]));
    // Usage packs are sold at face value, so their credit equivalents are flat-rate.
    assert!(!larger_packs_cost_less_per_credit(&[
        usage_pack(555, 1_000),
        usage_pack(1_111, 2_000),
        usage_pack(2_777, 5_000),
        usage_pack(5_555, 10_000),
    ]));
    assert!(!larger_packs_cost_less_per_credit(&[]));
}

#[test]
fn description_promises_a_better_rate_only_when_the_catalog_offers_one() {
    let discounted =
        addon_credits_description(&[credits_pack(1_000, 1_000), credits_pack(2_500, 2_000)]);
    assert_eq!(
        discounted,
        "Add-on credits are purchased in prepaid packages that roll over each billing cycle and \
         expire after one year. The more you purchase, the better the per-credit rate. Once your \
         base plan credits are used, add-on credits will be consumed."
    );

    let flat = addon_credits_description(&[usage_pack(555, 1_000), usage_pack(1_111, 2_000)]);
    assert_eq!(
        flat,
        "Add-on credits are purchased in prepaid packages that roll over each billing cycle and \
         expire after one year. Once your base plan credits are used, add-on credits will be \
         consumed."
    );
}

#[test]
fn menu_label_shows_price_after_premium_then_amount() {
    assert_eq!(
        pack_menu_label(&credits_pack(1_000, 1_000), 0, UsageDisplayUnit::Credits),
        "$10 / 1,000 credits"
    );
    assert_eq!(
        pack_menu_label(&usage_pack(1_000, 1_000), 1_000, UsageDisplayUnit::Dollars),
        "$11 / $10 of usage"
    );
    assert_eq!(
        pack_menu_label(&usage_pack(1_000, 1_000), 0, UsageDisplayUnit::Credits),
        "$10 / 1,000 credits"
    );
}
