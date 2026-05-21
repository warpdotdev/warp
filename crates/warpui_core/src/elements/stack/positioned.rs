use std::any::Any;

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use super::OffsetPositioning;
use crate::elements::{Point, SelectableElement, Selection, SelectionFragment};
use crate::event::DispatchedEvent;
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{IsRect, SelectionDirection, SelectionType};
use crate::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

pub(super) struct Positioned {
    child: Box<dyn Element>,
    parent_data: OffsetPositioning,
}

impl Positioned {
    pub(super) fn new(child: Box<dyn Element>) -> Self {
        Self {
            child,
            parent_data: OffsetPositioning::default(),
        }
    }

    pub(super) fn with_offset(mut self, positioning: OffsetPositioning) -> Self {
        self.parent_data = positioning;
        self
    }
}

impl Element for Positioned {
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
        self.child.paint(origin, ctx, app);
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn parent_data(&self) -> Option<&dyn Any> {
        Some(&self.parent_data)
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }
}

impl SelectableElement for Positioned {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.get_selection(selection_start, selection_end, is_rect)
            })
    }

    fn expand_selection(
        &self,
        point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.expand_selection(point, direction, unit, word_boundaries_policy)
            })
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.is_point_semantically_before(absolute_point, absolute_point_other)
            })
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        smart_select_fn: crate::elements::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        self.child
            .as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.smart_select(absolute_point, smart_select_fn)
            })
    }

    fn calculate_clickable_bounds(&self, current_selection: Option<Selection>) -> Vec<RectF> {
        self.child
            .as_selectable_element()
            .map(|selectable_child| selectable_child.calculate_clickable_bounds(current_selection))
            .unwrap_or_default()
    }
}
