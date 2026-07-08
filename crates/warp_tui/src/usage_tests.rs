use warp::tui_export::ConversationUsageTotals;

use super::*;

fn totals(credits_spent: f32, cost_in_cents: f64) -> ConversationUsageTotals {
    ConversationUsageTotals {
        credits_spent,
        cost_in_cents,
    }
}

#[test]
fn cost_formats_cents_as_dollars() {
    assert_eq!(format_cost(0.0), "$0.00");
    assert_eq!(format_cost(0.4), "$0.00");
    assert_eq!(format_cost(3.2), "$0.03");
    assert_eq!(format_cost(123.0), "$1.23");
    assert_eq!(format_cost(10_000.0), "$100.00");
}

#[test]
fn entry_text_matches_the_gui_credits_formatting() {
    // `format_credits` is the GUI's formatter: whole values pluralize and
    // drop the decimal, fractional values keep one decimal place.
    let toggle = UsageToggle::default();
    assert_eq!(toggle.entry_text(totals(1.0, 0.0)), "1 credit");
    assert_eq!(toggle.entry_text(totals(2.0, 0.0)), "2 credits");
    assert_eq!(toggle.entry_text(totals(2.5, 0.0)), "2.5 credits");
}

#[test]
fn toggle_flips_entry_between_credits_and_cost() {
    let toggle = UsageToggle::default();
    let usage = totals(2.5, 3.2);

    assert_eq!(toggle.entry_text(usage), "2.5 credits");
    toggle.toggle();
    assert_eq!(toggle.entry_text(usage), "$0.03");
    toggle.toggle();
    assert_eq!(toggle.entry_text(usage), "2.5 credits");
}

#[test]
fn cloned_toggles_share_display_mode() {
    // Render closures capture a clone; a click through the clone must be
    // visible to the view-owned original.
    let toggle = UsageToggle::default();
    let clone = toggle.clone();
    let usage = totals(2.5, 3.2);

    clone.toggle();
    assert_eq!(toggle.entry_text(usage), "$0.03");
}
