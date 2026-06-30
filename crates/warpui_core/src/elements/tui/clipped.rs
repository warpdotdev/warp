//! [`TuiClipped`]: renders a child through a clipped viewport.
//!
//! This is the row-grid equivalent of the GUI viewport clipping/translation
//! seam: the viewport owns the scroll offset, while children render as if they
//! are unscrolled. When an item starts above the viewport, this wrapper hides
//! the child rows before the first visible row.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::AppContext;

/// A single-child wrapper that paints a clipped window into the child.
pub struct TuiClipped {
    child: Box<dyn TuiElement>,
    hidden_top_rows: u16,
}

impl TuiClipped {
    /// Wraps `child` without clipping rows from the top.
    pub fn new(child: impl TuiElement + 'static) -> Self {
        Self {
            child: Box::new(child),
            hidden_top_rows: 0,
        }
    }

    /// Wraps an already-boxed child without skipping rows from the top.
    pub(crate) fn from_boxed(child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            hidden_top_rows: 0,
        }
    }

    /// Hides `rows` logical rows above the clipped viewport.
    ///
    /// This clips child rows above the viewport; it is not equivalent
    /// to adding top padding, which would move the child down instead.
    pub fn with_hidden_top_rows(mut self, rows: usize) -> Self {
        self.hidden_top_rows = rows.min(usize::from(u16::MAX)) as u16;
        self
    }

    fn child_height(&self, visible_height: u16) -> u16 {
        visible_height.saturating_add(self.hidden_top_rows)
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
            child_size.height.saturating_sub(self.hidden_top_rows),
        ))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        if area.is_empty() {
            return;
        }

        let child_height = self.child_height(area.height);
        let child_area = TuiRect::new(0, 0, area.width, child_height);
        let mut child_buffer = TuiBuffer::empty(child_area);
        self.child.render(child_area, &mut child_buffer, ctx);

        for y in 0..area.height {
            let source_y = y.saturating_add(self.hidden_top_rows);
            for x in 0..area.width {
                if let Some(cell) =
                    buffer.cell_mut((area.x.saturating_add(x), area.y.saturating_add(y)))
                {
                    *cell = child_buffer[(x, source_y)].clone();
                }
            }
        }
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        let child_area = TuiRect::new(area.x, area.y, area.width, self.child_height(area.height));
        let (x, y) = self.child.cursor_position(child_area, ctx)?;
        let y = y.checked_sub(self.hidden_top_rows)?;
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
        // child its full logical area translated so `hidden_top_rows`
        // aligns with the visible top: a container child then splits the
        // correct height, and a click at the visible top hits logical row
        // `hidden_top_rows`.
        if let Some(position) = event.position() {
            if !area.contains_point(position) {
                return false;
            }
        }
        let child_area = TuiRect::new(
            area.x,
            area.y.saturating_sub(self.hidden_top_rows),
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
