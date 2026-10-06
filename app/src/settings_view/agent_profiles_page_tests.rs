use super::{AllowanceCents, format_allowance_count};

#[test]
fn allowance_count_shows_dollars_when_billed_in_dollars() {
    let cents = Some(AllowanceCents {
        used: 70.2,
        limit: 1800.0,
    });
    assert_eq!(
        format_allowance_count(39, 1_000, false, cents),
        "$0.70/$18.00"
    );
}

#[test]
fn allowance_count_falls_back_to_credits() {
    assert_eq!(format_allowance_count(39, 1_000, false, None), "39/1000");
}

#[test]
fn allowance_count_keeps_unlimited_display() {
    let cents = Some(AllowanceCents {
        used: 70.2,
        limit: 1800.0,
    });
    assert_eq!(format_allowance_count(39, 999_999, true, None), "Unlimited");
    assert_eq!(
        format_allowance_count(39, 999_999, true, cents),
        "Unlimited"
    );
}
