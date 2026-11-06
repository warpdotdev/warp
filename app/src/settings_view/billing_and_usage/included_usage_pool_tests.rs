use chrono::{Duration, TimeZone};
use pathfinder_geometry::vector::vec2f;
use warpui::elements::{Expanded, Flex, ParentElement, Text};
use warpui::platform::WindowStyle;
use warpui::{App, EntityIdSet, Presenter, TypedActionView, WindowInvalidation};

use super::*;
use crate::server::ids::ServerId;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 6, 0, 0, 0).unwrap()
}

#[test]
fn included_pool_credit_display_preserves_server_fractions_without_currency_conversion() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(|ctx| {
            let card = render_card(
                &PoolState::Ready(pool(8_750.0)),
                now(),
                UsageDisplayUnit::Credits,
                Appearance::as_ref(ctx),
            );
            let text = card.debug_text_content().unwrap_or_default();
            assert!(text.contains("Included team credits"));
            assert!(text.contains("984.62"));
            assert!(text.contains("/ 1234.75 remaining"));
            assert!(!text.contains('$'));
        });
    });
}

fn pool(remaining_cents: f64) -> IncludedUsagePool {
    IncludedUsagePool {
        monthly_credits: 1234.75,
        used_credits: 250.125,
        monthly_cents: 10_000,
        granted_cents: 10_000.0,
        remaining_cents,
        expires_at: Some((now() + Duration::days(1)).into()),
    }
}

#[test]
fn included_pool_card_shows_remaining_over_monthly_total() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(|ctx| {
            let card = render_card(
                &PoolState::Ready(pool(8_750.0)),
                now(),
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
            exhausted.used_credits = 1500.0;
            let card = render_card(
                &PoolState::Ready(exhausted.clone()),
                now(),
                UsageDisplayUnit::Dollars,
                Appearance::as_ref(ctx),
            );
            let text = card.debug_text_content().unwrap_or_default();
            assert!(text.contains("$0.00"));
            assert!(text.contains("/ $100.00 remaining"));
            assert!(!text.contains("Unavailable"));
            let card = render_card(
                &PoolState::Ready(exhausted),
                now(),
                UsageDisplayUnit::Credits,
                Appearance::as_ref(ctx),
            );
            let text = card.debug_text_content().unwrap_or_default();
            assert!(text.contains("Included team credits"));
            assert!(text.contains("0.00"));
            assert!(text.contains("/ 1234.75 remaining"));
        });
    });
}

fn key(id: i64) -> PoolKey {
    PoolKey {
        workspace_uid: WorkspaceUid::from(ServerId::from(id)),
        user_uid: "user".to_string(),
    }
}

#[test]
fn included_pool_keeps_remaining_separate_from_monthly_total() {
    let PoolBalance::Available {
        remaining_credits,
        monthly_credits,
        remaining_cents,
        monthly_cents,
        reset,
    } = pool_balance(&PoolState::Ready(pool(8_750.25)), now())
    else {
        panic!("Expected an available balance")
    };
    assert_eq!(remaining_cents, 8_750.25);
    assert_eq!(remaining_credits, 984.625);
    assert_eq!(monthly_credits, 1234.75);
    assert_eq!(monthly_cents, 10_000.0);
    assert!(reset.starts_with("Resets "));
}

#[test]
fn included_pool_uses_configured_monthly_total_instead_of_hardcoded_allowance() {
    let mut configured = pool(300.0);
    configured.monthly_cents = 500;
    configured.granted_cents = 500.0;
    let PoolBalance::Available {
        remaining_cents,
        monthly_cents,
        ..
    } = pool_balance(&PoolState::Ready(configured), now())
    else {
        panic!("Expected an available balance")
    };
    assert_eq!(remaining_cents, 300.0);
    assert_eq!(monthly_cents, 500.0);
}

#[test]
fn included_pool_missing_or_expired_is_not_shown_as_exhausted() {
    let now = now();
    let unavailable = PoolBalance::Unavailable;
    assert_eq!(pool_balance(&PoolState::Unavailable, now), unavailable);
    let mut expired = pool(500.0);
    expired.expires_at = Some((now - Duration::seconds(1)).into());
    assert_eq!(pool_balance(&PoolState::Ready(expired), now), unavailable);
    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
        assert_eq!(
            pool_balance(&PoolState::Ready(pool(invalid)), now),
            unavailable,
        );
        let mut invalid_credits = pool(500.0);
        invalid_credits.used_credits = invalid;
        assert_eq!(
            pool_balance(&PoolState::Ready(invalid_credits), now),
            unavailable,
        );
        let mut invalid_allowance = pool(500.0);
        invalid_allowance.monthly_credits = invalid;
        assert_eq!(
            pool_balance(&PoolState::Ready(invalid_allowance), now),
            unavailable,
        );
    }
}

#[test]
fn included_pool_loading_and_null_results_are_distinct() {
    let mut request = PoolRequest::default();
    let generation = request.begin(Some(key(1)));
    assert!(matches!(request.state, PoolState::Loading));
    request.complete(generation, Ok(None));
    assert!(matches!(request.state, PoolState::Unavailable));
    assert_ne!(
        pool_balance(&PoolState::Loading, now()),
        pool_balance(&request.state, now()),
    );
}

#[test]
fn included_pool_ignores_stale_results_after_workspace_switch_and_signout() {
    let mut request = PoolRequest::default();
    let old = request.begin(Some(key(1)));
    let current = request.begin(Some(key(2)));
    request.complete(old, Ok(Some(pool(100.0))));
    assert!(matches!(request.state, PoolState::Loading));
    request.complete(current, Ok(Some(pool(200.0))));
    assert!(matches!(request.state, PoolState::Ready(_)));
    request.begin(None);
    request.complete(current, Ok(Some(pool(200.0))));
    assert!(matches!(request.state, PoolState::Idle));
    assert_eq!(request.key, None);
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

#[test]
fn included_pool_old_server_validation_error_is_unavailable_and_retryable() {
    let mut request = PoolRequest::default();
    let generation = request.begin(Some(key(1)));
    request.complete(
        generation,
        Err(anyhow::anyhow!(
            "Cannot query field includedUsagePool on type Workspace"
        )),
    );
    assert!(matches!(request.state, PoolState::Unavailable));
    let retry = request.begin(Some(key(1)));
    request.complete(retry, Ok(Some(pool(10_000.0))));
    assert!(matches!(request.state, PoolState::Ready(_)));
}

struct CardTestView {
    state: PoolState,
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
                        &self.state, now(), UsageDisplayUnit::Dollars, appearance,
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
        for state in [
            PoolState::Loading,
            PoolState::Unavailable,
            PoolState::Ready(pool(8_750.0)),
            PoolState::Ready(pool(0.0)),
        ] {
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
