use warp::settings::TuiUsageDisplayMode;
use warp::tui_export::ConversationUsageTotals;

use super::*;

fn totals(credits_spent: f32, cost_in_cents: f32) -> ConversationUsageTotals {
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
    let mode = TuiUsageDisplayMode::default();
    assert_eq!(entry_text(mode, totals(1.0, 0.0)), "1 credit");
    assert_eq!(entry_text(mode, totals(2.0, 0.0)), "2 credits");
    assert_eq!(entry_text(mode, totals(2.5, 0.0)), "2.5 credits");
}

#[test]
fn entry_text_follows_the_persisted_display_mode() {
    let usage = totals(2.5, 3.2);
    // Credits is the default mode; a click toggles to cost and back.
    let credits = TuiUsageDisplayMode::default();
    assert_eq!(entry_text(credits, usage), "2.5 credits");
    assert_eq!(entry_text(credits.toggled(), usage), "$0.03");
    assert_eq!(
        entry_text(credits.toggled().toggled(), usage),
        "2.5 credits"
    );
}
