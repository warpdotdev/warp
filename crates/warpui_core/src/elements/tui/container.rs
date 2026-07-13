//! [`TuiContainer`]: a single-child decorator that adds a background fill, an
//! optional box-drawing border, and padding around its child.
//!
//! # Construction
//! Wrap a child with [`TuiContainer::new`] and layer decorations:
//! - [`with_padding`](TuiContainer::with_padding): cells of empty space on every
//!   side, inside any border.
//! - [`with_padding_x`](TuiContainer::with_padding_x) /
//!   [`with_padding_y`](TuiContainer::with_padding_y): cells of empty space on
//!   one axis.
//! - [`with_padding_top`](TuiContainer::with_padding_top) and sibling side
//!   methods: cells of empty space on one side.
//! - [`with_border`](TuiContainer::with_border) /
//!   [`with_border_style`](TuiContainer::with_border_style): a one-cell box-drawn
//!   frame.
//! - [`with_background`](TuiContainer::with_background): a fill color painted
//!   behind the border and padding.
//!
//! # Layout policy
//! The child is inset on every side by `border (0 or 1) + side padding`. The
//! container reports its child's size grown by those insets (clamped to the
//! constraint), so the child occupies exactly the area left inside the frame and
//! padding.

use ratatui::style::Color;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPresentationContext, TuiRect, TuiSize, TuiStyle,
};
use crate::AppContext;

pub struct TuiContainer {
    child: Box<dyn TuiElement>,
    padding: TuiPadding,
    border: bool,
    border_style: TuiStyle,
    background: Option<Color>,
}

#[derive(Clone, Copy, Default)]
struct TuiPadding {
    top: u16,
    right: u16,
    bottom: u16,
    left: u16,
}

impl TuiPadding {
    /// Creates equal padding on every side.
    fn uniform(padding: u16) -> Self {
        Self {
            top: padding,
            right: padding,
            bottom: padding,
            left: padding,
        }
    }
}

impl TuiContainer {
    pub fn new(child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            padding: TuiPadding::default(),
            border: false,
            border_style: TuiStyle::default(),
            background: None,
        }
    }

    pub fn with_padding(mut self, padding: u16) -> Self {
        self.padding = TuiPadding::uniform(padding);
        self
    }

    /// Sets horizontal padding on both left and right sides.
    pub fn with_padding_x(mut self, padding: u16) -> Self {
        self.padding.left = padding;
        self.padding.right = padding;
        self
    }

    /// Sets vertical padding on both top and bottom sides.
    pub fn with_padding_y(mut self, padding: u16) -> Self {
        self.padding.top = padding;
        self.padding.bottom = padding;
        self
    }

    /// Sets padding above the child.
    pub fn with_padding_top(mut self, padding: u16) -> Self {
        self.padding.top = padding;
        self
    }

    /// Sets padding to the right of the child.
    pub fn with_padding_right(mut self, padding: u16) -> Self {
        self.padding.right = padding;
        self
    }

    /// Sets padding below the child.
    pub fn with_padding_bottom(mut self, padding: u16) -> Self {
        self.padding.bottom = padding;
        self
    }

    /// Sets padding to the left of the child.
    pub fn with_padding_left(mut self, padding: u16) -> Self {
        self.padding.left = padding;
        self
    }

    pub fn with_border(mut self) -> Self {
        self.border = true;
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

    /// The child inset from the left edge.
    fn left_inset(&self) -> u16 {
        u16::from(self.border).saturating_add(self.padding.left)
    }

    /// The child inset from the right edge.
    fn right_inset(&self) -> u16 {
        u16::from(self.border).saturating_add(self.padding.right)
    }

    /// The child inset from the top edge.
    fn top_inset(&self) -> u16 {
        u16::from(self.border).saturating_add(self.padding.top)
    }

    /// The child inset from the bottom edge.
    fn bottom_inset(&self) -> u16 {
        u16::from(self.border).saturating_add(self.padding.bottom)
    }

    /// The total horizontal space reserved by border and padding.
    fn horizontal_inset(&self) -> u16 {
        self.left_inset().saturating_add(self.right_inset())
    }

    /// The total vertical space reserved by border and padding.
    fn vertical_inset(&self) -> u16 {
        self.top_inset().saturating_add(self.bottom_inset())
    }

    /// The area available to the child after border and padding.
    fn child_area(&self, area: TuiRect) -> TuiRect {
        let left = self.left_inset().min(area.width);
        let top = self.top_inset().min(area.height);
        let right = self.right_inset().min(area.width.saturating_sub(left));
        let bottom = self.bottom_inset().min(area.height.saturating_sub(top));
        TuiRect::new(
            area.x.saturating_add(left),
            area.y.saturating_add(top),
            area.width.saturating_sub(left).saturating_sub(right),
            area.height.saturating_sub(top).saturating_sub(bottom),
        )
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
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let inner_max = TuiSize::new(
            constraint.max.width.saturating_sub(self.horizontal_inset()),
            constraint.max.height.saturating_sub(self.vertical_inset()),
        );
        let inner = self.child.layout(TuiConstraint::loose(inner_max), ctx, app);
        let size = TuiSize::new(
            inner.width.saturating_add(self.horizontal_inset()),
            inner.height.saturating_add(self.vertical_inset()),
        );
        constraint.clamp(size)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        if area.is_empty() {
            return;
        }

        if let Some(background) = self.background {
            buffer.set_style(area, TuiStyle::default().bg(background));
        }

        if self.border {
            draw_border(area, buffer, self.painted_border_style());
        }

        self.child.render(self.child_area(area), buffer, ctx);
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        // `cursor_position` is reported relative to `area`'s origin, but the
        // child reports relative to the child area. Add the child-area offset
        // back so the cursor lands inside the border/padding.
        let child_area = self.child_area(area);
        self.child.cursor_position(child_area, ctx).map(|(cx, cy)| {
            (
                child_area.x.saturating_sub(area.x).saturating_add(cx),
                child_area.y.saturating_sub(area.y).saturating_add(cy),
            )
        })
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        if area.is_empty() {
            return false;
        }
        self.child
            .dispatch_event(event, self.child_area(area), event_ctx, ctx, app)
    }
}

/// Paints a single-cell box-drawing frame around the perimeter of `area`.
fn draw_border(area: TuiRect, buffer: &mut TuiBuffer, style: TuiStyle) {
    let right = area.right().saturating_sub(1);
    let bottom = area.bottom().saturating_sub(1);
    let multi_column = area.width > 1;
    let multi_row = area.height > 1;

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

    put(buffer, area.x, area.y, "┌", style);
    if multi_column {
        put(buffer, right, area.y, "┐", style);
    }
    if multi_row {
        put(buffer, area.x, bottom, "└", style);
    }
    if multi_column && multi_row {
        put(buffer, right, bottom, "┘", style);
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
