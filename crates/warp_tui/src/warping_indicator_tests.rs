use std::time::Duration;

use warp::appearance::Appearance;
use warp::tui_export::ChargedUsageTotals;
use warp_core::features::FeatureFlag;
use warpui_core::App;
use warpui_core::elements::shimmer_math::ShimmerConfig;
use warpui_core::elements::tui::{Color, TuiBufferExt, TuiElement, TuiRect, TuiText};
use warpui_core::presenter::tui::TuiPresenter;

use super::{SPINNER_TIMELINE, render_response_summary, render_warping_indicator_row};
use crate::tui_builder::TuiUiBuilder;

fn charged_usage(total_cost_in_cents: f32) -> ChargedUsageTotals {
    ChargedUsageTotals {
        input_cost_in_cents: total_cost_in_cents,
        ..Default::default()
    }
}

#[test]
fn spinner_follows_the_prototype_choreography() {
    let frame_at = |millis| *SPINNER_TIMELINE.value_at(Duration::from_millis(millis));
    // 180° right at 200ms per 45° step...
    assert_eq!(frame_at(0), "⋮");
    assert_eq!(frame_at(200), "⋰");
    assert_eq!(frame_at(400), "⋯");
    assert_eq!(frame_at(600), "⋱");
    // ...then 180° back left...
    assert_eq!(frame_at(800), "⋮");
    assert_eq!(frame_at(1000), "⋱");
    assert_eq!(frame_at(1200), "⋯");
    assert_eq!(frame_at(1400), "⋰");
    // ...then a rest at vertical before the fast spins...
    assert_eq!(frame_at(1600), "⋮");
    assert_eq!(frame_at(1799), "⋮");
    // ...then fast spins right at 80ms per 45° step.
    assert_eq!(frame_at(1800), "⋰");
    assert_eq!(frame_at(1880), "⋯");
    assert_eq!(frame_at(1960), "⋱");
    assert_eq!(frame_at(2040), "⋮");
    assert_eq!(frame_at(2200), "⋯");
    assert_eq!(frame_at(2600), "⋱");
    // The restarting `⋮` completes the final spin at the loop boundary.
    assert_eq!(frame_at(2680), "⋮");
    // Each frame holds for its full duration.
    assert_eq!(frame_at(199), "⋮");
}

#[test]
fn renders_the_indicator_row_and_requests_a_repaint() {
    App::test((), |mut app| async move {
        // `TuiUiBuilder` reads theme colors from the `Appearance` singleton.
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let element = render_warping_indicator_row(
                "Warping",
                Duration::ZERO,
                TuiText::new("▶▶ Auto approve off")
                    .with_style(TuiUiBuilder::from_app(app_ctx).muted_text_style())
                    .finish(),
                app_ctx,
            );
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(element, TuiRect::new(0, 0, 80, 1), app_ctx);

            let lines = frame.buffer.to_lines();
            let line = &lines[0];
            // The spinner glyph could advance a frame on a slow machine, so
            // accept any frame; the label and fresh counter are exact.
            let spinner = line.chars().next().expect("row should not be empty");
            assert!(
                SPINNER_TIMELINE
                    .values()
                    .any(|glyph| *glyph == spinner.to_string().as_str()),
                "unexpected spinner glyph in row: {line:?}"
            );
            assert!(
                line.contains(" Warping... (0s)"),
                "unexpected indicator row: {line:?}"
            );
            assert!(line.ends_with("▶▶ Auto approve off  Ctrl + C to stop"));

            // The animated row must schedule the next repaint.
            assert!(frame.repaint_at.is_some());
        });
    });
}

#[test]
fn shimmer_only_applies_to_the_warping_label_and_groups_its_ellipsis() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let config = ShimmerConfig::default();
            let element = render_warping_indicator_row(
                "Warping",
                config.period / 2,
                TuiText::new("▶▶ Auto approve off").finish(),
                app_ctx,
            );
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(element, TuiRect::new(0, 0, 20, 1), app_ctx);

            let base = TuiUiBuilder::from_app(app_ctx).warping_base_color();
            let base = Color::Rgb(base.r, base.g, base.b);
            assert_eq!(frame.buffer[(0, 0)].fg, base);
            assert_ne!(frame.buffer[(5, 0)].fg, base);
            let first_dot_color = frame.buffer[(9, 0)].fg;
            assert_eq!(frame.buffer[(10, 0)].fg, first_dot_color);
            assert_eq!(frame.buffer[(11, 0)].fg, first_dot_color);
        });
    });
}

#[test]
fn renders_a_custom_progress_label() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let builder = TuiUiBuilder::from_app(app_ctx);
            let element = render_warping_indicator_row(
                "Summarizing conversation",
                Duration::ZERO,
                TuiText::new("▶▶ Auto approve on")
                    .with_style(builder.success_glyph_style())
                    .finish(),
                app_ctx,
            );
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(element, TuiRect::new(0, 0, 100, 1), app_ctx);

            assert!(
                frame.buffer.to_lines()[0].contains(" Summarizing conversation... (0s)"),
                "unexpected indicator row: {:?}",
                frame.buffer.to_lines()[0]
            );
            assert!(frame.buffer.to_lines()[0].ends_with("▶▶ Auto approve on  Ctrl + C to stop"));
            let status_column = frame.buffer.to_lines()[0]
                .find("▶▶ Auto approve on")
                .expect("auto-approve status should render");
            assert_eq!(
                frame.buffer[(u16::try_from(status_column).unwrap(), 0)].fg,
                builder
                    .success_glyph_style()
                    .fg
                    .expect("success status should have a foreground")
            );
            assert!(frame.repaint_at.is_some());
        });
    });
}

/// Surface #15: the per-response inline summary row shows a per-response
/// dollar delta (not the conversation cumulative) alongside credits, gated
/// by `FeatureFlag::PricingTransparency`. With the flag off, the row is
/// byte-identical to pre-saga output (credits only, no dollar figure).
#[test]
fn response_summary_shows_per_response_dollar_delta_when_flag_is_on() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(true);
            let element = render_response_summary(
                Duration::from_secs(5),
                Some(0.5),
                Some(charged_usage(3.0)),
                app_ctx,
            );
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(element, TuiRect::new(0, 0, 40, 1), app_ctx);
            let line = frame.buffer.to_lines()[0].clone();
            assert_eq!(line.trim_end(), "∷ 5s • 0.5 credits ($0.03)");
        });
    });
}

#[test]
fn response_summary_omits_dollar_delta_when_flag_is_off() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(false);
            let element = render_response_summary(
                Duration::from_secs(5),
                Some(0.5),
                Some(charged_usage(3.0)),
                app_ctx,
            );
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(element, TuiRect::new(0, 0, 40, 1), app_ctx);
            let line = frame.buffer.to_lines()[0].clone();
            assert_eq!(line.trim_end(), "∷ 5s • 0.5 credits");
        });
    });
}

/// The delta changes from response to response within the same conversation
/// -- it is per-response, not the conversation cumulative.
#[test]
fn response_summary_dollar_delta_changes_between_responses() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let _guard = FeatureFlag::PricingTransparency.override_enabled(true);
            let mut presenter = TuiPresenter::new();

            let first = render_response_summary(
                Duration::from_secs(2),
                Some(0.2),
                Some(charged_usage(3.0)),
                app_ctx,
            );
            let first_line = presenter
                .present_element(first, TuiRect::new(0, 0, 40, 1), app_ctx)
                .buffer
                .to_lines()[0]
                .trim_end()
                .to_owned();

            let second = render_response_summary(
                Duration::from_secs(4),
                Some(0.8),
                Some(charged_usage(12.0)),
                app_ctx,
            );
            let second_line = presenter
                .present_element(second, TuiRect::new(0, 0, 40, 1), app_ctx)
                .buffer
                .to_lines()[0]
                .trim_end()
                .to_owned();

            assert_eq!(first_line, "∷ 2s • 0.2 credits ($0.03)");
            assert_eq!(second_line, "∷ 4s • 0.8 credits ($0.12)");
            assert_ne!(first_line, second_line);
        });
    });
}
