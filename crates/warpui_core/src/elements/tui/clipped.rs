//! [`TuiClipped`]: renders a vertically offset window into a child element.
//!
//! This is the TUI equivalent of the clipping/translation seam the GUI
//! scrollable stack uses around children: scroll state can stay in the viewport
//! or scrollable owner, while the wrapped child remains unaware of scrolling.
//! Today this wrapper intentionally provides only the row-offset clipping needed
//! by viewported items. When the TUI needs a full clipped scrollable, this is
//! the place to build it out with richer layout, event translation, and
//! scrollbar/scroll-state integration rather than teaching leaf elements like
//! [`TuiText`](super::TuiText) to own scroll state.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::{AppContext, Event};

/// A single-child wrapper that paints from `vertical_offset` rows into the child.
pub struct TuiClipped<E> {
    child: E,
    vertical_offset: u16,
}

impl<E: TuiElement> TuiClipped<E> {
    /// Wraps `child` with no initial offset.
    pub fn new(child: E) -> Self {
        Self {
            child,
            vertical_offset: 0,
        }
    }

    /// Starts painting at `rows` logical rows into the child.
    pub fn with_vertical_offset(mut self, rows: usize) -> Self {
        self.vertical_offset = rows.min(usize::from(u16::MAX)) as u16;
        self
    }

    fn child_height(&self, visible_height: u16) -> u16 {
        visible_height.saturating_add(self.vertical_offset)
    }
}

impl<E: TuiElement> TuiElement for TuiClipped<E> {
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
            child_size.height.saturating_sub(self.vertical_offset),
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
            let source_y = y.saturating_add(self.vertical_offset);
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
        let y = y.checked_sub(self.vertical_offset)?;
        (y < area.height).then_some((x, y))
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        // The child was laid out at its full logical height (visible +
        // offset). Filter mouse events to the visible window, then give the
        // child its full logical area translated so logical row `offset`
        // aligns with the visible top: a container child then splits the
        // correct height, and a click at the visible top hits logical row
        // `offset`.
        if let Some(position) = event.position() {
            if !area.contains_point(position) {
                return false;
            }
        }
        let child_area = TuiRect::new(
            area.x,
            area.y.saturating_sub(self.vertical_offset),
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
