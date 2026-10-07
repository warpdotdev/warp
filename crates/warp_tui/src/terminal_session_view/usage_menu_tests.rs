use warp::tui_export::Appearance;
use warpui::{App, EntityIdMap};
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
};

use super::model::{TuiUsageCreditBar, TuiUsagePayAsYouGo, TuiUsageSnapshot};
use super::*;
use crate::link::TuiLink;
use crate::tui_builder::TuiUiBuilder;

const WIDTH: u16 = 110;

fn render_buffer(app: &App, element: &mut dyn TuiElement, size: TuiSize) -> TuiBuffer {
    app.read(|ctx| {
        let mut rendered_views = EntityIdMap::default();
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let size = element.layout(TuiConstraint::loose(size), &mut layout_ctx, ctx);
        let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
        buffer
    })
}

fn render_snapshot(snapshot: TuiUsageSnapshot) -> Vec<String> {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let mut element = app.read(|ctx| {
            render(
                &snapshot,
                &TuiLink::default(),
                &TuiLink::default(),
                "https://example.com/upgrade",
                &TuiUiBuilder::from_app(ctx),
            )
        });
        render_buffer(&app, element.as_mut(), TuiSize::new(WIDTH, 40))
            .to_lines()
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .collect()
    })
}

fn credit_bar(used: i64, limit: i64, note: &str) -> TuiUsageCreditBar {
    TuiUsageCreditBar {
        used,
        limit,
        note: note.to_owned(),
    }
}

fn snapshot(pay_as_you_go: TuiUsagePayAsYouGo) -> TuiUsageSnapshot {
    TuiUsageSnapshot {
        plan_name: "Build".to_owned(),
        team_name: Some("Product Eng".to_owned()),
        base_credits: Some(credit_bar(1500, 1500, "Resets July 31 at 5:00pm")),
        addon_credits: Some(credit_bar(
            100,
            500,
            "Auto-reload 500 credits July 31 at 5:00pm",
        )),
        pay_as_you_go: Some(pay_as_you_go),
        manage_billing_url: Some("https://example.com/billing".to_owned()),
    }
}

fn count(line: &str, glyph: char) -> usize {
    line.chars().filter(|candidate| *candidate == glyph).count()
}

#[test]
fn renders_account_usage_summary() {
    let rendered = render_snapshot(snapshot(TuiUsagePayAsYouGo {
        credits_used: 3500,
        cost_cents: 3000,
        has_kicked_in: true,
    }));

    assert!(rendered[0].starts_with(" ◔ Usage"));
    assert!(rendered[0].ends_with("Plan: Build | Team: Product Eng | Manage billing and usage"));
    assert!(rendered.iter().any(|line| line.contains("Base credits")));
    assert!(rendered.iter().any(|line| line.contains("Add-on credits")));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Spend: 3,500 credits / $30.00 • ● = 500 credits"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Buy more credits or upgrade plan (ctrl+o)"))
    );
    assert!(rendered.iter().any(|line| line == "Esc to exit"));
}

#[test]
fn pay_as_you_go_wraps_large_usage_across_two_rows() {
    let rendered = render_snapshot(snapshot(TuiUsagePayAsYouGo {
        credits_used: 60_000,
        cost_cents: 8053,
        has_kicked_in: true,
    }));
    let usage_rows: Vec<_> = rendered
        .iter()
        .filter(|line| count(line, '●') > 1)
        .collect();

    assert_eq!(usage_rows.len(), 2);
    assert_eq!(count(usage_rows[0], '●'), 108);
    assert_eq!(count(usage_rows[1], '●'), 12);
    assert_eq!(count(usage_rows[1], '-'), 96);
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Spend: 60,000 credits / $80.53 • ● = 500 credits"))
    );
}

#[test]
fn pay_as_you_go_scales_circle_value_at_high_usage() {
    let overflow = render_snapshot(snapshot(TuiUsagePayAsYouGo {
        credits_used: 841_138,
        cost_cents: 3_364_552,
        has_kicked_in: true,
    }));
    let overflow_rows: Vec<_> = overflow
        .iter()
        .filter(|line| count(line, '●') > 1)
        .collect();

    assert_eq!(overflow_rows.len(), 2);
    assert_eq!(count(overflow_rows[0], '●'), 108);
    assert_eq!(count(overflow_rows[1], '●'), 61);
    assert_eq!(count(overflow_rows[1], '-'), 47);
    assert!(
        overflow
            .iter()
            .any(|line| line.contains("Spend: 841,138 credits / $33,645.52 • ● = 5,000 credits"))
    );
}
