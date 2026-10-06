use warp_graphql::billing::AddonCreditsOption;

use super::{PackAmount, format_price, pack_menu_label, packs_are_sold_in_dollars};

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
fn pack_amount_follows_the_catalog_unit() {
    assert_eq!(
        PackAmount::of(&credits_pack(1_000, 1_000)),
        PackAmount::Credits(1_000)
    );
    assert_eq!(
        PackAmount::of(&usage_pack(1_000, 1_000)),
        PackAmount::UsageCents(1_000)
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
fn menu_label_shows_price_after_premium_then_amount() {
    assert_eq!(
        pack_menu_label(&credits_pack(1_000, 1_000), 0),
        "$10 / 1,000 credits"
    );
    assert_eq!(
        pack_menu_label(&usage_pack(1_000, 1_000), 1_000),
        "$11 / $10 of usage"
    );
}

#[test]
fn packs_are_sold_in_dollars_requires_every_pack_to_carry_usage() {
    assert!(!packs_are_sold_in_dollars(&[]));
    assert!(!packs_are_sold_in_dollars(&[credits_pack(1_000, 1_000)]));
    assert!(!packs_are_sold_in_dollars(&[
        usage_pack(1_000, 1_000),
        credits_pack(2_000, 2_000),
    ]));
    assert!(packs_are_sold_in_dollars(&[
        usage_pack(1_000, 1_000),
        usage_pack(2_000, 2_000),
    ]));
}
