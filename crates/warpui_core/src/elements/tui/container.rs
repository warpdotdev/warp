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
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPresentationContext, TuiRect, TuiScreenPoint, TuiScreenPosition, TuiSize,
    TuiStyle,
};
use crate::AppContext;

pub struct TuiContainer {
    child: Box<dyn TuiElement>,
    padding: TuiPadding,
    border: bool,
    border_style: TuiStyle,
    background: Option<Color>,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
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
            size: None,
            origin: None,
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
        let size = constraint.clamp(size);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
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
        let area = TuiRect::new(0, 0, size.width, size.height);
        if area.is_empty() {
            return;
        }
        if self.background.is_some() || self.border {
            if let Some(bounds) = self.bounds() {
                ctx.scene.record_hit_rect(bounds);
            }
        }

        if let Some(background) = self.background {
            surface.set_style(origin, size, TuiStyle::default().bg(background));
        }

        if self.border {
            draw_border(origin, size, surface, self.painted_border_style());
        }

        let child_area = self.child_area(area);
        self.child.render(
            origin.offset(i32::from(child_area.x), i32::from(child_area.y)),
            surface,
            ctx,
        );
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, event_ctx, app)
    }
}

/// Paints a single-cell box-drawing frame around the perimeter of `size`.
fn draw_border(
    origin: TuiScreenPosition,
    size: TuiSize,
    surface: &mut TuiPaintSurface<'_>,
    style: TuiStyle,
) {
    let right = size.width.saturating_sub(1);
    let bottom = size.height.saturating_sub(1);
    let multi_column = size.width > 1;
    let multi_row = size.height > 1;

    for x in 0..size.width {
        put(surface, origin.offset(i32::from(x), 0), "─", style);
        if multi_row {
            put(
                surface,
                origin.offset(i32::from(x), i32::from(bottom)),
                "─",
                style,
            );
        }
    }
    for y in 0..size.height {
        put(surface, origin.offset(0, i32::from(y)), "│", style);
        if multi_column {
            put(
                surface,
                origin.offset(i32::from(right), i32::from(y)),
                "│",
                style,
            );
        }
    }

    put(surface, origin, "┌", style);
    if multi_column {
        put(surface, origin.offset(i32::from(right), 0), "┐", style);
    }
    if multi_row {
        put(surface, origin.offset(0, i32::from(bottom)), "└", style);
    }
    if multi_column && multi_row {
        put(
            surface,
            origin.offset(i32::from(right), i32::from(bottom)),
            "┘",
            style,
        );
    }
}

/// Writes a single styled glyph, ignoring out-of-bounds positions.
fn put(
    surface: &mut TuiPaintSurface<'_>,
    position: TuiScreenPosition,
    symbol: &str,
    style: TuiStyle,
) {
    if let Some(cell) = surface.cell_mut(position) {
        cell.set_symbol(symbol).set_style(style);
    }
}

#[cfg(test)]
#[path = "container_tests.rs"]
mod tests;
