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
        cost_in_cents: Some(cost_in_cents),
        has_usage: true,
        charged_usage: None,
    }
}

/// A fixture breakdown shared with the GUI-side tests for the same sample
/// conversation, so Detail mode's rendered text can be cross-checked against
/// the GUI panels' numbers for identical input data (validation criterion 1).
fn fixture_charged_usage() -> ChargedUsageTotals {
    ChargedUsageTotals {
        input_cost_in_cents: 1.0,
        output_cost_in_cents: 2.0,
        input_cache_read_cost_in_cents: 0.2,
        input_cache_write_cost_in_cents: 0.1,
        platform_cost_in_cents: 0.0,
        input_tokens: 100,
        output_tokens: 50,
        input_cache_read_tokens: 20,
        input_cache_write_tokens: 10,
    }
}

fn totals_with_charged_usage(
    credits_spent: f32,
    cost_in_cents: f32,
    charged_usage: ChargedUsageTotals,
) -> ConversationUsageTotals {
    ConversationUsageTotals {
        charged_usage: Some(charged_usage),
        ..totals(credits_spent, cost_in_cents)
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
    // Credits is the default mode; a click cycles credits → cost → detail →
    // back to credits.
    let credits = TuiUsageDisplayMode::default();
    assert_eq!(entry_text(credits, usage), "2.5 credits");
    assert_eq!(entry_text(credits.toggled(), usage), "$0.03");
    assert_eq!(entry_text(credits.toggled().toggled(), usage), "$0.03");
    assert_eq!(
        entry_text(credits.toggled().toggled().toggled(), usage),
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
                cost_in_cents: None,
                has_usage: true,
                charged_usage: None,
            },
        ),
        "Cost unavailable"
    );
}

#[test]
fn detail_mode_explicitly_marks_unknown_historical_cost() {
    assert_eq!(
        entry_text(
            TuiUsageDisplayMode::Detail,
            ConversationUsageTotals {
                credits_spent: 0.0,
                cost_in_cents: None,
                has_usage: true,
                charged_usage: None,
            },
        ),
        "Cost unavailable"
    );
}

/// Detail mode renders the total cost, token count, and the per-category
/// input/cache-read/cache-write/output breakdown, all summed from a
/// `ChargedUsageTotals` fixture.
#[test]
fn detail_mode_renders_full_breakdown() {
    let usage = totals_with_charged_usage(2.5, 3.3, fixture_charged_usage());
    let text = entry_text(TuiUsageDisplayMode::Detail, usage);
    assert_eq!(
        text,
        "$0.03 \u{b7} 180 tok \u{b7} in $0.01 \u{b7} cr $0.00 \u{b7} cw $0.00 \u{b7} out $0.02"
    );
}

/// When the cost is known but no charged-usage breakdown was provided (e.g. a
/// legacy conversation, or a request predating the breakdown fields), Detail
/// mode degrades to just the total cost — no fabricated per-category split.
#[test]
fn detail_mode_without_charged_usage_shows_cost_only() {
    let usage = totals(2.5, 3.2);
    assert_eq!(entry_text(TuiUsageDisplayMode::Detail, usage), "$0.03");
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
        let toggle = UsageToggle::default();
        let usage = totals(2.5, 3.2);

        // Flag OFF: static credits; the persisted `Cost` mode is ignored so the
        // dollar cost is never surfaced.
        app.read(|ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(false);
            let entry = toggle.render_entry(TuiUsageDisplayMode::Cost, usage, ctx, |_, _| {});
            let line = TuiPresenter::new()
                .present_element(entry, TuiRect::new(0, 0, 20, 1), ctx)
                .buffer
                .to_lines()
                .join("");
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
            let entry = toggle.render_entry(TuiUsageDisplayMode::Cost, usage, ctx, |_, _| {});
            let line = TuiPresenter::new()
                .present_element(entry, TuiRect::new(0, 0, 20, 1), ctx)
                .buffer
                .to_lines()
                .join("");
            assert!(
                line.contains("$0.03"),
                "flag on must follow the persisted cost mode, got: {line:?}"
            );
        });
    });
}

/// Same gate, but for the `Detail` mode specifically: with the flag off, a
/// persisted `Detail` mode (e.g. left over from a session where the flag was
/// on) must still render as plain static credits, not the breakdown -- byte
/// identical to pre-saga (Credits/Cost-only) output.
#[test]
fn footer_usage_entry_gates_detail_mode_behind_the_feature_flag() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        let toggle = UsageToggle::default();
        let usage = totals_with_charged_usage(2.5, 3.3, fixture_charged_usage());

        app.read(|ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(false);
            let entry = toggle.render_entry(TuiUsageDisplayMode::Detail, usage, ctx, |_, _| {});
            let line = TuiPresenter::new()
                .present_element(entry, TuiRect::new(0, 0, 40, 1), ctx)
                .buffer
                .to_lines()
                .join("");
            assert!(
                line.contains("2.5 credits"),
                "flag off must show static credits, got: {line:?}"
            );
            assert!(
                !line.contains("tok") && !line.contains('$'),
                "flag off must not expose the breakdown, got: {line:?}"
            );
        });

        app.read(|ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(true);
            let entry = toggle.render_entry(TuiUsageDisplayMode::Detail, usage, ctx, |_, _| {});
            let line = TuiPresenter::new()
                .present_element(entry, TuiRect::new(0, 0, 40, 1), ctx)
                .buffer
                .to_lines()
                .join("");
            assert!(
                line.contains("180 tok"),
                "flag on must follow the persisted detail mode, got: {line:?}"
            );
        });
    });
}
