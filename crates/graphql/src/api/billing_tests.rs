use super::AddonCreditsOption;

fn option(credits: i32, price_usd_cents: i32) -> AddonCreditsOption {
    AddonCreditsOption {
        credits,
        price_usd_cents,
    }
}

#[test]
fn premium_price_is_list_price_at_zero_bps() {
    assert_eq!(option(1_000, 1_000).price_usd_cents_with_premium(0), 1_000);
    assert_eq!(
        option(10_000, 10_000).price_usd_cents_with_premium(0),
        10_000
    );
}

#[test]
fn premium_price_applies_ten_percent_surcharge_at_1000_bps() {
    // The standard packs: $10 / $20 / $50 / $100 become $11 / $22 / $55 / $110.
    assert_eq!(
        option(1_000, 1_000).price_usd_cents_with_premium(1_000),
        1_100
    );
    assert_eq!(
        option(2_000, 2_000).price_usd_cents_with_premium(1_000),
        2_200
    );
    assert_eq!(
        option(5_000, 5_000).price_usd_cents_with_premium(1_000),
        5_500
    );
    assert_eq!(
        option(10_000, 10_000).price_usd_cents_with_premium(1_000),
        11_000
    );
}

#[test]
fn premium_surcharge_rounds_up_to_the_next_cent() {
    // 999 * 1000 bps = 99.9 cents of surcharge, which must round up to 100.
    assert_eq!(option(100, 999).price_usd_cents_with_premium(1_000), 1_099);
    // 1 cent at 1 bp is a fractional surcharge (0.0001 cents) and still
    // rounds up to a full cent.
    assert_eq!(option(1, 1).price_usd_cents_with_premium(1), 2);
    // 1250 bps (+12.5%) on $10.01 => 125.125 cents of surcharge => 126.
    assert_eq!(
        option(1_000, 1_001).price_usd_cents_with_premium(1_250),
        1_127
    );
}

#[test]
fn premium_price_ignores_negative_bps() {
    assert_eq!(
        option(1_000, 1_000).price_usd_cents_with_premium(-500),
        1_000
    );
}
