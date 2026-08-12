//! Stateless projection for the `/usage` panel.
//!
//! Unlike the other read-only menus (`status_menu`, `shortcuts`, `todo_menu`),
//! this panel needs interactive links ("Manage billing and usage", "Buy more
//! credits or upgrade plan") and custom bar/circle visualizations that the
//! shared [`crate::read_only_menu::TuiReadOnlyMenu`] row model can't express,
//! so it composes its own element tree directly instead of going through that
//! component.

use warp::tui_export::{TuiUsageCreditBar, TuiUsagePayAsYouGo, TuiUsageSnapshot};
use warpui_core::elements::tui::{
    Modifier, TuiContainer, TuiElement, TuiFlex, TuiHoverable, TuiStyle, TuiText,
};
use warpui_core::elements::{CrossAxisAlignment, MouseStateHandle};

use crate::tui_builder::TuiUiBuilder;

/// Width (in cells) of the base/add-on credit bars and each pay-as-you-go
/// circle row. Fixed rather than reactive to the live terminal width,
/// matching the rest of this read-only-menu family (see
/// `status_menu`/`shortcuts`, which format rows to a fixed label width too).
/// Sized to reach the panel's own right edge in the design's 80-column
/// reference layout: 80 - 4 (the session's outer `with_padding_x(2)`) - 2
/// (this panel's own `with_padding_x(1)`) = 74.
const BAR_WIDTH: usize = 74;
/// Width of the leading label column in a `label   value` row.
const LABEL_WIDTH: usize = 40;
/// Credits represented by a single pay-as-you-go circle.
const CREDITS_PER_CIRCLE: i64 = 500;

const KICKED_IN_NOTE: &str = "Kicks in after credits are exhausted.";
const NOT_KICKED_IN_NOTE: &str = "Kicks in after base and add-on credits are exhausted.";

fn plain_row(text: impl Into<String>, style: TuiStyle) -> Box<dyn TuiElement> {
    TuiText::new(text.into())
        .with_style(style)
        .truncate()
        .finish()
}

fn label_value_row(label: &str, value: &str, style: TuiStyle) -> Box<dyn TuiElement> {
    TuiText::from_spans([(format!("{label:<LABEL_WIDTH$}{value}"), style)])
        .truncate()
        .finish()
}

fn spans_row(spans: Vec<(String, TuiStyle)>) -> Box<dyn TuiElement> {
    TuiText::from_spans(spans).truncate().finish()
}

/// Renders a fixed-width bar: `used / limit` filled cells followed by empty
/// cells for the remainder. `used` is drawn as a strict percentage of
/// `limit`, regardless of the bar's current visual state.
fn credit_bar_row(
    used: i64,
    limit: i64,
    filled_style: TuiStyle,
    empty_style: TuiStyle,
) -> Box<dyn TuiElement> {
    let ratio = if limit <= 0 {
        1.0
    } else {
        (used as f64 / limit as f64).clamp(0.0, 1.0)
    };
    let filled = ((ratio * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);
    // The filled segment is a solid block; the empty segment is a lighter,
    // dithered fill — confirmed against the designer's own screenshot.
    let mut spans = vec![("█".repeat(filled), filled_style)];
    if filled < BAR_WIDTH {
        spans.push(("░".repeat(BAR_WIDTH - filled), empty_style));
    }
    spans_row(spans)
}

/// Splits pay-as-you-go usage into rows of filled/empty circles, wrapping
/// onto as many rows as needed. Unlike the fixed-limit credit bars,
/// pay-as-you-go spend has no ceiling, so full rows have no empty remainder
/// and only the last (possibly partial) row does.
fn pay_as_you_go_rows(
    credits_used: i64,
    filled_style: TuiStyle,
    empty_style: TuiStyle,
) -> Vec<Box<dyn TuiElement>> {
    let total_circles = (credits_used.max(0) as f64 / CREDITS_PER_CIRCLE as f64).ceil() as usize;
    if total_circles == 0 {
        return vec![spans_row(vec![("-".repeat(BAR_WIDTH), empty_style)])];
    }
    let mut rows = Vec::new();
    let mut remaining = total_circles;
    while remaining > 0 {
        let filled_in_row = remaining.min(BAR_WIDTH);
        remaining -= filled_in_row;
        let mut spans = vec![("●".repeat(filled_in_row), filled_style)];
        if filled_in_row < BAR_WIDTH {
            spans.push(("-".repeat(BAR_WIDTH - filled_in_row), empty_style));
        }
        rows.push(spans_row(spans));
    }
    rows
}

fn hoverable_link(
    text: &str,
    mouse_state: &MouseStateHandle,
    builder: &TuiUiBuilder,
    url: String,
) -> Box<dyn TuiElement> {
    let is_hovered = mouse_state.lock().is_ok_and(|state| state.is_hovered());
    let mut style = builder.link_text_style().add_modifier(Modifier::UNDERLINED);
    if is_hovered {
        style = style.add_modifier(Modifier::BOLD);
    }
    TuiHoverable::new(
        mouse_state.clone(),
        TuiText::new(text.to_owned()).with_style(style).finish(),
    )
    .on_click(move |_, app| app.open_url(&url))
    .finish()
}

fn dollars(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    format!("{sign}${:.2}", (cents.unsigned_abs() as f64) / 100.0)
}

fn credit_section(
    title: &str,
    bar: &TuiUsageCreditBar,
    filled_style: TuiStyle,
    builder: &TuiUiBuilder,
) -> Vec<Box<dyn TuiElement>> {
    let primary = builder.primary_text_style();
    let primary_bold = primary.add_modifier(Modifier::BOLD);
    let muted = builder.muted_text_style();
    let empty = builder.dim_text_style();
    let remaining = (bar.limit - bar.used).max(0);

    // Only the numeric value is bold; the label and "remaining" stay regular
    // weight, per the designer's screenshot ("Base credits **400** remaining").
    let title_row = spans_row(vec![
        (format!("{title:<LABEL_WIDTH$}"), primary),
        (remaining.to_string(), primary_bold),
        (" remaining".to_owned(), primary),
    ]);

    // The "Credits used:" label is muted; only the used/limit figure is
    // brightened to match "Base credits", per the designer's screenshot.
    let used_value = format!("{}/{}", bar.used, bar.limit);
    let prefix = "Credits used: ".to_owned();
    let padding = " ".repeat(LABEL_WIDTH.saturating_sub(prefix.len() + used_value.len()));
    let credits_used_row = spans_row(vec![
        (prefix, muted),
        (used_value, primary),
        (padding, muted),
        (bar.note.clone(), muted),
    ]);

    vec![
        title_row,
        credit_bar_row(bar.used, bar.limit, filled_style, empty),
        credits_used_row,
    ]
}

fn pay_as_you_go_section(
    payg: &TuiUsagePayAsYouGo,
    builder: &TuiUiBuilder,
) -> Vec<Box<dyn TuiElement>> {
    let primary = builder.primary_text_style();
    let muted = builder.muted_text_style();
    let filled = builder.link_text_style();
    let empty = builder.dim_text_style();

    let mut rows = vec![label_value_row("Pay-as-you-go", "No limit", primary)];
    rows.extend(pay_as_you_go_rows(payg.credits_used, filled, empty));
    let kicks_in_note = if payg.has_kicked_in {
        KICKED_IN_NOTE
    } else {
        NOT_KICKED_IN_NOTE
    };
    // Same dim-label/bright-value split as "Credits used:", for consistency.
    let spend_value = format!(
        "{} credits / {}",
        payg.credits_used,
        dollars(payg.cost_cents)
    );
    let spend_prefix = "Spend: ".to_owned();
    let padding = " ".repeat(LABEL_WIDTH.saturating_sub(spend_prefix.len() + spend_value.len()));
    rows.push(spans_row(vec![
        (spend_prefix, muted),
        (spend_value, primary),
        (padding, muted),
        (kicks_in_note.to_owned(), muted),
    ]));
    rows
}

/// Builds the `/usage` panel element tree for the given snapshot. `manage_billing_mouse`
/// and `upgrade_mouse` are owned by the caller (not created inline here) so hover state
/// survives element-tree rebuilds, per the shared `MouseStateHandle` convention.
pub(super) fn render(
    info: &TuiUsageSnapshot,
    manage_billing_mouse: &MouseStateHandle,
    upgrade_mouse: &MouseStateHandle,
    upgrade_url: &str,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let primary_bold = builder.primary_text_style().add_modifier(Modifier::BOLD);
    let muted = builder.muted_text_style();
    let dim = builder.dim_text_style();
    let accent = builder.accent_text_style();

    let mut column = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    // Title on the left, metadata (and the admin-only manage-billing link)
    // flush right on the same row, matching the design's single header line.
    // The leading glyph and the " | " separators between the trailing items
    // match the design's header row exactly; there is no distinct header
    // background — the whole card shares one background with the title
    // distinguished only by bold weight, matching the `?` shortcuts menu.
    let mut metadata = format!("Plan: {}", info.plan_name);
    if let Some(team_name) = &info.team_name {
        metadata.push_str(&format!(" | Team: {team_name}"));
    }
    let mut trailing = TuiFlex::row().child(plain_row(metadata, muted));
    if let Some(manage_billing_url) = info.manage_billing_url.clone() {
        trailing = trailing
            .child(plain_row(" | ", muted))
            .child(hoverable_link(
                "Manage billing and usage",
                manage_billing_mouse,
                builder,
                manage_billing_url,
            ));
    }
    column = column.child(
        TuiFlex::row()
            .child(plain_row("\u{25D4} Usage", primary_bold))
            .flex_child(TuiText::new(String::new()).finish())
            .child(trailing.finish())
            .finish(),
    );

    if let Some(base) = &info.base_credits {
        column = column.child(plain_row(" ", muted));
        for row in credit_section("Base credits", base, builder.success_glyph_style(), builder) {
            column = column.child(row);
        }
    }

    if let Some(addon) = &info.addon_credits {
        column = column.child(plain_row(" ", muted));
        for row in credit_section(
            "Add-on credits",
            addon,
            builder.credential_entry_accent_style(),
            builder,
        ) {
            column = column.child(row);
        }
    }

    if let Some(payg) = &info.pay_as_you_go {
        column = column.child(plain_row(" ", muted));
        for row in pay_as_you_go_section(payg, builder) {
            column = column.child(row);
        }
    }

    column = column.child(plain_row(" ", muted));
    let upgrade_link = hoverable_link(
        "Buy more credits or upgrade plan",
        upgrade_mouse,
        builder,
        upgrade_url.to_owned(),
    );
    column = column.child(
        TuiFlex::row()
            .child(upgrade_link)
            .child(plain_row(" ", muted))
            .child(plain_row("(ctrl+o)", accent))
            .finish(),
    );

    let panel = TuiContainer::new(column.finish())
        .with_padding_x(1)
        .with_background(builder.read_only_menu_background())
        .finish();

    // "Esc to exit" sits outside the panel's background, on the plain
    // terminal background below it, per the design.
    TuiFlex::column()
        .child(panel)
        .child(plain_row("Esc to exit", dim))
        .finish()
}

#[cfg(test)]
#[path = "usage_menu_tests.rs"]
mod tests;
