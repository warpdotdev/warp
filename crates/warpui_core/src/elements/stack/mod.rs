//! Stack lets you render multiple elements "on top of each other". It lets you offset elements
//! from the parent element or between different layers in one Stack, etc.
//!
//! Stacks order their elements in z-space using layers.
//! E.g.
//! stack 1
//!   --> start layer 1
//!   child 1
//!   <-- stop layer 1
//!   --> start layer 2
//!   stack 2
//!     --> start layer 3
//!     child 2
//!     <-- stop layer 3
//!     --> start layer 4
//!     child 3
//!     <-- stop layer 4
//!   <-- stop layer 2
//!   --> start layer 5
//!   child 4
//!   <-- stop layer 5
//!
//! Note that by default, all Stack's children contribute to its final size computation. For
//! example, if you wanted to render a small square, and then a bigger translucent square that
//! covers it.
//! However, if you'd rather have them arranged differently, use `Stack::add_positioned_child`. The
//! simple way of thinking about the two is that using `Stack::add_child` renders a stack as if all
//! the layers were "merged" together, while positioned children are rendered as separate layers.
//! More context here: https://medium.flutterdevs.com/stack-and-positioned-widget-in-flutter-3d1a7b30b09a

mod offset_positioning;
mod overlay;
mod positioned;
mod save_position;

pub use offset_positioning::*;
use overlay::Overlay;
use pathfinder_geometry::rect::RectF;
use positioned::*;
pub use save_position::*;

use crate::{
    event::DispatchedEvent,
    text::{word_boundaries::WordBoundariesPolicy, IsRect, SelectionDirection, SelectionType},
};

use super::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext, Point,
    SelectableElement, Selection, SelectionFragment, SizeConstraint,
};
use crate::ClipBounds;
use log::warn;
use pathfinder_geometry::vector::{vec2f, Vector2F};

#[derive(Clone, Copy, Default)]
pub enum EventDispatchMode {
    /// Current behavior: dispatch event to every child regardless of
    /// whether a prior child already handled it.
    #[default]
    Broadcast,
    /// Waterfall: stop dispatching to subsequent children once one
    /// reports the event as handled.
    Waterfall,
}

struct StackChild {
    element: Box<dyn Element>,
    painted: bool,
}

impl StackChild {
    fn new(element: Box<dyn Element>) -> Self {
        Self {
            element,
            painted: false,
        }
    }
}

#[derive(Default)]
pub struct Stack {
    children: Vec<StackChild>,
    size: Option<Vector2F>,
    origin: Option<Point>,
    constrain_absolute_children: bool,
    event_dispatch_mode: EventDispatchMode,
}

/// Since this is in the UI package, I can't access feature flags.
/// We can flip this bool to disable this functionality if we
/// run into any other regressions. When false, all stacks will constrain
/// their absolute positioned children, which is behavior we'd like to move
/// away from.
const SHOULD_ENABLE_NEW_STACK_CONSTRAINT_BEHAVIOR: bool = true;

impl Stack {
    pub fn new() -> Self {
        Stack {
            children: Default::default(),
            size: Default::default(),
            origin: Default::default(),
            constrain_absolute_children: !SHOULD_ENABLE_NEW_STACK_CONSTRAINT_BEHAVIOR,
            event_dispatch_mode: if cfg!(debug_assertions) {
                EventDispatchMode::Waterfall
            } else {
                EventDispatchMode::Broadcast
            },
        }
    }

    pub fn with_constrain_absolute_children(mut self) -> Self {
        self.constrain_absolute_children = true;
        self
    }

    pub fn with_event_dispatch_mode(mut self, mode: EventDispatchMode) -> Self {
        self.event_dispatch_mode = mode;
        self
    }

    /// Add a new child to the stack with a specific positioning.
    pub fn with_positioned_child(
        mut self,
        child: Box<dyn Element>,
        positioning: OffsetPositioning,
    ) -> Self {
        self.add_positioned_child(child, positioning);
        self
    }

    /// Add a new child to the stack with a specific positioning.
    pub fn add_positioned_child(
        &mut self,
        child: Box<dyn Element>,
        positioning: OffsetPositioning,
    ) {
        self.extend(Some(
            Positioned::new(child).with_offset(positioning).finish(),
        ));
    }

    /// Add a new child to the stack as an overlay
    ///
    /// The child (and its children) will be layered above the normal UI elements. This will allow
    /// it to float above the rest of the UI—useful for things like dropdowns and menus. The new
    /// layer will be unclipped by default.
    pub fn add_overlay_child(&mut self, child: Box<dyn Element>) {
        self.extend(Some(Overlay::new(child).finish()));
    }

    /// Add a new child to the stack as an overlay with a specific positioning
    ///
    /// The child (and its children) will be layered above the normal UI elements. This will allow
    /// it to float above the rest of the UI—useful for things like dropdowns and menus. The new
    /// layer will be unclipped by default.
    pub fn with_positioned_overlay_child(
        mut self,
        child: Box<dyn Element>,
        positioning: OffsetPositioning,
    ) -> Self {
        self.add_positioned_overlay_child(child, positioning);
        self
    }

    /// Add a new child to the stack as an overlay with a specific positioning
    ///
    /// The child (and its children) will be layered above the normal UI elements. This will allow
    /// it to float above the rest of the UI—useful for things like dropdowns and menus. The new
    /// layer will be unclipped by default.
    pub fn add_positioned_overlay_child(
        &mut self,
        child: Box<dyn Element>,
        positioning: OffsetPositioning,
    ) {
        self.add_positioned_child(Overlay::new(child).finish(), positioning);
    }
}

impl Element for Stack {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let mut size = constraint.min;
        for child in &mut self.children {
            if child
                .element
                .parent_data()
                .and_then(|d| d.downcast_ref::<OffsetPositioning>())
                .is_none()
            {
                // Only take child size into account if it's not an absolutely positioned element.
                // (Absolutely positioned elements must have an `OffsetPositioning` parent_data.
                size = size.max(child.element.layout(constraint, ctx, app));
            }
        }

        let absolute_constraints = if self.constrain_absolute_children {
            constraint
        } else {
            SizeConstraint::new(Vector2F::zero(), ctx.window_size)
        };
        for child in &mut self.children {
            if let Some(offset_positioning) = child
                .element
                .parent_data()
                .and_then(|d| d.downcast_ref::<OffsetPositioning>())
            {
                child.element.layout(
                    offset_positioning.size_constraint(
                        size,
                        ctx.window_size,
                        absolute_constraints,
                        ctx.position_cache,
                    ),
                    ctx,
                    app,
                );
            }
        }

        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for child in &mut self.children {
            child.element.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let parent_rect = self.bounds().unwrap();
        for child in &mut self.children {
            ctx.scene.start_layer(ClipBounds::ActiveLayer);
            ctx.position_cache.start();
            let child_origin = if let Some(offset_positioning) = child
                .element
                .parent_data()
                .and_then(|d| d.downcast_ref::<OffsetPositioning>())
            {
                let child_size = child.element.size().unwrap();
                match (
                    offset_positioning.x_axis.compute_child_position(
                        child_size,
                        parent_rect,
                        ctx.window_size,
                        ctx.position_cache,
                    ),
                    offset_positioning.y_axis.compute_child_position(
                        child_size,
                        parent_rect,
                        ctx.window_size,
                        ctx.position_cache,
                    ),
                ) {
                    (Ok(x), Ok(y)) => vec2f(x, y),
                    (x_res, y_res) => {
                        // Log a warning when position computation fails.
                        // This can happen when conditional positioning fails or when position cache
                        // doesn't have the required position data.
                        if !offset_positioning.x_axis.anchor.is_conditional()
                            && !offset_positioning.y_axis.anchor.is_conditional()
                        {
                            warn!(
                                "Failed to compute position for stack child element. Skipping child. X: {x_res:?}, Y: {y_res:?}."
                            );
                        }

                        ctx.position_cache.end();
                        ctx.scene.stop_layer();
                        continue;
                    }
                }
            } else {
                origin
            };

            child.element.paint(child_origin, ctx, app);
            child.painted = true;

            ctx.position_cache.end();
            ctx.scene.stop_layer();
        }
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let mut handled = false;

        match self.event_dispatch_mode {
            EventDispatchMode::Broadcast => {
                for child in self.children.iter_mut() {
                    // We should not dispatch event to children that are not painted.
                    if child.painted {
                        handled |= child.element.dispatch_event(event, ctx, app);
                    }
                }
            }
            EventDispatchMode::Waterfall => {
                // For waterfall, we want to dispatch event to children in the reverse order (top first).
                for child in self.children.iter_mut().rev() {
                    // We should not dispatch event to children that are not painted.
                    if child.painted && child.element.dispatch_event(event, ctx, app) {
                        return true;
                    }
                }
            }
        }
        handled
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        let texts: Vec<String> = self
            .children
            .iter()
            .filter_map(|child| child.element.debug_text_content())
            .collect();
        if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n"))
        }
    }
}

impl SelectableElement for Stack {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        let mut selection_fragments = Vec::new();
        for child in self.children.iter() {
            if let Some(selectable_child) = child.element.as_selectable_element() {
                if let Some(child_fragments) =
                    selectable_child.get_selection(selection_start, selection_end, is_rect)
                {
                    selection_fragments.extend(child_fragments);
                }
            }
        }
        if !selection_fragments.is_empty() {
            return Some(selection_fragments);
        }
        None
    }

    fn expand_selection(
        &self,
        point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        for child in self.children.iter() {
            if let Some(selectable_child) = child.element.as_selectable_element() {
                if let Some(selection) = selectable_child.expand_selection(
                    point,
                    direction,
                    unit,
                    word_boundaries_policy,
                ) {
                    return Some(selection);
                }
            }
        }
        None
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        for child in self.children.iter() {
            if let Some(selectable_child) = child.element.as_selectable_element() {
                if let Some(is_point_semantically_before) = selectable_child
                    .is_point_semantically_before(absolute_point, absolute_point_other)
                {
                    return Some(is_point_semantically_before);
                }
            }
        }
        None
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        smart_select_fn: crate::elements::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        for child in self.children.iter() {
            if let Some(selectable_child) = child.element.as_selectable_element() {
                if let Some(selection) =
                    selectable_child.smart_select(absolute_point, smart_select_fn)
                {
                    return Some(selection);
                }
            }
        }
        None
    }

    fn calculate_clickable_bounds(&self, current_selection: Option<Selection>) -> Vec<RectF> {
        let mut clickable_bounds = Vec::new();
        for child in self.children.iter() {
            if let Some(selectable_child) = child.element.as_selectable_element() {
                clickable_bounds
                    .append(&mut selectable_child.calculate_clickable_bounds(current_selection));
            }
        }
        clickable_bounds
    }
}

impl Extend<Box<dyn Element>> for Stack {
    fn extend<T: IntoIterator<Item = Box<dyn Element>>>(&mut self, children: T) {
        self.children
            .extend(children.into_iter().map(StackChild::new))
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
