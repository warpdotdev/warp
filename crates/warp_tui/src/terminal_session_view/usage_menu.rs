//! Rendering for the interactive `/usage` panel.

use warp::tui_export::{TuiUsageCreditBar, TuiUsagePayAsYouGo, TuiUsageSnapshot};
use warpui_core::AppContext;
use warpui_core::elements::tui::{
    Modifier, TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiHoverable, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle,
    TuiText,
};
use warpui_core::elements::{CrossAxisAlignment, MouseStateHandle};

use crate::tui_builder::TuiUiBuilder;

const CREDITS_PER_CIRCLE: i64 = 500;
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
    total_circles: usize,
    filled_style: TuiStyle,
    empty_style: TuiStyle,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiUsagePayAsYouGoRows {
    fn new(total_circles: usize, filled_style: TuiStyle, empty_style: TuiStyle) -> Self {
        Self {
            total_circles,
            filled_style,
            empty_style,
            size: None,
            origin: None,
        }
    }
}

impl TuiElement for TuiUsagePayAsYouGoRows {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let width = usize::from(constraint.max.width.max(1));
        let rows = if self.total_circles == 0 {
            1
        } else {
            self.total_circles.div_ceil(width)
        };
        let rows = u16::try_from(rows).unwrap_or(u16::MAX);
        let size = TuiSize::new(constraint.max.width, constraint.constrain_height(rows));
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
        let width = usize::from(size.width);
        let mut remaining = self.total_circles;
        for row in 0..size.height {
            let filled_in_row = remaining.min(width);
            remaining -= filled_in_row;
            for column in 0..size.width {
                let (glyph, style) = if usize::from(column) < filled_in_row {
                    ('\u{25CF}', self.filled_style)
                } else {
                    ('-', self.empty_style)
                };
                if let Some(cell) =
                    surface.cell_mut(origin.offset(i32::from(column), i32::from(row)))
                {
                    cell.set_symbol(glyph.to_string().as_str()).set_style(style);
                }
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

const KICKED_IN_NOTE: &str = "Kicks in after credits are exhausted.";
const NOT_KICKED_IN_NOTE: &str = "Kicks in after base and add-on credits are exhausted.";

fn plain_row(text: impl Into<String>, style: TuiStyle) -> Box<dyn TuiElement> {
    TuiText::new(text.into())
        .with_style(style)
        .truncate()
        .finish()
}

fn flex_row(left: Vec<(String, TuiStyle)>, right: Vec<(String, TuiStyle)>) -> Box<dyn TuiElement> {
    TuiFlex::row()
        .child(spans_row(left))
        .flex_child(TuiText::new(String::new()).finish())
        .child(spans_row(right))
        .finish()
}

fn label_value_row(label: &str, value: &str, style: TuiStyle) -> Box<dyn TuiElement> {
    flex_row(
        vec![(label.to_owned(), style)],
        vec![(value.to_owned(), style)],
    )
}

fn spans_row(spans: Vec<(String, TuiStyle)>) -> Box<dyn TuiElement> {
    TuiText::from_spans(spans).truncate().finish()
}

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
    TuiUsageBar::new(ratio, '█', '░', filled_style, empty_style).finish()
}
fn pay_as_you_go_rows(
    credits_used: i64,
    filled_style: TuiStyle,
    empty_style: TuiStyle,
) -> Box<dyn TuiElement> {
    let total_circles = (credits_used.max(0) as f64 / CREDITS_PER_CIRCLE as f64).ceil() as usize;
    TuiUsagePayAsYouGoRows::new(total_circles, filled_style, empty_style).finish()
}

fn hoverable_link(
    text: &str,
    mouse_state: &MouseStateHandle,
    style: TuiStyle,
    url: String,
) -> Box<dyn TuiElement> {
    let is_hovered = mouse_state.lock().is_ok_and(|state| state.is_hovered());
    let mut style = style.add_modifier(Modifier::UNDERLINED);
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
    if cents == 0 {
        return "$0".to_owned();
    }
    let sign = if cents < 0 { "-" } else { "" };
    let cents = cents.unsigned_abs();
    format!(
        "{sign}${}.{:02}",
        decimal_with_commas((cents / 100) as i64),
        cents % 100
    )
}

fn decimal_with_commas(value: i64) -> String {
    if value.unsigned_abs() < 10_000 {
        return value.to_string();
    }
    let mut digits = value.unsigned_abs().to_string();
    let mut separator = digits.len();
    while separator > 3 {
        separator -= 3;
        digits.insert(separator, ',');
    }
    if value < 0 {
        digits.insert(0, '-');
    }
    digits
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

    let title_row = flex_row(
        vec![(title.to_owned(), primary)],
        vec![
            (remaining.to_string(), primary_bold),
            (" remaining".to_owned(), primary),
        ],
    );

    let credits_used_row = flex_row(
        vec![
            ("Credits used: ".to_owned(), muted),
            (format!("{}/{}", bar.used, bar.limit), primary),
        ],
        vec![(bar.note.clone(), muted)],
    );

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
    let muted = builder.read_only_menu_label_style();
    let filled = builder.link_text_style();
    let empty = muted;

    let mut rows = vec![
        label_value_row("Pay-as-you-go", "No limit", primary),
        pay_as_you_go_rows(payg.credits_used, filled, empty),
    ];
    let kicks_in_note = if payg.has_kicked_in {
        KICKED_IN_NOTE
    } else {
        NOT_KICKED_IN_NOTE
    };
    rows.push(flex_row(
        vec![
            ("Spend: ".to_owned(), muted),
            (
                format!(
                    "{} credits / {}",
                    decimal_with_commas(payg.credits_used),
                    dollars(payg.cost_cents)
                ),
                primary,
            ),
        ],
        vec![(kicks_in_note.to_owned(), muted)],
    ));
    rows
}

pub(super) fn render(
    info: &TuiUsageSnapshot,
    manage_billing_mouse: &MouseStateHandle,
    upgrade_mouse: &MouseStateHandle,
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
    let mut trailing = TuiFlex::row().child(plain_row(metadata, primary));
    if let Some(manage_billing_url) = info.manage_billing_url.clone() {
        trailing = trailing
            .child(plain_row(" | ", primary))
            .child(hoverable_link(
                "Manage billing and usage",
                manage_billing_mouse,
                primary,
                manage_billing_url,
            ));
    }
    let header = TuiContainer::new(
        TuiFlex::row()
            .child(plain_row("\u{25D4} Usage", primary_bold))
            .flex_child(TuiText::new(String::new()).finish())
            .child(trailing.finish())
            .finish(),
    )
    .with_padding_x(1)
    .with_background(builder.read_only_menu_background())
    .finish();

    let mut body = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    if let Some(base) = &info.base_credits {
        body = body.child(plain_row(" ", muted));
        for row in credit_section("Base credits", base, builder.success_glyph_style(), builder) {
            body = body.child(row);
        }
    }

    if let Some(addon) = &info.addon_credits {
        body = body.child(plain_row(" ", muted));
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
        body = body.child(plain_row(" ", muted));
        for row in pay_as_you_go_section(payg, builder) {
            body = body.child(row);
        }
    }

    body = body.child(plain_row(" ", muted));
    let upgrade_link = hoverable_link(
        "Buy more credits or upgrade plan",
        upgrade_mouse,
        primary,
        upgrade_url.to_owned(),
    );
    body = body.child(
        TuiFlex::row()
            .child(upgrade_link)
            .child(plain_row(" ", muted))
            .child(plain_row("(ctrl+o)", shortcut))
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
        .child(plain_row(" ", muted))
        .child(spans_row(vec![
            ("Esc ".to_owned(), primary),
            ("to exit".to_owned(), muted),
        ]))
        .finish()
}

#[cfg(test)]
#[path = "usage_menu_tests.rs"]
mod tests;
