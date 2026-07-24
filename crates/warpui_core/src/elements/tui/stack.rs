//! [`TuiStack`]: layers children in the same cell-grid rectangle.
//!
//! Children share an origin and are painted in insertion order, from back to
//! front. A visually blank cell is transparent unless it has a background or
//! a modifier that makes the blank itself visible. This lets sparse elements
//! show through styled text padding without losing intentional background
//! fills.
//!
//! # Layout
//! Every child receives the same constraint and keeps its natural size. The
//! stack reports the component-wise maximum child size, clamped to that
//! constraint; it does not stretch, align, or relayout smaller children. Paint
//! is clipped to each child's retained size.
//!
//! # Paint and input
//! Children paint back-to-front in insertion order and receive distinct scene
//! layers, while events dispatch front-to-back and stop at the first handler.
//! All children share the caller's paint context, so animation repaint requests
//! propagate normally and coalesce at the earliest deadline.

use ratatui::buffer::CellWidth;

use super::{
    Cell, Color, Modifier, TuiBuffer, TuiClipBounds, TuiConstraint, TuiElement, TuiEvent,
    TuiEventContext, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPresentationContext,
    TuiRect, TuiScreenPoint, TuiScreenPosition, TuiScreenRect, TuiSize,
};
use crate::AppContext;

/// A back-to-front stack of children that share the same origin.
pub struct TuiStack {
    children: Vec<Box<dyn TuiElement>>,
    child_sizes: Vec<TuiSize>,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiStack {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            child_sizes: Vec::new(),
            size: None,
            origin: None,
        }
    }

    /// Appends a child above the children already in the stack.
    pub fn child(mut self, child: Box<dyn TuiElement>) -> Self {
        self.children.push(child);
        self
    }
}

impl Default for TuiStack {
    fn default() -> Self {
        Self::new()
    }
}

impl Extend<Box<dyn TuiElement>> for TuiStack {
    fn extend<I: IntoIterator<Item = Box<dyn TuiElement>>>(&mut self, iter: I) {
        self.children.extend(iter);
    }
}

impl TuiElement for TuiStack {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.child_sizes.clear();
        let mut content_size = TuiSize::ZERO;
        for child in &mut self.children {
            let child_size = child.layout(constraint, ctx, app);
            content_size = TuiSize::new(
                content_size.width.max(child_size.width),
                content_size.height.max(child_size.height),
            );
            self.child_sizes.push(child_size);
        }
        let size = constraint.clamp(content_size);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        for child in &mut self.children {
            child.after_layout(ctx, app);
        }
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        let screen_origin = ctx.scene_point(origin);
        self.origin = Some(screen_origin);
        let Some(size) = self.size else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return;
        }

        for (child, child_size) in self.children.iter_mut().zip(&self.child_sizes) {
            let child_size = TuiSize::new(
                child_size.width.min(size.width),
                child_size.height.min(size.height),
            );
            let child_area = TuiRect::new(0, 0, child_size.width, child_size.height);
            let mut layer = TuiBuffer::empty(child_area);
            let child_bounds = TuiScreenRect::new(screen_origin, child_size);
            ctx.with_scene_layer(
                TuiClipBounds::BoundedByActiveLayerAnd(child_bounds),
                |ctx| {
                    let mut child_surface = TuiPaintSurface::mapped(&mut layer, origin);
                    child.render(origin, &mut child_surface, ctx);
                },
            );
            composite_buffer(&layer, origin, child_size, size, surface);
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        for child in &mut self.children {
            child.present(ctx);
        }
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        for child in self.children.iter_mut().rev() {
            if child.dispatch_event(event, event_ctx, app) {
                return true;
            }
        }
        false
    }
}

/// Paints the opaque cells in `source` onto `destination`.
fn composite_buffer(
    source: &TuiBuffer,
    origin: TuiScreenPosition,
    source_size: TuiSize,
    destination_size: TuiSize,
    destination: &mut TuiPaintSurface<'_>,
) {
    for y in 0..source_size.height {
        let mut x = 0;
        while x < source_size.width {
            let cell = &source[(x, y)];
            if is_transparent(cell) {
                x = x.saturating_add(1);
                continue;
            }

            let width = cell
                .cell_width()
                .max(1)
                .min(source_size.width.saturating_sub(x));
            clear_intersecting_graphemes(destination, origin, destination_size, x, y, width);
            destination.set_cell(origin.offset(i32::from(x), i32::from(y)), cell.clone());
            for continuation in 1..width {
                destination.set_cell(
                    origin.offset(i32::from(x.saturating_add(continuation)), i32::from(y)),
                    Cell::default(),
                );
            }
            x = x.saturating_add(width);
        }
    }
}

/// Returns whether a cell has no visible pixels of its own.
fn is_transparent(cell: &Cell) -> bool {
    let blank = cell.symbol().is_empty() || cell.symbol() == " ";
    let visible_blank_modifier = cell
        .modifier
        .intersects(Modifier::REVERSED | Modifier::UNDERLINED | Modifier::CROSSED_OUT);
    blank && cell.bg == Color::Reset && !visible_blank_modifier
}

/// Clears every destination grapheme touched by the incoming cell span.
fn clear_intersecting_graphemes(
    destination: &mut TuiPaintSurface<'_>,
    origin: TuiScreenPosition,
    size: TuiSize,
    start_x: u16,
    y: u16,
    width: u16,
) {
    let end_x = start_x.saturating_add(width).min(size.width);
    let mut spans = Vec::new();
    for x in start_x..end_x {
        let span = destination_grapheme_span(destination, origin, size, x, y);
        if !spans.contains(&span) {
            spans.push(span);
        }
    }
    for (span_start, span_width) in spans {
        let span_end = span_start.saturating_add(span_width).min(size.width);
        for x in span_start..span_end {
            destination.set_cell(origin.offset(i32::from(x), i32::from(y)), Cell::default());
        }
    }
}

/// Finds the lead cell and width of the destination grapheme covering `x`.
fn destination_grapheme_span(
    destination: &TuiPaintSurface<'_>,
    origin: TuiScreenPosition,
    size: TuiSize,
    x: u16,
    y: u16,
) -> (u16, u16) {
    for lead_x in 0..=x {
        let Some(cell) = destination.cell(origin.offset(i32::from(lead_x), i32::from(y))) else {
            continue;
        };
        let width = cell
            .cell_width()
            .max(1)
            .min(size.width.saturating_sub(lead_x));
        if lead_x.saturating_add(width) > x {
            return (lead_x, width);
        }
    }
    (x, 1)
}

#[cfg(test)]
#[path = "stack_tests.rs"]
mod tests;
