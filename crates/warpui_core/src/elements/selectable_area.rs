//! This element adds cross-element selectability to the UI framework.
//!
//! Any elements underneath a SelectableArea which both implement SelectableElement and
//! pass in a selectable state handle will be selectable underneath that SelectableArea.
//!
//! For an example of basic usage, refer to the selectable UI sample.

use super::SelectionFragment;
use super::{
    AfterLayoutContext, AppContext, ColorU, Element, Event, EventContext, LayoutContext,
    PaintContext, Point, SizeConstraint,
};
use crate::event::{DispatchedEvent, ModifiersState};
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{IsRect, SelectionDirection, SelectionType};
use pathfinder_geometry::vector::{vec2f, Vector2F};

use lazy_static::lazy_static;
use std::ops::Range;
use std::sync::Arc;
use std::sync::Mutex;
use string_offset::ByteOffset;

/// A function that, given some content and a double-click index offset in that content,
/// returns the resulting smart selection range.
pub type SmartSelectFn = fn(content: &str, click_offset: ByteOffset) -> Option<Range<ByteOffset>>;

pub struct SelectableArea {
    child: Box<dyn Element>,
    size: Option<Vector2F>,
    origin: Option<Point>,
    selection_handler: SelectionHandler,
    selection_updated_handler: Option<SelectionUpdatedHandler>,
    selection_right_click_handler: Option<SelectionRightClickHandler>,

    // To preserve selections when scrolling, the selectable area stores the current selection's
    // state as points relative to the origin. When rendering the selection, these points
    // are then converted back to absolute points using the origin.
    selectable_area_state: SelectionHandle,

    word_boundaries_policy: WordBoundariesPolicy,

    smart_select_fn: Option<SmartSelectFn>,

    should_support_rect_select: bool,
}

/// Stores the selection start and end points. We include the option to store
/// bounds alongside raw selection points since we may need to clamp the selection
/// points to the SelectableArea's bounds when it's not laid out (i.e. to support
/// across-block selections with AI blocks).
#[derive(Clone, Copy, Debug, Default)]
pub struct InternalSelection {
    /// The point where the user first clicked before dragging.
    /// This could be after tail if the selection is reversed.
    pub head: Option<SelectionBound>,
    /// The latest point the user dragged the selection to.
    /// This could be before head if the selection is reversed.
    pub tail: Option<SelectionBound>,
    /// The head of the selection after semantic expansion. Note the direction of expansion
    /// depends on whether the selection was reversed.
    /// This could be after tail if the selection is reversed.
    pub expanded_head: Option<SelectionBound>,
    /// The tail of the selection after semantic expansion. Note the direction of expansion
    /// depends on whether the selection was reversed.
    /// This could be before head if the selection is reversed.
    pub expanded_tail: Option<SelectionBound>,
    /// The initial smart selection on double-click, set only if
    /// smart_select_fn successfully returned a smart selection.
    /// We store this separately because selection updates after dragging should never be smaller than
    /// the initial smart selection range.
    pub initial_smart_selection: Option<InitialSmartSelection>,
    /// The semantic selection unit.
    pub unit: SelectionType,
    pub is_selecting: bool,
    /// If true, head is after tail.
    pub is_reversed: bool,
    /// Whether we should return the smart selection's start when computing the selection start.
    /// This is caching whether the smart selection's start is earlier than the expanded start.
    pub should_use_smart_start: bool,
    /// Whether we should return the smart selection's end when computing the selection end.
    /// This is caching whether the smart selection's end is later than the expanded end.
    pub should_use_smart_end: bool,
}

/// The initial smart selection on double-click, set only if
/// smart_select_fn successfully returned a smart selection.
/// We store this separately because selection updates after dragging should never be smaller than
/// the initial smart selection range.
#[derive(Clone, Copy, Debug)]
pub struct InitialSmartSelection {
    /// Always before end.
    pub start: SelectionBound,
    /// Always after start.
    pub end: SelectionBound,
}

impl InternalSelection {
    // Returns the start point of the selection, using expanded points
    // if they exist. This is always before end.
    pub fn start(&self) -> Option<SelectionBound> {
        if self.should_use_smart_start {
            self.initial_smart_selection
                .map(|smart_selection| smart_selection.start)
        } else if self.is_reversed {
            self.expanded_tail.or(self.tail)
        } else {
            self.expanded_head.or(self.head)
        }
    }

    // Returns the end point of the selection, using expanded points
    // if they exist. This is always after start.
    pub fn end(&self) -> Option<SelectionBound> {
        if self.should_use_smart_end {
            self.initial_smart_selection
                .map(|smart_selection| smart_selection.end)
        } else if self.is_reversed {
            self.expanded_head.or(self.head)
        } else {
            self.expanded_tail.or(self.tail)
        }
    }

    /// Clears the current selection state.
    ///
    /// This is `pub` so callers may imperatively clear selection state in cases where a
    /// selection-clearing mouse or keyboard event is handled prior to being received by this
    /// `SelectableArea`.
    pub fn clear(&mut self) {
        *self = InternalSelection::default();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Selection {
    pub start: Vector2F,
    pub end: Vector2F,
    pub is_rect: IsRect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectionBound {
    /// Relative to the SelectableArea's origin
    Relative(Vector2F),
    /// Start from the top left point of the SelectableArea.
    TopLeft,
    /// Start from the bottom right of the SelectableArea.
    BottomRight,
    /// Start from the top column of the SelectableArea. The row is defined by the x_bound.
    Top { x_bound: f32 },
    /// Start from the bottom column of the SelectableArea. The row is defined by the x_bound.
    Bottom { x_bound: f32 },
}

impl SelectionBound {
    fn as_absolute_point(&self, size: Vector2F, origin: Vector2F) -> Vector2F {
        match self {
            SelectionBound::Relative(point) => *point + origin,
            SelectionBound::TopLeft => origin,
            SelectionBound::BottomRight => origin + size,
            SelectionBound::Top { x_bound } => vec2f(*x_bound, origin.y()),
            SelectionBound::Bottom { x_bound } => vec2f(*x_bound, size.y() + origin.y()),
        }
    }
}

pub struct SelectionUpdateArgs {
    pub selection: Option<String>,
}

lazy_static! {
    pub static ref SELECTED_HIGHLIGHT_COLOR: ColorU =
        ColorU::new(118, 167, 250, (0.4 * 255.) as u8);
}

#[derive(Default, Clone, Debug)]
pub struct SelectionHandle {
    selection: Arc<Mutex<InternalSelection>>,
}
pub type SelectionHandler = Box<dyn FnMut(SelectionUpdateArgs, &mut EventContext, &AppContext)>;
type SelectionUpdatedHandler = Box<dyn FnMut(&mut EventContext, &AppContext)>;
type SelectionRightClickHandler = Box<dyn FnMut(&mut EventContext, Vector2F)>;

impl SelectionHandle {
    /// This isn't meant for general use. It's used specifically in cases where a selection is started
    /// outside the SelectableArea's bounds and the SelectableArea's start point needs to be clamped manually.
    pub fn start_selection_outside(&self, bound: SelectionBound, unit: SelectionType) {
        let mut selection = self.selection.lock().expect("Should not be poisoned.");
        selection.head = Some(bound);
        selection.unit = unit;
        selection.is_selecting = true;
    }

    /// Whether there is an active selection in the SelectableArea.
    /// An active selection is not necessarily a non-empty selection.
    pub fn is_selecting(&self) -> bool {
        self.selection
            .lock()
            .expect("Should not be poisoned.")
            .is_selecting
    }

    pub fn clear(&self) {
        self.selection
            .lock()
            .expect("Mutex is not poisoned.")
            .clear();
    }

    #[cfg(feature = "integration_tests")]
    pub fn selection_type(&self) -> SelectionType {
        self.selection.lock().expect("Mutex is not poisoned.").unit
    }
}

impl SelectableArea {
    pub fn new<F>(
        selectable_area_state: SelectionHandle,
        selection_handler: F,
        child: Box<dyn Element>,
    ) -> Self
    where
        F: 'static + FnMut(SelectionUpdateArgs, &mut EventContext, &AppContext),
    {
        Self {
            child,
            size: None,
            origin: None,
            selectable_area_state,
            selection_handler: Box::new(selection_handler),
            selection_updated_handler: None,
            selection_right_click_handler: None,
            word_boundaries_policy: WordBoundariesPolicy::Default,
            smart_select_fn: None,
            should_support_rect_select: false,
        }
    }

    pub fn should_support_rect_select(mut self) -> Self {
        self.should_support_rect_select = true;
        self
    }

    pub fn with_word_boundaries_policy(self, word_boundaries_policy: WordBoundariesPolicy) -> Self {
        Self {
            word_boundaries_policy,
            ..self
        }
    }

    pub fn with_smart_select_fn(self, smart_select_fn: Option<SmartSelectFn>) -> Self {
        Self {
            smart_select_fn,
            ..self
        }
    }

    /// The selection updated handler is invoked only when a selection is actively being made.
    /// Clearing the text selection in a `SelectableArea` via `LeftMouseDown` doesn't count.
    pub fn on_selection_updated<F>(self, selection_updated_handler: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext),
    {
        Self {
            selection_updated_handler: Some(Box::new(selection_updated_handler)),
            ..self
        }
    }

    pub fn on_selection_right_click<F>(self, selection_right_click_fn: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, Vector2F),
    {
        Self {
            selection_right_click_handler: Some(Box::new(selection_right_click_fn)),
            ..self
        }
    }

    /// Clears any existing selection, and starts a new selection if the click is in the element.
    /// Does not handle cases where a selection is started outside the `SelectableArea`'s bounds.
    /// Returns `true` if a new selection was successfully started.
    fn on_mouse_down(
        &mut self,
        position: Vector2F,
        modifiers: &ModifiersState,
        click_count: u32,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let Some(selectable_child_ref) = self.child.as_selectable_element() else {
            return false;
        };
        // Clear any previously existing selection on mouse down.
        let mut selection_state = self
            .selectable_area_state
            .selection
            .lock()
            .expect("Should not be poisoned.");
        selection_state.clear();

        // Only if this click was in the element, start a new selection.
        if !is_mouse_in(self.origin, self.size, ctx, position) {
            return false;
        }
        let Some(origin) = self.origin else {
            return false;
        };
        selection_state.is_selecting = true;

        let unit = if click_count == 1 {
            if self.should_support_rect_select && modifiers.alt && modifiers.cmd {
                SelectionType::Rect
            } else {
                SelectionType::Simple
            }
        } else if click_count == 2 {
            SelectionType::Semantic
        } else {
            SelectionType::Lines
        };
        selection_state.unit = unit;
        selection_state.head = Some(SelectionBound::Relative(position - origin.xy));
        selection_state.tail = Some(SelectionBound::Relative(position - origin.xy));

        // First, try smart selection if it's configured and this is a double click.
        let smart_select_range = match (unit, self.smart_select_fn) {
            (SelectionType::Semantic, Some(smart_select_fn)) => {
                selectable_child_ref.smart_select(position, smart_select_fn)
            }
            // In all other cases, use the default selection expansion
            _ => None,
        };
        let (expanded_head, expanded_tail) = match smart_select_range {
            // If smart_select_range is set, use that as expanded head / tail.
            Some((start, end)) => {
                selection_state.initial_smart_selection = Some(InitialSmartSelection {
                    start: SelectionBound::Relative(start - origin.xy),
                    end: SelectionBound::Relative(end - origin.xy),
                });
                (Some(start), Some(end))
            }
            _ => {
                // Otherwise, expand the selection normally.
                let expanded_head = selectable_child_ref.expand_selection(
                    position,
                    SelectionDirection::Backward,
                    unit,
                    &self.word_boundaries_policy,
                );
                let expanded_tail = selectable_child_ref.expand_selection(
                    position,
                    SelectionDirection::Forward,
                    unit,
                    &self.word_boundaries_policy,
                );
                (expanded_head, expanded_tail)
            }
        };

        // Set the expanded head and tail. Since the resulting expanded selection could be
        // non-empty, we should invoke the selection update handler as well.
        if let Some((head, tail)) = expanded_head.zip(expanded_tail) {
            selection_state.expanded_head = Some(SelectionBound::Relative(head - origin.xy));
            selection_state.expanded_tail = Some(SelectionBound::Relative(tail - origin.xy));

            if head != tail {
                if let Some(selection_updated_handler) = self.selection_updated_handler.as_mut() {
                    selection_updated_handler(ctx, app);
                }
            }
        }

        // By this point, we've determined that a selection has successfully started.
        // Returning `true` ensures that parent `SelectableArea`s don't attempt to start their
        // own text selections at the same time.
        true
    }

    fn on_right_mouse_down(&mut self, position: Vector2F, ctx: &mut EventContext) -> bool {
        // Ignore this right-click unless it took place within this SelectableArea element
        if !is_mouse_in(self.origin, self.size, ctx, position) {
            return false;
        }

        let current_selection = self.get_current_selection_absolute();
        let Some(selectable_child_ref) = self.child.as_selectable_element() else {
            return false;
        };
        let Some(right_click_handler) = self.selection_right_click_handler.as_mut() else {
            return false;
        };

        let clickable_bounds = selectable_child_ref.calculate_clickable_bounds(current_selection);
        let is_within_bounds = clickable_bounds
            .iter()
            .any(|bounds| bounds.contains_point(position));

        if is_within_bounds {
            let origin = self
                .origin
                .expect("Origin should be defined before mouse clicks")
                .xy();
            let position_in_block = position - origin;
            right_click_handler(ctx, position_in_block);
        }
        is_within_bounds
    }

    /// Updates the selection using the latest tail position (where the user dragged to).
    /// Expands selections as needed and computes whether the selection is_reversed.
    /// Returns whether the selection was actually updated.
    fn update_selection(&mut self, tail_absolute_position: Vector2F) -> bool {
        let Some(selectable_child_ref) = self.child.as_selectable_element() else {
            return false;
        };
        let mut selection_state = self
            .selectable_area_state
            .selection
            .lock()
            .expect("Should not be poisoned.");

        // We can only update the selection if we have a selection head the selection was initiated from.
        let (Some(relative_selection_head), Some(origin), Some(size)) =
            (selection_state.head, self.origin, self.size)
        else {
            return false;
        };

        let new_selection_tail = SelectionBound::Relative(tail_absolute_position - origin.xy);
        // Don't update or cache the selection if it hasn't changed
        if selection_state
            .tail
            .is_some_and(|old_selection_tail| old_selection_tail == new_selection_tail)
        {
            return false;
        }

        // Update the selection's raw end.
        selection_state.tail = Some(new_selection_tail);

        // Compute whether the selection is reversed. This is needed
        // to determine how to expand the selection.
        let head_absolute_position = relative_selection_head.as_absolute_point(size, origin.xy());
        let is_reversed = if matches!(
            relative_selection_head,
            SelectionBound::TopLeft | SelectionBound::Top { .. }
        ) {
            Some(false)
        } else if matches!(
            relative_selection_head,
            SelectionBound::BottomRight | SelectionBound::Bottom { .. }
        ) {
            Some(true)
        } else {
            // If the end is before the start, this is a reversed selection.
            selectable_child_ref
                .is_point_semantically_before(tail_absolute_position, head_absolute_position)
        };
        // If we can't tell whether the selection is reversed, don't do semantic expansion.
        let Some(is_reversed_selection) = is_reversed else {
            // We return true here because the selection was already successfully updated with the latest unexpanded tail.
            return true;
        };

        let (head_direction, tail_direction) = if is_reversed_selection {
            // If this is a reversed selection, the tail (point the user dragged to) should be expanded backward
            // since it will be the start of the selection.
            (SelectionDirection::Forward, SelectionDirection::Backward)
        } else {
            // If this is a forward selection, the head (point user originally clicked before dragging)
            // should be expanded backward since it will be the start of the selection.
            (SelectionDirection::Backward, SelectionDirection::Forward)
        };

        // We always need to expand the new tail.
        // If we're changing the value of is_reversed, we also need to re-expand
        // the head, since the direction of head expansion changes.
        // There are expected cases where only one is expanded successfully and not the other.
        // For example, if a semantic selection was started outside the selectable area and then
        // dragged in, the original head would be a max/min bound of the selectable area which
        // can't always be expanded.
        let expanded_tail = selectable_child_ref.expand_selection(
            tail_absolute_position,
            tail_direction,
            selection_state.unit,
            &self.word_boundaries_policy,
        );
        selection_state.expanded_tail =
            expanded_tail.map(|expanded_tail| SelectionBound::Relative(expanded_tail - origin.xy));
        if selection_state.is_reversed != is_reversed_selection {
            let expanded_head = selectable_child_ref.expand_selection(
                head_absolute_position,
                head_direction,
                selection_state.unit,
                &self.word_boundaries_policy,
            );
            selection_state.expanded_head = expanded_head
                .map(|expanded_head| SelectionBound::Relative(expanded_head - origin.xy));
        }
        selection_state.is_reversed = is_reversed_selection;
        // Now that we've set the new expanded head and tail, make sure our new selection is not smaller than
        // the original smart selection if there was one.
        // First reset the cached values to get the selection start/end without considering the initial smart selection.
        selection_state.should_use_smart_start = false;
        selection_state.should_use_smart_end = false;
        let (Some(new_start), Some(new_end)) = (selection_state.start(), selection_state.end())
        else {
            return true;
        };
        let Some(initial_smart_selection) = selection_state.initial_smart_selection else {
            return true;
        };
        // Use the smart selection start/end if they would make the selection range bigger than the expanded selection.
        if selectable_child_ref
            .is_point_semantically_before(
                initial_smart_selection
                    .start
                    .as_absolute_point(size, origin.xy()),
                new_start.as_absolute_point(size, origin.xy()),
            )
            .unwrap_or(false)
        {
            selection_state.should_use_smart_start = true
        }
        if selectable_child_ref
            .is_point_semantically_before(
                new_end.as_absolute_point(size, origin.xy()),
                initial_smart_selection
                    .end
                    .as_absolute_point(size, origin.xy()),
            )
            .unwrap_or(false)
        {
            selection_state.should_use_smart_end = true
        }
        true
    }

    // Returns the current selection in absolute coordinates.
    fn get_current_selection_absolute(&self) -> Option<Selection> {
        let (Some(origin), Some(size)) = (self.origin, self.size) else {
            return None;
        };
        let selection = self
            .selectable_area_state
            .selection
            .lock()
            .expect("Should not be poisoned.");
        let (Some(start), Some(end)) = (selection.start(), selection.end()) else {
            return None;
        };

        Some(Selection {
            start: start.as_absolute_point(size, origin.xy()),
            end: end.as_absolute_point(size, origin.xy()),
            is_rect: selection.unit.into(),
        })
    }

    fn get_current_selection_text_fragments(&self) -> Option<Vec<SelectionFragment>> {
        let updated_selection = self.get_current_selection_absolute()?;
        let selectable_child_ref = self.child.as_selectable_element()?;

        // Order selected text fragments
        selectable_child_ref.get_selection(
            updated_selection.start,
            updated_selection.end,
            updated_selection.is_rect,
        )
    }

    fn is_current_selection_empty(&self) -> bool {
        self.get_current_selection_text_fragments()
            .unwrap_or_default()
            .is_empty()
    }

    fn invoke_selection_handler(&mut self, ctx: &mut EventContext, app: &AppContext) {
        let text_fragments = self.get_current_selection_text_fragments();
        let update_args = SelectionUpdateArgs {
            // If `text_fragments` is `None`, we still need to invoke the selection_handler accordingly.
            // Otherwise, clicking away from text within an AIBlock won't clear the underlying selected_text state.
            selection: text_fragments.map(order_and_concatenate_fragments),
        };
        (self.selection_handler)(update_args, ctx, app);
        ctx.notify();
    }
}

/// Determine if the mouse is over the element
fn is_mouse_in(
    origin: Option<Point>,
    size: Option<Vector2F>,
    ctx: &EventContext,
    position: Vector2F,
) -> bool {
    let Some(origin) = origin else {
        log::warn!("self.origin was None in `SelectableArea::is_mouse_in`");
        return false;
    };
    let Some(size) = size else {
        log::warn!("self.size() was None in `SelectableArea::is_mouse_in`");
        return false;
    };

    ctx.visible_rect(origin, size)
        .is_some_and(|bound| bound.contains_point(position))
}

fn order_and_concatenate_fragments(mut selection_fragments: Vec<SelectionFragment>) -> String {
    selection_fragments.sort_by(|a, b| {
        if a.origin.y() == b.origin.y() {
            a.origin.x().total_cmp(&b.origin.x())
        } else {
            a.origin.y().total_cmp(&b.origin.y())
        }
    });

    selection_fragments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<&str>>()
        .concat()
}

impl Element for SelectableArea {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let child_constraint = SizeConstraint {
            min: (constraint.min).max(Vector2F::zero()),
            max: (constraint.max).max(Vector2F::zero()),
        };
        let child_size = self.child.layout(child_constraint, ctx, app);
        let size = child_size;
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        ctx.current_selection = self.get_current_selection_absolute();
        self.child.paint(origin, ctx, app);
        ctx.current_selection = None;
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        // Only dispatch to the child if we're not in the middle of a non-empty selection.
        // Nested `SelectableArea` elements should pick up on mouse events if said events
        // are not being used to create a non-trivial selection in this `SelectableArea`.
        // Do not handle the event with this element if the child handles it, as doing so
        // could result in nested `SelectableArea` elements being unnecessarily cleared.
        let should_dispatch_to_child =
            !self.selectable_area_state.is_selecting() || self.is_current_selection_empty();
        if should_dispatch_to_child {
            let handled = self.child.as_mut().dispatch_event(event, ctx, app);
            if handled {
                return true;
            }
        }
        if matches!(
            event.raw_event(),
            Event::LeftMouseDown { .. } | Event::RightMouseDown { .. }
        ) && self
            .z_index()
            .is_some_and(|z_index| event.at_z_index(z_index, ctx).is_none())
        {
            return false;
        }

        match event.raw_event() {
            Event::LeftMouseDown {
                position,
                click_count,
                modifiers,
                ..
            } => {
                let selection_started =
                    self.on_mouse_down(*position, modifiers, *click_count, ctx, app);

                // Invoking the selection handler is necessary to notify parent views that we've
                // cleared the internal selection state of the `SelectableArea`.
                self.invoke_selection_handler(ctx, app);
                selection_started
            }
            Event::LeftMouseDragged { position, .. } => {
                if !self.selectable_area_state.is_selecting() {
                    return false;
                }
                let (Some(origin), Some(size)) = (self.origin, self.size) else {
                    return false;
                };

                let selection_updated = self.update_selection(*position);
                if !selection_updated {
                    return false;
                }
                if let Some(selection_updated_handler) = self.selection_updated_handler.as_mut() {
                    selection_updated_handler(ctx, app);
                }

                // Materialize and cache the selected text if SelectableArea is about to go off-screen.
                // Since origin isn't available when SelectableArea is off-screen, we aren't able to
                // materialize the selection on mouse up if that's the case. As a workaround,
                // we cache it here ahead of time.
                if origin.y() < 0.
                    || origin.y() + size.y() > app.windows().active_display_bounds().height()
                {
                    self.invoke_selection_handler(ctx, app)
                }

                // Returning true ensures that this SelectableArea's ongoing selections won't
                // conflict with parent or child SelectableAreas in the element tree.
                ctx.notify();
                true
            }
            Event::LeftMouseUp { position, .. } => {
                self.selectable_area_state
                    .selection
                    .lock()
                    .expect("Should not be poisoned.")
                    .is_selecting = false;
                self.invoke_selection_handler(ctx, app);

                // If the mouse is inside this element no other element needs to handle this event
                // because this is the "lowest level" element so we return `true`. If the mouse is
                // outside this element we return `false` so other elements can handle the event
                // as well. We need to handle `LeftMouseUp` in either case to support selections
                // across elements. Note that this behavior may need to change in the future.
                is_mouse_in(self.origin, self.size, ctx, *position)
            }
            Event::RightMouseDown { position, .. } => self.on_right_mouse_down(*position, ctx),
            _ => false,
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
