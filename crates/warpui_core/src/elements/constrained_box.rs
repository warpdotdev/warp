use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use super::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext, Point,
    SelectableElement, Selection, SelectionFragment, SizeConstraint,
};
use crate::event::DispatchedEvent;
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{IsRect, SelectionDirection, SelectionType};

pub struct ConstrainedBox {
    child: Box<dyn Element>,
    constraint: SizeConstraint,
}

impl ConstrainedBox {
    pub fn new(child: Box<dyn Element>) -> Self {
        Self {
            child,
            constraint: SizeConstraint {
                min: Vector2F::zero(),
                max: Vector2F::splat(f32::INFINITY),
            },
        }
    }

    pub fn with_max_width(mut self, max_width: f32) -> Self {
        self.constraint.max.set_x(max_width);
        self
    }

    pub fn with_min_width(mut self, min_width: f32) -> Self {
        self.constraint.min.set_x(min_width);
        self
    }

    pub fn with_max_height(mut self, max_height: f32) -> Self {
        self.constraint.max.set_y(max_height);
        self
    }

    pub fn with_min_height(mut self, min_height: f32) -> Self {
        self.constraint.min.set_y(min_height);
        self
    }

    pub fn with_height(mut self, height: f32) -> Self {
        self.constraint.min.set_y(height);
        self.constraint.max.set_y(height);
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.constraint.min.set_x(width);
        self.constraint.max.set_x(width);
        self
    }
}

impl Element for ConstrainedBox {
    fn layout(
        &mut self,
        mut constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        constraint.min = constraint.min.max(self.constraint.min);
        constraint.max = constraint.max.min(self.constraint.max);
        constraint.min = constraint.min.min(constraint.max);

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

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}

impl SelectableElement for ConstrainedBox {
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
