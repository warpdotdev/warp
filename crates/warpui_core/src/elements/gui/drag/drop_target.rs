use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use ordered_float::OrderedFloat;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use crate::elements::Point;
use crate::event::DispatchedEvent;
use crate::{AppContext, Element, EventContext, LayoutContext, PaintContext, SizeConstraint};

/// Trait to identify data that is passed to a [`crate::elements::Draggable`] when dropped on
/// a [`DropTarget`].
pub trait DropTargetData: Debug + Any {
    fn as_any(&self) -> &dyn Any;
}

/// Position for a [`DropTarget`] with the data should be passed to the
/// [`crate::elements::Draggable`] when dropped.
#[derive(Clone, Debug)]
pub(crate) struct DropTargetPosition {
    bounds: RectF,
    drop_data: Arc<dyn DropTargetData>,
}

impl DropTargetPosition {
    pub fn bounds(&self) -> RectF {
        self.bounds
    }

    pub fn data(&self) -> &Arc<dyn DropTargetData> {
        &self.drop_data
    }

    /// Returns the area encompassed by this drop target position.
    pub fn area(&self) -> OrderedFloat<f32> {
        OrderedFloat::from(self.bounds.width() * self.bounds.height())
    }
}

/// An element that marks whether a [`crate::elements::Draggable`] was dropped on top of it.
///
/// Each `DropTarget` is instantiated with data that implements the [`DropTargetData`] trait. When
/// an item is dropped on the `DropTarget`, the `Draggable` includes the data in the `on_drop`
/// callback.
pub struct DropTarget {
    child: Box<dyn Element>,
    data: Arc<dyn DropTargetData>,
}

impl DropTarget {
    pub fn new(child: Box<dyn Element>, data: impl DropTargetData + 'static) -> Self {
        Self {
            child,
            data: Arc::new(data),
        }
    }
}

impl Element for DropTarget {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut crate::AfterLayoutContext, app: &crate::AppContext) {
        self.child.after_layout(ctx, app)
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);

        let Some(bounds) = self.child.bounds() else {
            return;
        };
        ctx.position_cache
            .cache_drop_target_position(DropTargetPosition {
                bounds,
                drop_data: self.data.clone(),
            });
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }
}
