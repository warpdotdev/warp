use super::{AllowanceCents, allowance_cents, format_allowance_count};
use crate::workspaces::workspace::ChargeUnit;

#[test]
fn allowance_cents_requires_a_tier_charged_in_cents_and_both_figures() {
    assert_eq!(
        allowance_cents(ChargeUnit::Cents, false, Some(70.2), Some(1800.0)),
        Some(AllowanceCents {
            used: 70.2,
            limit: 1800.0,
        })
    );
    assert_eq!(
        allowance_cents(ChargeUnit::Cents, false, None, Some(1800.0)),
        None
    );
    assert_eq!(
        allowance_cents(ChargeUnit::Cents, false, Some(70.2), None),
        None
    );
    // Cents the server sends to a tier charged in credits are never shown.
    assert_eq!(
        allowance_cents(ChargeUnit::Credits, false, Some(70.2), Some(1800.0)),
        None
    );
    // Unlimited subjects keep the credit display.
    assert_eq!(
        allowance_cents(ChargeUnit::Cents, true, Some(70.2), Some(1800.0)),
        None
    );
}

#[test]
fn allowance_count_shows_dollars_when_charged_in_cents() {
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
