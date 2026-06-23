//! [`TuiContainer`]: a single-child decorator that adds a background fill, an
//! optional box-drawing border, and uniform padding around its child.
//!
//! # Construction
//! Wrap a child with [`TuiContainer::new`] and layer decorations:
//! - [`with_padding`](TuiContainer::with_padding): cells of empty space on every
//!   side, inside any border.
//! - [`with_border`](TuiContainer::with_border) /
//!   [`with_border_style`](TuiContainer::with_border_style): a one-cell box-drawn
//!   frame.
//! - [`with_background`](TuiContainer::with_background): a fill color painted
//!   behind the border and padding.
//!
//! # Layout policy
//! The child is inset on every side by `border (0 or 1) + padding`. The
//! container reports its child's size grown by that inset on both axes (clamped
//! to the constraint), so the child occupies exactly the area left inside the
//! frame and padding.

use ratatui::style::Color;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize, TuiStyle,
};
use crate::{AppContext, Event};

pub struct TuiContainer {
    child: Box<dyn TuiElement>,
    padding: u16,
    border: bool,
    rounded: bool,
    border_style: TuiStyle,
    background: Option<Color>,
}

impl TuiContainer {
    pub fn new(child: impl TuiElement + 'static) -> Self {
        Self {
            child: Box::new(child),
            padding: 0,
            border: false,
            rounded: false,
            border_style: TuiStyle::default(),
            background: None,
        }
    }

    pub fn with_padding(mut self, padding: u16) -> Self {
        self.padding = padding;
        self
    }

    pub fn with_border(mut self) -> Self {
        self.border = true;
        self
    }

    /// Enables a border drawn with rounded corners (`╭╮╰╯`) instead of square
    /// ones.
    pub fn with_rounded_border(mut self) -> Self {
        self.border = true;
        self.rounded = true;
        self
    }

    pub fn with_border_style(mut self, style: TuiStyle) -> Self {
        self.border = true;
        self.border_style = style;
        self
    }

    pub fn with_background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// The number of cells the child is inset on each side.
    fn inset(&self) -> u16 {
        u16::from(self.border) + self.padding
    }

    /// The style used to paint border glyphs, inheriting the background fill so
    /// the frame sits seamlessly on the filled area.
    fn painted_border_style(&self) -> TuiStyle {
        let mut style = self.border_style;
        if style.bg.is_none() {
            style.bg = self.background;
        }
        style
    }
}

impl TuiElement for TuiContainer {
    fn layout(&mut self, constraint: TuiConstraint, ctx: &mut TuiLayoutContext) -> TuiSize {
        let total = self.inset().saturating_mul(2);
        let inner_max = TuiSize::new(
            constraint.max.width.saturating_sub(total),
            constraint.max.height.saturating_sub(total),
        );
        let inner = self.child.layout(TuiConstraint::loose(inner_max), ctx);
        let size = TuiSize::new(
            inner.width.saturating_add(total),
            inner.height.saturating_add(total),
        );
        constraint.clamp(size)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        if area.is_empty() {
            return;
        }

        if let Some(background) = self.background {
            buffer.set_style(area, TuiStyle::default().bg(background));
        }

        if self.border {
            draw_border(area, buffer, self.painted_border_style(), self.rounded);
        }

        self.child.render(area.inset(self.inset()), buffer, ctx);
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        let inset = self.inset();
        self.child
            .cursor_position(area.inset(inset), ctx)
            .map(|(x, y)| (x.saturating_add(inset), y.saturating_add(inset)))
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        if area.is_empty() {
            return false;
        }
        self.child
            .dispatch_event(event, area.inset(self.inset()), event_ctx, ctx, app)
    }
}

/// Paints a single-cell box-drawing frame around the perimeter of `area`,
/// using rounded corner glyphs when `rounded` is set.
fn draw_border(area: TuiRect, buffer: &mut TuiBuffer, style: TuiStyle, rounded: bool) {
    let right = area.right().saturating_sub(1);
    let bottom = area.bottom().saturating_sub(1);
    let multi_column = area.width > 1;
    let multi_row = area.height > 1;
    let (top_left, top_right, bottom_left, bottom_right) = if rounded {
        ("╭", "╮", "╰", "╯")
    } else {
        ("┌", "┐", "└", "┘")
    };

    for x in area.x..area.right() {
        put(buffer, x, area.y, "─", style);
        if multi_row {
            put(buffer, x, bottom, "─", style);
        }
    }
    for y in area.y..area.bottom() {
        put(buffer, area.x, y, "│", style);
        if multi_column {
            put(buffer, right, y, "│", style);
        }
    }

    put(buffer, area.x, area.y, top_left, style);
    if multi_column {
        put(buffer, right, area.y, top_right, style);
    }
    if multi_row {
        put(buffer, area.x, bottom, bottom_left, style);
    }
    if multi_column && multi_row {
        put(buffer, right, bottom, bottom_right, style);
    }
}

/// Writes a single styled glyph at `(x, y)`, ignoring out-of-bounds positions.
fn put(buffer: &mut TuiBuffer, x: u16, y: u16, symbol: &str, style: TuiStyle) {
    if let Some(cell) = buffer.cell_mut((x, y)) {
        cell.set_symbol(symbol).set_style(style);
    }
}

#[cfg(test)]
#[path = "container_tests.rs"]
mod tests;
