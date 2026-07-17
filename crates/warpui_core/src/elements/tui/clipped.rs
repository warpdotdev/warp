//! [`TuiClipped`]: renders a child through a clipped viewport.
//!
//! This is the row-grid equivalent of the GUI viewport clipping/translation
//! seam: the viewport owns the scroll offset, while children render as if they
//! are unscrolled. When an item starts above the viewport, this wrapper hides
//! the child rows before the first visible row.

use super::{
    TuiBuffer, TuiClipBounds, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPresentationContext, TuiRect,
    TuiScreenPoint, TuiScreenPosition, TuiScreenRect, TuiSize,
};
use crate::AppContext;

/// A single-child wrapper that paints a clipped window into the child.
pub struct TuiClipped {
    child: Box<dyn TuiElement>,
    viewport_origin_y: u16,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiClipped {
    /// Wraps `child` without clipping rows from the top.
    pub fn new(child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            viewport_origin_y: 0,
            size: None,
            origin: None,
        }
    }

    /// Wraps an already-laid-out child with retained viewport geometry.
    pub(crate) fn from_laid_out_child(
        child: Box<dyn TuiElement>,
        viewport_origin_y: usize,
        size: TuiSize,
    ) -> Self {
        debug_assert!(
            child.size().is_some(),
            "TuiClipped child size must be retained before wrapping"
        );
        Self {
            child,
            viewport_origin_y: viewport_origin_y.min(usize::from(u16::MAX)) as u16,
            size: Some(size),
            origin: None,
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
        let size = constraint.clamp(TuiSize::new(
            child_size.width,
            child_size.height.saturating_sub(self.viewport_origin_y),
        ));
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
        let screen_origin = ctx.scene_point(origin);
        self.origin = Some(screen_origin);
        let Some(size) = self.size else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return;
        }
        let child_size = self
            .child
            .size()
            .expect("TuiClipped child size must be retained after layout");
        let child_area = TuiRect::new(
            0,
            0,
            size.width.max(child_size.width),
            self.child_height(size.height).max(child_size.height),
        );
        let mut child_buffer = TuiBuffer::empty(child_area);
        let clip = TuiScreenRect::new(screen_origin, size);
        let child_origin = origin.offset(0, -i32::from(self.viewport_origin_y));
        ctx.with_scene_layer(TuiClipBounds::BoundedByActiveLayerAnd(clip), |ctx| {
            let mut child_surface = TuiPaintSurface::mapped(&mut child_buffer, child_origin);
            self.child.render(child_origin, &mut child_surface, ctx);
        });

        for y in 0..size.height {
            let source_y = y.saturating_add(self.viewport_origin_y);
            for x in 0..size.width {
                surface.set_cell(
                    origin.offset(i32::from(x), i32::from(y)),
                    child_buffer[(x, source_y)].clone(),
                );
            }
        }
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

#[cfg(test)]
#[path = "clipped_tests.rs"]
mod tests;
