use warp::appearance::Appearance;
use warp::settings::TuiUsageDisplayMode;
use warp::tui_export::{ChargedUsageTotals, ConversationUsageTotals};
use warp_core::features::FeatureFlag;
use warpui_core::App;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::*;

fn totals(credits_spent: f32, cost_in_cents: f32) -> ConversationUsageTotals {
    ConversationUsageTotals {
        credits_spent,
        billed_cost_in_cents: Some(cost_in_cents),
        has_usage: true,
        charged_usage: None,
    }
}

/// Renders the entry for `totals` into a single line of text.
fn render_entry_line(
    mode: TuiUsageDisplayMode,
    totals: ConversationUsageTotals,
    billed_in_dollars: bool,
    ctx: &AppContext,
) -> String {
    let entry =
        UsageToggle::default().render_entry(mode, totals, billed_in_dollars, ctx, |_, _| {});
    TuiPresenter::new()
        .present_element(entry, TuiRect::new(0, 0, 20, 1), ctx)
        .buffer
        .to_lines()
        .join("")
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
fn entry_text_never_rounds_a_positive_cost_to_zero() {
    assert_eq!(
        entry_text(TuiUsageDisplayMode::Cost, totals(0.0, 0.4)),
        "<$0.01"
    );
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

#[test]
fn cost_mode_explicitly_marks_unknown_historical_cost() {
    assert_eq!(
        entry_text(
            TuiUsageDisplayMode::Cost,
            ConversationUsageTotals {
                credits_spent: 0.0,
                billed_cost_in_cents: None,
                has_usage: true,
                charged_usage: None,
            },
        ),
        "Cost unavailable"
    );
}

/// Cost mode shows what the server streamed as charged before falling back to
/// the billed snapshot, mirroring `ConversationUsageTotals::total_cost_in_cents`.
#[test]
fn cost_mode_prefers_charged_usage_over_the_billed_snapshot() {
    assert_eq!(
        entry_text(
            TuiUsageDisplayMode::Cost,
            ConversationUsageTotals {
                credits_spent: 2.5,
                billed_cost_in_cents: Some(500.0),
                has_usage: true,
                charged_usage: Some(ChargedUsageTotals {
                    input_cost_in_cents: 4.0,
                    ..Default::default()
                }),
            },
        ),
        "$0.04"
    );
}

/// The credits⇄dollars toggle is gated behind `PricingTransparency`: with the
/// flag off (prod/stable) the footer renders a static credits total and never
/// exposes the dollar cost even when the persisted mode is `Cost`; with the
/// flag on (dogfood/staging + local/dev) the entry follows the persisted mode.
#[test]
fn footer_usage_entry_gates_the_cost_toggle_behind_the_feature_flag() {
    App::test((), |mut app| async move {
        // `render_entry` resolves theme styles via `TuiUiBuilder`, which reads
        // the `Appearance` singleton.
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        let usage = totals(2.5, 3.2);

        // Flag OFF: static credits; the persisted `Cost` mode is ignored so the
        // dollar cost is never surfaced.
        app.read(|ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(false);
            let line = render_entry_line(TuiUsageDisplayMode::Cost, usage, false, ctx);
            assert!(
                line.contains("2.5 credits"),
                "flag off must show static credits, got: {line:?}"
            );
            assert!(
                !line.contains("$0.03"),
                "flag off must not expose the dollar cost, got: {line:?}"
            );
        });

        // Flag ON: the entry follows the persisted display mode, exposing the
        // dollar cost the toggle switches to.
        app.read(|ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(true);
            let line = render_entry_line(TuiUsageDisplayMode::Cost, usage, false, ctx);
            assert!(
                line.contains("$0.03"),
                "flag on must follow the persisted cost mode, got: {line:?}"
            );
        });
    });
}

/// A subject the server bills in dollars sees the billed total regardless of
/// the flag or the persisted mode, and falls back to the usual credits entry
/// when the conversation has no billed total yet.
#[test]
fn footer_usage_entry_shows_billed_dollars_for_dollar_billed_subjects() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        let usage = totals(2.5, 3.2);

        app.read(|ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(false);
            let line = render_entry_line(TuiUsageDisplayMode::Credits, usage, true, ctx);
            assert!(
                line.contains("$0.03"),
                "a dollars-billed subject must see the billed total, got: {line:?}"
            );
        });

        app.read(|ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(false);
            let line = render_entry_line(
                TuiUsageDisplayMode::Cost,
                ConversationUsageTotals {
                    credits_spent: 2.5,
                    billed_cost_in_cents: None,
                    has_usage: true,
                    charged_usage: None,
                },
                true,
                ctx,
            );
            assert!(
                line.contains("2.5 credits"),
                "without a billed total the entry falls back to credits, got: {line:?}"
            );
        });
    });
}
