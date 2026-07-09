//! [`TuiClipped`]: renders a child through a clipped viewport.
//!
//! This is the row-grid equivalent of the GUI viewport clipping/translation
//! seam: the viewport owns the scroll offset, while children render as if they
//! are unscrolled. When an item starts above the viewport, this wrapper hides
//! the child rows before the first visible row.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::AppContext;

/// A single-child wrapper that paints a clipped window into the child.
pub struct TuiClipped {
    child: Box<dyn TuiElement>,
    viewport_origin_y: u16,
}

impl TuiClipped {
    /// Wraps `child` without clipping rows from the top.
    pub fn new(child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            viewport_origin_y: 0,
        }
    }

    /// Sets the child row rendered at the top of the clipped viewport.
    ///
    /// The child still lays out and renders from its own logical row 0. The
    /// clipped viewport then copies a window out of that rendered child buffer:
    /// `viewport_origin_y` is the child row that appears at viewport y=0.
    ///
    /// ```text
    /// With viewport_origin_y = 1:
    /// =========================
    /// |                       |
    /// |      child row 0      |
    /// |                       |
    /// =========================
    /// |      viewport y=0     | <- child row 1
    /// |                       |
    /// |      viewport y=1     | <- child row 2
    /// =========================
    /// |                       |
    /// |      child row 3      |
    /// |                       |
    /// =========================
    /// ```
    pub fn with_viewport_origin_y(mut self, origin_y: usize) -> Self {
        self.viewport_origin_y = origin_y.min(usize::from(u16::MAX)) as u16;
        self
    }

    fn child_height(&self, visible_height: u16) -> u16 {
        visible_height.saturating_add(self.viewport_origin_y)
    }
}

impl TuiElement for TuiClipped {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let child_max = TuiSize::new(
            constraint.max.width,
            self.child_height(constraint.max.height),
        );
        let child_size = self.child.layout(TuiConstraint::loose(child_max), ctx, app);
        constraint.clamp(TuiSize::new(
            child_size.width,
            child_size.height.saturating_sub(self.viewport_origin_y),
        ))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        if area.is_empty() {
            return;
        }

        let child_height = self.child_height(area.height);
        let child_area = TuiRect::new(0, 0, area.width, child_height);
        let mut child_buffer = TuiBuffer::empty(child_area);
        self.child.render(child_area, &mut child_buffer, ctx);

        for y in 0..area.height {
            let source_y = y.saturating_add(self.viewport_origin_y);
            for x in 0..area.width {
                if let Some(cell) =
                    buffer.cell_mut((area.x.saturating_add(x), area.y.saturating_add(y)))
                {
                    *cell = child_buffer[(x, source_y)].clone();
                }
            }
        }
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        let child_area = TuiRect::new(area.x, area.y, area.width, self.child_height(area.height));
        let (x, y) = self.child.cursor_position(child_area, ctx)?;
        let y = y.checked_sub(self.viewport_origin_y)?;
        (y < area.height).then_some((x, y))
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        // The child was laid out at its full logical height (visible +
        // clipped rows). Filter mouse events to the visible window, then give the
        // child its full logical area translated so `viewport_origin_y`
        // aligns with the visible top: a container child then splits the
        // correct height, and a click at the visible top hits logical row
        // `viewport_origin_y`.
        if let Some(position) = event.position() {
            if !area.contains_point(position) {
                return false;
            }
        }
        let child_area = TuiRect::new(
            area.x,
            area.y.saturating_sub(self.viewport_origin_y),
            area.width,
            self.child_height(area.height),
        );
        self.child
            .dispatch_event(event, child_area, event_ctx, ctx, app)
    }
}

#[cfg(test)]
#[path = "clipped_tests.rs"]
mod tests;
