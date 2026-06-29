//! [`TuiClipped`]: renders a top-clipped window into a child element.
//!
//! This is the TUI equivalent of the clipping/translation seam the GUI
//! scrollable stack uses around children: scroll state can stay in the viewport
//! or scrollable owner, while the wrapped child remains unaware of scrolling.
//! Today this wrapper intentionally provides only the top-row clipping needed
//! by viewported items. When the TUI needs a full clipped scrollable, this is
//! the place to build it out with richer layout, event translation, and
//! scrollbar/scroll-state integration rather than teaching leaf elements like
//! [`TuiText`](super::TuiText) to own scroll state.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::AppContext;

/// A single-child wrapper that paints a clipped window into the child.
pub struct TuiClipped {
    child: Box<dyn TuiElement>,
    skip_top_rows: u16,
}

impl TuiClipped {
    /// Wraps `child` without clipping rows from the top.
    pub fn new(child: impl TuiElement + 'static) -> Self {
        Self {
            child: Box::new(child),
            skip_top_rows: 0,
        }
    }

    /// Wraps an already-boxed child without skipping rows from the top.
    pub(crate) fn from_boxed(child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            skip_top_rows: 0,
        }
    }

    /// Starts painting after skipping `rows` logical rows from the child top.
    ///
    /// This skips child rows behind the clipping window; it is not equivalent
    /// to adding top padding, which would move the child down instead.
    pub fn with_skip_top_rows(mut self, rows: usize) -> Self {
        self.skip_top_rows = rows.min(usize::from(u16::MAX)) as u16;
        self
    }

    fn child_height(&self, visible_height: u16) -> u16 {
        visible_height.saturating_add(self.skip_top_rows)
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
            child_size.height.saturating_sub(self.skip_top_rows),
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
            let source_y = y.saturating_add(self.skip_top_rows);
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
        let y = y.checked_sub(self.skip_top_rows)?;
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
        // child its full logical area translated so `skip_top_rows`
        // aligns with the visible top: a container child then splits the
        // correct height, and a click at the visible top hits logical row
        // `skip_top_rows`.
        if let Some(position) = event.position() {
            if !area.contains_point(position) {
                return false;
            }
        }
        let child_area = TuiRect::new(
            area.x,
            area.y.saturating_sub(self.skip_top_rows),
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
