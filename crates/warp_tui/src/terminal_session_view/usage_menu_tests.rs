use warp::tui_export::{Appearance, TuiUsageCreditBar, TuiUsagePayAsYouGo, TuiUsageSnapshot};
use warpui::{App, EntityIdMap};
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
};

use super::*;
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

fn render_snapshot(snapshot: TuiUsageSnapshot) -> TuiBuffer {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let mut element = app.read(|ctx| {
            render(
                &snapshot,
                &MouseStateHandle::default(),
                &MouseStateHandle::default(),
                "https://example.com/upgrade",
                &TuiUiBuilder::from_app(ctx),
            )
        });
        render_buffer(&app, element.as_mut(), TuiSize::new(WIDTH, 40))
    })
}

fn lines(buffer: &TuiBuffer) -> Vec<String> {
    buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect()
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
fn active_pay_as_you_go_matches_the_figma_card() {
    let buffer = render_snapshot(snapshot(TuiUsagePayAsYouGo {
        credits_used: 3500,
        cost_cents: 3000,
        has_kicked_in: true,
    }));
    let rendered = lines(&buffer);

    assert!(rendered[0].starts_with(" ◔ Usage"));
    assert!(rendered[0].ends_with("Plan: Build | Team: Product Eng | Manage billing and usage"));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Spend: 3500 credits / $30.00"))
    );
    assert_eq!(buffer[(0, 0)].bg, buffer[(0, 2)].bg);
    assert!(buffer[(1, 0)].modifier.contains(Modifier::BOLD));

    let manage_column = rendered[0]
        .find("Manage billing and usage")
        .expect("manage billing link should render") as u16;
    assert!(
        buffer[(manage_column, 0)]
            .modifier
            .contains(Modifier::UNDERLINED)
    );
    assert_eq!(buffer[(manage_column, 0)].fg, buffer[(2, 0)].fg);

    let upgrade_row = rendered
        .iter()
        .position(|line| line.contains("Buy more credits"))
        .expect("upgrade link should render") as u16;
    let shortcut_column = rendered[usize::from(upgrade_row)]
        .find("(ctrl+o)")
        .expect("shortcut should render") as u16;
    assert!(
        buffer[(1, upgrade_row)]
            .modifier
            .contains(Modifier::UNDERLINED)
    );
    assert_eq!(
        buffer[(shortcut_column, upgrade_row)].fg,
        buffer[(1, 11)].fg
    );

    let footer_row = rendered
        .iter()
        .position(|line| line == "Esc to exit")
        .expect("footer should render") as u16;
    assert_eq!(buffer[(0, footer_row)].fg, buffer[(1, 2)].fg);
    assert_eq!(buffer[(4, footer_row)].fg, buffer[(1, 4)].fg);
}

#[test]
fn inactive_pay_as_you_go_matches_the_figma_copy() {
    let rendered = lines(&render_snapshot(snapshot(TuiUsagePayAsYouGo {
        credits_used: 0,
        cost_cents: 0,
        has_kicked_in: false,
    })))
    .join("\n");

    assert!(rendered.contains("Spend: 0 credits / $0"));
    assert!(!rendered.contains("$0.00"));
    assert!(rendered.contains("Kicks in after base and add-on credits are exhausted."));
}

#[test]
fn pay_as_you_go_wraps_and_formats_large_usage_like_figma() {
    let rendered = lines(&render_snapshot(snapshot(TuiUsagePayAsYouGo {
        credits_used: 60_000,
        cost_cents: 8053,
        has_kicked_in: true,
    })));
    let usage_rows: Vec<_> = rendered.iter().filter(|line| line.contains('●')).collect();

    assert_eq!(usage_rows.len(), 2);
    assert_eq!(count(usage_rows[0], '●'), 108);
    assert_eq!(count(usage_rows[1], '●'), 12);
    assert_eq!(count(usage_rows[1], '-'), 96);
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Spend: 60,000 credits / $80.53"))
    );
}

#[test]
fn sections_without_account_data_are_omitted() {
    let rendered = lines(&render_snapshot(TuiUsageSnapshot {
        plan_name: "Free".to_owned(),
        team_name: None,
        base_credits: None,
        addon_credits: None,
        pay_as_you_go: None,
        manage_billing_url: None,
    }))
    .join("\n");

    assert!(!rendered.contains("Base credits"));
    assert!(!rendered.contains("Add-on credits"));
    assert!(!rendered.contains("Pay-as-you-go"));
}
