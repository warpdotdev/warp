use super::{Point, SelectableElement, Selection, SelectionFragment, ZIndex};
use crate::platform::Cursor;
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{IsRect, SelectionDirection, SelectionType};
use crate::TaskId;
use crate::{
    event::DispatchedEvent, AfterLayoutContext, AppContext, Element, Event, EventContext,
    PaintContext,
};
use instant::Instant;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;
use std::mem;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// First arg is is_hovered. True when hovering in, false when hovering out.
type HoverHandler = Box<dyn FnMut(bool, &mut EventContext, &AppContext, Vector2F)>;
type ClickHandler = Box<dyn FnMut(&mut EventContext, &AppContext, Vector2F)>;

pub struct Hoverable {
    child: Box<dyn Element>,
    state: MouseStateHandle,
    origin: Option<Point>,
    hover_handler: Option<HoverHandler>,
    // A click is comprised of a mouse down and a mouse up,
    // both within the hoverable.
    click_handler: Option<ClickHandler>,
    mouse_down_handler: Option<ClickHandler>,
    double_click_handler: Option<ClickHandler>,
    middle_click_handler: Option<ClickHandler>,
    right_click_handler: Option<ClickHandler>,
    forward_click_handler: Option<ClickHandler>,
    back_click_handler: Option<ClickHandler>,
    disabled: bool,
    hover_in_delay: Option<Duration>,
    hover_out_delay: Option<Duration>,
    skip_synthetic_hover_out: bool,
    hover_cursor: Option<Cursor>,
    reset_cursor_after_click: bool,
    // This is a short-term solution for properly handling events on stacks. A stack will always
    // put its children on higher z-indexes than its origin, so a hit test using the standard
    // `z_index` method would always result in the event being covered (by the children of the
    // stack). Instead, we track the upper-bound of z-indexes _contained by_ the child element.
    // Then we use that upper bound to do the hit testing, which means a parent will always get
    // events from its children, regardless of whether they are stacks or not.
    child_max_z_index: Option<ZIndex>,
    //
    suppress_drag: bool,
    defer_events_to_children: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MouseState {
    click_count: Option<u32>,

    /// Whether the element should be considered hovered.
    ///
    /// When there are hover delays, this does not necessarily
    /// mean that the mouse is actively over the element;
    /// see [`Self::is_mouse_over_element`] and [`Self::hovered`] for more details.
    pub(crate) is_hovered: bool,

    /// Whether the mouse is currently over the element.
    ///
    /// This property is _not_ delayed by hover delays.
    is_mouse_over_element: bool,

    /// Keep track of whether the last event changing the hover
    /// state is a synthetic mouse move. If there are two consecutive
    /// events that both want to alter the hover state, we stop the
    /// invocation to prevent the potential infinite loop. Note that
    /// any non-synthetic event should reset this state to false.
    last_event_is_synthetic_hover: bool,

    /// A timer that starts when the mouse begins hovering the element.
    ///
    /// Only [`Some`] if [`Hoverable::hover_in_delay`] is set.
    hover_in_timer: Option<HoverTimer>,

    /// A timer that starts when the mouse is no longer hovering the element.
    ///
    /// Only [`Some`] if [`Hoverable::hover_out_delay`] is set.
    hover_out_timer: Option<HoverTimer>,
}

impl MouseState {
    /// True iff the element is actively being clicked.
    pub fn is_clicked(&self) -> bool {
        self.click_count.is_some()
    }

    /// [`Some`] iff the element is actively being clicked.
    /// The number represents how many clicks were registered
    /// in the corresponding mouse down event.
    pub fn click_count(&self) -> Option<u32> {
        self.click_count
    }

    /// True iff the element is considered hovered.
    ///
    /// This does not necessarily imply that the mouse
    /// is actively hovering the element because this
    /// takes into account any delays. For example,
    /// if there is a hover-in delay, this will be
    /// true _after_ the delay (if the mouse is still covering the element).
    /// See [`Self::is_mouse_over_element`] for that.
    pub fn is_hovered(&self) -> bool {
        self.is_hovered
    }

    /// True iff the mouse is currently over the element.
    /// This is not affected by any hover delays.
    pub fn is_mouse_over_element(&self) -> bool {
        self.is_mouse_over_element
    }

    pub fn reset_hover_state(&mut self) {
        self.is_hovered = false;
    }

    /// Fully clear interaction state. Useful when a click triggers navigation or focus changes,
    /// and the original element will no longer receive follow-up mouse events (e.g. mouseup).
    /// This prevents immediate re-hover from synthetic mouse events during layout.
    pub fn reset_interaction_state(&mut self) {
        // Clear pressed state so clicked styles don't persist
        self.click_count = None;
        // Clear hover states so hover styles/tooltips don't persist
        self.is_hovered = false;
        self.is_mouse_over_element = false;
        // Treat the next synthetic hover as a no-op (avoids instant re-hover during layout)
        self.last_event_is_synthetic_hover = true;
        // Cancel any pending hover timers
        self.hover_in_timer = None;
        self.hover_out_timer = None;
    }

    fn set_hover_timer(&mut self, timer_type: HoverTimerType, hover_timer: HoverTimer) {
        match timer_type {
            HoverTimerType::HoverIn => self.hover_in_timer = Some(hover_timer),
            HoverTimerType::HoverOut => self.hover_out_timer = Some(hover_timer),
        }
    }

    fn hover_timer(&self, timer_type: HoverTimerType) -> Option<&HoverTimer> {
        match timer_type {
            HoverTimerType::HoverIn => self.hover_in_timer.as_ref(),
            HoverTimerType::HoverOut => self.hover_out_timer.as_ref(),
        }
    }

    fn take_hover_timer(&mut self, timer_type: HoverTimerType) -> Option<HoverTimer> {
        match timer_type {
            HoverTimerType::HoverIn => self.hover_in_timer.take(),
            HoverTimerType::HoverOut => self.hover_out_timer.take(),
        }
    }
}

pub type MouseStateHandle = Arc<Mutex<MouseState>>;

#[derive(Clone, Debug)]
struct HoverTimer {
    hover_at: Instant,
    timer_id: TaskId,
}

#[derive(Clone, Copy, Debug)]
enum HoverTimerType {
    HoverIn,
    HoverOut,
}

impl HoverTimerType {
    fn opposite(&self) -> HoverTimerType {
        match self {
            Self::HoverIn => Self::HoverOut,
            Self::HoverOut => Self::HoverIn,
        }
    }
}

impl Hoverable {
    pub fn new<F>(state: MouseStateHandle, build_child: F) -> Self
    where
        F: FnOnce(&MouseState) -> Box<dyn Element>,
    {
        let child = build_child(&state.lock().unwrap());
        Self {
            child,
            state,
            origin: None,
            hover_handler: None,
            click_handler: None,
            mouse_down_handler: None,
            double_click_handler: None,
            middle_click_handler: None,
            right_click_handler: None,
            forward_click_handler: None,
            back_click_handler: None,
            hover_in_delay: None,
            hover_out_delay: None,
            skip_synthetic_hover_out: false,
            hover_cursor: None,
            reset_cursor_after_click: false,
            disabled: false,
            child_max_z_index: None,
            suppress_drag: true,
            defer_events_to_children: false,
        }
    }

    /// Adds additional behavior on hover to any existing hover handler, instead
    /// of replacing the existing handler.
    pub fn additional_on_hover<F>(mut self, mut callback: F) -> Self
    where
        F: 'static + FnMut(bool, &mut EventContext, &AppContext, Vector2F),
    {
        let Some(mut hover_handler) = self.hover_handler else {
            return self.on_hover(callback);
        };

        hover_handler = Box::new(move |is_hovered, ctx, app, pos| {
            hover_handler(is_hovered, ctx, app, pos);
            callback(is_hovered, ctx, app, pos);
        });
        self.hover_handler = Some(hover_handler);
        self
    }

    /// Fires whenever [`MouseState::hovered`] changes.
    pub fn on_hover<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(bool, &mut EventContext, &AppContext, Vector2F),
    {
        self.hover_handler = Some(Box::new(callback));
        self
    }

    /// Fires when the mouse is released within the hoverable after it was pressed within the hoverable.
    pub fn on_click<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
    {
        self.click_handler = Some(Box::new(callback));
        self
    }

    /// Fires on `LeftMouseDown` (instead of on mouse up).
    /// Useful when an action should happen immediately on press (e.g. tab activation).
    pub fn on_mouse_down<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
    {
        self.mouse_down_handler = Some(Box::new(callback));
        self
    }

    pub fn on_double_click<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
    {
        self.double_click_handler = Some(Box::new(callback));
        self
    }

    pub fn on_middle_click<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
    {
        self.middle_click_handler = Some(Box::new(callback));
        self
    }

    pub fn on_right_click<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
    {
        self.right_click_handler = Some(Box::new(callback));
        self
    }

    pub fn on_back_click<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
    {
        self.back_click_handler = Some(Box::new(callback));
        self
    }

    pub fn on_forward_click<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
    {
        self.forward_click_handler = Some(Box::new(callback));
        self
    }

    /// Sets a delay between the time that the mouse hovers
    /// over the element and the time that the mouse state is
    /// considered hovered via [`MouseState::hovered`], including
    /// when the [`Hoverable::on_hover`] fires.
    pub fn with_hover_in_delay(mut self, delay: Duration) -> Self {
        self.hover_in_delay = Some(delay);
        self
    }

    /// Sets a delay between the time that the mouse stops hovering
    /// over the element and the time that the mouse state is
    /// considered unhovered via [`MouseState::hovered`], including
    /// when the [`Hoverable::on_hover`] fires.
    pub fn with_hover_out_delay(mut self, delay: Duration) -> Self {
        self.hover_out_delay = Some(delay);
        self
    }

    /// Skip firing [`Hoverable::on_hover`] when an item is hovered on synthetic mouse events.
    /// Synthetic events are generated by the UI framework when layout changes,
    /// even though the mouse hasn't actually moved.
    pub fn with_skip_synthetic_hover_out(mut self) -> Self {
        self.skip_synthetic_hover_out = true;
        self
    }

    /// Change the mouse cursor when hovered
    pub fn with_cursor(mut self, cursor: Cursor) -> Self {
        self.hover_cursor = Some(cursor);
        self
    }

    pub fn with_reset_cursor_after_click(mut self) -> Self {
        self.reset_cursor_after_click = true;
        self
    }

    pub fn with_propagate_drag(mut self) -> Self {
        self.suppress_drag = false;
        self
    }

    /// When enabled, skips this Hoverable's click handler if a child element
    /// already handled the click event.
    pub fn with_defer_events_to_children(mut self) -> Self {
        self.defer_events_to_children = true;
        self
    }

    pub fn disable(mut self) -> Self {
        self.disabled = true;
        self
    }

    fn state(&mut self) -> MutexGuard<'_, MouseState> {
        self.state.lock().unwrap()
    }

    /// Determine if the mouse is currently over the element.
    ///
    /// If there is another element above this one at the cursor position, then we treat that as
    /// outside the element for purposes of [`MouseState`].
    fn is_mouse_over_element(&self, ctx: &EventContext, position: Vector2F) -> bool {
        let Some(origin) = self.origin else {
            log::warn!("self.origin was None in `Hoverable::is_mouse_over_element`");
            return false;
        };
        let Some(size) = self.size() else {
            log::warn!("self.size() was None in `Hoverable::is_mouse_over_element`");
            return false;
        };
        let Some(z_index) = self.child_max_z_index else {
            log::warn!("self.child_max_z_index was None in `Hoverable::is_mouse_over_element`");
            return false;
        };

        let is_hovering = ctx
            .visible_rect(origin, size)
            .is_some_and(|bound| bound.contains_point(position));

        let point = Point::from_vec2f(position, z_index);
        let is_covered = ctx.is_covered(point);

        is_hovering && !is_covered
    }

    fn set_cursor(&mut self, ctx: &mut EventContext) {
        if let Some((z_index, cursor)) = self.z_index().zip(self.hover_cursor) {
            ctx.set_cursor(cursor, z_index);
        }
    }

    fn reset_cursor(&mut self, ctx: &mut EventContext) {
        if self.hover_cursor.is_some() {
            ctx.reset_cursor();
        }
    }

    fn hover_delay(&self, is_hovered: bool) -> Option<Duration> {
        if is_hovered {
            self.hover_in_delay
        } else {
            self.hover_out_delay
        }
    }

    /// The main handler for [`Event::MouseMoved`] events.
    fn handle_mouse_moved(
        &mut self,
        position: Vector2F,
        is_synthetic: bool,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let was_mouse_over_element = self.state().is_mouse_over_element;
        let is_hovered = self.is_mouse_over_element(ctx, position);
        self.state().is_mouse_over_element = is_hovered;

        // The type of timer that we might need to set, if there's a corresponding delay.
        let hover_timer_type = if is_hovered {
            HoverTimerType::HoverIn
        } else {
            HoverTimerType::HoverOut
        };

        // If there's a pending hover task for the opposite delay,
        // cancel it because we're now handling a new hover action.
        if let Some(timer) = self.state().take_hover_timer(hover_timer_type.opposite()) {
            ctx.clear_notify_timer(timer.timer_id);
        }

        // We set / reset cursors immediately (not taking into account
        // delays) because we want to reflect the correct cursor as
        // the user is moving their mouse.
        if was_mouse_over_element != is_hovered {
            if is_hovered {
                self.set_cursor(ctx);
            } else {
                self.reset_cursor(ctx);
            }
            ctx.notify();
        }

        // If there aren't any delays, then we can just handle
        // the mouse movement immediately.
        let Some(hover_delay) = self.hover_delay(is_hovered) else {
            return self.handle_mouse_moved_without_delay(
                is_hovered,
                position,
                is_synthetic,
                ctx,
                app,
            );
        };

        // If a timer has already been started, then only handle
        // the event if the timer is expired. Otherwise, we'll wait
        // until the timer expires.
        let timer = self.state().hover_timer(hover_timer_type).cloned();
        if let Some(timer) = timer {
            if Instant::now() >= timer.hover_at {
                return self.handle_mouse_moved_without_delay(
                    is_hovered,
                    position,
                    is_synthetic,
                    ctx,
                    app,
                );
            }
        } else {
            // If a timer has not been started, start it now.
            let (timer_id, hover_at) = ctx.notify_after(hover_delay);
            self.state()
                .set_hover_timer(hover_timer_type, HoverTimer { hover_at, timer_id });
        }
        false
    }

    /// Handles [`Event::MouseMoved`] events when the
    /// element is going transitioning between hovered <-> unhovered
    /// states (identified by `is_hovered`).
    ///
    /// This does _not_ take into account any delays; the handler
    /// immediately sets the hovered state and fires any related
    /// callbacks.
    fn handle_mouse_moved_without_delay(
        &mut self,
        is_hovered: bool,
        position: Vector2F,
        is_synthetic: bool,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        // If there's no change in hover-state, then there's
        // no work to do.
        //
        // Note: we intentionally compare the `hovered` property
        // and not the `is_mouse_over_element` property.
        let was_hovered = self.state().is_hovered;
        if was_hovered == is_hovered {
            return false;
        }
        self.state().is_hovered = is_hovered;

        // We should only handle this event if not both the previous and current instance of the state change
        // is triggered by a synthetic mouse event. This is to prevent infinite loops when a child element
        // conditional on the state of the hoverable might in return trigger the state change of the hoverable.
        //
        // TODO: we should re-consider this approach. It can lead to missed `on_hover` dispatches.
        let was_synthetic = mem::replace(
            &mut self.state().last_event_is_synthetic_hover,
            is_synthetic,
        );
        if was_synthetic && is_synthetic {
            log::warn!(
                "Not handling MouseMoved event in Hoverable due to back-to-back synthetic events."
            );
            return false;
        }

        // Skip synthetic hover-out events if configured to do so.
        if !is_hovered && is_synthetic && self.skip_synthetic_hover_out {
            return false;
        }

        // If there's a [`Hoverable::on_hover`] callback registered, call it.
        if let Some(handler) = self.hover_handler.as_mut() {
            handler(is_hovered, ctx, app, position);
        };

        ctx.notify();
        true
    }
}

impl Element for Hoverable {
    fn layout(
        &mut self,
        constraint: crate::SizeConstraint,
        ctx: &mut crate::LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app)
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        self.child.paint(origin, ctx, app);

        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let handled = self.child.dispatch_event(event, ctx, app);
        if self.disabled {
            return handled;
        }

        if self.defer_events_to_children && handled {
            return true;
        }

        if self.bounds().is_none() {
            return handled;
        }

        if !matches!(event.raw_event(), Event::MouseMoved { .. }) {
            self.state().last_event_is_synthetic_hover = false;
        }

        // If there's a mouse-down event outside of the element,
        // there's nothing to do except reset the hover state
        // (because there might have been a hover delay in-progress).
        if let Some(position) = event.raw_event().mouse_down_position() {
            if !self.is_mouse_over_element(ctx, position) {
                self.state().is_hovered = false;
                self.state().is_mouse_over_element = false;
                return handled;
            }
        }

        match event.raw_event() {
            Event::MiddleMouseDown { position, .. } => {
                if let Some(handler) = self.middle_click_handler.as_mut() {
                    handler(ctx, app, *position);
                    ctx.notify();
                    return true;
                }
            }
            Event::BackMouseDown { position, .. } => {
                if let Some(handler) = self.back_click_handler.as_mut() {
                    handler(ctx, app, *position);
                    ctx.notify();
                    return true;
                }
            }
            Event::ForwardMouseDown { position, .. } => {
                if let Some(handler) = self.forward_click_handler.as_mut() {
                    handler(ctx, app, *position);
                    ctx.notify();
                    return true;
                }
            }
            Event::RightMouseDown { position, .. } => {
                if let Some(handler) = self.right_click_handler.as_mut() {
                    handler(ctx, app, *position);
                    ctx.notify();
                    return true;
                }
            }
            Event::LeftMouseDown {
                click_count,
                position,
                ..
            } => {
                // Mouse-down sets the mouse state handle accordingly.
                self.state().click_count = Some(*click_count);

                // Fire the mouse-down handler immediately if one is set.
                if let Some(handler) = self.mouse_down_handler.as_mut() {
                    handler(ctx, app, *position);
                    ctx.notify();
                    return true;
                }

                // We mark this as handled if we have a handler waiting to take action on the mouse-up event.
                if self.click_handler.is_some()
                    || (*click_count == 2 && self.double_click_handler.is_some())
                {
                    ctx.notify();
                    return true;
                }
            }
            Event::LeftMouseUp { position, .. } => {
                // Mouse-up should always reset clicked and double-clicked to false.
                let click_count = self.state().click_count.take();

                // If the event occurs outside the element, don't handle it.
                if !self.is_mouse_over_element(ctx, *position) {
                    return handled;
                }

                if self.reset_cursor_after_click {
                    ctx.reset_cursor();
                }

                // The double-clicked handler takes precendence. However, we should still fall back to the single-click handler
                // on a double-click if there's no double-click handler set.
                if matches!(click_count, Some(2)) && self.double_click_handler.is_some() {
                    let handler = self
                        .double_click_handler
                        .as_mut()
                        .expect("handler should exist");
                    handler(ctx, app, *position);
                    ctx.notify();
                    return true;
                } else if click_count.is_some() && self.click_handler.is_some() {
                    let handler = self.click_handler.as_mut().expect("handler should exist");
                    handler(ctx, app, *position);
                    ctx.notify();
                    return true;
                }
            }
            Event::MouseMoved {
                position,
                is_synthetic,
                ..
            } => {
                if self.handle_mouse_moved(*position, *is_synthetic, ctx, app) {
                    return true;
                }
            }
            Event::LeftMouseDragged { .. } => {
                if self.suppress_drag && self.state().is_clicked() {
                    return true;
                }
            }
            _ => {}
        }

        handled
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}

impl SelectableElement for Hoverable {
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

#[cfg(test)]
#[path = "hoverable_tests.rs"]
mod tests;
