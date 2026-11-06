use chrono::Duration;
use pathfinder_geometry::vector::vec2f;
use warpui::platform::WindowStyle;
use warpui::{App, EntityIdSet, Presenter, TypedActionView, WindowInvalidation};

use super::*;
use crate::server::ids::ServerId;

fn pool(remaining_cents: f64) -> IncludedUsagePool {
    IncludedUsagePool {
        monthly_cents: 10_000,
        granted_cents: 10_000.0,
        remaining_cents,
        expires_at: Some((Utc::now() + Duration::days(1)).into()),
    }
}

fn key(id: i64) -> PoolKey {
    PoolKey {
        workspace_uid: WorkspaceUid::from(ServerId::from(id)),
        user_uid: "user".to_string(),
    }
}

#[test]
fn included_pool_uses_server_amounts_without_individual_allowance_or_addons() {
    for (remaining, balance, remaining_label) in [
        (10_000.0, "$0.00 used of $100.00", "$100.00 remaining"),
        (8_750.0, "$12.50 used of $100.00", "$87.50 remaining"),
        (0.0, "$100.00 used of $100.00", "$0.00 remaining"),
        (8_750.25, "$12.50 used of $100.00", "$87.50 remaining"),
    ] {
        let labels = pool_labels(&PoolState::Ready(pool(remaining)), Utc::now());
        assert_eq!(labels.balance, balance);
        assert_eq!(labels.remaining.as_deref(), Some(remaining_label));
        assert!(labels.reset.unwrap().starts_with("Resets "));
    }
    let mut configured = pool(300.0);
    configured.monthly_cents = 500;
    configured.granted_cents = 500.0;
    assert_eq!(
        pool_labels(&PoolState::Ready(configured), Utc::now()).balance,
        "$2.00 used of $5.00",
    );
}

#[test]
fn included_pool_missing_or_expired_is_not_shown_as_exhausted() {
    let now = Utc::now();
    let unavailable = pool_labels(&PoolState::Unavailable, now);
    assert!(unavailable.balance.contains("unavailable"));
    assert_eq!(unavailable.remaining, None);
    let mut expired = pool(500.0);
    expired.expires_at = Some((now - Duration::seconds(1)).into());
    assert_eq!(pool_labels(&PoolState::Ready(expired), now), unavailable);
    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
        assert_eq!(
            pool_labels(&PoolState::Ready(pool(invalid)), now),
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
        pool_labels(&PoolState::Loading, Utc::now()).balance,
        pool_labels(&request.state, Utc::now()).balance,
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
    refresh_button: MouseStateHandle,
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
        render_card(
            &self.state,
            Utc::now(),
            self.refresh_button.clone(),
            Appearance::as_ref(ctx),
        )
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
            let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| CardTestView {
                state,
                refresh_button: MouseStateHandle::default(),
            });
            app.update(|ctx| {
                let element = view.as_ref(ctx).render(ctx);
                let text = element.debug_text_content().unwrap_or_default();
                assert!(text.contains("Included team usage"));
                assert!(text.contains("Resets monthly without rollover"));
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
