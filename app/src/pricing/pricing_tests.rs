use warp_graphql::billing::AddonCreditsOption;

use super::onboarding_credit_pack_options;

/// The production add-on credit packs (`GetAddonCreditsOptions` on the server).
fn production_packs() -> Vec<AddonCreditsOption> {
    [
        (400, 1_000),
        (1_000, 2_000),
        (3_000, 5_000),
        (6_500, 10_000),
    ]
    .into_iter()
    .map(|(credits, price_usd_cents)| AddonCreditsOption {
        credits,
        price_usd_cents,
    })
    .collect()
}

#[test]
fn subscriber_packs_are_offered_at_list_price() {
    let packs = onboarding_credit_pack_options(&production_packs(), 0);

    let prices: Vec<_> = packs.iter().map(|pack| pack.price_label()).collect();
    assert_eq!(prices, ["$10", "$20", "$50", "$100"]);
}

/// Free-plan buyers pay the `price_premium_bps` surcharge (2000 bps = +20%) on
/// top of the list price. Regression test for REV-1886: the onboarding offer
/// must show the premium-adjusted price the server actually charges, never the
/// list price.
#[test]
fn free_plan_packs_apply_the_twenty_percent_premium() {
    let packs = onboarding_credit_pack_options(&production_packs(), 2_000);

    let labels: Vec<_> = packs
        .iter()
        .map(|pack| (pack.credits_label(), pack.price_label()))
        .collect();
    assert_eq!(
        labels,
        [
            ("400".to_string(), "$12".to_string()),
            ("1,000".to_string(), "$24".to_string()),
            ("3,000".to_string(), "$60".to_string()),
            ("6,500".to_string(), "$120".to_string()),
        ]
    );
}

/// Volume savings are relative to the smallest pack's per-credit rate, and are
/// unaffected by the premium (which scales every pack equally).
#[test]
fn volume_savings_are_relative_to_the_smallest_pack() {
    for premium_bps in [0, 2_000] {
        let packs = onboarding_credit_pack_options(&production_packs(), premium_bps);

        let savings: Vec<_> = packs.iter().map(|pack| pack.savings_percent).collect();
        assert_eq!(savings, [0, 20, 33, 38], "premium_bps = {premium_bps}");
    }
}

#[test]
fn no_packs_produces_no_options() {
    assert!(onboarding_credit_pack_options(&[], 2_000).is_empty());
}

/// A pack that is a worse per-credit deal than the smallest one must not
/// render a negative or wrapped-around "savings" badge.
#[test]
fn packs_worse_than_the_base_rate_show_no_savings() {
    let packs = vec![
        AddonCreditsOption {
            credits: 400,
            price_usd_cents: 1_000,
        },
        AddonCreditsOption {
            credits: 400,
            price_usd_cents: 1_500,
        },
    ];

    let savings: Vec<_> = onboarding_credit_pack_options(&packs, 0)
        .iter()
        .map(|pack| pack.savings_percent)
        .collect();
    assert_eq!(savings, [0, 0]);
}
