//! Generic hosting element for editor view zones.
//!
//! A view zone reserves vertical space in an editor's render tree at an anchor line and hosts an
//! app-supplied element there (see `warp_editor::render::model::ViewZone`). Zones belong to a
//! single editor view's `RenderState` — they are never part of the shared buffer, so they don't
//! serialize, copy, or edit as document content. This module provides the [`RenderableBlock`]
//! that hosts the zone's element: it lays the child out against the zone's content width, reports
//! the measured size back to the owner (so the next reconcile can reserve exactly the child's
//! height), and paints and routes events to the child.

use pathfinder_geometry::vector::{vec2f, Vector2F};
use warp_editor::render::element::{RenderContext, RenderableBlock};
use warp_editor::render::model::viewport::ViewportItem;
use warp_editor::render::model::RenderState;
use warpui::event::DispatchedEvent;
use warpui::{AppContext, Element, EventContext, LayoutContext, SizeConstraint};

/// Callback invoked with the hosted child's measured size after each layout pass.
pub(crate) type ZoneSizeWriteBack = Box<dyn Fn(Vector2F, &AppContext)>;

/// How a view zone positions itself horizontally when painting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewZoneXOrigin {
    /// Pin to the visible viewport's left edge so the zone stays in view while long content
    /// lines scroll horizontally.
    ViewportPinned,
    /// Paint in content space so the zone scrolls horizontally with surrounding content.
    Content,
}

impl ViewZoneXOrigin {
    fn origin(self, viewport_item: &ViewportItem, ctx: &RenderContext) -> Vector2F {
        let mut origin = viewport_item.content_bounds(ctx).origin();
        if self == Self::ViewportPinned {
            origin.set_x(ctx.bounds.origin_x());
        }
        origin
    }
}

/// Hosts a view zone's element inside the editor render tree. It lays out the child against the
/// zone's content width (height capped at `max_height`), writes the measured size back via
/// `write_back_size` so the next reconcile reserves exactly the child's height, and paints and
/// routes events to the child so it scrolls with its anchor line.
pub(crate) struct RenderableViewZone {
    viewport_item: ViewportItem,
    child: Box<dyn Element>,
    max_height: f32,
    x_origin: ViewZoneXOrigin,
    write_back_size: ZoneSizeWriteBack,
}

impl RenderableViewZone {
    /// Creates a hosting renderable for one view zone's child element.
    pub(crate) fn new(
        viewport_item: ViewportItem,
        child: Box<dyn Element>,
        max_height: f32,
        x_origin: ViewZoneXOrigin,
        write_back_size: ZoneSizeWriteBack,
    ) -> Self {
        Self {
            viewport_item,
            child,
            max_height,
            x_origin,
            write_back_size,
        }
    }
}

impl RenderableBlock for RenderableViewZone {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(&mut self, _model: &RenderState, ctx: &mut LayoutContext, app: &AppContext) {
        let width = self.viewport_item.content_size.x();
        let measured = self.child.layout(
            SizeConstraint::new(vec2f(0., 0.), vec2f(width, self.max_height)),
            ctx,
            app,
        );

        (self.write_back_size)(measured, app);
    }

    fn paint(&mut self, _model: &RenderState, ctx: &mut RenderContext, app: &AppContext) {
        let origin = self.x_origin.origin(&self.viewport_item, ctx);
        ctx.paint.scene.start_layer(warpui::ClipBounds::ActiveLayer);
        self.child.paint(origin, ctx.paint, app);
        ctx.paint.scene.stop_layer();
    }

    fn after_layout(&mut self, ctx: &mut warpui::AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn dispatch_event(
        &mut self,
        _model: &RenderState,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn is_view_zone(&self) -> bool {
        true
    }
}

/// A non-interactive spacer that reserves a view zone's height without hosting an element. Used
/// when the zone's hosted view can no longer be resolved.
pub(crate) struct RenderableViewZoneSpacer {
    viewport_item: ViewportItem,
}

impl RenderableViewZoneSpacer {
    /// Creates a spacer renderable for one view zone.
    pub(crate) fn new(viewport_item: ViewportItem) -> Self {
        Self { viewport_item }
    }
}

impl RenderableBlock for RenderableViewZoneSpacer {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(&mut self, _model: &RenderState, _ctx: &mut LayoutContext, _app: &AppContext) {}

    fn paint(&mut self, _model: &RenderState, _ctx: &mut RenderContext, _app: &AppContext) {}

    fn dispatch_event(
        &mut self,
        _model: &RenderState,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    fn is_view_zone(&self) -> bool {
        true
    }
}
