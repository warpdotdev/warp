use super::{elements::Axis, Event};
use crate::assets::asset_cache::AssetHandle;
use crate::elements::{DropTargetPosition, Selection};

use crate::fonts;
use crate::zoom::Scale;
use crate::{
    elements::Point,
    event::DispatchedEvent,
    fonts::Cache as FontCache,
    platform::Cursor,
    scene::{Scene, ZIndex},
    text_layout::LayoutCache,
    Action, AppContext, ClipBounds, EntityId, TaskId, View, ViewHandle, WindowId,
    WindowInvalidation,
};
use instant::Instant;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    rc::Rc,
    time::Duration,
};

pub struct Presenter {
    // Number of frames rendered so far by this presenter
    frame_count: usize,
    window_id: WindowId,
    scene: Option<Rc<Scene>>,
    rendered_views: HashMap<EntityId, Box<dyn Element>>,
    parents: HashMap<EntityId, EntityId>,
    text_layout_cache: LayoutCache,
    position_cache: PositionCache,
    highlighted_view: Option<EntityId>,
}

pub struct LayoutContext<'a> {
    rendered_views: &'a mut HashMap<EntityId, Box<dyn Element>>,
    parents: &'a mut HashMap<EntityId, EntityId>,
    pub text_layout_cache: &'a LayoutCache,
    view_stack: Vec<EntityId>,
    pub window_size: Vector2F,
    pub position_cache: &'a PositionCache,
}

pub struct AfterLayoutContext<'a> {
    rendered_views: &'a mut HashMap<EntityId, Box<dyn Element>>,
    pub text_layout_cache: &'a LayoutCache,
}

pub struct PaintContext<'a> {
    rendered_views: &'a mut HashMap<EntityId, Box<dyn Element>>,
    pub font_cache: &'a FontCache,
    pub text_layout_cache: &'a LayoutCache,
    pub position_cache: &'a mut PositionCache,
    pub scene: &'a mut Scene,
    pub window_size: Vector2F,
    /// The maximum dimension size in pixels, either width or height, for a 2D-texture. `None`
    /// will be treated as unbounded.
    pub max_texture_dimension_2d: Option<u32>,
    pub highlighted_view: Option<EntityId>,
    pub current_selection: Option<Selection>,
    /// Holds the time the scene should be repainted next, if animated.
    repaint_at: Option<Instant>,
    pending_assets: HashSet<AssetHandle>,
    /// Keep track of all the views that were actually painted in this scene.
    views_painted: HashSet<EntityId>,
}

#[derive(Default)]
pub struct DispatchResult {
    /// Whether the event was marked as handled, either by the RootView or a descendent
    pub handled: bool,

    /// All actions to dispatch as a result of the event being handled
    pub actions: Vec<DispatchedAction>,

    /// All views to notify as a result of the event being handled
    pub notified: HashSet<EntityId>,

    /// Views that need to be notified after a delay
    pub notify_timers_to_set: HashMap<TaskId, ViewToNotify>,

    /// Views that need to have notify timers cleared
    pub notify_timers_to_clear: HashSet<TaskId>,

    /// An optional update to the mouse cursor
    pub cursor_update: Option<CursorUpdate>,

    /// Whether the soft keyboard was requested by an element during dispatch.
    /// Used on mobile WASM to trigger the keyboard in user gesture context.
    pub soft_keyboard_requested: bool,
}

#[derive(Debug, Copy, Clone)]
pub struct ViewToNotify {
    /// The view to notify after a delay
    pub view_id: EntityId,

    /// The time to notify the view
    pub notify_at: Instant,
}

#[derive(Debug, Clone)]
pub enum CursorUpdate {
    /// Set the cursor to the given cursor type at the given z-index
    Set {
        cursor: Cursor,
        z_index: ZIndex,
        view_id: EntityId,
    },

    /// Reset top the default cursor (usually the pointer)
    Reset,
}

/// A set of element rects that are cached across frames
/// The API allows for callers to control how conflicting position ids are
/// handled.  The typical usage is that in a stack, every time you paint at
/// a higher z-index, you start a new stack context.  Every element at the
/// current z-index is in the same position namespace.  Elements at lower
/// z-indices will take precedence over elements at higher z-indices if there
/// are naming conflicts.
#[derive(Default, Clone)]
pub struct PositionCache {
    /// A stack of pending positions to cache
    pending_positions: Vec<HashMap<String, RectF>>,

    /// The positions that have been committed to the cache.
    committed_positions: HashMap<String, RectF>,

    /// Positions that are only cached for a single frame
    single_frame_positions: HashSet<String>,

    /// Positions for a drop target. These positions are always cleared on every frame.
    drop_target_positions: Vec<DropTargetPosition>,
}

impl PositionCache {
    pub fn new() -> Self {
        PositionCache {
            pending_positions: Default::default(),
            committed_positions: Default::default(),
            single_frame_positions: Default::default(),
            drop_target_positions: Default::default(),
        }
    }

    /// Starts a new namespace for position id caching.
    pub fn start(&mut self) {
        self.pending_positions.push(HashMap::new());
    }

    /// Ends the current namespace for position id caching, and commits all
    /// of the positions.
    pub fn end(&mut self) {
        let mut last = self
            .pending_positions
            .pop()
            .expect("mismatched stack start/end");
        self.committed_positions.extend(last.drain());
    }

    /// Caches a position in the current namespace.  This position will remain
    /// cached until it's explicitly cleared.
    pub fn cache_position_indefinitely(&mut self, position_id: String, bounds: RectF) {
        if let Some(last) = self.pending_positions.last_mut() {
            last.insert(position_id.clone(), bounds);
            self.single_frame_positions.remove(&position_id);
        }
    }

    /// Caches a position in the current namespace until the next frame is rendered.
    pub fn cache_position_for_one_frame(&mut self, position_id: String, bounds: RectF) {
        if let Some(last) = self.pending_positions.last_mut() {
            last.insert(position_id.clone(), bounds);
            self.single_frame_positions.insert(position_id);
        }
    }

    pub(crate) fn cache_drop_target_position(&mut self, drop_target_position: DropTargetPosition) {
        self.drop_target_positions.push(drop_target_position);
    }

    /// Clears a position from the cache.
    pub fn clear_position<S>(&mut self, position_id: S)
    where
        S: AsRef<str>,
    {
        self.committed_positions.remove(position_id.as_ref());
        self.single_frame_positions.remove(position_id.as_ref());
    }

    /// Clears any positions that should be cached for a single frame. This always clears any cached
    /// drop target positions--we don't permit them to be cached for multiple frames.
    pub fn clear_single_frame_positions(&mut self) {
        for position_id in self.single_frame_positions.drain() {
            self.committed_positions.remove(&position_id);
        }
        self.drop_target_positions.clear();
    }

    /// Returns a cached position, if there is one.
    pub fn get_position<S>(&self, position_id: S) -> Option<RectF>
    where
        S: AsRef<str>,
    {
        self.committed_positions.get(position_id.as_ref()).copied()
    }

    /// Returns an iterator of `DropTargetPosition`s. Used to determine if a draggable element
    /// was dropped on a `DropTarget`.
    pub(crate) fn drop_target_data(&self) -> impl Iterator<Item = DropTargetPosition> + '_ {
        self.drop_target_positions.iter().cloned()
    }
}

pub struct EventContext<'a> {
    // Scene is optional because it's technically possible for a window event to
    // be fired before the first scene has been rendered.
    scene: Option<Rc<Scene>>,
    rendered_views: &'a mut HashMap<EntityId, Box<dyn Element>>,
    actions: Vec<DispatchedAction>,
    pub font_cache: &'a FontCache,
    pub text_layout_cache: &'a LayoutCache,
    position_cache: &'a PositionCache,
    view_stack: Vec<EntityId>,
    notified: HashSet<EntityId>,
    /// A map of timer ids to (view_id, duration) pairs for delayed notification
    notify_timers_to_set: HashMap<TaskId, ViewToNotify>,
    notify_timers_to_clear: HashSet<TaskId>,
    /// Any update to the cursor after the processing of events
    /// For now it's highest z-index wins if multiple elements try to set the
    /// cursor (later we could make this more sophisticated)
    cursor_update: Option<CursorUpdate>,
    /// Flag indicating the soft keyboard should be shown.
    /// Used on mobile WASM to trigger the keyboard in user gesture context.
    soft_keyboard_requested: bool,
}

impl<'a> EventContext<'a> {
    /// Returns whether the given position is covered by a rect at a higher index
    pub fn is_covered(&self, position: Point) -> bool {
        self.scene
            .as_ref()
            .is_some_and(|scene| scene.is_covered(position))
    }

    /// Returns the visible portion of rect at the given origin and size
    pub fn visible_rect(&self, origin: Point, size: Vector2F) -> Option<RectF> {
        self.scene
            .as_ref()
            .and_then(|scene| scene.visible_rect(origin, size))
    }

    /// Returns the position of an element that has been saved via the SavePosition
    /// element type
    pub fn element_position_by_id<S>(&self, position_id: S) -> Option<RectF>
    where
        S: AsRef<str>,
    {
        self.position_cache.get_position(position_id)
    }

    /// Returns an iterator of `DropTargetPosition`s. Used to determine if a draggable element
    /// was dropped on a `DropTarget`.
    pub(crate) fn drop_target_data(&self) -> impl Iterator<Item = DropTargetPosition> + 'a {
        self.position_cache.drop_target_data()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct SizeConstraint {
    pub min: Vector2F,
    pub max: Vector2F,
}

pub struct ChildView<T> {
    view_id: EntityId,
    size: Option<Vector2F>,
    origin: Option<Point>,
    phantom_data: PhantomData<T>,
}

pub struct DispatchedAction {
    pub view_id: EntityId,
    pub kind: DispatchedActionKind,
}

/// Temporary Enum to support both Legacy and Typed actions at the same time
///
/// Will be removed when all views have been converted to Typed actions
pub enum DispatchedActionKind {
    Legacy {
        name: &'static str,
        arg: Box<dyn Any>,
    },
    Typed(Box<dyn Action>),
}

impl Presenter {
    pub fn new(window_id: WindowId) -> Self {
        Self {
            frame_count: 0,
            window_id,
            rendered_views: HashMap::new(),
            parents: HashMap::new(),
            scene: None,
            text_layout_cache: LayoutCache::new(),
            position_cache: PositionCache::default(),
            highlighted_view: None,
        }
    }

    pub fn invalidate(&mut self, invalidation: WindowInvalidation, app: &AppContext) {
        // Don't try to update views that were also removed
        for &view_id in invalidation.updated.difference(&invalidation.removed) {
            match app.render_view(self.window_id, view_id) {
                Ok(element) => {
                    self.rendered_views.insert(view_id, element);
                }
                Err(e) => log::warn!("View was not rendered, error: {e:?}"),
            };
        }
        for view_id in invalidation.removed {
            self.rendered_views.remove(&view_id);
            self.parents.remove(&view_id);
        }
    }

    pub fn build_scene(
        &mut self,
        window_size: Vector2F,
        scale_factor: f32,
        max_texture_dimension_2d: Option<u32>,
        ctx: &mut AppContext,
    ) -> Rc<Scene> {
        self.position_cache.clear_single_frame_positions();

        // Scale the window size by the zoom factor. We implement zoom by faking a window size that
        // is proportionally smaller based on the current zoom factor. Once we build up a scene
        // with the fake window bounds, we then adjust the scale factor to include the zoom level
        // so every item in the scene is blown up to fit in the actual window bounds.
        let zoomed_window_size = window_size.scale_down(ctx.zoom_factor());
        let zoomed_scale_factor = scale_factor.scale_up(ctx.zoom_factor());

        self.layout(zoomed_window_size, ctx);
        // In theory, after_layout would be a good place for Elements to update app state with the
        // results of layout (for example, if a View stored the heights of its children to
        // implement scrolling). However, it's not safe to pass a AppContext to after_layout
        // because the presenter is mutably borrowed. Doing so can cause crashes like CORE-1544.
        // In the future, we might:
        // * Decouple after_layout from the presenter so it can take a AppContext
        // * Extend the AfterLayoutContext API to allow state updates, but not other effects
        self.after_layout(ctx);
        let (scene, repaint_at, pending_assets) = self.paint(
            zoomed_scale_factor,
            zoomed_window_size,
            max_texture_dimension_2d,
            ctx,
        );
        // After paint, collect a delayed repaint if it exists and start the timer.
        if let Some(repaint_at) = repaint_at {
            ctx.manage_delayed_repaint_timers(self.window_id, repaint_at);
        }
        ctx.manage_pending_assets(self.window_id, pending_assets);
        let scene = Rc::new(scene);
        self.scene = Some(scene.clone());
        self.text_layout_cache.finish_frame();
        self.frame_count += 1;
        ctx.load_requested_fallback_families(self.window_id);
        scene
    }

    fn layout(&mut self, window_size: Vector2F, app: &AppContext) {
        if let Some(root_view_id) = app.root_view_id(self.window_id) {
            // Parent links for rendered views are layout-derived. Clear the
            // previous frame's links before walking the current tree so views
            // that are conditionally no longer rendered do not keep a stale
            // responder-chain path through removed parents.
            for view_id in self.rendered_views.keys() {
                self.parents.remove(view_id);
            }
            for (child_id, parent_id) in app.structural_view_parents() {
                self.parents.entry(child_id).or_insert(parent_id);
            }

            let mut layout_ctx = LayoutContext {
                rendered_views: &mut self.rendered_views,
                parents: &mut self.parents,
                text_layout_cache: &self.text_layout_cache,
                view_stack: Vec::new(),
                window_size,
                position_cache: &self.position_cache,
            };
            layout_ctx.layout(
                root_view_id,
                SizeConstraint::new(Vector2F::zero(), window_size),
                app,
            );
        }
    }

    fn after_layout(&mut self, app: &AppContext) {
        if let Some(root_view_id) = app.root_view_id(self.window_id) {
            let mut ctx = AfterLayoutContext {
                rendered_views: &mut self.rendered_views,
                text_layout_cache: &self.text_layout_cache,
            };
            ctx.after_layout(root_view_id, app);
        }
    }

    fn paint(
        &mut self,
        scale_factor: f32,
        window_size: Vector2F,
        max_texture_dimension_2d: Option<u32>,
        ctx: &mut AppContext,
    ) -> (Scene, Option<Instant>, HashSet<AssetHandle>) {
        let mut scene = Scene::new(scale_factor, ctx.rendering_config());
        let mut repaint_at = None;
        let mut pending_assets = HashSet::new();

        if let Some(root_view_id) = ctx.root_view_id(self.window_id) {
            let mut paint_ctx = PaintContext {
                font_cache: ctx.font_cache(),
                text_layout_cache: &self.text_layout_cache,
                rendered_views: &mut self.rendered_views,
                position_cache: &mut self.position_cache,
                scene: &mut scene,
                window_size,
                max_texture_dimension_2d,
                highlighted_view: self.highlighted_view,
                current_selection: None,
                repaint_at: None,
                pending_assets: HashSet::new(),
                views_painted: HashSet::new(),
            };
            paint_ctx.paint(root_view_id, Vector2F::zero(), ctx);

            repaint_at = paint_ctx.repaint_at;
            pending_assets.extend(paint_ctx.pending_assets);

            // If the cursor shape had been changed by a view and that view is no longer being
            // rendered, reset the cursor.
            if let Some((window_id, view_id)) = ctx.cursor_updated_for_view {
                if self.window_id == window_id && !paint_ctx.views_painted.contains(&view_id) {
                    ctx.reset_cursor();
                }
            }
        }

        // If there is a highlighted view, draw a box over the entire scene with
        // the same bounds as the highlighted view.  This ensures that views
        // which are fully covered by a child view can still be highlighted.
        if let Some(view_id) = self.highlighted_view.as_ref() {
            if let Some(view) = self.rendered_views.get(view_id) {
                if let Some(bounds) = view.bounds() {
                    scene.start_overlay_layer(ClipBounds::None);
                    scene.draw_rect_with_hit_recording(bounds).with_border(
                        crate::elements::Border::all(2.)
                            // Use a semi-transparent color so that overlapping
                            // content can still be seen through the border.
                            .with_border_color(pathfinder_color::ColorU::new(0, 255, 255, 128)),
                    );
                    scene.stop_layer();
                }
            }
        }

        (scene, repaint_at, pending_assets)
    }

    pub fn ancestors(&self, mut view_id: EntityId) -> Vec<EntityId> {
        let mut chain = vec![view_id];
        while let Some(parent_id) = self.parents.get(&view_id) {
            view_id = *parent_id;
            chain.push(view_id);
        }
        chain.reverse();
        chain
    }

    /// Returns all descendant view IDs of the given root view.
    /// This is computed by finding all views whose ancestor chain includes the root.
    pub fn descendants(&self, root_view_id: EntityId) -> Vec<EntityId> {
        self.parents
            .keys()
            .filter(|&&view_id| {
                let mut current = view_id;
                while let Some(&parent_id) = self.parents.get(&current) {
                    if parent_id == root_view_id {
                        return true;
                    }
                    current = parent_id;
                }
                false
            })
            .copied()
            .collect()
    }

    fn create_event_context<'a>(&'a mut self, font_cache: &'a fonts::Cache) -> EventContext<'a> {
        EventContext {
            scene: self.scene.clone(),
            rendered_views: &mut self.rendered_views,
            position_cache: &self.position_cache,
            actions: Default::default(),
            font_cache,
            text_layout_cache: &self.text_layout_cache,
            view_stack: Default::default(),
            notified: Default::default(),
            notify_timers_to_set: Default::default(),
            notify_timers_to_clear: Default::default(),
            cursor_update: Default::default(),
            soft_keyboard_requested: false,
        }
    }

    #[cfg(test)]
    pub fn mock_event_context<'a>(&'a mut self, font_cache: &'a fonts::Cache) -> EventContext<'a> {
        self.create_event_context(font_cache)
    }

    pub fn dispatch_event(&mut self, event: Event, app: &AppContext) -> DispatchResult {
        // Translate all events to be in the coordinate space after factoring in the
        // zoom factor.
        let event = event.scale_down(app.zoom_factor());
        let window_id = self.window_id;
        let mut event_ctx = self.create_event_context(app.font_cache());
        let handled = app.root_view_id(window_id).is_some_and(|root_view_id| {
            event_ctx.dispatch_event_on_view(root_view_id, &DispatchedEvent::from(event), app)
        });

        DispatchResult {
            handled,
            actions: event_ctx.actions,
            notified: event_ctx.notified,
            notify_timers_to_set: event_ctx.notify_timers_to_set,
            notify_timers_to_clear: event_ctx.notify_timers_to_clear,
            cursor_update: event_ctx.cursor_update,
            soft_keyboard_requested: event_ctx.soft_keyboard_requested,
        }
    }

    pub fn scene(&self) -> Option<&Rc<Scene>> {
        self.scene.as_ref()
    }

    pub fn position_cache(&self) -> &PositionCache {
        &self.position_cache
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn position_cache_mut(&mut self) -> &mut PositionCache {
        &mut self.position_cache
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub(crate) fn parents(&self) -> HashMap<EntityId, EntityId> {
        self.parents.clone()
    }

    pub fn set_highlighted_view(&mut self, view_id: EntityId) {
        self.highlighted_view = Some(view_id);
    }

    pub fn clear_highlighted_view(&mut self) {
        self.highlighted_view = None;
    }

    pub fn text_layout_cache(&self) -> &LayoutCache {
        &self.text_layout_cache
    }

    /// Set the parent of a view.
    /// This will be overwritten on the next layout pass, but is useful before the initial layout
    /// of a view.
    pub(crate) fn set_parent(&mut self, view_id: EntityId, parent_id: EntityId) {
        self.parents.insert(view_id, parent_id);
    }
}

impl LayoutContext<'_> {
    fn layout(
        &mut self,
        view_id: EntityId,
        constraint: SizeConstraint,
        app: &AppContext,
    ) -> Vector2F {
        let Some(mut rendered_view) = self.rendered_views.remove(&view_id) else {
            return vec2f(0., 0.);
        };

        if let Some(parent_id) = self.view_stack.last() {
            self.parents.insert(view_id, *parent_id);
        }
        self.view_stack.push(view_id);
        let size = rendered_view.layout(constraint, self, app);
        self.rendered_views.insert(view_id, rendered_view);
        self.view_stack.pop();
        size
    }
}

impl AfterLayoutContext<'_> {
    fn after_layout(&mut self, view_id: EntityId, app: &AppContext) {
        if let Some(mut view) = self.rendered_views.remove(&view_id) {
            view.after_layout(self, app);
            self.rendered_views.insert(view_id, view);
        }
    }
}

impl PaintContext<'_> {
    fn paint(&mut self, view_id: EntityId, origin: Vector2F, app: &AppContext) {
        if let Some(mut tree) = self.rendered_views.remove(&view_id) {
            // If this is the highlighted view, draw a debug rectangle with the
            // same bounds as the view.
            if self.highlighted_view == Some(view_id) {
                if let Some(size) = tree.size() {
                    self.scene
                        .draw_rect_with_hit_recording(RectF::new(origin, size))
                        .with_border(
                            crate::elements::Border::all(2.)
                                .with_border_color(pathfinder_color::ColorU::new(0, 255, 255, 255)),
                        );
                }
            }
            self.views_painted.insert(view_id);
            tree.paint(origin, self, app);
            self.rendered_views.insert(view_id, tree);
        }
    }

    /// Notifies the window it needs a repaint after a certain duration.
    pub fn repaint_after(&mut self, delay: Duration) {
        let start_time = Instant::now();
        let new_repaint_at = start_time + delay;

        // We want the repaint timer with the nearest repaint time.
        if self
            .repaint_at
            .is_some_and(|repaint_at| repaint_at <= new_repaint_at)
        {
            return;
        }
        self.repaint_at(new_repaint_at);
    }

    /// Notifies the window it needs a repaint at a certain Instant.
    /// If there's an existing repaint_at time, keeps the earlier time.
    pub fn repaint_at(&mut self, new_repaint_at: Instant) {
        // We want the repaint timer with the nearest repaint time.
        if self
            .repaint_at
            .is_some_and(|repaint_at| repaint_at <= new_repaint_at)
        {
            return;
        }
        self.repaint_at = Some(new_repaint_at);
    }

    pub fn repaint_after_load(&mut self, asset: AssetHandle) {
        self.pending_assets.insert(asset);
    }
}

impl EventContext<'_> {
    pub fn dispatch_event_on_view(
        &mut self,
        view_id: EntityId,
        event: &DispatchedEvent,
        app: &AppContext,
    ) -> bool {
        if let Some(mut element) = self.rendered_views.remove(&view_id) {
            self.view_stack.push(view_id);
            let handled = element.dispatch_event(event, self, app);
            self.rendered_views.insert(view_id, element);
            self.view_stack.pop();
            handled
        } else {
            false
        }
    }

    pub fn dispatch_action<A: 'static + Any>(&mut self, name: &'static str, arg: A) {
        self.actions.push(DispatchedAction {
            view_id: *self.view_stack.last().unwrap(),
            kind: DispatchedActionKind::Legacy {
                name,
                arg: Box::new(arg),
            },
        });
    }

    pub fn dispatch_typed_action<A: Action>(&mut self, action: A) {
        self.actions.push(DispatchedAction {
            view_id: *self.view_stack.last().unwrap(),
            kind: DispatchedActionKind::Typed(Box::new(action)),
        });
    }

    pub fn notify(&mut self) {
        self.notified.insert(*self.view_stack.last().unwrap());
    }

    /// Notifies the view it needs a redraw after a certain duration and returns
    /// a timer_id and end_time associated with the notify
    pub fn notify_after(&mut self, delay: Duration) -> (TaskId, Instant) {
        let timer_id = TaskId::new();
        let start_time = Instant::now();
        let notify_at = start_time + delay;
        self.notify_timers_to_set.insert(
            timer_id,
            ViewToNotify {
                view_id: *self
                    .view_stack
                    .last()
                    .expect("last view id should be defined"),
                notify_at,
            },
        );
        (timer_id, notify_at)
    }

    /// Clears the given notify timer
    pub fn clear_notify_timer(&mut self, timer_id: TaskId) {
        self.notify_timers_to_clear.insert(timer_id);
    }

    /// Sets a cursor update.  If one is already set, then only
    /// reset if this one is at a higher z-index.
    pub fn set_cursor(&mut self, cursor: Cursor, at_z_index: ZIndex) {
        match self.cursor_update {
            // Don't override cursor if the current z_index is higher.
            Some(CursorUpdate::Set { z_index, .. }) if z_index > at_z_index => (),
            _ => {
                self.cursor_update = Some(CursorUpdate::Set {
                    cursor,
                    z_index: at_z_index,
                    view_id: *self
                        .view_stack
                        .last()
                        .expect("view stack cannot be empty when dispatching event"),
                })
            }
        };
    }

    /// Resets the cursor if a new one is not already set as part of
    /// this dispatch
    pub fn reset_cursor(&mut self) {
        if self.cursor_update.is_none() {
            self.cursor_update = Some(CursorUpdate::Reset);
        }
    }

    /// Request that the soft keyboard be shown on mobile devices.
    /// This is used on mobile WASM to trigger the keyboard when a text input area is tapped.
    pub fn request_soft_keyboard(&mut self) {
        self.soft_keyboard_requested = true;
    }
}

impl SizeConstraint {
    pub fn new(min: Vector2F, max: Vector2F) -> Self {
        Self { min, max }
    }

    pub fn strict(size: Vector2F) -> Self {
        Self {
            min: size,
            max: size,
        }
    }

    /// Computes constraints for child elements of a Flex where children should
    /// be tightly bound to the parent constraints along the cross axis, but
    /// are unbounded along the main axis.
    pub fn tight_on_cross_axis(main_axis: Axis, parent_constraint: SizeConstraint) -> Self {
        match main_axis {
            Axis::Horizontal => Self {
                min: vec2f(0.0, parent_constraint.max.y()),
                max: vec2f(f32::INFINITY, parent_constraint.max.y()),
            },
            Axis::Vertical => Self {
                min: vec2f(parent_constraint.max.x(), 0.),
                max: vec2f(parent_constraint.max.x(), f32::INFINITY),
            },
        }
    }

    /// For the child elements of the Flex, we want unbounded constraint on the
    /// axis which Flex is expanding upon (horizontally for rows and vertically for columns)
    /// and the same max constraint as the parent on the axis which Flex is constrained on.
    /// We don't set any min cross-axis constraint for the child - it is allowed to be
    /// smaller than the flex parent.
    pub fn child_constraint_along_axis(axis: Axis, parent_constraint: SizeConstraint) -> Self {
        let (_, max) = parent_constraint.constraint_for_axis(axis.invert());
        match axis {
            Axis::Horizontal => Self {
                min: vec2f(0.0, 0.0),
                max: vec2f(f32::INFINITY, max),
            },
            Axis::Vertical => Self {
                min: vec2f(0.0, 0.0),
                max: vec2f(max, f32::INFINITY),
            },
        }
    }

    /// Apply this size constraint to a Vector2f that represents a size.
    pub fn apply(&self, size: Vector2F) -> Vector2F {
        size.clamp(self.min, self.max)
    }

    pub fn max_along(&self, axis: Axis) -> f32 {
        match axis {
            Axis::Horizontal => self.max.x(),
            Axis::Vertical => self.max.y(),
        }
    }

    /// Returns a min, max pair along the given axis..
    pub fn constraint_for_axis(&self, axis: Axis) -> (f32, f32) {
        match axis {
            Axis::Horizontal => (self.min.x(), self.max.x()),
            Axis::Vertical => (self.min.y(), self.max.y()),
        }
    }
}

use super::Element;
use crate::geometry::rect::RectF;

impl<T: View> ChildView<T> {
    pub fn new(handle: &ViewHandle<T>) -> Self {
        Self::with_id(handle.id())
    }

    pub fn with_id(view_id: EntityId) -> Self {
        Self {
            view_id,
            size: None,
            origin: None,
            phantom_data: Default::default(),
        }
    }
}

impl<T> Element for ChildView<T> {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size = ctx.layout(self.view_id, constraint, app);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        ctx.after_layout(self.view_id, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        ctx.paint(self.view_id, origin, app);
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        ctx.dispatch_event_on_view(self.view_id, event, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

#[cfg(test)]
#[path = "presenter_tests.rs"]
mod tests;
