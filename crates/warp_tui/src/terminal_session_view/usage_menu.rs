//! Rendering for the interactive `/usage` panel.

use warp::tui_export::{format_usage_cost_cents, format_usage_credits};
use warpui_core::AppContext;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    Modifier, TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle, TuiText,
};

pub(super) use self::model::TuiUsageSnapshot;
use self::model::{TuiUsageCreditBar, TuiUsagePayAsYouGo};
use crate::link::TuiLink;
use crate::tui_builder::TuiUiBuilder;
mod model;

/// Smallest credit value represented by one pay-as-you-go circle.
const MIN_CREDITS_PER_CIRCLE: u64 = 500;
/// Maximum rows available to the pay-as-you-go circle visualization.
const MAX_PAY_AS_YOU_GO_ROWS: usize = 2;
/// A one-row credit bar that fills the width assigned by its parent.
struct TuiUsageBar {
    ratio: f64,
    filled_char: char,
    empty_char: char,
    filled_style: TuiStyle,
    empty_style: TuiStyle,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiUsageBar {
    fn new(
        ratio: f64,
        filled_char: char,
        empty_char: char,
        filled_style: TuiStyle,
        empty_style: TuiStyle,
    ) -> Self {
        Self {
            ratio: ratio.clamp(0.0, 1.0),
            filled_char,
            empty_char,
            filled_style,
            empty_style,
            size: None,
            origin: None,
        }
    }
}

impl TuiElement for TuiUsageBar {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let size = TuiSize::new(constraint.max.width, constraint.constrain_height(1));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        let Some(size) = self.size else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return;
        }
        let filled = ((self.ratio * f64::from(size.width)).round() as u16).min(size.width);
        for column in 0..size.width {
            let (glyph, style) = if column < filled {
                (self.filled_char, self.filled_style)
            } else {
                (self.empty_char, self.empty_style)
            };
            if let Some(cell) = surface.cell_mut(origin.offset(i32::from(column), 0)) {
                cell.set_symbol(glyph.to_string().as_str()).set_style(style);
            }
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

struct TuiUsagePayAsYouGoRows {
    credits_used: u64,
    cost_cents: i64,
    summary_note: &'static str,
    credits_per_circle: u64,
    visible_circles: usize,
    circle_rows: u16,
    filled_style: TuiStyle,
    empty_style: TuiStyle,
    primary_style: TuiStyle,
    muted_style: TuiStyle,
    content: Option<Box<dyn TuiElement>>,
}

impl TuiUsagePayAsYouGoRows {
    fn new(
        payg: &TuiUsagePayAsYouGo,
        filled_style: TuiStyle,
        empty_style: TuiStyle,
        primary_style: TuiStyle,
        muted_style: TuiStyle,
    ) -> Self {
        Self {
            credits_used: payg.credits_used.max(0) as u64,
            cost_cents: payg.cost_cents,
            summary_note: if payg.has_kicked_in {
                "Kicks in after credits are exhausted."
            } else {
                "Kicks in after base and add-on credits are exhausted."
            },
            credits_per_circle: MIN_CREDITS_PER_CIRCLE,
            visible_circles: 0,
            circle_rows: 1,
            filled_style,
            empty_style,
            primary_style,
            muted_style,
            content: None,
        }
    }
}

impl TuiElement for TuiUsagePayAsYouGoRows {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let width = usize::from(constraint.max.width.max(1));
        let capacity = width.saturating_mul(MAX_PAY_AS_YOU_GO_ROWS);
        let minimum_credits_per_circle = self.credits_used.div_ceil(capacity as u64);
        self.credits_per_circle = MIN_CREDITS_PER_CIRCLE;
        while self.credits_per_circle < minimum_credits_per_circle {
            self.credits_per_circle = self.credits_per_circle.saturating_mul(10);
        }
        self.visible_circles = usize::try_from(self.credits_used.div_ceil(self.credits_per_circle))
            .unwrap_or(capacity)
            .min(capacity);
        let circle_rows = if self.visible_circles == 0 {
            1
        } else {
            self.visible_circles.div_ceil(width)
        };
        self.circle_rows = u16::try_from(circle_rows).unwrap_or(u16::MAX);

        let mut circle_spans = Vec::new();
        let mut remaining = self.visible_circles;
        for row in 0..self.circle_rows {
            let filled_in_row = remaining.min(width);
            remaining -= filled_in_row;
            circle_spans.push(("●".repeat(filled_in_row), self.filled_style));
            circle_spans.push((
                format!(
                    "{}{}",
                    "-".repeat(width.saturating_sub(filled_in_row)),
                    if row + 1 < self.circle_rows { "\n" } else { "" }
                ),
                self.empty_style,
            ));
        }
        let summary = TuiFlex::row()
            .child(
                TuiText::from_spans([
                    ("Spend: ".to_owned(), self.muted_style),
                    (
                        format!(
                            "{} credits / {}",
                            format_usage_credits(self.credits_used as i64),
                            format_usage_cost_cents(self.cost_cents)
                        ),
                        self.primary_style,
                    ),
                    (" • ".to_owned(), self.muted_style),
                    ("●".to_owned(), self.filled_style),
                    (
                        format!(
                            " = {} credits",
                            format_usage_credits(self.credits_per_circle as i64)
                        ),
                        self.muted_style,
                    ),
                ])
                .truncate()
                .finish(),
            )
            .flex_child(TuiText::new(String::new()).finish())
            .child(
                TuiText::new(self.summary_note)
                    .with_style(self.muted_style)
                    .truncate()
                    .finish(),
            )
            .finish();
        let mut content = TuiFlex::column()
            .child(TuiText::from_spans(circle_spans).truncate().finish())
            .child(summary)
            .finish();
        let size = content.layout(constraint, ctx, app);
        self.content = Some(content);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        if let Some(content) = self.content.as_mut() {
            content.render(origin, surface, ctx);
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.content.as_ref().and_then(|content| content.size())
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.content.as_ref().and_then(|content| content.origin())
    }
}

fn credit_section(
    title: &str,
    bar: &TuiUsageCreditBar,
    filled_style: TuiStyle,
    builder: &TuiUiBuilder,
) -> Vec<Box<dyn TuiElement>> {
    let primary = builder.primary_text_style();
    let primary_bold = primary.add_modifier(Modifier::BOLD);
    let muted = builder.read_only_menu_label_style();
    let empty = builder.usage_bar_empty_style();
    let remaining = (bar.limit - bar.used).max(0);

    let ratio = if bar.limit <= 0 {
        1.0
    } else {
        (bar.used as f64 / bar.limit as f64).clamp(0.0, 1.0)
    };
    vec![
        TuiFlex::row()
            .child(TuiText::new(title).with_style(primary).truncate().finish())
            .flex_child(TuiText::new(String::new()).finish())
            .child(
                TuiText::from_spans([
                    (remaining.to_string(), primary_bold),
                    (" remaining".to_owned(), primary),
                ])
                .truncate()
                .finish(),
            )
            .finish(),
        TuiUsageBar::new(ratio, '█', '░', filled_style, empty).finish(),
        TuiFlex::row()
            .child(
                TuiText::from_spans([
                    ("Credits used: ".to_owned(), muted),
                    (format!("{}/{}", bar.used, bar.limit), primary),
                ])
                .truncate()
                .finish(),
            )
            .flex_child(TuiText::new(String::new()).finish())
            .child(
                TuiText::new(bar.note.clone())
                    .with_style(muted)
                    .truncate()
                    .finish(),
            )
            .finish(),
    ]
}

fn pay_as_you_go_section(
    payg: &TuiUsagePayAsYouGo,
    builder: &TuiUiBuilder,
) -> Vec<Box<dyn TuiElement>> {
    let primary = builder.primary_text_style();
    let muted = builder.read_only_menu_label_style();
    let filled = builder.link_text_style();
    let empty = muted;

    vec![
        TuiFlex::row()
            .child(
                TuiText::new("Pay-as-you-go")
                    .with_style(primary)
                    .truncate()
                    .finish(),
            )
            .flex_child(TuiText::new(String::new()).finish())
            .child(
                TuiText::new("No limit")
                    .with_style(primary)
                    .truncate()
                    .finish(),
            )
            .finish(),
        TuiUsagePayAsYouGoRows::new(payg, filled, empty, primary, muted).finish(),
    ]
}

pub(super) fn render(
    info: &TuiUsageSnapshot,
    manage_billing_link: &TuiLink,
    upgrade_link: &TuiLink,
    upgrade_url: &str,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let primary = builder.primary_text_style();
    let primary_bold = primary.add_modifier(Modifier::BOLD);
    let muted = builder.read_only_menu_label_style();
    let shortcut = builder.link_text_style();

    let mut metadata = format!("Plan: {}", info.plan_name);
    if let Some(team_name) = &info.team_name {
        metadata.push_str(&format!(" | Team: {team_name}"));
    }
    let mut trailing = TuiFlex::row().child(
        TuiText::new(metadata)
            .with_style(primary)
            .truncate()
            .finish(),
    );
    if let Some(manage_billing_url) = info.manage_billing_url.clone() {
        trailing = trailing
            .child(TuiText::new(" | ").with_style(primary).truncate().finish())
            .child(manage_billing_link.render(
                "Manage billing and usage",
                primary,
                move |_, app| app.open_url(&manage_billing_url),
            ));
    }
    let header = TuiContainer::new(
        TuiFlex::row()
            .child(
                TuiText::new("\u{25D4} Usage")
                    .with_style(primary_bold)
                    .truncate()
                    .finish(),
            )
            .flex_child(TuiText::new(String::new()).finish())
            .child(trailing.finish())
            .finish(),
    )
    .with_padding_x(1)
    .with_background(builder.read_only_menu_background())
    .finish();

    let mut body = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    if let Some(base) = &info.base_credits {
        body = body.child(TuiText::new(" ").with_style(muted).truncate().finish());
        for row in credit_section("Base credits", base, builder.success_glyph_style(), builder) {
            body = body.child(row);
        }
    }

    if let Some(addon) = &info.addon_credits {
        body = body.child(TuiText::new(" ").with_style(muted).truncate().finish());
        for row in credit_section(
            "Add-on credits",
            addon,
            builder.credential_entry_accent_style(),
            builder,
        ) {
            body = body.child(row);
        }
    }

    if let Some(payg) = &info.pay_as_you_go {
        body = body.child(TuiText::new(" ").with_style(muted).truncate().finish());
        for row in pay_as_you_go_section(payg, builder) {
            body = body.child(row);
        }
    }

    body = body.child(TuiText::new(" ").with_style(muted).truncate().finish());
    let upgrade_url = upgrade_url.to_owned();
    let upgrade_link = upgrade_link.render(
        "Buy more credits or upgrade plan",
        primary,
        move |_, app| app.open_url(&upgrade_url),
    );
    body = body.child(
        TuiFlex::row()
            .child(upgrade_link)
            .child(TuiText::new(" ").with_style(muted).truncate().finish())
            .child(
                TuiText::new("(ctrl+o)")
                    .with_style(shortcut)
                    .truncate()
                    .finish(),
            )
            .finish(),
    );

    let body = TuiContainer::new(body.finish()).with_padding_x(1).finish();

    let panel = TuiContainer::new(
        TuiFlex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header)
            .child(body)
            .finish(),
    )
    .with_background(builder.read_only_menu_background())
    .finish();

    // "Esc to exit" sits outside the panel's background, on the plain
    // terminal background below it, per the design, with a blank row
    // between the panel and it.
    TuiFlex::column()
        .child(panel)
        .child(TuiText::new(" ").with_style(muted).truncate().finish())
        .child(
            TuiText::from_spans([("Esc ".to_owned(), primary), ("to exit".to_owned(), muted)])
                .truncate()
                .finish(),
        )
        .finish()
}

#[cfg(test)]
#[path = "usage_menu_tests.rs"]
mod tests;
