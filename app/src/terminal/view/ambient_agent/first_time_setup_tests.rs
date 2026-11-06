use super::TrialCredits;

#[test]
fn trial_banner_shows_dollars_when_grants_carry_a_dollar_value() {
    let credits = TrialCredits {
        credits: 100,
        usage_cents: Some(180.0),
    };
    assert_eq!(
        credits.banner_text(),
        "You have $1.80 of free usage for Oz cloud agents."
    );
}

#[test]
fn trial_banner_falls_back_to_credits() {
    let credits = TrialCredits {
        credits: 100,
        usage_cents: None,
    };
    assert_eq!(
        credits.banner_text(),
        "You have 100 free credits to use on Oz cloud agents."
    );
    let one_credit = TrialCredits {
        credits: 1,
        usage_cents: None,
    };
    assert_eq!(
        one_credit.banner_text(),
        "You have 1 free credit to use on Oz cloud agents."
    );
}
