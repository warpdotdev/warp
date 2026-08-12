use warp::tui_export::{Appearance, TuiUsageCreditBar, TuiUsagePayAsYouGo, TuiUsageSnapshot};
use warpui::{App, EntityIdMap};
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{
    Color, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
};

use super::*;
use crate::tui_builder::TuiUiBuilder;

fn render_buffer(app: &App, element: &mut dyn TuiElement, size: TuiSize) -> TuiBuffer {
    app.read(|ctx| {
        let mut rendered_views = EntityIdMap::default();
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let size = element.layout(TuiConstraint::loose(size), &mut layout_ctx, ctx);
        let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        {
            let mut surface = TuiPaintSurface::new(&mut buffer);
            element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
        }
        buffer
    })
}

fn lines_of(app: &App, mut element: Box<dyn TuiElement>, size: TuiSize) -> Vec<String> {
    render_buffer(app, element.as_mut(), size)
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect()
}

fn count(line: &str, glyph: char) -> usize {
    line.chars().filter(|c| *c == glyph).count()
}

fn credit_bar(used: i64, limit: i64) -> TuiUsageCreditBar {
    TuiUsageCreditBar {
        used,
        limit,
        note: "Resets Jul 31 at 5:00 PM".to_owned(),
    }
}

fn base_snapshot() -> TuiUsageSnapshot {
    TuiUsageSnapshot {
        plan_name: "Build".to_owned(),
        team_name: Some("Product Eng".to_owned()),
        base_credits: None,
        addon_credits: None,
        pay_as_you_go: None,
        manage_billing_url: None,
    }
}

/// Renders the full `/usage` panel for `snapshot` and returns its lines.
fn render_snapshot_lines(snapshot: TuiUsageSnapshot) -> Vec<String> {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let element = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            render(
                &snapshot,
                &MouseStateHandle::default(),
                &MouseStateHandle::default(),
                "https://example.com/upgrade",
                &builder,
            )
        });
        // Wide enough that even the longest label+note combination (e.g. the
        // "Spend: ..." row paired with the longest "Kicks in..." copy) isn't
        // clipped, so these assertions test content rather than truncation.
        lines_of(&app, element, TuiSize::new(110, 30))
    })
}

#[test]
fn credit_bar_row_is_empty_at_zero_percent() {
    let row = credit_bar_row(0, 1500, TuiStyle::default(), TuiStyle::default());
    let lines = App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        lines_of(&app, row, TuiSize::new(BAR_WIDTH as u16, 1))
    });
    let line = &lines[0];
    assert_eq!(count(line, '█'), 0, "filled segment is a solid block");
    assert_eq!(count(line, '░'), BAR_WIDTH, "empty segment is light-shade");
}

#[test]
fn credit_bar_row_reflects_partial_percentage() {
    // 100/500 = 20% used, matching the designer-confirmed rule that the bar
    // fill is a strict function of credits used / limit.
    let row = credit_bar_row(100, 500, TuiStyle::default(), TuiStyle::default());
    let lines = App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        lines_of(&app, row, TuiSize::new(BAR_WIDTH as u16, 1))
    });
    let line = &lines[0];
    let expected_filled = (BAR_WIDTH as f64 * 0.2).round() as usize;
    assert_eq!(
        count(line, '█'),
        expected_filled,
        "filled segment is a solid block"
    );
    assert_eq!(
        count(line, '░'),
        BAR_WIDTH - expected_filled,
        "empty segment is light-shade"
    );
}

#[test]
fn credit_bar_row_is_full_at_limit() {
    let row = credit_bar_row(1500, 1500, TuiStyle::default(), TuiStyle::default());
    let lines = App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        lines_of(&app, row, TuiSize::new(BAR_WIDTH as u16, 1))
    });
    let line = &lines[0];
    assert_eq!(
        count(line, '█'),
        BAR_WIDTH,
        "filled segment is a solid block"
    );
    assert_eq!(count(line, '░'), 0, "no empty segment remains");
}

#[test]
fn pay_as_you_go_not_kicked_in_renders_a_single_dashed_row() {
    let rows = pay_as_you_go_rows(0, TuiStyle::default(), TuiStyle::default());
    assert_eq!(rows.len(), 1);
    let lines = App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        lines_of(
            &app,
            rows.into_iter().next().unwrap(),
            TuiSize::new(BAR_WIDTH as u16, 1),
        )
    });
    assert_eq!(count(&lines[0], '●'), 0);
    assert_eq!(count(&lines[0], '-'), BAR_WIDTH);
}

#[test]
fn pay_as_you_go_renders_one_row_of_circles_when_under_a_row() {
    // 3500 credits / 500 credits-per-circle = 7 circles, well under BAR_WIDTH.
    let rows = pay_as_you_go_rows(3500, TuiStyle::default(), TuiStyle::default());
    assert_eq!(rows.len(), 1);
    let lines = App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        lines_of(
            &app,
            rows.into_iter().next().unwrap(),
            TuiSize::new(BAR_WIDTH as u16, 1),
        )
    });
    assert_eq!(count(&lines[0], '●'), 7);
    assert_eq!(count(&lines[0], '-'), BAR_WIDTH - 7);
}

#[test]
fn pay_as_you_go_wraps_across_multiple_rows_when_it_overflows_a_row() {
    // 60,000 credits / 500 = 120 circles, which overflows a single BAR_WIDTH-wide row.
    let total_circles = 120usize;
    let rows = pay_as_you_go_rows(60_000, TuiStyle::default(), TuiStyle::default());
    let expected_rows = total_circles.div_ceil(BAR_WIDTH);
    assert_eq!(rows.len(), expected_rows);
    assert!(expected_rows > 1, "this scenario must actually wrap");

    let lines = App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        rows.into_iter()
            .map(|row| lines_of(&app, row, TuiSize::new(BAR_WIDTH as u16, 1))[0].clone())
            .collect::<Vec<_>>()
    });

    let mut remaining = total_circles;
    for (index, line) in lines.iter().enumerate() {
        let filled_in_row = remaining.min(BAR_WIDTH);
        remaining -= filled_in_row;
        assert_eq!(
            count(line, '●'),
            filled_in_row,
            "row {index} filled-circle count"
        );
        assert_eq!(
            count(line, '-'),
            BAR_WIDTH - filled_in_row,
            "row {index} dash count"
        );
    }
}

#[test]
fn render_omits_sections_the_account_does_not_have() {
    let mut snapshot = base_snapshot();
    snapshot.base_credits = Some(credit_bar(1500, 1500));
    // add-on credits and pay-as-you-go remain None.

    let text = render_snapshot_lines(snapshot).join("\n");
    assert!(text.contains("Base credits"));
    assert!(!text.contains("Add-on credits"));
    assert!(!text.contains("Pay-as-you-go"));
}

#[test]
fn render_shows_all_sections_when_all_apply() {
    let mut snapshot = base_snapshot();
    snapshot.base_credits = Some(credit_bar(1500, 1500));
    snapshot.addon_credits = Some(credit_bar(100, 500));
    snapshot.pay_as_you_go = Some(TuiUsagePayAsYouGo {
        credits_used: 3500,
        cost_cents: 3000,
        has_kicked_in: true,
    });
    snapshot.manage_billing_url = Some("https://example.com/billing".to_owned());

    let text = render_snapshot_lines(snapshot).join("\n");
    assert!(text.contains("Base credits"));
    assert!(text.contains("Add-on credits"));
    assert!(text.contains("Pay-as-you-go"));
    assert!(text.contains("Manage billing and usage"));
    assert!(text.contains("Buy more credits or upgrade plan"));
    assert!(text.contains("(ctrl+o)"));
    assert!(text.contains("Kicks in after credits are exhausted."));
    assert!(text.contains("Spend: 3500 credits / $30.00"));
    assert!(text.contains("Esc to exit"));
}

#[test]
fn header_places_title_metadata_and_manage_billing_link_on_one_row() {
    let mut snapshot = base_snapshot();
    snapshot.manage_billing_url = Some("https://example.com/billing".to_owned());

    let lines = render_snapshot_lines(snapshot);
    let header = &lines[0];
    assert!(header.contains("\u{25D4} Usage"), "{header}");
    // The trailing items are pipe-separated, matching the design's header row.
    assert!(
        header.contains("Plan: Build | Team: Product Eng | Manage billing and usage"),
        "{header}"
    );
}

#[test]
fn esc_to_exit_row_has_no_panel_background() {
    // "Esc to exit" must render outside the panel's background container, on
    // the plain terminal background — unlike every row inside the panel.
    let mut snapshot = base_snapshot();
    snapshot.base_credits = Some(credit_bar(1500, 1500));

    let buffer = App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let mut element = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            render(
                &snapshot,
                &MouseStateHandle::default(),
                &MouseStateHandle::default(),
                "https://example.com/upgrade",
                &builder,
            )
        });
        render_buffer(&app, element.as_mut(), TuiSize::new(110, 30))
    });
    let lines: Vec<String> = buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect();
    let body_row = lines
        .iter()
        .position(|line| line.contains("Base credits"))
        .expect("base credits row should render") as u16;
    let esc_row = lines
        .iter()
        .position(|line| line.trim() == "Esc to exit")
        .expect("Esc to exit row should render") as u16;

    assert_ne!(
        buffer[(0, body_row)].bg,
        Color::Reset,
        "sanity check: rows inside the panel do have a background"
    );
    assert_eq!(
        buffer[(0, esc_row)].bg,
        Color::Reset,
        "Esc to exit should render outside the panel's background"
    );
}

#[test]
fn header_places_title_and_metadata_on_one_row_without_the_manage_billing_link() {
    // Non-admins (or teams without a manage-billing link) still get a single
    // header row, just without the trailing link segment.
    let snapshot = base_snapshot();

    let lines = render_snapshot_lines(snapshot);
    let header = &lines[0];
    assert!(header.contains("Usage"), "{header}");
    assert!(
        header.contains("Plan: Build | Team: Product Eng"),
        "{header}"
    );
    assert!(!header.contains("Manage billing and usage"), "{header}");
}

#[test]
fn header_row_shares_the_same_background_as_the_panel_body() {
    // There is no separate header-row shade in the design (per the designer's
    // `?` shortcuts-menu screenshot): verify by construction, not just
    // visually, that the header can't drift from the body's background.
    let mut snapshot = base_snapshot();
    snapshot.base_credits = Some(credit_bar(1500, 1500));

    let buffer = App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let mut element = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            render(
                &snapshot,
                &MouseStateHandle::default(),
                &MouseStateHandle::default(),
                "https://example.com/upgrade",
                &builder,
            )
        });
        render_buffer(&app, element.as_mut(), TuiSize::new(110, 30))
    });
    let lines: Vec<String> = buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect();
    let body_row = lines
        .iter()
        .position(|line| line.contains("Base credits"))
        .expect("base credits row should render") as u16;
    assert_eq!(
        buffer[(0, 0)].bg,
        buffer[(0, body_row)].bg,
        "the header row and panel body must share one background"
    );
}

#[test]
fn title_row_bolds_only_the_numeric_value() {
    // "Base credits" and "remaining" stay regular weight; only the number
    // (which may be multiple digits) is bold, per the designer's screenshot.
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let mut rows = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            credit_section(
                "Base credits",
                &credit_bar(1400, 1500),
                builder.success_glyph_style(),
                &builder,
            )
        });
        let mut title_row = rows.remove(0);
        let buffer = render_buffer(&app, title_row.as_mut(), TuiSize::new(110, 1));
        assert_eq!(
            buffer.to_lines()[0].trim_end(),
            format!(
                "{:<width$}100 remaining",
                "Base credits",
                width = LABEL_WIDTH
            )
        );
        assert!(
            !buffer[(0, 0)].modifier.contains(Modifier::BOLD),
            "the label must not be bold"
        );
        for column in LABEL_WIDTH as u16..LABEL_WIDTH as u16 + 3 {
            assert!(
                buffer[(column, 0)].modifier.contains(Modifier::BOLD),
                "digit at column {column} should be bold"
            );
        }
        assert!(
            !buffer[(LABEL_WIDTH as u16 + 3, 0)]
                .modifier
                .contains(Modifier::BOLD),
            "\" remaining\" must not be bold"
        );
    });
}

#[test]
fn credits_used_row_dims_the_label_and_brightens_only_the_value() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (mut credits_used_row, muted_fg, primary_fg) = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let mut rows = credit_section(
                "Base credits",
                &credit_bar(100, 150),
                builder.success_glyph_style(),
                &builder,
            );
            (
                rows.remove(2),
                builder
                    .muted_text_style()
                    .fg
                    .expect("muted style has a foreground"),
                builder
                    .primary_text_style()
                    .fg
                    .expect("primary style has a foreground"),
            )
        });
        let buffer = render_buffer(&app, credits_used_row.as_mut(), TuiSize::new(110, 1));
        assert_eq!(
            buffer.to_lines()[0].trim_end(),
            format!(
                "Credits used: 100/150{}Resets Jul 31 at 5:00 PM",
                " ".repeat(LABEL_WIDTH - "Credits used: 100/150".len())
            )
        );
        // "Credits used: " is muted.
        for column in 0.."Credits used: ".len() as u16 {
            assert_eq!(
                buffer[(column, 0)].fg,
                muted_fg,
                "column {column} should be muted"
            );
        }
        // "100/150" is bright, matching the title row's color.
        let value_start = "Credits used: ".len() as u16;
        for column in value_start..value_start + "100/150".len() as u16 {
            assert_eq!(
                buffer[(column, 0)].fg,
                primary_fg,
                "column {column} should be bright"
            );
        }
    });
}

#[test]
fn render_grows_past_the_shared_read_only_menu_row_cap_when_pay_as_you_go_wraps() {
    // A spend large enough to wrap across many circle rows. The panel's own
    // `render` must not self-truncate: the design requires it to grow
    // vertically rather than clip, and the row cap that used to enforce a
    // ceiling lived in the caller (`terminal_session_view::render_input_area`),
    // not here — this pins down that `render` itself imposes none.
    let mut snapshot = base_snapshot();
    snapshot.base_credits = Some(credit_bar(1500, 1500));
    snapshot.pay_as_you_go = Some(TuiUsagePayAsYouGo {
        credits_used: 5_000_000,
        cost_cents: 5_000_000,
        has_kicked_in: true,
    });

    let lines = App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let element = app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            render(
                &snapshot,
                &MouseStateHandle::default(),
                &MouseStateHandle::default(),
                "https://example.com/upgrade",
                &builder,
            )
        });
        // Generous height so the constraint itself isn't what limits the
        // output — proving the content genuinely needs more than the old
        // 24-row cap when nothing artificially truncates it.
        lines_of(&app, element, TuiSize::new(110, 400))
    });
    assert!(
        lines.len() > 24,
        "expected the wrapped panel to need more than 24 rows, got {} rows",
        lines.len()
    );
    assert_eq!(
        lines.last().map(|line| line.trim()),
        Some("Esc to exit"),
        "the footer hint must still be the last row, not clipped off"
    );
}

#[test]
fn render_shows_not_kicked_in_copy_before_any_pay_as_you_go_spend() {
    let mut snapshot = base_snapshot();
    snapshot.base_credits = Some(credit_bar(1500, 1500));
    snapshot.addon_credits = Some(credit_bar(100, 500));
    snapshot.pay_as_you_go = Some(TuiUsagePayAsYouGo {
        credits_used: 0,
        cost_cents: 0,
        has_kicked_in: false,
    });

    let text = render_snapshot_lines(snapshot).join("\n");
    assert!(text.contains("Kicks in after base and add-on credits are exhausted."));
    assert!(text.contains("Spend: 0 credits / $0.00"));
}
