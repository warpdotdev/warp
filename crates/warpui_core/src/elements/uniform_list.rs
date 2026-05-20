use crate::event::{DispatchedEvent, ModifiersState};

use super::{
    try_rect_with_z, AfterLayoutContext, AppContext, Element, Event, EventContext, LayoutContext,
    PaintContext, Point, ScrollData, ScrollableElement, SizeConstraint, ZIndex,
};

use crate::units::{IntoLines, IntoPixels, Lines, Pixels};
use crate::ClipBounds;
use async_channel::Sender;
use parking_lot::Mutex;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use std::{cmp, ops::Range, sync::Arc};

#[derive(Clone)]
pub struct UniformListState(Arc<Mutex<StateInner>>);

struct StateInner {
    /// The number of lines from the visible viewport to the top.
    scroll_top: Lines,
    scroll_to: Option<usize>,
}

impl Default for UniformListState {
    fn default() -> Self {
        Self::new()
    }
}

impl UniformListState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(StateInner {
            scroll_top: Default::default(),
            scroll_to: None,
        })))
    }

    pub fn scroll_to(&self, item_ix: usize) {
        self.0.lock().scroll_to = Some(item_ix);
    }

    pub fn scroll_top(&self) -> Lines {
        self.0.lock().scroll_top
    }

    /// Adjusts the current scroll position by the given number of lines.
    /// Negative values scroll towards the top of the list.
    pub fn add_scroll_top(&self, delta: f32) {
        let mut state = self.0.lock();
        state.scroll_top = (state.scroll_top + delta.into_lines()).max(Lines::zero());
    }
}

pub struct UniformList<F, G>
where
    F: Fn(Range<usize>, &AppContext) -> G,
    G: Iterator<Item = Box<dyn Element>>,
{
    state: UniformListState,
    item_count: usize,
    build_items: F,
    scroll_max: Option<Lines>,
    items: Vec<Box<dyn Element>>,
    origin: Option<Point>,
    size: Option<Vector2F>,
    line_height: Option<Pixels>,
    visible_items_tx: Option<Sender<Range<usize>>>,
    // This is a short-term solution for properly handling events on stacks. A stack will always
    // put its children on higher z-indexes than its origin, so a hit test using the standard
    // `z_index` method would always result in the event being covered (by the children of the
    // stack). Instead, we track the upper-bound of z-indexes _contained by_ the child element.
    // Then we use that upper bound to do the hit testing, which means a parent will always get
    // events from its children, regardless of whether they are stacks or not.
    child_max_z_index: Option<ZIndex>,
}

impl<F, G> UniformList<F, G>
where
    F: Fn(Range<usize>, &AppContext) -> G,
    G: Iterator<Item = Box<dyn Element>>,
{
    pub fn new(state: UniformListState, item_count: usize, build_items: F) -> Self {
        Self {
            state,
            item_count,
            build_items,
            scroll_max: None,
            items: Vec::new(),
            origin: None,
            size: None,
            line_height: None,
            visible_items_tx: None,
            child_max_z_index: None,
        }
    }

    /// Notifies the visible items using the given Sender.
    pub fn notify_visible_items(mut self, visible_items_tx: Sender<Range<usize>>) -> Self {
        self.visible_items_tx = Some(visible_items_tx);
        self
    }

    fn scroll_internal(
        &self,
        position: Vector2F,
        delta: Vector2F,
        precise: bool,
        ctx: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        if !self.rect().unwrap().contains_point(position) {
            return false;
        }

        let delta = if precise {
            // Non-precise scrolling is in terms of pixels, so convert it to lines.
            delta.y() / self.items.first().unwrap().size().unwrap().y()
        } else {
            delta.y()
        };

        let mut state = self.state.0.lock();
        state.scroll_top = (state.scroll_top - delta.into_lines())
            .max(Lines::zero())
            .min(self.scroll_max.unwrap());

        ctx.notify();
        true
    }

    fn autoscroll(&mut self, list_height: Pixels, item_height: Pixels) {
        let mut state = self.state.0.lock();

        // The scroll_max can be negative if the list height is much bigger than item_height *
        // item. Negative scroll_max results in random behavior where the list is rendered with
        // "shadow" elements.
        // To handle this, we make sure that it's set to either a positive number or 0.
        let test: Pixels = list_height / item_height;
        let scroll_max = (self.item_count as f32 - (test).as_f32())
            .max(0.)
            .into_lines();
        if state.scroll_top > scroll_max {
            state.scroll_top = scroll_max;
        }

        if let Some(item_ix) = state.scroll_to.take() {
            let item_top = (item_ix as f32).into_lines();
            let item_bottom = item_top + 1.0.into_lines();

            if item_top < state.scroll_top {
                state.scroll_top = item_top;
            } else if item_bottom > (state.scroll_top + list_height.to_lines(item_height)) {
                state.scroll_top = item_bottom - list_height.to_lines(item_height);
            }
        }
    }

    fn scroll_top(&self) -> Lines {
        self.state.0.lock().scroll_top
    }

    fn rect(&self) -> Option<RectF> {
        try_rect_with_z(self.origin, self.size)
    }
}

impl<F, G> Element for UniformList<F, G>
where
    F: Fn(Range<usize>, &AppContext) -> G,
    G: Iterator<Item = Box<dyn Element>>,
{
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        if constraint.max.y().is_infinite() {
            unimplemented!(
                "UniformList does not support being rendered with an unconstrained height"
            );
        }
        let mut size = constraint.max;
        let mut item_constraint =
            SizeConstraint::new(vec2f(size.x(), 0.0), vec2f(size.x(), f32::INFINITY));

        let first_item = (self.build_items)(0..1, app).next();
        if let Some(mut first_item) = first_item {
            let mut item_size = first_item.layout(item_constraint, ctx, app);
            item_size.set_x(size.x());
            item_constraint.min = item_size;
            item_constraint.max = item_size;

            self.line_height = Some(item_size.y().into_pixels());
            let scroll_height = self.item_count as f32 * item_size.y();
            if scroll_height < size.y() {
                size.set_y(size.y().min(scroll_height).max(constraint.min.y()));
            }

            self.autoscroll(size.y().into_pixels(), item_size.y().into_pixels());

            let start = cmp::min(self.scroll_top().as_f64() as usize, self.item_count);
            let end = cmp::min(
                self.item_count,
                start + (size.y() / item_size.y()).ceil() as usize + 1,
            );

            if let Some(visible_items_notifier) = &self.visible_items_tx {
                visible_items_notifier
                    .try_send(start..end)
                    .expect("unable to send visible_items");
            };

            self.items.clear();
            self.items.extend((self.build_items)(start..end, app));

            self.scroll_max =
                Some((self.item_count as f32 - size.y() / item_size.y()).into_lines());

            for item in &mut self.items {
                item.layout(item_constraint, ctx, app);
            }
        }

        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for item in &mut self.items {
            item.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        ctx.scene.start_layer(ClipBounds::BoundedBy(RectF::new(
            origin,
            self.size().unwrap(),
        )));
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        if let Some(item) = self.items.first() {
            let item_height = item.size().unwrap().y();

            let mut item_origin =
                origin - vec2f(0.0, self.scroll_top().as_f64().fract() as f32 * item_height);
            for item in &mut self.items {
                item.paint(item_origin, ctx, app);
                item_origin += vec2f(0.0, item_height);
            }
        }
        ctx.scene.stop_layer();
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.items.iter_mut().fold(false, |was_handled, child| {
            let current_handled = child.dispatch_event(event, ctx, app);
            was_handled || current_handled
        }) {
            return true;
        }

        let z_index = *self.child_max_z_index.as_ref().unwrap();

        if let Some(Event::ScrollWheel {
            position,
            delta,
            precise,
            modifiers: ModifiersState { ctrl: false, .. },
        }) = event.at_z_index(z_index, ctx)
        {
            self.scroll_internal(*position, *delta, *precise, ctx, app)
        } else {
            false
        }
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

impl<F, G> ScrollableElement for UniformList<F, G>
where
    F: Fn(Range<usize>, &AppContext) -> G,
    G: Iterator<Item = Box<dyn Element>>,
{
    #[allow(clippy::unwrap_in_result)]
    fn scroll_data(&self, _app: &AppContext) -> Option<ScrollData> {
        let line_height = self.line_height.unwrap_or_default();
        Some(ScrollData {
            scroll_start: self
                .state
                .scroll_top()
                .to_pixels(self.line_height.unwrap_or_default()),
            visible_px: match self.line_height {
                Some(_line_height) => {
                    (self.size.expect("Size must be set during layout").y()).into_pixels()
                }
                None => Pixels::zero(),
            },
            total_size: (self.item_count as f32).into_lines().to_pixels(line_height),
        })
    }

    fn scroll(&mut self, delta: Pixels, ctx: &mut EventContext) {
        let mut state = self.state.0.lock();
        state.scroll_top = (state.scroll_top
            - delta.to_lines(self.line_height.unwrap_or_default()))
        .max(Lines::zero())
        .min(self.scroll_max.unwrap());
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "uniform_list_tests.rs"]
mod tests;
