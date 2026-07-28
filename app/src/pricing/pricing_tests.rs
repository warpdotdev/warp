use warp_graphql::billing::AddonCreditsOption;

use super::*;

/// Helper to build a paid-plan option (zero markup).
fn paid_option(credits: i32, price_cents: i32) -> AddonCreditsOption {
    AddonCreditsOption {
        credits,
        price_usd_cents: price_cents,
        base_price_usd_cents: Some(price_cents),
        markup_usd_cents: Some(0),
        total_price_usd_cents: Some(price_cents),
    }
}

/// Helper to build a Free-plan option with a 10% markup.
fn free_option_with_markup(credits: i32, base_cents: i32, markup_cents: i32) -> AddonCreditsOption {
    AddonCreditsOption {
        credits,
        price_usd_cents: base_cents, // legacy field unchanged
        base_price_usd_cents: Some(base_cents),
        markup_usd_cents: Some(markup_cents),
        total_price_usd_cents: Some(base_cents + markup_cents),
    }
}

// ── AddonCreditsOption helpers ─────────────────────────────────────────────

/// Spec validation criterion 2: Free fixture with markup.
#[test]
fn test_free_option_has_markup() {
    let opt = free_option_with_markup(1000, 100, 10);
    assert!(
        opt.has_markup(),
        "markup_usd_cents > 0 should set has_markup"
    );
    assert_eq!(opt.markup_usd_cents, Some(10));
    assert_eq!(opt.base_price_usd_cents, Some(100));
    assert_eq!(opt.total_price_cents(), 110);
}

/// Spec validation criterion 2: Paid fixture with zero markup.
#[test]
fn test_paid_option_no_markup() {
    let opt = paid_option(1000, 100);
    assert!(
        !opt.has_markup(),
        "markup_usd_cents == 0 should not set has_markup"
    );
    assert_eq!(opt.total_price_cents(), 100);
    assert_eq!(opt.base_price_usd_cents, Some(100));
}

/// Spec validation criterion 2: rate() uses base price, not total.
/// Volume-discount badges must not be inflated by markup.
#[test]
fn test_rate_uses_base_price_not_total() {
    let opt = free_option_with_markup(1000, 100, 10);
    // rate() = base / credits = 100 / 1000 = 0.1
    let expected_rate = 100_f32 / 1000_f32;
    assert!(
        (opt.rate() - expected_rate).abs() < f32::EPSILON,
        "rate() should use effective_base_price_cents, not total"
    );
}

// ── AddonPackPriceInfo display helpers ─────────────────────────────────────────────

/// Spec validation criterion 2: formatted values for Free fixture.
#[test]
fn test_addon_pack_price_info_free_fixture() {
    let opt = free_option_with_markup(1000, 100, 10);
    let info = AddonPackPriceInfo::from_option(&opt);

    assert!(info.has_markup());
    assert!(info.is_valid());
    assert_eq!(info.formatted_base_price(), "$1.00");
    assert_eq!(info.formatted_markup(), "$0.10");
    assert_eq!(info.formatted_total_price(), "$1.10");
}

/// Spec validation criterion 2: formatted values for paid fixture.
#[test]
fn test_addon_pack_price_info_paid_fixture() {
    let opt = paid_option(1000, 100);
    let info = AddonPackPriceInfo::from_option(&opt);

    assert!(!info.has_markup());
    assert!(info.is_valid());
    assert_eq!(info.formatted_base_price(), "$1.00");
    assert_eq!(info.formatted_markup(), ""); // empty for zero markup
    assert_eq!(info.formatted_total_price(), "$1.00");
}

/// Spec validation criterion 2 and Behavior 18: inconsistent prices are invalid.
/// `AddonCreditsOption::is_price_valid` returns false for malformed server data,
/// and `AddonPackPriceInfo::is_valid` likewise rejects it so callers can disable
/// the purchase button without rendering a bogus price.
#[test]
fn test_inconsistent_option_total_less_than_base() {
    // total < base: the server sent internally inconsistent data.
    let opt = AddonCreditsOption {
        credits: 1000,
        price_usd_cents: 100,
        base_price_usd_cents: Some(200), // base is 200
        markup_usd_cents: Some(0),
        total_price_usd_cents: Some(100), // total < base — inconsistent
    };
    assert!(
        !opt.is_price_valid(),
        "total < base should be detected as invalid"
    );

    // Negative total: another form of malformed server response.
    let opt_neg = AddonCreditsOption {
        credits: 1000,
        price_usd_cents: 100,
        base_price_usd_cents: Some(100),
        markup_usd_cents: Some(0),
        total_price_usd_cents: Some(-1),
    };
    assert!(
        !opt_neg.is_price_valid(),
        "negative total should be detected as invalid"
    );

    // The AddonPackPriceInfo wrapper must also report invalid.
    let info = AddonPackPriceInfo::from_option(&opt);
    assert!(
        !info.is_valid(),
        "AddonPackPriceInfo must propagate invalidity"
    );
}

// ── format_usd_cents ─────────────────────────────────────────────────────────

#[test]
fn test_format_usd_cents() {
    assert_eq!(format_usd_cents(0), "$0.00");
    assert_eq!(format_usd_cents(100), "$1.00");
    assert_eq!(format_usd_cents(110), "$1.10");
    assert_eq!(format_usd_cents(1099), "$10.99");
    assert_eq!(format_usd_cents(2000), "$20.00");
}

// ── Spend-limit boundary tests ───────────────────────────────────────────────

/// Spec validation criterion 5: spend-limit check uses total_price_usd_cents.
#[test]
fn test_spend_limit_uses_total_price() {
    use crate::workspaces::workspace::{AddonCreditsSettings, BonusGrantsPurchased, Workspace};

    let mut workspace = Workspace::from_local_cache(
        "workspace_uid123456789".to_string().into(),
        "Test".to_string(),
        None,
    );
    workspace.bonus_grants_purchased_this_month = BonusGrantsPurchased {
        total_credits_purchased: 0,
        cents_spent: 100, // $1.00 already spent
    };
    workspace.settings.addon_credits_settings = AddonCreditsSettings {
        auto_reload_enabled: false,
        max_monthly_spend_cents: Some(200), // $2.00 limit
        selected_auto_reload_credit_denomination: None,
    };

    // Base price $0.90 + markup $0.10 = total $1.00 — does not exceed remaining $1.00.
    let free_opt = free_option_with_markup(1000, 90, 10);
    assert!(
        !workspace.would_addon_purchase_reach_limit(free_opt.total_price_cents()),
        "total $1.00 with $1.00 remaining should not exceed limit"
    );

    // Base price $1.00 alone would fit, but with markup of $0.10 the total is $1.10 > $1.00.
    let exceeding_opt = free_option_with_markup(2000, 100, 10);
    assert!(
        workspace.would_addon_purchase_reach_limit(exceeding_opt.total_price_cents()),
        "total $1.10 with $1.00 remaining should exceed limit"
    );
}
