use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{Vector2F, vec2f};

use super::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext, Point,
    SelectableElement, Selection, SelectionFragment, SizeConstraint,
};
use crate::event::DispatchedEvent;
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{IsRect, SelectionDirection, SelectionType};

pub struct Align {
    child: Box<dyn Element>,
    alignment: Vector2F,
    size: Option<Vector2F>,
}

/// By default, Align centers a child element
impl Align {
    pub fn new(child: Box<dyn Element>) -> Self {
        Self {
            child,
            alignment: Vector2F::zero(),
            size: None,
        }
    }

    pub fn top_center(mut self) -> Self {
        self.alignment = vec2f(0.0, -1.0);
        self
    }

    pub fn top_right(mut self) -> Self {
        self.alignment = vec2f(1.0, -1.0);
        self
    }

    pub fn top_left(mut self) -> Self {
        self.alignment = vec2f(-1., -1.);
        self
    }

    pub fn bottom_center(mut self) -> Self {
        self.alignment = vec2f(0.0, 1.0);
        self
    }

    pub fn bottom_right(mut self) -> Self {
        self.alignment = vec2f(1.0, 1.0);
        self
    }

    pub fn bottom_left(mut self) -> Self {
        self.alignment = vec2f(-1., 1.0);
        self
    }

    pub fn right(mut self) -> Self {
        self.alignment = vec2f(1.0, 0.);
        self
    }

    pub fn left(mut self) -> Self {
        self.alignment = vec2f(-1., 0.);
        self
    }
}

impl Element for Align {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let mut size = constraint.max;

        let child_constraint = SizeConstraint::new(Vector2F::zero(), constraint.max);
        let child_size = self.child.layout(child_constraint, ctx, app);

        if size.x().is_infinite() {
            size.set_x(child_size.x().max(constraint.min.x()));
        }
        if size.y().is_infinite() {
            size.set_y(child_size.y().max(constraint.min.y()));
        }
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let self_center = self.size.unwrap() / 2.0;
        let self_target = self_center + self_center * self.alignment;
        let child_center = self.child.size().unwrap() / 2.0;
        let mut child_target = child_center + child_center * self.alignment;
        // Make sure the child_target cannot extend past self which may happen if child size is
        // larger than self size.
        child_target = child_target.min(self_target);
        let child_origin = origin - (child_target - self_target);
        self.child.paint(child_origin, ctx, app);
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
        self.size
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

impl SelectableElement for Align {
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
