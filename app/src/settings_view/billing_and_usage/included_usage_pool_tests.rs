use chrono::{DateTime, Duration, TimeZone, Utc};
use pathfinder_geometry::vector::vec2f;
use warp_graphql::queries::get_request_limit_info::WorkspaceIncludedUsageInfo;
use warpui::elements::{Expanded, Flex, ParentElement, Text};
use warpui::platform::WindowStyle;
use warpui::{App, EntityIdSet, Presenter, TypedActionView, WindowInvalidation};

use super::*;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 6, 0, 0, 0).unwrap()
}

#[test]
fn included_pool_credit_display_rounds_exact_remainder_without_currency_conversion() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(|ctx| {
            let card = render_card(
                &pool(8_750.0),
                UsageDisplayUnit::Credits,
                Appearance::as_ref(ctx),
            );
            let text = card.debug_text_content().unwrap_or_default();
            assert!(text.contains("Included team credits"));
            assert!(text.contains("985"));
            assert!(text.contains("/ 1,235 remaining"));
            assert!(!text.contains('$'));
        });
    });
}

#[test]
fn included_pool_rounds_after_subtraction_and_preserves_positive_remainders() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(|ctx| {
            let mut info = pool(8750.0);
            info.included_usage_credits = 1.49;
            for (used, expected) in [(1.01, "<1 / 1 remaining"), (1.49, "0 / 1 remaining")] {
                info.usage_credits_used_since_last_refresh = used;
                let card = render_card(&info, UsageDisplayUnit::Credits, Appearance::as_ref(ctx));
                let text = card.debug_text_content().unwrap_or_default();
                assert!(text.contains(expected), "{text}");
            }
        });
    });
}

fn pool(remaining_cents: f64) -> WorkspaceIncludedUsageInfo {
    WorkspaceIncludedUsageInfo {
        included_usage_credits: 1234.75,
        usage_credits_used_since_last_refresh: 250.125,
        included_usage_cents: 10_000.0,
        usage_cents_used_since_last_refresh: 10_000.0 - remaining_cents,
        next_refresh_time: (now() + Duration::days(1)).into(),
    }
}

#[test]
fn included_pool_card_shows_remaining_over_monthly_total() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(|ctx| {
            let card = render_card(
                &pool(8_750.0),
                UsageDisplayUnit::Dollars,
                Appearance::as_ref(ctx),
            );
            let text = card.debug_text_content().unwrap_or_default();
            assert!(text.contains("$87.50"));
            assert!(text.contains("/ $100.00 remaining"));
            assert!(text.contains("Resets "));
            assert!(!text.contains("$12.50"));
            assert!(!text.contains("used of"));
        });
    });
}

#[test]
fn included_pool_exhausted_card_keeps_monthly_allowance_visible() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(|ctx| {
            let mut exhausted = pool(0.0);
            exhausted.usage_credits_used_since_last_refresh = 1500.0;
            let card = render_card(
                &exhausted.clone(),
                UsageDisplayUnit::Dollars,
                Appearance::as_ref(ctx),
            );
            let text = card.debug_text_content().unwrap_or_default();
            assert!(text.contains("$0.00"));
            assert!(text.contains("/ $100.00 remaining"));
            assert!(!text.contains("Unavailable"));
            let card = render_card(
                &exhausted,
                UsageDisplayUnit::Credits,
                Appearance::as_ref(ctx),
            );
            let text = card.debug_text_content().unwrap_or_default();
            assert!(text.contains("Included team credits"));
            assert!(text.contains("0 / 1,235 remaining"));
            assert!(text.contains("/ 1,235 remaining"));
        });
    });
}

#[test]
fn included_pool_keeps_remaining_separate_from_monthly_total() {
    let PoolBalance {
        remaining_credits,
        monthly_credits,
        remaining_cents,
        monthly_cents,
        reset,
    } = pool_balance(&pool(8_750.25));
    assert_eq!(remaining_cents, 8_750.25);
    assert_eq!(remaining_credits, 984.625);
    assert_eq!(monthly_credits, 1234.75);
    assert_eq!(monthly_cents, 10_000.0);
    assert!(reset.starts_with("Resets "));
}

#[test]
fn included_pool_uses_configured_monthly_total_instead_of_hardcoded_allowance() {
    let mut configured = pool(300.0);
    configured.included_usage_cents = 500.0;
    configured.usage_cents_used_since_last_refresh = 200.0;
    let PoolBalance {
        remaining_cents,
        monthly_cents,
        ..
    } = pool_balance(&configured);
    assert_eq!(remaining_cents, 300.0);
    assert_eq!(monthly_cents, 500.0);
}

#[test]
fn included_pool_preserves_fractional_cents_in_allowance_and_usage() {
    let mut info = pool(0.0);
    info.included_usage_cents = 500.125;
    info.usage_cents_used_since_last_refresh = 200.0625;
    let PoolBalance {
        remaining_cents,
        monthly_cents,
        ..
    } = pool_balance(&info);
    assert_eq!(monthly_cents, 500.125);
    assert_eq!(remaining_cents, 300.0625);
}

#[test]
fn included_pool_clamps_overshoot_in_each_server_unit_independently() {
    let mut info = pool(10_000.0);
    info.usage_credits_used_since_last_refresh = info.included_usage_credits + 0.125;
    info.usage_cents_used_since_last_refresh = info.included_usage_cents + 0.25;
    let PoolBalance {
        remaining_credits,
        remaining_cents,
        ..
    } = pool_balance(&info);
    assert_eq!(remaining_credits, 0.0);
    assert_eq!(remaining_cents, 0.0);
}

#[test]
fn scale_plan_identity_and_paid_plan_behavior() {
    assert_eq!(CustomerType::Scale.to_display_string(), "Scale");
    assert!(
        crate::workspaces::workspace::BillingMetadata::is_stripe_paid_plan(CustomerType::Scale)
    );
    let gql: CustomerType = warp_graphql::billing::CustomerType::Scale.into();
    assert_eq!(gql, CustomerType::Scale);
}

struct CardTestView {
    state: WorkspaceIncludedUsageInfo,
}

impl Entity for CardTestView {
    type Event = ();
}

impl TypedActionView for CardTestView {
    type Action = ();
}

impl View for CardTestView {
    fn ui_name() -> &'static str {
        "Included pool card test"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        Flex::column()
            .with_child(Text::new_inline("Balance", appearance.ui_font_family(), 16.).finish())
            .with_child(
                Flex::row()
                    .with_spacing(8.)
                    .with_child(Expanded::new(1., render_card(
                        &self.state, UsageDisplayUnit::Dollars, appearance,
                    )).finish())
                    .with_child(
                        Expanded::new(
                            1.,
                            render_balance_card(
                                appearance,
                                crate::settings_view::billing_and_usage_page_v2::BONUS_CREDITS_DOT_COLOR,
                                "Team usage",
                                "Expires Nov 02, 2027",
                                BalanceAmount::Cents(989.),
                                None,
                                appearance.theme().outline().into_solid(),
                            ),
                        ).finish(),
                    ).finish(),
            ).finish()
    }
}

#[test]
fn included_pool_card_renders_and_lays_out_at_narrow_and_wide_widths() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        for state in [pool(8_750.0), pool(0.0)] {
            let (window_id, view) =
                app.add_window(WindowStyle::NotStealFocus, |_| CardTestView { state });
            app.update(|ctx| {
                let element = view.as_ref(ctx).render(ctx);
                let text = element.debug_text_content().unwrap_or_default();
                assert!(text.contains("Included team usage"));
                assert!(text.contains("Balance"));
                assert!(text.contains("Team usage"));
                assert!(text.contains("$9.89"));
                assert!(!text.contains("Team add-on"));
                assert!(!text.contains("used of"));
                assert!(!text.contains("$0.00/$0.00"));
                let mut presenter = Presenter::new(window_id);
                for width in [320., 640., 1000.] {
                    presenter.invalidate(
                        WindowInvalidation {
                            updated: EntityIdSet::from_iter([view.id()]),
                            ..Default::default()
                        },
                        ctx,
                    );
                    let scene = presenter.build_scene(vec2f(width, 700.), 1., None, ctx);
                    assert!(scene.layers().flat_map(|layer| &layer.rects).any(|rect| {
                        rect.bounds.width() > 0.
                            && rect.bounds.height() > 0.
                            && rect.bounds.width().is_finite()
                            && rect.bounds.height().is_finite()
                    }));
                }
            });
        }
    });
}
