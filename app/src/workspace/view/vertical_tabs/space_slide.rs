//! Horizontal slide of the vertical tab list when the active space changes.

use std::cmp::Ordering;
use std::time::Duration;

use instant::Instant;
use warpui::elements::Point;
use warpui::event::DispatchedEvent;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

/// How long a space's tab list takes to slide into place.
pub(crate) const DURATION: Duration = Duration::from_millis(200);

/// Repaint interval while sliding. The platform coalesces redraws to the display refresh, so this
/// only needs to be short enough for 120 Hz displays.
const FRAME_INTERVAL: Duration = Duration::from_millis(8);

/// The panel edge a space's tab list slides in from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlideFrom {
    Left,
    Right,
}

/// A slide of the active space's tab list that started when the space was switched to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SpaceSlide {
    pub(crate) from: SlideFrom,
    pub(crate) started_at: Instant,
}

impl SpaceSlide {
    /// The slide for switching from the space at `from_index` to the one at `to_index`: spaces
    /// further right in the switcher come in from the right, like the chips' order. `None` when
    /// the index doesn't change.
    pub(crate) fn between(from_index: usize, to_index: usize, started_at: Instant) -> Option<Self> {
        let from = match to_index.cmp(&from_index) {
            Ordering::Greater => SlideFrom::Right,
            Ordering::Less => SlideFrom::Left,
            Ordering::Equal => return None,
        };
        Some(Self { from, started_at })
    }

    /// Horizontal offset of the list at `now` for a list `width` wide, or `None` once the slide
    /// has finished.
    pub(crate) fn offset_at(&self, width: f32, now: Instant) -> Option<f32> {
        let elapsed = now.saturating_duration_since(self.started_at);
        if elapsed >= DURATION {
            return None;
        }
        let distance_left =
            width * (1. - ease_out_cubic(elapsed.as_secs_f32() / DURATION.as_secs_f32()));
        Some(match self.from {
            SlideFrom::Right => distance_left,
            SlideFrom::Left => -distance_left,
        })
    }
}

/// Cubic ease-out of `progress` in `[0, 1]`: fast start, gentle landing.
pub(crate) fn ease_out_cubic(progress: f32) -> f32 {
    1. - (1. - progress.clamp(0., 1.)).powi(3)
}

/// Paints its child shifted horizontally by an in-progress [`SpaceSlide`] and keeps requesting
/// frames until the slide ends; afterwards (or without a slide) it paints the child in place and
/// requests nothing. The parent clips: the tab list's scrollable already clips to the panel.
pub(crate) struct SpaceSlideElement {
    child: Box<dyn Element>,
    slide: Option<SpaceSlide>,
}

impl SpaceSlideElement {
    pub(crate) fn new(child: Box<dyn Element>, slide: Option<SpaceSlide>) -> Self {
        Self { child, slide }
    }
}

impl Element for SpaceSlideElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let width = self.child.size().map_or(0., |size| size.x());
        let offset = self
            .slide
            .and_then(|slide| slide.offset_at(width, Instant::now()));
        match offset {
            Some(offset) => {
                self.child.paint(origin + vec2f(offset, 0.), ctx, app);
                ctx.repaint_after(FRAME_INTERVAL);
            }
            None => {
                // Redraws reuse this element until the next render, so stop checking the clock.
                self.slide = None;
                self.child.paint(origin, ctx, app);
            }
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    // The child's (shifted) geometry, so a scrollable parent still finds the rows it scrolls to
    // inside this element mid-slide.
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn bounds(&self) -> Option<RectF> {
        self.child.bounds()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    #[cfg(test)]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }

    #[cfg(test)]
    fn debug_child_view_ids(&self) -> Vec<warpui::EntityId> {
        self.child.debug_child_view_ids()
    }
}

#[cfg(test)]
#[path = "space_slide_tests.rs"]
mod tests;
