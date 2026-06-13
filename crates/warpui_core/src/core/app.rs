use std::any::{Any, TypeId};
use std::cell::{RefCell, RefMut};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::pin::pin;
use std::rc::{self, Rc};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::prelude::*;
use instant::Instant;
use itertools::Itertools;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use pathfinder_geometry::rect::RectF;
use rustc_hash::FxHashMap;

use super::{
    autotracking, ActionHandlersByName, AddWindowOptions, BlurContext, FocusContext,
    GlobalActionCallback, GlobalShortcut, GuiPresenterState, InvalidationCallback, Observation,
    PendingUnsubscribes, RefCounts, RenderOutput, Subscription, TaskCallback, TypedActionCallback,
    TypedActionView, View, ViewContext, ViewHandle, ViewType,
};
use crate::accessibility::{AccessibilityVerbosity, ActionAccessibilityContent};
use crate::actions::StandardAction;
use crate::app_focus_telemetry::AppFocusInfo;
use crate::assets::asset_cache::{AssetCache, AssetHandle, AssetSource};
use crate::assets::AssetProvider;
use crate::core::{ActionType, Window};
use crate::fonts::{self, FallbackFontModel};
use crate::image_cache::{self, ImageCache};
use crate::keymap::{
    BindingLens, Context, CustomTag, DescriptionContext, EditableBinding, EditableBindingLens,
    FixedBinding, IsBindingValid, Keystroke, MatchResult, Matcher, Trigger,
};
use crate::modals::{
    AlertDialog, AlertDialogWithCallbacks, AppModalCallback, ModalId, PlatformModalResponseData,
};
use crate::notification::{NotificationSendError, RequestPermissionsOutcome, UserNotification};
use crate::platform::app::TerminationResult;
use crate::platform::file_picker::{FilePickerConfiguration, FilePickerError};
use crate::platform::{
    self, Cursor, FullscreenState, MicrophoneAccessState, SaveFilePickerConfiguration, SystemTheme,
    TerminationMode, WindowBounds, WindowStyle,
};
use crate::r#async::executor::{self, Background, Foreground, ForegroundTask};
use crate::r#async::{block_on, FutureId, SpawnableOutput, Timer};
use crate::util::post_inc;
use crate::windowing::{self, WindowManager};
use crate::{
    assets, Action, AddSingletonModel, AnyModel, AnyModelHandle, ApplicationBundleInfo, Clipboard,
    Effect, Entity, EntityId, Event, GetSingletonModelHandle, ModelAsRef, ModelContext,
    ModelHandle, NextNewWindowsHasThisWindowsBoundsUponClose, ReadModel, ReadView, SingletonEntity,
    SpawnedFuture, TaskId, UpdateModel, UpdateView, ViewAsRef, WindowId, WindowInvalidation,
    ZoomFactor,
};

mod gui;

lazy_static! {
    static ref LAST_USER_ACTION_UNIX_TIMESTAMP: AtomicI64 = AtomicI64::new(0);
}

#[derive(Clone)]
pub struct App(Rc<RefCell<AppContext>>);

impl App {
    pub fn test<A: assets::AssetProvider, T: 'static, F: Future<Output = T> + 'static>(
        asset_provider: A,
        f: impl FnOnce(App) -> F,
    ) -> T {
        log::info!("Starting test app...");
        let platform = Box::new(platform::test::AppDelegate::new().unwrap());
        let window_manager = Box::new(platform::test::WindowManager::new());
        let font_cache = Box::new(platform::test::FontDB);
        let executor = Rc::new(executor::Foreground::test());
        let app = Self(Rc::new(RefCell::new(AppContext::with_foreground_executor(
            executor.clone(),
            platform,
            window_manager,
            font_cache,
            Box::new(asset_provider),
            true, /* is_unit_test */
        ))));
        {
            let mut app_mut = app.0.borrow_mut();
            app_mut.weak_self = Rc::downgrade(&app.0);
        }
        log::info!("Executing test...");
        let result = block_on(executor.run(f(app)));
        log::info!("Test complete; terminating application...");
        result
    }

    pub fn new(
        platform_delegate: Box<dyn platform::Delegate>,
        window_manager: Box<dyn platform::WindowManager>,
        font_db: Box<dyn platform::FontDB>,
        asset_provider: Box<dyn AssetProvider>,
    ) -> Result<Self> {
        let app = Self(Rc::new(RefCell::new(AppContext::new(
            platform_delegate,
            window_manager,
            font_db,
            asset_provider,
        )?)));
        app.0.borrow_mut().weak_self = Rc::downgrade(&app.0);
        Ok(app)
    }

    pub(crate) fn can_borrow_mut(&self) -> bool {
        self.0.try_borrow_mut().is_ok()
    }

    pub fn has_window_invalidations(&self, window_id: WindowId) -> bool {
        self.0.borrow().has_window_invalidations(window_id)
    }

    pub fn window_bounds(&self, window_id: &WindowId) -> Option<RectF> {
        self.0.borrow().window_bounds(window_id)
    }

    pub fn next_window_bounds_and_style(&self) -> (WindowBounds, WindowStyle) {
        self.0.borrow().next_window_bounds_and_style()
    }

    pub fn focused_view_id(&self, window_id: WindowId) -> Option<EntityId> {
        self.0.borrow().focused_view_id(window_id)
    }

    pub fn is_window_open(&self, window_id: WindowId) -> bool {
        self.0.borrow().is_window_open(window_id)
    }

    pub fn foreground_executor(&self) -> Rc<Foreground> {
        self.0.borrow().foreground.clone()
    }

    pub fn background_executor(&self) -> Arc<Background> {
        self.0.borrow().background.clone()
    }

    pub fn set_a11y_verbosity(&mut self, verbosity: AccessibilityVerbosity) {
        self.0.borrow_mut().set_a11y_verbosity(verbosity);
    }

    pub fn on_window_invalidated<F: 'static + FnMut(WindowId, &mut AppContext)>(
        &self,
        window_id: WindowId,
        callback: F,
    ) {
        self.0
            .borrow_mut()
            .on_window_invalidated(window_id, callback);
    }

    /// Adds an action with a given handler that is executed when the action is dispatched. The
    /// action handler should return whether the action was handled. If it returns false this means
    /// a parent view that listens on the same action name will receive the action.
    pub fn add_action<S, V, T, F>(&self, name: S, handler: F)
    where
        S: Into<String>,
        V: View,
        T: Any,
        F: 'static + FnMut(&mut V, &T, &mut ViewContext<V>) -> bool,
    {
        self.0.borrow_mut().add_action(name, handler);
    }

    pub fn add_global_action<S, T, F>(&self, name: S, handler: F)
    where
        S: Into<String>,
        T: 'static + Any,
        F: 'static + FnMut(&T, &mut AppContext),
    {
        self.0.borrow_mut().add_global_action(name, handler);
    }

    pub fn dispatch_action<T: 'static + Any>(
        &self,
        window_id: WindowId,
        responder_chain: &[EntityId],
        name: &str,
        arg: T,
    ) {
        self.0.borrow_mut().dispatch_action(
            window_id,
            responder_chain,
            name,
            &arg,
            log::Level::Info,
        );
    }

    pub fn dispatch_typed_action(
        &self,
        window_id: WindowId,
        responder_chain: &[EntityId],
        action: &dyn Action,
    ) {
        self.0.borrow_mut().dispatch_typed_action(
            window_id,
            responder_chain,
            action,
            log::Level::Info,
        );
    }

    pub fn last_active_timestamp() -> i64 {
        LAST_USER_ACTION_UNIX_TIMESTAMP.load(Ordering::SeqCst)
    }

    pub fn record_last_active_timestamp() {
        LAST_USER_ACTION_UNIX_TIMESTAMP.fetch_max(Utc::now().timestamp(), Ordering::SeqCst);
    }

    pub fn dispatch_global_action<T: 'static + Any>(&self, name: &str, arg: T) {
        self.0.borrow_mut().dispatch_global_action(name, &arg);
    }

    pub fn dispatch_standard_action(&mut self, window_id: WindowId, action: StandardAction) {
        log::info!("Dispatching standard action {action:?}");
        self.update(move |ctx| {
            let responder_chain = ctx.get_responder_chain(window_id);
            let res = ctx.dispatch_standard_action(action, window_id, &responder_chain);
            if let Err(error) = res {
                log::error!("error dispatching standard action: {error}");
            }
        });
    }

    pub fn key_bindings_dispatching_enabled(&mut self, window_id: WindowId) -> bool {
        self.0.borrow_mut().key_bindings_enabled(window_id)
    }

    pub fn dispatch_keystroke(
        &self,
        window_id: WindowId,
        responder_chain: &[EntityId],
        keystroke: &Keystroke,
        is_composing: bool,
    ) -> Result<bool> {
        let mut state = self.0.borrow_mut();
        state.dispatch_keystroke(window_id, responder_chain, keystroke, is_composing)
    }

    pub fn add_model<T, F>(&mut self, build_model: F) -> ModelHandle<T>
    where
        T: Entity,
        F: FnOnce(&mut ModelContext<T>) -> T,
    {
        let mut state = self.0.borrow_mut();
        state.pending_flushes += 1;
        let handle = state.add_model(build_model);
        state.flush_effects();
        handle
    }

    pub fn add_singleton_model<T, F>(&self, build_model: F) -> ModelHandle<T>
    where
        T: SingletonEntity,
        F: FnOnce(&mut ModelContext<T>) -> T,
    {
        let mut state = self.0.borrow_mut();
        state.pending_flushes += 1;
        let handle = state.add_singleton_model(build_model);
        state.flush_effects();
        handle
    }

    pub fn get_singleton_model_handle<T>(&self) -> ModelHandle<T>
    where
        T: SingletonEntity,
    {
        self.0.borrow().get_singleton_model_handle()
    }

    pub fn add_window_with_bounds<T, F>(
        &mut self,
        style: WindowStyle,
        bounds: WindowBounds,
        build_root_view: F,
    ) -> (WindowId, ViewHandle<T>)
    where
        T: View + TypedActionView,
        F: FnOnce(&mut ViewContext<T>) -> T,
    {
        self.0.borrow_mut().add_window(
            AddWindowOptions {
                window_style: style,
                window_bounds: bounds,
                ..Default::default()
            },
            build_root_view,
        )
    }

    pub fn add_window<T, F>(
        &mut self,
        style: WindowStyle,
        build_root_view: F,
    ) -> (WindowId, ViewHandle<T>)
    where
        T: View + TypedActionView,
        F: FnOnce(&mut ViewContext<T>) -> T,
    {
        self.add_window_with_bounds(style, WindowBounds::Default, build_root_view)
    }

    pub fn window_ids(&self) -> Vec<WindowId> {
        self.0.borrow().window_ids().collect()
    }

    pub fn root_view<T: View>(&self, window_id: WindowId) -> Option<ViewHandle<T>> {
        self.0.borrow().root_view(window_id)
    }

    pub fn root_view_id(&self, window_id: WindowId) -> Option<EntityId> {
        self.0.borrow().root_view_id(window_id)
    }

    pub fn add_view<T, F>(&mut self, window_id: WindowId, build_view: F) -> ViewHandle<T>
    where
        T: View,
        F: FnOnce(&mut ViewContext<T>) -> T,
    {
        let mut state = self.0.borrow_mut();
        state.pending_flushes += 1;
        let handle = state.add_view(window_id, build_view);
        state.flush_effects();
        handle
    }

    pub fn add_typed_action_view<V, F>(
        &mut self,
        window_id: WindowId,
        build_view: F,
    ) -> ViewHandle<V>
    where
        V: TypedActionView + View,
        F: FnOnce(&mut ViewContext<V>) -> V,
    {
        self.0
            .borrow_mut()
            .add_typed_action_view(window_id, build_view)
    }

    pub fn add_option_view<T, F>(
        &mut self,
        window_id: WindowId,
        build_view: F,
    ) -> Option<ViewHandle<T>>
    where
        T: View,
        F: FnOnce(&mut ViewContext<T>) -> Option<T>,
    {
        let mut state = self.0.borrow_mut();
        state.pending_flushes += 1;
        let handle = state.add_option_view(window_id, build_view);
        state.flush_effects();
        handle
    }

    pub fn read<T, F: FnOnce(&AppContext) -> T>(&self, callback: F) -> T {
        callback(&self.0.borrow())
    }

    pub fn update<T, F: FnOnce(&mut AppContext) -> T>(&mut self, callback: F) -> T {
        let mut state = self.0.borrow_mut();
        state.pending_flushes += 1;
        let result = callback(&mut state);
        state.flush_effects();
        result
    }

    // Returns objects in the order they were added
    pub fn views_of_type<T: View>(&self, window_id: WindowId) -> Option<Vec<ViewHandle<T>>> {
        let views = self.0.borrow().views_of_type(window_id);
        // Since iter yields arbitrary order, sort by id
        match views {
            Some(mut views) => {
                views.sort_by_key(|view| view.id());
                Some(views)
            }
            None => None,
        }
    }

    // Returns objects in the order they were added
    pub fn models_of_type<M: Entity>(&self) -> Vec<ModelHandle<M>> {
        let mut models = self.0.borrow().models_of_type();
        // Since iter yields arbitrary order, sort by id
        models.sort_by_key(|models| models.id());
        models
    }

    #[cfg(test)]
    pub fn finish_pending_tasks(&self) -> impl Future<Output = ()> {
        self.0.borrow().finish_pending_tasks()
    }

    pub(crate) fn as_mut(&mut self) -> AppContextRefMut<'_> {
        AppContextRefMut::new(self.0.as_ref().borrow_mut())
    }

    pub fn termination_result(self) -> Option<TerminationResult> {
        self.0.borrow_mut().termination_result.take()
    }
}

impl ModelAsRef for App {
    // Unsupported for the same reasons as [`ViewAsRef`] being
    // unsupported for [`App`].
    fn model<T: Entity>(&self, _: &ModelHandle<T>) -> &T {
        unimplemented!("Read from [`App::read_model`] instead");
    }
}

impl ReadModel for App {
    fn read_model<T, F, S>(&self, handle: &ModelHandle<T>, read: F) -> S
    where
        T: Entity,
        F: FnOnce(&T, &AppContext) -> S,
    {
        let state = self.0.borrow();
        state.read_model(handle, read)
    }
}

impl UpdateModel for App {
    fn update_model<T, F, S>(&mut self, handle: &ModelHandle<T>, update: F) -> S
    where
        T: Entity,
        F: FnOnce(&mut T, &mut ModelContext<T>) -> S,
    {
        self.as_mut().update_model(handle, update)
    }
}

impl UpdateView for App {
    fn update_view<T, F, S>(&mut self, handle: &ViewHandle<T>, update: F) -> S
    where
        T: View,
        F: FnOnce(&mut T, &mut ViewContext<T>) -> S,
    {
        self.as_mut().update_view(handle, update)
    }
}

impl ReadView for App {
    fn read_view<T, F, S>(&self, handle: &ViewHandle<T>, read: F) -> S
    where
        T: View,
        F: FnOnce(&T, &AppContext) -> S,
    {
        let state = self.0.borrow();
        state.read_view(handle, read)
    }
}

impl ViewAsRef for App {
    // This is unimplemented because we would need to do
    // some borrow-gymnastics, like returning a wrapper type
    // that internally holds the reference, to get around a
    // "returned reference to temporary value" error.
    //
    // That effort is currently unjustified because we want
    // to ideally strip the *AsRef, Read* and Update* implementations
    // from [`App`].
    fn view<T: View>(&self, _handle: &ViewHandle<T>) -> &T {
        unimplemented!("Read from [`App::read_view`] instead");
    }

    fn try_view<T: View>(&self, _handle: &ViewHandle<T>) -> Option<&T> {
        unimplemented!("Read from [`App::read_view`] instead");
    }
}

impl AddSingletonModel for App {
    fn add_singleton_model<T, F>(&mut self, build_model: F) -> ModelHandle<T>
    where
        T: SingletonEntity,
        F: FnOnce(&mut super::ModelContext<T>) -> T,
    {
        self.0.borrow_mut().add_singleton_model(build_model)
    }
}

impl GetSingletonModelHandle for App {
    fn get_singleton_model_handle<T: SingletonEntity>(&self) -> ModelHandle<T> {
        self.0.borrow_mut().get_singleton_model_handle()
    }
}

/// A wrapper around a mutably-borrowed app context.
///
/// This is necessary in order to ensure that all pending effects are flushed
/// before we return the mutable borrow.
pub(crate) struct AppContextRefMut<'a>(RefMut<'a, AppContext>);

impl<'a> AppContextRefMut<'a> {
    fn new(mut inner: RefMut<'a, AppContext>) -> Self {
        inner.pending_flushes += 1;
        Self(inner)
    }
}

impl std::ops::Drop for AppContextRefMut<'_> {
    fn drop(&mut self) {
        self.0.flush_effects();
    }
}

impl<'a> std::ops::Deref for AppContextRefMut<'a> {
    type Target = RefMut<'a, AppContext>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for AppContextRefMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct RepaintTask {
    pub task: ForegroundTask,

    /// The window from which this comes
    pub window_id: WindowId,

    pub repaint_trigger: RepaintTrigger,
}

pub enum RepaintTrigger {
    Timer { instant: Instant },
    AssetLoaded { asset_handle: AssetHandle },
}

type EventMunger = dyn Fn(&mut Event, &mut AppContext);

pub type DrawFrameErrorCallback = dyn Fn(&mut AppContext, WindowId);

pub type FrameDrawnCallback = dyn Fn(&mut AppContext, WindowId);

pub type BeforeOpenUrlCallback = dyn Fn(&str, &AppContext) -> String;

/// The application context: the global object owning all models, views,
/// windows, and the registries that drive event/action dispatch.
pub struct AppContext {
    /////////////////////////
    // Fields from AppContext
    /////////////////////////
    pub(super) models: HashMap<EntityId, Box<dyn AnyModel>>,
    /// A mapping from type ID -> handle to the single global model of that
    /// type.  The handle is a strong reference, ensuring the model will not be
    /// dropped during the lifetime of the application.
    ///
    /// We use an FxHashMap here because `TypeId` hashes as a u64, and FxHasher
    /// is the fastest commonly-used hasher for this type.
    singleton_models: FxHashMap<TypeId, AnyModelHandle>,
    pub(super) windows: HashMap<WindowId, Window>,
    pub(super) ref_counts: Arc<Mutex<RefCounts>>,
    pub(super) platform_delegate: Box<dyn platform::Delegate>,

    ////////////////////////////////
    // Fields from MutableAppContext
    ////////////////////////////////
    actions: HashMap<TypeId, ActionHandlersByName>,
    /// Map of typed actions to their internal handler functions
    ///
    /// We use a nested HashMap to key on both the ActionType and the ViewType, which allows us to
    /// efficiently look up the appropriate handler while iterating the responder_chain on dispatch
    ///
    /// Safety Note: The `TypedActionCallback` must only be called with parameters that match the
    /// type keys, as it requires the values to appropriately downcast.
    typed_actions: HashMap<ActionType, HashMap<ViewType, Box<TypedActionCallback>>>,
    /// GUI presentation state (presenters + position caches + rendering config).
    presentation: GuiPresenterState,
    /// Per-window dirty tracking. Backend-neutral (no GUI type), so both the GUI
    /// render loop and the TUI runtime drive redraws through the same map.
    window_invalidations: HashMap<WindowId, WindowInvalidation>,
    /// Per-window "a window needs redrawing" callbacks. Backend-neutral.
    invalidation_callbacks: HashMap<WindowId, Box<InvalidationCallback>>,
    global_actions: HashMap<String, Vec<Box<GlobalActionCallback>>>,
    keystroke_matcher: Matcher,
    next_task_id: usize,
    weak_self: rc::Weak<RefCell<Self>>,
    pub(super) subscriptions: HashMap<EntityId, Vec<Subscription>>,
    pub(super) observations: HashMap<EntityId, Vec<Observation>>,
    /// Tracks pending unsubscribes during event emission.
    /// When `emit_event` is processing callbacks, unsubscribes are deferred here to avoid
    /// O(N²) tombstone scanning. The unsubscribes are processed at the end of event emission.
    pub(super) pending_unsubscribes: Option<PendingUnsubscribes>,
    disabled_key_bindings_windows: HashSet<WindowId>,
    window_bounds: HashMap<WindowId, Option<RectF>>,
    next_window_bounds_map: HashMap<WindowId, NextNewWindowsHasThisWindowsBoundsUponClose>,

    /// The bounds of the next window to open.  Typically this is set
    /// using the position of the last window that was closed.
    next_window_bounds: Option<RectF>,

    /// Map of window id to the last mouse moved event on that window.  Used
    /// for dispatch of synthetic mouse moved events to trigger hover behavior.
    window_last_mouse_moved_event: HashMap<WindowId, Rc<RefCell<Option<Event>>>>,
    foreground: Rc<executor::Foreground>,
    background: Arc<executor::Background>,
    pub(super) task_callbacks: HashMap<usize, TaskCallback>,
    /// Callbacks that trigger a view redraw on a delay
    /// We store references in case we need to cancel them.
    /// The map key is the timer id returned by EventContext.notify_after
    notify_tasks: HashMap<TaskId, ForegroundTask>,
    repaint_tasks: HashMap<TaskId, RepaintTask>,

    /// A channel used in tests that receives messages whenever a task
    /// completes.
    #[cfg(test)]
    task_done: (async_channel::Sender<usize>, async_channel::Receiver<usize>),
    pub(super) pending_effects: VecDeque<Effect>,
    pending_flushes: usize,
    flushing_effects: bool,
    app_focus_info: AppFocusInfo,
    #[allow(clippy::type_complexity)]
    first_frame_callback: Option<Box<dyn Fn(&mut AppContext)>>,
    frame_drawn_callback: Option<Box<FrameDrawnCallback>>,
    global_shortcuts: HashMap<Keystroke, GlobalShortcut>,
    next_frame_callbacks: HashMap<WindowId, Vec<Box<dyn Fn()>>>,
    on_draw_frame_error_callback: Option<Box<DrawFrameErrorCallback>>,

    /// The "event munger" allows for clients to hook into events and modify them
    /// before they are dispatched.
    event_munger: Box<EventMunger>,

    /// Callback invoked before opening any URL.
    before_open_url_callback: Box<BeforeOpenUrlCallback>,

    pub(super) a11y_verbosity: AccessibilityVerbosity,

    #[allow(dead_code)]
    spawned_futures: HashMap<FutureId, SpawnedFuture>,

    /// This maps modal IDs to the struct of data needed to send the response back to the context
    /// which requested a platform native modal. This is needed because these modals are opened
    /// asynchronously in platform code. Only the ID is propagated to/from platform code. Once we
    /// have the modal response, we only have its ID, and so we look up the data in this map from
    /// that ID.
    platform_modal_data_map: HashMap<ModalId, PlatformModalResponseData>,

    /// If the cursor shape was changed by a view, keep track of that so that we can reset the
    /// cursor if that view goes away.
    pub(crate) cursor_updated_for_view: Option<(WindowId, EntityId)>,
    is_unit_test: bool,

    termination_result: OnceLock<TerminationResult>,

    /// The current zoom (magnification) factor of the application.
    zoom_factor: ZoomFactor,

    /// Maps view entity IDs to their containing window.
    /// This is the source of truth for which window a view belongs to,
    /// enabling views to be transferred between windows.
    ///
    /// Visibility is `pub(super)` because the `view::handle` module needs to access
    /// this field for dynamic window lookup in `ViewHandle::window_id()`,
    /// `WeakViewHandle::upgrade()`, and `WeakViewHandle::window_id()`.
    pub(super) view_to_window: HashMap<EntityId, WindowId>,

    /// Maps child view → parent view for views created via `add_typed_action_view_with_parent`.
    /// Unlike the presenter's layout-time parent map, this persists across renders and
    /// includes views that are conditionally rendered. Used by `transfer_view_tree_to_window`
    /// to discover non-rendered child views that must move with their parent.
    ///
    /// Populated by `add_typed_action_view_internal` when a parent_view_id is provided,
    /// and cleaned up in `remove_dropped_items` when views are dropped.
    structural_child_to_parent: HashMap<EntityId, EntityId>,

    /// Reverse of `structural_child_to_parent`: maps parent view → set of child views.
    /// Enables efficient traversal in `transfer_structural_children` without iterating
    /// all views in the source window.
    structural_parent_to_children: HashMap<EntityId, HashSet<EntityId>>,

    /// Backend-neutral child-view → parent-view map, per window. This is the view
    /// hierarchy the shared core walks for [`Self::view_ancestors`], the responder
    /// chain, and focus ancestor propagation — for any backend.
    ///
    /// Populated from two sources: creation-time structural parentage
    /// ([`Self::record_view_parent`], called when a typed-action view is created
    /// with a parent) and the active backend's render pass, which reports the
    /// child-view embeddings it discovers while laying out a frame
    /// ([`Self::report_view_embeddings`]). Entries are removed when views are
    /// dropped or transferred out of the window, and when the window closes.
    view_parents: HashMap<WindowId, HashMap<EntityId, EntityId>>,

    /// When set, all focus changes to this window are suppressed.
    /// Used during tab drag to prevent the new window from stealing focus.
    suppress_focus_for_window: Option<WindowId>,

    /// An optional provider that creates an [`AssetSource`] for loading a
    /// fallback font from a URL string. Injected by the application layer so
    /// that `warpui_core` does not depend on `reqwest`.
    #[allow(clippy::type_complexity)]
    fallback_font_source_provider: Option<Box<dyn Fn(&str) -> AssetSource>>,
}

impl AppContext {
    pub(crate) fn new(
        platform_delegate: Box<dyn platform::Delegate>,
        window_manager: Box<dyn platform::WindowManager>,
        font_db: Box<dyn platform::FontDB>,
        asset_provider: Box<dyn AssetProvider>,
    ) -> Result<Self> {
        Ok(Self::with_foreground_executor(
            Rc::new(executor::Foreground::platform(
                platform_delegate.dispatch_delegate(),
            )?),
            platform_delegate,
            window_manager,
            font_db,
            asset_provider,
            false, /* is_unit_test */
        ))
    }

    pub fn is_window_open(&self, window_id: WindowId) -> bool {
        self.windows.contains_key(&window_id)
    }

    /// Sets or clears the window for which focus changes should be suppressed.
    /// When set, all `ctx.focus()` calls targeting this window will be ignored.
    pub fn set_suppress_focus_for_window(&mut self, window_id: Option<WindowId>) {
        self.suppress_focus_for_window = window_id;
    }

    fn with_foreground_executor(
        foreground: Rc<executor::Foreground>,
        platform_delegate: Box<dyn platform::Delegate>,
        window_manager: Box<dyn platform::WindowManager>,
        font_db: Box<dyn platform::FontDB>,
        asset_provider: Box<dyn assets::AssetProvider>,
        is_unit_test: bool,
    ) -> Self {
        let mut ctx = Self {
            // AppContext fields
            models: Default::default(),
            singleton_models: Default::default(),
            windows: Default::default(),
            ref_counts: Arc::new(Mutex::new(RefCounts::default())),
            platform_delegate,
            // AppContext fields
            actions: Default::default(),
            typed_actions: Default::default(),
            global_actions: Default::default(),
            presentation: Default::default(),
            window_invalidations: Default::default(),
            invalidation_callbacks: Default::default(),
            keystroke_matcher: Default::default(),
            disabled_key_bindings_windows: Default::default(),
            next_task_id: 0,
            weak_self: rc::Weak::default(),
            subscriptions: Default::default(),
            next_window_bounds: None,
            next_window_bounds_map: Default::default(),
            observations: Default::default(),
            pending_unsubscribes: None,
            window_bounds: Default::default(),
            window_last_mouse_moved_event: Default::default(),
            foreground: foreground.clone(),
            background: Default::default(),
            task_callbacks: Default::default(),
            notify_tasks: Default::default(),
            repaint_tasks: Default::default(),
            spawned_futures: Default::default(),
            #[cfg(test)]
            task_done: async_channel::unbounded(),
            pending_effects: VecDeque::new(),
            pending_flushes: 0,
            flushing_effects: false,
            app_focus_info: AppFocusInfo::new(),
            first_frame_callback: None,
            frame_drawn_callback: None,
            global_shortcuts: Default::default(),
            next_frame_callbacks: Default::default(),
            event_munger: Box::new(|_evt, _ctx| {}),
            before_open_url_callback: Box::new(|url, _ctx| url.to_owned()),
            a11y_verbosity: Default::default(),
            platform_modal_data_map: Default::default(),
            on_draw_frame_error_callback: None,
            cursor_updated_for_view: None,
            is_unit_test,
            termination_result: Default::default(),
            zoom_factor: ZoomFactor::default(),
            view_to_window: Default::default(),
            structural_child_to_parent: Default::default(),
            structural_parent_to_children: Default::default(),
            view_parents: Default::default(),
            suppress_focus_for_window: None,
            fallback_font_source_provider: None,
        };

        // Register a variety of required/core singleton models.
        ctx.add_singleton_model(|_| fonts::Cache::new(font_db));
        ctx.add_singleton_model(|_| WindowManager::new(window_manager));
        ctx.add_singleton_model(|ctx| {
            AssetCache::new(asset_provider, foreground, ctx.background_executor())
        });
        ctx.add_singleton_model(|_| ImageCache::new());
        ctx.add_singleton_model(|_| FallbackFontModel::new());

        if !is_unit_test {
            ctx.background_executor()
                .spawn(async {
                    image_cache::prewarm_svg_font_db();
                })
                .detach();
        }

        ctx
    }

    /// Registers a provider that creates an [`AssetSource`] for a given URL.
    /// Used to load fallback fonts without pulling `reqwest` into `warpui_core`.
    pub fn set_fallback_font_source_provider(
        &mut self,
        provider: impl Fn(&str) -> AssetSource + 'static,
    ) {
        self.fallback_font_source_provider = Some(Box::new(provider));
    }

    pub fn foreground_executor(&self) -> &Rc<executor::Foreground> {
        &self.foreground
    }

    pub fn background_executor(&self) -> &Arc<executor::Background> {
        &self.background
    }

    pub fn window_bounds(&self, window_id: &WindowId) -> Option<RectF> {
        *self.window_bounds.get(window_id)?
    }

    pub fn update_window_bounds(&mut self, window_id: WindowId, bounds: RectF) {
        self.window_bounds.insert(window_id, Some(bounds));
    }

    /// Moves the OS window to `bounds` and immediately updates the local cache.
    /// Unlike `update_window_bounds` (which only updates the cache when the
    /// platform reports a move), this also commands the platform — so
    /// `window_bounds()` returns the new rect on the same frame, without
    /// waiting for a move-event callback.
    pub fn set_and_cache_window_bounds(&mut self, window_id: WindowId, bounds: RectF) {
        self.windows().set_window_bounds(window_id, bounds);
        self.window_bounds.insert(window_id, Some(bounds));
    }

    fn matches_any_window_bounds(&self, r: RectF) -> bool {
        self.window_bounds.values().any(|b| *b == Some(r))
    }

    /// Create a window showing a modal dialog native to the platform. The modal will synchronously
    /// block all other interactions with the app until dismissed. Each button can have a callback
    /// attached to it in the [`crate::modals::ModalButton`] struct.
    pub fn show_native_platform_modal(
        &mut self,
        alert_data: AlertDialogWithCallbacks<AppModalCallback>,
    ) {
        let id = ModalId::new();
        let (button_titles, button_callbacks) = alert_data
            .button_data
            .into_iter()
            .map(|button| (button.title, button.on_click))
            .unzip();
        let dialog = AlertDialog {
            message_text: alert_data.message_text,
            info_text: alert_data.info_text,
            buttons: button_titles,
        };

        let response_data = PlatformModalResponseData {
            button_callbacks,
            disable_callback: alert_data.on_disable,
        };
        self.platform_modal_data_map.insert(id, response_data);
        self.platform_delegate
            .show_native_platform_modal(id, dialog);
    }

    /// When a native platform modal which was requested by an app/view context returns a response,
    /// this method handles dispatching the right handler for the button clicked. The response is
    /// encoded as a 0-based index into the list of buttons on the modal, and the callback will be
    /// at the same index in the Vec of callbacks.
    /// TODO(CORE-2323): Implement native Windows OS modal
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(crate) fn process_platform_modal_response(
        &mut self,
        modal_id: ModalId,
        response_button_index: usize,
        disable_modal: bool,
    ) {
        if let Some(mut response_data) = self.platform_modal_data_map.remove(&modal_id) {
            if disable_modal {
                (response_data.disable_callback)(self);
            }
            response_data.button_callbacks.remove(response_button_index)(self);
        }
    }

    pub fn zoom_factor(&self) -> ZoomFactor {
        self.zoom_factor
    }

    pub fn report_active_cursor_position_update(&self) {
        self.windows().active_cursor_position_updated();
    }

    pub fn has_window_invalidations(&self, window_id: WindowId) -> bool {
        self.window_invalidations
            .get(&window_id)
            .is_some_and(|invalidation| {
                !invalidation.updated.is_empty() || !invalidation.removed.is_empty()
            })
    }

    fn invalidate_all_views_for_window(&mut self, window_id: WindowId) {
        let Some(window) = self.windows.get(&window_id) else {
            return;
        };
        self.window_invalidations
            .entry(window_id)
            .or_default()
            .updated = window.views.keys().cloned().collect();
    }

    pub fn invalidate_all_views(&mut self) {
        // Mark all views in all windows as invalid.
        for window_id in self.windows.keys().cloned().collect_vec() {
            self.invalidate_all_views_for_window(window_id)
        }
    }

    pub(crate) fn on_global_shortcut_triggered(&mut self, shortcut: Keystroke) {
        if let Some(global_action) = self.global_shortcuts.remove(&shortcut) {
            self.dispatch_global_action(global_action.action, global_action.args.as_ref());
            self.global_shortcuts.insert(shortcut, global_action);
        }
    }

    pub fn on_window_invalidated<F: 'static + FnMut(WindowId, &mut AppContext)>(
        &mut self,
        window_id: WindowId,
        callback: F,
    ) {
        self.invalidation_callbacks
            .insert(window_id, Box::new(callback));
    }

    /// Observes a [`ModelHandle`] for changes, calling `callback` whenever the model is invalidated.
    pub fn observe_model<S, F>(&mut self, handle: &ModelHandle<S>, mut callback: F)
    where
        S: Entity,
        F: 'static + FnMut(ModelHandle<S>, &mut AppContext),
    {
        self.observations
            .entry(handle.id())
            .or_default()
            .push(Observation::FromApp {
                callback: Box::new(move |observed_id, app| {
                    let model = ModelHandle::new(observed_id, &app.ref_counts);
                    callback(model, app)
                }),
            });
    }

    /// Subscribes to a [`ModelHandle`] for changes, calling `callback` with the emitted event whenever the model is invalidated.
    pub fn subscribe_to_model<S, F>(&mut self, handle: &ModelHandle<S>, mut callback: F)
    where
        S: Entity,
        S::Event: 'static,
        F: 'static + FnMut(ModelHandle<S>, &S::Event, &mut AppContext),
    {
        self.subscriptions
            .entry(handle.id())
            .or_default()
            .push(Subscription::FromApp {
                callback: Box::new(move |payload, app, entity_id| {
                    let model = ModelHandle::new(entity_id, &app.ref_counts);
                    let payload: &<S as Entity>::Event =
                        payload.downcast_ref().expect("downcast is type safe");
                    callback(model, payload, app);
                }),
            });
    }

    /// Subscribes to a [`ViewHandle`] for changes, calling `callback` with the emitted event whenever the view is invalidated.
    pub fn subscribe_to_view<S, F>(&mut self, handle: &ViewHandle<S>, mut callback: F)
    where
        S: View,
        S::Event: 'static,
        F: 'static + FnMut(ViewHandle<S>, &S::Event, &mut AppContext),
    {
        self.subscriptions
            .entry(handle.id())
            .or_default()
            .push(Subscription::FromApp {
                callback: Box::new(move |payload, app, entity_id| {
                    let Some(current_window_id) = app.view_to_window.get(&entity_id).copied()
                    else {
                        log::warn!("subscribe_to_view callback: view {entity_id:?} not found in view_to_window");
                        return;
                    };
                    if !app.windows.contains_key(&current_window_id) {
                        log::warn!("subscribe_to_view callback: window {current_window_id:?} not found for view {entity_id:?}");
                        return;
                    }

                    let view = ViewHandle::new(current_window_id, entity_id, &app.ref_counts);
                    let payload: &<S as Entity>::Event =
                        payload.downcast_ref().expect("downcast is type safe");
                    callback(view, payload, app);
                }),
            });
    }

    #[cfg(feature = "test-util")]
    pub(crate) fn register_spawned_future(&mut self, future: SpawnedFuture) -> FutureId {
        let future_id = FutureId::new();
        self.spawned_futures.insert(future_id, future);

        future_id
    }

    #[cfg(not(feature = "test-util"))]
    pub(crate) fn register_spawned_future(&mut self, _future: SpawnedFuture) -> FutureId {
        FutureId::new()
    }

    /// Returns a future that can be awaited to ensure the spawned background future identified by
    /// `future_id` has finished. If no spawned future exists with `future_id` a future that
    /// immediately resolves to the unit type is returned.
    ///
    /// This is useful for tests to ensure that calls to `ctx#spawn` have
    /// completed before asserting correct state.
    #[cfg(feature = "test-util")]
    pub fn await_spawned_future(&mut self, future_id: FutureId) -> future::BoxFuture<'static, ()> {
        match self.spawned_futures.remove(&future_id) {
            None => futures::future::ready(()).boxed(),
            Some(future) => future,
        }
    }

    /// This callback will be invoked immediately after the very first frame in the app is drawn.
    /// After that, the callback will be removed.
    pub fn on_first_frame_drawn<F: 'static + Fn(&mut AppContext)>(&mut self, callback: F) {
        self.first_frame_callback = Some(Box::new(callback));
    }

    ///  Callback that is called whenever a frame is successfully drawn.
    pub fn on_frame_drawn<F: 'static + Fn(&mut AppContext, WindowId)>(&mut self, callback: F) {
        self.frame_drawn_callback = Some(Box::new(callback));
    }

    /// Callback invoked whenever a frame fails to render in a given [`WindowId`].
    pub fn on_draw_frame_error<F: 'static + Fn(&mut AppContext, WindowId)>(&mut self, callback: F) {
        self.on_draw_frame_error_callback = Some(Box::new(callback));
    }

    pub fn unregister_global_shortcut(&mut self, shortcut: &Keystroke) {
        if self.is_wayland() {
            return;
        }
        self.global_shortcuts.remove(shortcut);
        self.platform_delegate.unregister_global_shortcut(shortcut);
    }

    pub fn register_global_shortcut<T: 'static + Any>(
        &mut self,
        mut shortcut: Keystroke,
        action: &'static str,
        arg: T,
    ) {
        if self.is_wayland() {
            return;
        }
        // Note that for global hotkey we don't support registering the meta key so
        // we will treat meta key as alt.
        if shortcut.meta {
            shortcut.meta = false;
            shortcut.alt = true;
        }

        self.global_shortcuts.insert(
            shortcut.clone(),
            GlobalShortcut {
                action,
                args: Box::new(arg),
            },
        );

        self.platform_delegate.register_global_shortcut(shortcut);
    }

    /// Installs the callback so that it'll be invoked after the next frame for the window is
    /// drawn.
    pub fn on_next_frame_drawn<F: 'static + Fn()>(&mut self, window_id: WindowId, callback: F) {
        let entry = self.next_frame_callbacks.entry(window_id).or_default();
        entry.push(Box::new(callback));
    }

    /// Sets a function which gets to inspect and modify events before they are dispatched.
    pub fn set_event_munger<F>(&mut self, handler: F)
    where
        F: 'static + Fn(&mut Event, &mut AppContext),
    {
        self.event_munger = Box::new(handler)
    }

    /// Sets the zoom factor for the application. Changing the zoom factor adjusts
    /// the magnification of every element rendered within the application.
    ///
    /// All views in every window are invalidated when this is invoked.
    ///
    /// ## Validation
    /// The zoom factor is clamped to the range [0.5, 4.0].
    pub fn set_zoom_factor(&mut self, zoom_factor: f32) {
        let zoom_factor = ZoomFactor::new(zoom_factor.clamp(0.5, 4.0));
        self.zoom_factor = zoom_factor;
        self.invalidate_all_views();
    }

    /// Sets the callback invoked before opening a URL.
    pub fn set_before_open_url<F>(&mut self, handler: F)
    where
        F: 'static + Fn(&str, &AppContext) -> String,
    {
        self.before_open_url_callback = Box::new(handler);
    }

    /// Internal helper method to store the handler for a `TypedActionView` being registered
    ///
    /// Creates a handler which will dispatch to `TypedActionView::handle_action` for the given
    /// View + Action combination.
    fn add_typed_action<V>(&mut self)
    where
        V: TypedActionView + View,
    {
        let handler = Box::new(
            |view: &mut dyn Any,
             action: &dyn Any,
             app: &mut AppContext,
             window_id: WindowId,
             view_id: EntityId| {
                let is_screen_reader_enabled = app
                    .platform_delegate
                    .is_screen_reader_enabled()
                    .unwrap_or(false);
                // Safety: The handler is stored in a map keyed on both the ActionType and the
                // ViewType, so we will only call it if both match, making the downcasts safe
                let action = action
                    .downcast_ref()
                    .expect("Handlers are hashed by action type");
                let view = view
                    .downcast_mut()
                    .expect("Handlers are hashed by view type");
                let mut ctx = ViewContext::new(app, window_id, view_id);
                V::handle_action(view, action, &mut ctx);
                if is_screen_reader_enabled {
                    match V::action_accessibility_contents(view, action, &mut ctx) {
                        ActionAccessibilityContent::CustomFn(f) => {
                            app.platform_delegate.set_accessibility_contents(
                                f(action).with_verbosity(app.a11y_verbosity),
                            );
                        }
                        ActionAccessibilityContent::Custom(content) => {
                            app.platform_delegate.set_accessibility_contents(
                                content.with_verbosity(app.a11y_verbosity),
                            );
                        }
                        ActionAccessibilityContent::Empty => {}
                    };
                }
            },
        );

        // Insert the action handler for this view into the `typed_actions` hash
        // We only need to do this once per View type, since the handler is the same for every
        // instance.
        self.typed_actions
            .entry(ActionType::of::<V::Action>())
            .or_default()
            .entry(ViewType::of::<V>())
            .or_insert(handler);
    }

    pub fn add_action<S, V, T, F>(&mut self, name: S, mut handler: F)
    where
        S: Into<String>,
        V: View,
        T: Any,
        F: 'static + FnMut(&mut V, &T, &mut ViewContext<V>) -> bool,
    {
        let name = name.into();
        let name_clone = name.clone();
        let handler = Box::new(
            move |view: &mut dyn Any,
                  arg: &dyn Any,
                  app: &mut AppContext,
                  window_id: WindowId,
                  view_id: EntityId| {
                match arg.downcast_ref() {
                    Some(arg) => {
                        let mut ctx = ViewContext::new(app, window_id, view_id);
                        handler(
                            view.downcast_mut().expect("downcast is type safe"),
                            arg,
                            &mut ctx,
                        )
                    }
                    None => {
                        log::error!("Could not downcast argument for action {name_clone}");
                        false
                    }
                }
            },
        );

        self.actions
            .entry(TypeId::of::<V>())
            .or_default()
            .entry(name)
            .or_default()
            .push(handler);
    }

    pub fn add_global_action<S, T, F>(&mut self, name: S, mut handler: F)
    where
        S: Into<String>,
        T: 'static + Any,
        F: 'static + FnMut(&T, &mut AppContext),
    {
        let name = name.into();
        let name_clone = name.clone();
        let handler = Box::new(
            move |arg: &dyn Any,
                  location: &'static std::panic::Location<'static>,
                  app: &mut AppContext| {
                if let Some(arg) = arg.downcast_ref() {
                    handler(arg, app);
                } else {
                    debug_assert!(
                        false,
                        "Could not downcast argument for action {name_clone}: {location:?}"
                    );
                    log::error!("Could not downcast argument for action {name_clone}");
                }
            },
        );

        self.global_actions.entry(name).or_default().push(handler);
    }

    pub fn pending_flushes(&self) -> usize {
        self.pending_flushes
    }

    pub fn models_of_type<M: Entity>(&self) -> Vec<ModelHandle<M>> {
        let ref_counts = &self.ref_counts;
        self.models
            .iter()
            .filter(|(_, m)| (*m).as_any().type_id() == TypeId::of::<M>())
            .map(|(model_id, _)| ModelHandle::new(*model_id, ref_counts))
            .collect::<Vec<ModelHandle<M>>>()
    }

    pub fn root_view<T: View>(&self, window_id: WindowId) -> Option<ViewHandle<T>> {
        self.windows
            .get(&window_id)
            .and_then(|window| window.root_view.as_ref())
            .and_then(|root_view| root_view.clone().downcast::<T>())
    }

    /// Records that `parent_view_id` is the parent of `view_id` in `window_id`'s
    /// view hierarchy. Called when a view is created with an explicit parent,
    /// before the first render pass reports the embedding.
    pub fn record_view_parent(
        &mut self,
        window_id: WindowId,
        view_id: EntityId,
        parent_view_id: EntityId,
    ) {
        self.view_parents
            .entry(window_id)
            .or_default()
            .insert(view_id, parent_view_id);
    }

    /// Render-time hook: merges the child-view → parent-view embeddings the
    /// active backend discovered while laying out a frame into the window's
    /// neutral view hierarchy.
    ///
    /// Reported embeddings overwrite previously recorded parentage for the same
    /// child view; entries for dropped or transferred views are removed by the
    /// view lifecycle rather than by this hook. This accumulate-and-remove
    /// semantic matches the GUI presenter's historical layout-time parent map,
    /// so views that are alive but not embedded in the current frame keep their
    /// last known ancestry.
    pub fn report_view_embeddings(
        &mut self,
        window_id: WindowId,
        embeddings: HashMap<EntityId, EntityId>,
    ) {
        self.view_parents
            .entry(window_id)
            .or_default()
            .extend(embeddings);
    }

    /// Returns the ancestor chain of `view_id` in `window_id`, ordered from the
    /// root down to (and including) `view_id` itself, by walking the neutral
    /// view hierarchy.
    pub fn view_ancestors(&self, window_id: WindowId, mut view_id: EntityId) -> Vec<EntityId> {
        let mut chain = vec![view_id];
        if let Some(parents) = self.view_parents.get(&window_id) {
            while let Some(parent_id) = parents.get(&view_id) {
                if chain.contains(parent_id) {
                    log::error!("Cycle detected in the view hierarchy at view {parent_id}");
                    break;
                }
                view_id = *parent_id;
                chain.push(view_id);
            }
        }
        chain.reverse();
        chain
    }

    /// Returns all descendant view IDs of `root_view_id` in `window_id`,
    /// computed by finding all views in the neutral view hierarchy whose
    /// ancestor chain includes the root.
    pub fn view_descendants(&self, window_id: WindowId, root_view_id: EntityId) -> Vec<EntityId> {
        let Some(parents) = self.view_parents.get(&window_id) else {
            return Vec::new();
        };
        parents
            .keys()
            .filter(|&&view_id| {
                let mut current = view_id;
                let mut steps = 0;
                while let Some(&parent_id) = parents.get(&current) {
                    if parent_id == root_view_id {
                        return true;
                    }
                    current = parent_id;
                    // Defend against a (should-be-impossible) cycle.
                    steps += 1;
                    if steps > parents.len() {
                        break;
                    }
                }
                false
            })
            .copied()
            .collect()
    }

    pub fn dispatch_action_for_view(
        &mut self,
        window_id: WindowId,
        view_id: EntityId,
        name: &str,
        arg: &dyn Any,
    ) -> bool {
        if self.is_window_open(window_id) {
            let responder_chain = self.view_ancestors(window_id, view_id);
            self.dispatch_action(window_id, &responder_chain, name, arg, log::Level::Info)
        } else {
            false
        }
    }

    /// Dispatch a typed action using the given view as the base of the responder_chain
    pub fn dispatch_typed_action_for_view(
        &mut self,
        window_id: WindowId,
        view_id: EntityId,
        action: &dyn Action,
    ) {
        if self.is_window_open(window_id) {
            let responder_chain = self.view_ancestors(window_id, view_id);
            self.dispatch_typed_action(window_id, &responder_chain, action, log::Level::Info);
        }
    }

    pub fn dispatch_action(
        &mut self,
        window_id: WindowId,
        responder_chain: &[EntityId],
        name: &str,
        arg: &dyn Any,
        log_level: log::Level,
    ) -> bool {
        log::log!(log_level, "dispatching action for {name:?}");
        self.pending_flushes += 1;

        let mut any_action_handled = false;
        let mut dispatched_actions = HashSet::<String>::new();

        for view_id in responder_chain.iter().rev() {
            if let Some(mut view) = self
                .windows
                .get_mut(&window_id)
                .and_then(|w| w.views.remove(view_id))
            {
                let type_id = view.as_any().type_id();

                if let Some((name, mut handlers)) = self
                    .actions
                    .get_mut(&type_id)
                    .and_then(|h| h.remove_entry(name))
                {
                    // Only dispatch the action if a previous view hasn't already handled it or the
                    // child view determined it should be propagated to the parent view.
                    if !dispatched_actions.contains(name.as_str()) {
                        for handler in handlers.iter_mut().rev() {
                            let handled =
                                handler(view.as_any_mut(), arg, self, window_id, *view_id);
                            any_action_handled |= handled;
                            if handled {
                                dispatched_actions.insert(name.clone());
                            }
                        }
                    } else {
                        log::log!(log_level, "not propagating action {name:?} to parent view");
                    }
                    self.actions
                        .get_mut(&type_id)
                        .unwrap()
                        .insert(name, handlers);
                }

                if let Some(window) = self.windows.get_mut(&window_id) {
                    window.views.insert(*view_id, view);
                }
            }
        }

        if !any_action_handled {
            log::warn!("Action {name:?} was dispatched, but no view handled it");
        }

        self.dispatch_global_action(name, arg);

        self.flush_effects();
        any_action_handled
    }

    /// Dispatch a typed action to a view that was registered with `add_typed_action_view`
    ///
    /// The action will be dispatched to deepest view in the `responder_chain` that registered a
    /// handler for the action's type
    pub fn dispatch_typed_action(
        &mut self,
        window_id: WindowId,
        responder_chain: &[EntityId],
        action: &dyn Action,
        log_level: log::Level,
    ) -> bool {
        log::log!(
            log_level,
            "dispatching typed action: {}::{action:?}",
            action.type_name()
        );

        let action_type: ActionType = action.into();
        // If there are no handlers registered for the given action, then we can return early
        // without needing to look at the responder chain at all, since no views will handle it
        let Some(mut handlers) = self.typed_actions.remove(&action_type) else {
            log::warn!("Dispatched action has no handlers: {:?}", &action);
            return false;
        };

        // Increment pending flushes only after we know there is a handler for the action.
        self.pending_flushes += 1;

        // Traverse the responder chain from the leaf view to see if any views handle the action
        // We stop on the first View that handles the action, we explicitly do not propagate to
        // parent views
        let handled = responder_chain.iter().rev().any(|view_id| {
            let mut view = match self
                .windows
                .get_mut(&window_id)
                .and_then(|w| w.views.remove(view_id))
            {
                Some(view) => view,
                None => return false,
            };

            // Check to see if we have a handler for this action on the view
            let view_type = ViewType(view.as_any().type_id());
            let found = match handlers.get_mut(&view_type) {
                Some(handler) => {
                    handler(
                        view.as_any_mut(),
                        action.as_any(),
                        self,
                        window_id,
                        *view_id,
                    );
                    true
                }
                None => false,
            };

            // Reinsert the view before moving on to the next link in the responder chain
            if let Some(window) = self.windows.get_mut(&window_id) {
                window.views.insert(*view_id, view);
            }

            found
        });

        // Reinsert the action handlers for this action type
        self.typed_actions.insert(action_type, handlers);

        if !handled {
            log::warn!("Action {action:?} was dispatched, but no view handled it");
        }

        self.flush_effects();
        handled
    }

    /// Global actions are being phased out. Prefer dispatching typed actions instead of global actions.
    #[track_caller]
    pub fn dispatch_global_action(&mut self, name: &str, arg: &dyn Any) {
        let location = std::panic::Location::caller();
        self.dispatch_global_action_internal(name, location, arg);
    }

    fn dispatch_global_action_internal(
        &mut self,
        name: &str,
        location: &'static std::panic::Location<'static>,
        arg: &dyn Any,
    ) {
        if let Some((name, mut handlers)) = self.global_actions.remove_entry(name) {
            log::info!("dispatching global action for {}", &name);
            self.pending_flushes += 1;
            for handler in handlers.iter_mut().rev() {
                handler(arg, location, self);
            }
            self.global_actions.insert(name, handlers);
            self.flush_effects();
        }
    }
    /// Registers a validator that validates every binding that matches the given view's default
    /// [`Context`].
    /// After the app is initialized, the provided `binding_validator` function is called for every
    /// binding that matches the View's default context. If the binding is invalid (indicated by
    /// [`IsBindingValid::No`]), the app will panic if `debug_assertions` are enabled.
    #[cfg(debug_assertions)]
    pub fn register_binding_validator<T: View>(
        &mut self,
        binding_validator: impl Fn(BindingLens) -> IsBindingValid + 'static,
    ) {
        let context = T::default_keymap_context();
        self.keystroke_matcher
            .register_binding_validator(context, binding_validator)
    }

    #[cfg(not(debug_assertions))]
    pub fn register_binding_validator<T: View>(
        &mut self,
        binding_validator: impl Fn(BindingLens) -> IsBindingValid + 'static,
    ) {
    }

    /// Sets a default binding validator that runs on _every_ binding that is registered by the
    /// application.
    /// Noops if `debug_assertions` are disabled.
    #[cfg(debug_assertions)]
    pub fn set_default_binding_validator(
        &mut self,
        binding_validator: impl Fn(BindingLens) -> IsBindingValid + 'static,
    ) {
        self.keystroke_matcher
            .set_default_binding_validator(binding_validator)
    }

    /// Sets a default binding validator that runs on _every_ binding that is registered by the
    /// application.
    /// Noops if `debug_assertions` are disabled.
    #[cfg(not(debug_assertions))]
    pub fn set_default_binding_validator(
        &mut self,
        binding_validator: impl Fn(BindingLens) -> IsBindingValid + 'static,
    ) {
    }

    /// Runs through each registered binding validator, asserting that each matching binding is
    /// valid. Noop if debug assertions are not enabled.
    #[cfg(debug_assertions)]
    pub(crate) fn validate_bindings(&mut self) {
        self.keystroke_matcher.validate_bindings();
    }

    /// Runs through each registered binding validator, asserting that each matching binding is
    /// valid. Noop if debug assertions are not enabled.
    #[cfg(not(debug_assertions))]
    pub(crate) fn validate_bindings(&mut self) {}

    /// Add new fixed (immutable) key bindings to the app
    pub fn register_fixed_bindings<T: IntoIterator<Item = FixedBinding>>(&mut self, bindings: T) {
        self.keystroke_matcher.register_fixed_bindings(bindings);
    }

    /// Register new actions with the app
    ///
    /// Editable Bindings have a name identifier which can be used to override their key bindings
    /// via the `set_custom_trigger` method.
    pub fn register_editable_bindings<A: IntoIterator<Item = EditableBinding>>(
        &mut self,
        actions: A,
    ) {
        self.keystroke_matcher.register_editable_bindings(actions);
    }

    /// Set a custom trigger for a given editable binding name
    ///
    /// This will override the default trigger for that action
    pub fn set_custom_trigger(&mut self, name: String, trigger: Trigger) {
        self.keystroke_matcher.set_custom_trigger(name, trigger);
    }

    pub(super) fn disable_key_bindings(&mut self, window_id: WindowId) {
        self.disabled_key_bindings_windows.insert(window_id);
    }

    pub(super) fn enable_key_bindings(&mut self, window_id: WindowId) {
        self.disabled_key_bindings_windows.remove(&window_id);
    }

    pub fn key_bindings_enabled(&self, window_id: WindowId) -> bool {
        !self.disabled_key_bindings_windows.contains(&window_id)
    }

    /// Remove any custom trigger associated with a given action
    ///
    /// This will return the trigger to its default state (if any)
    pub fn remove_custom_trigger<N>(&mut self, name: N)
    where
        N: AsRef<str>,
    {
        self.keystroke_matcher.remove_custom_trigger(name);
    }

    /// Fetch the key bindings that apply to the given window / view
    ///
    /// This will look at key bindings in precedence order (closest to the view) first and only
    /// return the _first_ binding for a given trigger condition. That binding is the one that
    /// would run if the keys were pressed, so it matches what bindings are "available" from that
    /// view.
    pub fn key_bindings_for_view(
        &self,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Vec<BindingLens<'_>> {
        let contexts = self.contexts_for_window_and_view(window_id, view_id);
        let mut triggers = HashSet::with_capacity(contexts.len());
        let mut results = Vec::with_capacity(contexts.len());

        for binding in contexts
            .into_iter()
            // The contexts are ordered top-down (from the root view down to the current view),
            // however the precedence order is bottom-up, so we need to reverse the iteration order
            .rev()
            .flat_map(|c| self.keystroke_matcher.bindings_for_context(c))
        {
            // Include all empty triggers, since they can't "overlap"
            if binding.trigger.is_empty() || triggers.insert(binding.trigger) {
                results.push(binding);
            }
        }

        results
    }

    /// Fetch an iterator of `BindingLens` objects, with the editable key bindings
    /// modified by the custom bindings, where appropriate.
    ///
    /// Editable bindings will be returned first, followed by any fixed bindings in the reverse
    /// order they were added.
    pub fn get_key_bindings(&self) -> impl Iterator<Item = BindingLens<'_>> {
        self.keystroke_matcher.get_bindings()
    }

    /// Returns the first registered binding with the given name, if one exists.
    pub fn get_binding_by_name(&self, name: &str) -> Option<BindingLens<'_>> {
        self.keystroke_matcher.get_binding_by_name(name)
    }

    /// Executes an updater callback against the current binding for
    /// a custom action.  A typical use of an updater is to update menu
    /// state based on the current contextual binding for a given custom action.
    pub fn update_custom_action_binding<F>(&self, custom_tag: CustomTag, mut updater: F)
    where
        F: FnMut(Option<BindingLens<'_>>),
    {
        updater(self.active_binding_for_custom_action(custom_tag));
    }

    /// Returns an optional binding for a custom action in the context of
    /// the currently focused window and view, if there is one.
    fn active_binding_for_custom_action(&self, custom_tag: CustomTag) -> Option<BindingLens<'_>> {
        let window_id = self.windows().active_window()?;
        let view_id = self.focused_view_id(window_id)?;
        let contexts = self.contexts_for_window_and_view(window_id, view_id);
        self.binding_for_custom_action(custom_tag, contexts)
    }

    pub fn default_binding_for_custom_action(
        &self,
        custom_tag: CustomTag,
    ) -> Option<BindingLens<'_>> {
        self.keystroke_matcher
            .default_binding_for_custom_action(custom_tag)
    }

    pub fn description_for_custom_action(
        &self,
        custom_tag: CustomTag,
        description_for: DescriptionContext,
    ) -> Option<String> {
        // Resolve via the dynamic override (if any) so menu-bar callers pick
        // up state-dependent labels automatically. `in_context` would skip
        // the override and return the static fallback.
        self.default_binding_for_custom_action(custom_tag)
            .and_then(|binding| binding.description)
            .map(|s| s.resolve(self, description_for).into_owned())
    }

    /// Returns an optional binding for a custom action in any of the given contexts.
    /// Expects contexts to be ordered from more general (root view) to more specific (leafs)
    /// and uses the first context that matches.
    pub fn binding_for_custom_action(
        &self,
        custom_tag: CustomTag,
        contexts: Vec<Context>,
    ) -> Option<BindingLens<'_>> {
        contexts
            .into_iter()
            // The contexts are ordered top-down (from the root view down to the current view),
            // however the precedence order is bottom-up, so we need to reverse the iteration order
            .rev()
            .find_map(|context| {
                self.keystroke_matcher
                    .binding_for_custom_action_in_context(custom_tag, &context)
            })
    }

    fn contexts_for_window_and_view(&self, window_id: WindowId, view_id: EntityId) -> Vec<Context> {
        let responder_chain = self.view_ancestors(window_id, view_id);
        match self.contexts_from_responder_chain(window_id, &responder_chain) {
            Ok(ctxs) => ctxs,
            Err(error) => {
                log::error!("Unable to fetch Key Bindings for View: {error}");
                Vec::new()
            }
        }
    }

    /// Fetch an iterator of editable bindings
    ///
    /// The triggers for those actions will be overwritten by any custom triggers
    ///
    /// Items will be returned in the reverse order they were registered, the most recently
    /// registered editable binding will have the highest precedence
    pub fn editable_bindings(&self) -> impl Iterator<Item = EditableBindingLens<'_>> {
        self.keystroke_matcher.editable_bindings()
    }

    /// Overrides any registered binding with has a [`Trigger::Custom`] to one that is keystroke
    /// based ([`Trigger::Keystrokes`]) using the provided `custom_to_keystroke` fn.
    pub fn convert_custom_triggers_to_keystroke_triggers(
        &mut self,
        custom_to_keystroke_fn: impl Fn(CustomTag) -> Option<Keystroke> + 'static,
    ) {
        self.keystroke_matcher
            .convert_custom_triggers_to_keystroke_triggers(custom_to_keystroke_fn);
    }

    pub fn register_default_keystroke_triggers_for_custom_actions(
        &mut self,
        custom_to_keystroke_fn: impl Fn(CustomTag) -> Option<Keystroke> + 'static,
    ) {
        self.keystroke_matcher
            .register_default_keystroke_triggers_for_custom_actions(custom_to_keystroke_fn);
    }

    /// Fetch an iterator of editable bindings that apply in a given window / view
    ///
    /// This will return all editable bindings, regardless of whether or not they have
    /// associated key bindings
    pub fn editable_bindings_for_view(
        &self,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Vec<EditableBindingLens<'_>> {
        let responder_chain = self.view_ancestors(window_id, view_id);
        let contexts = match self.contexts_from_responder_chain(window_id, &responder_chain) {
            Ok(ctxs) => ctxs,
            Err(error) => {
                log::error!("Unable to fetch Key Bindings for View: {error}");
                return Vec::new();
            }
        };

        self.keystroke_matcher
            .editable_bindings()
            .filter(|action| contexts.iter().any(|ctx| action.in_context(ctx)))
            .collect()
    }

    /// return a list of contexts corresponding to a responder chain.
    fn contexts_from_responder_chain(
        &self,
        window_id: WindowId,
        responder_chain: &[EntityId],
    ) -> Result<Vec<Context>> {
        let mut context_chain = Vec::new();
        for view_id in responder_chain {
            if let Some(view) = self
                .windows
                .get(&window_id)
                .and_then(|w| w.views.get(view_id))
            {
                let mut context = view.keymap_context(self);
                if self.platform_delegate.is_ime_open() {
                    context.set.insert("IMEOpen");
                }
                context_chain.push(context);
            } else {
                return Err(anyhow!(
                    "View {} in responder chain does not exist",
                    view_id
                ));
            }
        }
        Ok(context_chain)
    }

    fn dispatch_standard_action(
        &mut self,
        action: StandardAction,
        window_id: WindowId,
        responder_chain: &[EntityId],
    ) -> Result<bool> {
        let mut context_chain = self.contexts_from_responder_chain(window_id, responder_chain)?;
        for (i, ctx) in context_chain.iter_mut().enumerate().rev() {
            let handled = match self.keystroke_matcher.match_standard(action, ctx) {
                MatchResult::Action(action) => self.dispatch_typed_action(
                    window_id,
                    &responder_chain[0..=i],
                    action.as_ref(),
                    log::Level::Info,
                ),
                _ => false,
            };
            // In this case, paste is the only valid interaction
            if handled && matches!(action, StandardAction::Paste) {
                self.dispatch_self_or_child_interacted_with(window_id, responder_chain);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Returns the responder chain for a given context and window ID.
    ///
    /// The "responder chain" is the view hierarchy to match against with bindings.
    /// This prefers the focused view and its ancestors; if no view is focused it
    /// dispatches to the root view.
    fn get_responder_chain(&self, window_id: WindowId) -> Vec<EntityId> {
        if let Some(focused) = self.focused_view_id(window_id) {
            self.view_ancestors(window_id, focused)
        } else if let Some(root) = self.root_view_id(window_id) {
            vec![root]
        } else {
            vec![]
        }
    }

    pub fn custom_action_bindings(&self) -> impl Iterator<Item = BindingLens<'_>> {
        self.keystroke_matcher.custom_action_bindings()
    }

    pub fn dispatch_keystroke(
        &mut self,
        window_id: WindowId,
        responder_chain: &[EntityId],
        keystroke: &Keystroke,
        is_composing: bool,
    ) -> Result<bool> {
        let mut context_chain = self.contexts_from_responder_chain(window_id, responder_chain)?;
        let mut pending = false;
        for (i, ctx) in context_chain.iter_mut().enumerate().rev() {
            if is_composing {
                ctx.set.insert("IMEOpen");
            }
            let handled = match self.keystroke_matcher.push_keystroke(
                keystroke.clone(),
                responder_chain[i],
                ctx,
            ) {
                MatchResult::None => false,
                MatchResult::Pending => {
                    pending = true;
                    false
                }
                MatchResult::Action(action) => self.dispatch_typed_action(
                    window_id,
                    &responder_chain[0..=i],
                    action.as_ref(),
                    log::Level::Info,
                ),
            };

            if handled {
                return Ok(true);
            }
        }
        Ok(pending)
    }

    /// Dispatches `self_or_child_interacted_with` on every view in the responder chain,
    /// and should only be called if the event was handled by a view in the chain.
    fn dispatch_self_or_child_interacted_with(
        &mut self,
        window_id: WindowId,
        responder_chain: &[EntityId],
    ) {
        for view_id in responder_chain {
            if let Some(view) = self
                .windows
                .get_mut(&window_id)
                .and_then(|w| w.views.remove(view_id))
            {
                view.self_or_child_interacted_with(self, window_id, *view_id);

                // Reinsert the view before moving on to the next link in the responder chain
                if let Some(window) = self.windows.get_mut(&window_id) {
                    window.views.insert(*view_id, view);
                }
            }
        }
    }

    pub fn default_keystroke_trigger_for_custom_action(
        &self,
        custom_tag: CustomTag,
    ) -> Option<Keystroke> {
        self.keystroke_matcher
            .default_keystroke_trigger_for_custom_action(custom_tag)
    }

    pub fn add_model<T, F>(&mut self, build_model: F) -> ModelHandle<T>
    where
        T: Entity,
        F: FnOnce(&mut ModelContext<T>) -> T,
    {
        self.pending_flushes += 1;
        let model_id = EntityId::new();
        let mut ctx = ModelContext::new(self, model_id);
        let model = build_model(&mut ctx);
        self.models.insert(model_id, Box::new(model));
        self.flush_effects();
        ModelHandle::new(model_id, &self.ref_counts)
    }

    pub fn add_singleton_model<T, F>(&mut self, build_model: F) -> ModelHandle<T>
    where
        T: SingletonEntity,
        F: FnOnce(&mut ModelContext<T>) -> T,
    {
        let model_handle = self.add_model(build_model);
        let prev_value = self
            .singleton_models
            .insert(std::any::TypeId::of::<T>(), model_handle.clone().into());
        // Panic in debug mode if this is the second time a singleton model was
        // registered for type T.
        debug_assert!(
            prev_value.is_none(),
            "add_singleton_model() was called twice for {:?}",
            std::any::type_name::<T>()
        );
        model_handle
    }

    /// Delegates to the OS to request the user attention within the app. For mac this bounces the
    /// icon in the dock.
    pub(super) fn request_user_attention(&self, window_id: WindowId) {
        self.platform_delegate.request_user_attention(window_id);
    }

    /// Delegates to the OS to show the system character palette.
    pub fn open_character_palette(&mut self) {
        self.platform_delegate.open_character_palette();
    }

    /// Delegates to the OS to request permissions for sending desktop notifications.
    ///
    /// ## Platform-Specific
    /// * Linux: Always calls the `on_completion_callback` with a value of [`RequestPermissionsOutcome::Accepted`].
    pub(super) fn request_desktop_notification_permissions<F, T>(
        &mut self,
        view_id: EntityId,
        window_id: WindowId,
        on_completion_callback: F,
    ) where
        F: 'static + Send + Sync + FnOnce(&mut T, RequestPermissionsOutcome, &mut ViewContext<T>),
        T: View,
    {
        self.platform_delegate
            .request_desktop_notification_permissions(Box::new(
                move |request_permissions_outcome, ctx| {
                    if let Some(mut view) = ctx
                        .windows
                        .get_mut(&window_id)
                        .and_then(|w| w.views.remove(&view_id))
                    {
                        let mut view_context = ViewContext::new(ctx, window_id, view_id);
                        on_completion_callback(
                            view.as_mut()
                                .as_any_mut()
                                .downcast_mut()
                                .expect("Should be able to downcast to mutable view."),
                            request_permissions_outcome,
                            &mut view_context,
                        );
                        ctx.windows
                            .get_mut(&window_id)
                            .expect("Should be able to retrieve window.")
                            .views
                            .insert(view_id, view);
                    }
                },
            ))
    }

    pub fn reset_cursor(&mut self) {
        self.cursor_updated_for_view = None;
        self.platform_delegate.set_cursor_shape(Cursor::Arrow)
    }

    pub(crate) fn set_cursor_shape(
        &mut self,
        cursor: Cursor,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        self.cursor_updated_for_view = Some((window_id, view_id));
        self.platform_delegate.set_cursor_shape(cursor)
    }

    #[cfg(test)]
    pub fn get_cursor_shape(&self) -> Cursor {
        self.platform_delegate.get_cursor_shape()
    }

    /// Delegates to the OS to send a desktop notification.
    pub(super) fn send_desktop_notification<F, T>(
        &mut self,
        content: UserNotification,
        view_id: EntityId,
        window_id: WindowId,
        on_error_callback: F,
    ) where
        F: 'static + Send + Sync + FnOnce(&mut T, NotificationSendError, &mut ViewContext<T>),
        T: View,
    {
        self.platform_delegate.send_desktop_notification(
            content,
            window_id,
            Box::new(move |notification_error, ctx| {
                if let Some(mut view) = ctx
                    .windows
                    .get_mut(&window_id)
                    .and_then(|w| w.views.remove(&view_id))
                {
                    let mut view_context = ViewContext::new(ctx, window_id, view_id);
                    on_error_callback(
                        view.as_mut()
                            .as_any_mut()
                            .downcast_mut()
                            .expect("Should be able to downcast to mutable view."),
                        notification_error,
                        &mut view_context,
                    );
                    ctx.windows
                        .get_mut(&window_id)
                        .expect("Should be able to retrieve window.")
                        .views
                        .insert(view_id, view);
                }
            }),
        )
    }

    pub fn clipboard(&mut self) -> &mut dyn Clipboard {
        self.platform_delegate.clipboard()
    }

    pub fn set_last_mouse_move_event(&mut self, window_id: WindowId, event: Event) {
        match event {
            Event::MouseMoved { .. } => {
                let last_mouse_moved_event = self.get_last_mouse_moved_event(window_id);
                *last_mouse_moved_event.borrow_mut() = Some(event);
            }
            _ => {
                panic!("not a mouse move event")
            }
        }
    }

    fn get_last_mouse_moved_event(&mut self, window_id: WindowId) -> Rc<RefCell<Option<Event>>> {
        self.window_last_mouse_moved_event
            .entry(window_id)
            .or_default()
            .clone()
    }

    /// Creates a new window with the view returned by the `build_root_view` function as its root
    /// view.
    pub fn add_window<T, F>(
        &mut self,
        options: AddWindowOptions,
        build_root_view: F,
    ) -> (WindowId, ViewHandle<T>)
    where
        T: View + TypedActionView,
        F: FnOnce(&mut ViewContext<T>) -> T,
    {
        let (window_id, _root_view_id) =
            self.insert_window_internal(None, options, |window_id, ctx| {
                ctx.windows.insert(window_id, Window::default());
                let root_handle = ctx.add_typed_action_view(window_id, build_root_view);
                let root_view_id = root_handle.id();
                ctx.windows
                    .get_mut(&window_id)
                    .expect("this window was just inserted and should still exist")
                    .root_view = Some(root_handle.into());
                root_view_id
            });
        (
            window_id,
            self.root_view(window_id)
                .expect("should have just inserted a window and root view"),
        )
    }

    #[allow(clippy::unwrap_in_result)]
    pub(crate) fn handle_window_closed(&mut self, window_id: WindowId) -> Option<ClosedWindowData> {
        // Defer ALL effect flushes until the window is fully removed from
        // self.windows. Without this guard, the flush that used to happen during
        // build_scene (in the test-only code path in on_window_invalidated) would
        // fire cascading effects (save_app, active_window_changed,
        // ClearMarkedText, etc.) while the window is still registered. Those
        // effects can trigger nested update_view calls on views in the
        // half-dead window, causing "circular view reference" panics.
        self.pending_flushes += 1;

        let view_ids = self
            .windows
            .get(&window_id)
            .map(|window| window.views.keys().copied().collect_vec());

        if let Some(view_ids) = &view_ids {
            for view_id in view_ids {
                // We're using .expect here even though the function returns an Option because we
                // know that the window exists at this point.
                if let Some(mut view) = self
                    .windows
                    .get_mut(&window_id)
                    .expect("Window exists")
                    .views
                    .remove(view_id)
                {
                    view.on_window_closed(self, window_id, *view_id);

                    self.windows
                        .get_mut(&window_id)
                        .expect("Window exists")
                        .views
                        .insert(*view_id, view);
                }
            }
        }

        let fullscreen_state = self
            .windows()
            .platform_window(window_id)
            .map(|window| window.fullscreen_state())
            .unwrap_or_default();

        if let (Some(bounds), Some(NextNewWindowsHasThisWindowsBoundsUponClose::Yes)) = (
            self.window_bounds.get(&window_id),
            self.next_window_bounds_map.get(&window_id),
        ) {
            // Store the bounds of the window that was closed so that the next window
            // we reopen is positioned there.
            self.next_window_bounds = *bounds;
        }
        WindowManager::handle(self).update(self, |windowing_state, ctx| {
            windowing_state.remove_window(window_id, ctx);
        });
        self.drop_window_presentation(window_id);
        self.invalidation_callbacks.remove(&window_id);
        self.window_invalidations.remove(&window_id);
        self.view_parents.remove(&window_id);
        autotracking::close_window(window_id);

        let mut subscriptions = HashMap::new();
        let mut observations = HashMap::new();
        // Back up view_to_window mappings so they can be restored if the window is reopened
        // via reopen_closed_window(). This preserves the view-to-window associations.
        let mut view_to_window_backup = HashMap::new();
        for view_id in view_ids.into_iter().flatten() {
            if let Some(subs) = self.subscriptions.remove(&view_id) {
                subscriptions.insert(view_id, subs);
            }
            if let Some(obs) = self.observations.remove(&view_id) {
                observations.insert(view_id, obs);
            }
            if let Some(view_window_id) = self.view_to_window.remove(&view_id) {
                debug_assert_eq!(
                    view_window_id, window_id,
                    "View {view_id:?} was in window {view_window_id:?} but expected {window_id:?} - was it transferred during close?"
                );
                view_to_window_backup.insert(view_id, view_window_id);
            }
        }

        if !self.window_bounds.contains_key(&window_id) {
            log::info!("missing window bounds!");
        }
        if !self.windows.contains_key(&window_id) {
            log::info!("missing core window data!");
        }
        let (Some(window), Some(bounds)) = (
            self.windows.remove(&window_id),
            self.window_bounds.remove(&window_id),
        ) else {
            log::error!("Closed a window that was missing underlying window data!");
            self.flush_effects();
            return None;
        };

        let result = Some(ClosedWindowData {
            window_id,
            window,
            subscriptions,
            observations,
            view_to_window: view_to_window_backup,
            bounds,
            fullscreen_state,
        });

        self.flush_effects();
        result
    }

    /// Returns the bounds and window style for the next window to create.
    /// This is a function of what window was last closed plus the positions
    /// of all current windows.
    pub fn next_window_bounds_and_style(&self) -> (WindowBounds, WindowStyle) {
        let active_window_id = self.windows().active_window();
        match (
            self.next_window_bounds,
            active_window_id.and_then(|id| self.window_bounds(&id)),
        ) {
            // If the last closed window position exactly overlays any current
            // window, then do a cascade from the active window instead of
            // using the last closed window position
            (Some(last_closed_rect), Some(active_window_rect))
                if self.matches_any_window_bounds(last_closed_rect) =>
            {
                (
                    WindowBounds::ExactPosition(active_window_rect),
                    WindowStyle::Cascade,
                )
            }
            // Otherwise, use the last closed window position and size as the spot
            // for launching the new window.
            (Some(last_closed_rect), _) => (
                WindowBounds::ExactPosition(last_closed_rect),
                WindowStyle::Normal,
            ),
            // If there is no last closed window but there is an active window,
            // cascade from the active window.
            (_, Some(active_window_rect)) => (
                WindowBounds::ExactPosition(active_window_rect),
                WindowStyle::Cascade,
            ),
            // And finally fall back to using the default window position.
            (_, _) => (WindowBounds::Default, WindowStyle::Normal),
        }
    }

    pub fn add_view<T, F>(&mut self, window_id: WindowId, build_view: F) -> ViewHandle<T>
    where
        T: View,
        F: FnOnce(&mut ViewContext<T>) -> T,
    {
        self.add_option_view(window_id, |ctx| Some(build_view(ctx)))
            .unwrap()
    }

    pub fn add_option_view<T, F>(
        &mut self,
        window_id: WindowId,
        build_view: F,
    ) -> Option<ViewHandle<T>>
    where
        T: View,
        F: FnOnce(&mut ViewContext<T>) -> Option<T>,
    {
        let view_id = EntityId::new();
        self.pending_flushes += 1;
        let mut ctx = ViewContext::new(self, window_id, view_id);
        let handle = if let Some(view) = build_view(&mut ctx) {
            if let Some(window) = self.windows.get_mut(&window_id) {
                window.views.insert(view_id, Box::new(view));
            } else {
                panic!("Window does not exist");
            }
            self.view_to_window.insert(view_id, window_id);
            self.window_invalidations
                .entry(window_id)
                .or_default()
                .updated
                .insert(view_id);
            Some(ViewHandle::new(window_id, view_id, &self.ref_counts))
        } else {
            None
        };
        self.flush_effects();
        handle
    }

    /// Add a view that implements the `TypedAction` trait, including the default parent view
    ///
    /// This will create the view as normal as well as register it's `handle_action` method in the
    /// typed_actions hash.
    ///
    /// Note: This is intended to be the replacement for `add_view` with the conversion to typed
    /// actions (and will subsequently be renamed to `add_view` once that is complete)
    pub(crate) fn add_typed_action_view_with_parent<V, F>(
        &mut self,
        window_id: WindowId,
        build_view: F,
        parent_view_id: EntityId,
    ) -> ViewHandle<V>
    where
        V: TypedActionView + View,
        F: FnOnce(&mut ViewContext<V>) -> V,
    {
        self.add_typed_action_view_internal(window_id, build_view, Some(parent_view_id))
    }

    /// Add a view that implements the `TypedAction` trait
    ///
    /// This will create the view as normal as well as register it's `handle_action` method in the
    /// typed_actions hash.
    ///
    /// Note: This is intended to be the replacement for `add_view` with the conversion to typed
    /// actions (and will subsequently be renamed to `add_view` once that is complete)
    pub fn add_typed_action_view<V, F>(
        &mut self,
        window_id: WindowId,
        build_view: F,
    ) -> ViewHandle<V>
    where
        V: TypedActionView + View,
        F: FnOnce(&mut ViewContext<V>) -> V,
    {
        self.add_typed_action_view_internal(window_id, build_view, None)
    }

    fn add_typed_action_view_internal<V, F>(
        &mut self,
        window_id: WindowId,
        build_view: F,
        parent_view_id: Option<EntityId>,
    ) -> ViewHandle<V>
    where
        V: TypedActionView + View,
        F: FnOnce(&mut ViewContext<V>) -> V,
    {
        self.pending_flushes += 1;

        // Build the view and insert it into the window `views` map
        let view_id = EntityId::new();
        let mut ctx = ViewContext::new(self, window_id, view_id);
        let view = build_view(&mut ctx);
        let window = self
            .windows
            .get_mut(&window_id)
            .expect("Window does not exist");
        window.views.insert(view_id, Box::new(view));

        // Register in view_to_window mapping
        self.view_to_window.insert(view_id, window_id);

        // If a parent view ID was provided, add the view as a child of the parent
        if let Some(parent_view_id) = parent_view_id {
            self.record_view_parent(window_id, view_id, parent_view_id);
            self.structural_child_to_parent
                .insert(view_id, parent_view_id);
            self.structural_parent_to_children
                .entry(parent_view_id)
                .or_default()
                .insert(view_id);
        }

        // Register the action handler for this view type (if it hasn't already been added)
        self.add_typed_action::<V>();
        // Mark the view as needing to be drawn
        self.window_invalidations
            .entry(window_id)
            .or_default()
            .updated
            .insert(view_id);

        // Create the handle for managing the view lifetime
        let handle = ViewHandle::new(window_id, view_id, &self.ref_counts);
        self.flush_effects();
        handle
    }

    /// Transfers a single view from one window to another.
    ///
    /// This moves the view to the target window and updates all internal mappings.
    /// The view's subscriptions and observations will continue to work correctly
    /// because they now use dynamic window lookup.
    ///
    /// Returns `true` if the transfer was successful, `false` if the view doesn't exist.
    pub fn transfer_view_to_window(
        &mut self,
        view_id: EntityId,
        source_window_id: WindowId,
        target_window_id: WindowId,
    ) -> bool {
        if source_window_id == target_window_id {
            return true;
        }

        let Some(source_window) = self.windows.get_mut(&source_window_id) else {
            return false;
        };

        let Some(view) = source_window.views.remove(&view_id) else {
            return false;
        };

        // Mark the view as removed from the source window's invalidation set.
        // This tells the renderer to stop tracking this view in the source window.
        self.window_invalidations
            .entry(source_window_id)
            .or_default()
            .removed
            .insert(view_id);

        // The view's parentage in the source window no longer applies; the
        // target window's render pass will report its new embedding.
        if let Some(parents) = self.view_parents.get_mut(&source_window_id) {
            parents.remove(&view_id);
        }

        let Some(target_window) = self.windows.get_mut(&target_window_id) else {
            // Target window doesn't exist - roll back by putting the view back in source window
            if let Some(source_window) = self.windows.get_mut(&source_window_id) {
                source_window.views.insert(view_id, view);
            }
            return false;
        };

        target_window.views.insert(view_id, view);
        self.view_to_window.insert(view_id, target_window_id);

        self.window_invalidations
            .entry(target_window_id)
            .or_default()
            .updated
            .insert(view_id);

        // Remove from autotracking in the old window. The view will be automatically
        // added to autotracking in the new window when it renders, since render_view()
        // tracks dependencies based on the current window_id.
        autotracking::remove_view(source_window_id, view_id);

        if let Some(mut view) = self
            .windows
            .get_mut(&target_window_id)
            .and_then(|w| w.views.remove(&view_id))
        {
            view.on_window_transferred(source_window_id, target_window_id, self, view_id);
            if let Some(window) = self.windows.get_mut(&target_window_id) {
                window.views.insert(view_id, view);
            }
        }

        true
    }

    /// Transfers a view and all its descendant views from one window to another.
    ///
    /// This is useful when transferring a component like a tab that contains
    /// multiple nested views. The view tree is determined by the neutral view
    /// hierarchy's parent-child relationships.
    ///
    /// Returns the list of view IDs that were transferred.
    pub fn transfer_view_tree_to_window(
        &mut self,
        root_view_id: EntityId,
        source_window_id: WindowId,
        target_window_id: WindowId,
    ) -> Vec<EntityId> {
        if source_window_id == target_window_id {
            return vec![root_view_id];
        }

        let descendants = self.view_descendants(source_window_id, root_view_id);

        let mut transferred = Vec::with_capacity(descendants.len() + 1);

        if self.transfer_view_to_window(root_view_id, source_window_id, target_window_id) {
            transferred.push(root_view_id);
        }

        for view_id in descendants {
            if self.transfer_view_to_window(view_id, source_window_id, target_window_id) {
                transferred.push(view_id);
            }
        }

        self.transfer_structural_children(source_window_id, target_window_id, &mut transferred);

        transferred
    }

    /// Transfers structural children of already-transferred views.
    ///
    /// Uses `structural_parent_to_children` to walk from transferred parents
    /// to their children, avoiding iteration over all views in the source window.
    fn transfer_structural_children(
        &mut self,
        source_window_id: WindowId,
        target_window_id: WindowId,
        transferred: &mut Vec<EntityId>,
    ) {
        let mut transferred_set: HashSet<EntityId> = transferred.iter().copied().collect();
        let mut to_process: Vec<EntityId> = transferred.clone();

        while let Some(parent_id) = to_process.pop() {
            let children: Vec<EntityId> = self
                .structural_parent_to_children
                .get(&parent_id)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();

            for child_id in children {
                if transferred_set.contains(&child_id) {
                    continue;
                }
                if self.transfer_view_to_window(child_id, source_window_id, target_window_id) {
                    transferred.push(child_id);
                    transferred_set.insert(child_id);
                    to_process.push(child_id);
                }
            }
        }
    }

    fn remove_dropped_items(&mut self) {
        loop {
            let dropped_items = self.ref_counts.lock().take_dropped();
            if dropped_items.is_empty() {
                break;
            }

            for model_id in dropped_items.models {
                self.models.remove(&model_id);
                self.subscriptions.remove(&model_id);
                self.observations.remove(&model_id);
            }

            for (handle_window_id, view_id) in dropped_items.views {
                // Look up the current window from view_to_window mapping, which may differ
                // from handle_window_id if the view was transferred between windows.
                //
                // The entry may be missing when handle_window_closed() eagerly removes
                // view_to_window entries while the views are still alive in ClosedWindowData.
                // When those views' handles are later dropped, we correctly fall back to
                // handle_window_id.
                let current_window_id = self
                    .view_to_window
                    .remove(&view_id)
                    .unwrap_or(handle_window_id);

                // Focus the root view if the view being removed is focused
                if let Some(focused_view_id) = self.focused_view_id(current_window_id) {
                    if view_id == focused_view_id {
                        if let Some(root_view_id) = self.root_view_id(current_window_id) {
                            self.focus(current_window_id, root_view_id);
                        }
                    }
                }

                self.subscriptions.remove(&view_id);
                self.observations.remove(&view_id);
                if let Some(parent_id) = self.structural_child_to_parent.remove(&view_id) {
                    if let Some(children) = self.structural_parent_to_children.get_mut(&parent_id) {
                        children.remove(&view_id);
                        if children.is_empty() {
                            self.structural_parent_to_children.remove(&parent_id);
                        }
                    }
                }
                self.structural_parent_to_children.remove(&view_id);

                if let Some(window) = self.windows.get_mut(&current_window_id) {
                    self.window_invalidations
                        .entry(current_window_id)
                        .or_default()
                        .removed
                        .insert(view_id);
                    window.views.remove(&view_id);
                }
                if let Some(parents) = self.view_parents.get_mut(&current_window_id) {
                    parents.remove(&view_id);
                }

                autotracking::remove_view(current_window_id, view_id);
            }
        }
    }

    fn flush_effects(&mut self) {
        self.pending_flushes -= 1;

        if !self.flushing_effects && self.pending_flushes == 0 {
            self.flushing_effects = true;

            self.remove_dropped_items();

            while let Some(effect) = self.pending_effects.pop_front() {
                match effect {
                    Effect::Event { entity_id, payload } => self.emit_event(entity_id, payload),
                    Effect::ModelNotification { model_id } => self.notify_model_observers(model_id),
                    Effect::ViewNotification { window_id, view_id } => {
                        self.notify_view_observers(window_id, view_id)
                    }
                    Effect::Focus { window_id, view_id } => {
                        self.focus(window_id, view_id);
                    }
                    Effect::TypedAction {
                        window_id,
                        view_id,
                        action,
                    } => {
                        self.dispatch_typed_action_for_view(window_id, view_id, action.as_ref());
                    }
                    Effect::GlobalAction {
                        name,
                        location,
                        arg,
                    } => {
                        self.dispatch_global_action_internal(name, location, arg.as_ref());
                    }
                }

                self.remove_dropped_items();
            }

            self.flushing_effects = false;
            self.update_windows();
        }
    }

    fn update_windows(&mut self) {
        let invalidated_window_ids = self
            .window_invalidations
            .keys()
            .chain(autotracking::windows_with_invalidations().iter())
            .unique()
            .cloned()
            .collect_vec();
        for window_id in invalidated_window_ids {
            if let Some(mut callback) = self.invalidation_callbacks.remove(&window_id) {
                callback(window_id, self);
                self.invalidation_callbacks.insert(window_id, callback);
            }
        }
    }

    /// Schedules an asynchronous task for each delayed notify. Removes any existing notify jobs that've been cancelled.
    pub fn manage_delayed_repaint_timers(&mut self, window_id: WindowId, repaint_at: Instant) {
        // Avoid creating new timers if a timer with a closer repaint time for
        // the same window already exists.
        if self.repaint_tasks.iter().any(|(_, task)| {
            task.window_id == window_id &&
            matches!(task.repaint_trigger, RepaintTrigger::Timer { instant } if instant <= repaint_at)
        }) {
            return;
        }

        let weak_app = self.weak_self.clone();

        let task_id = TaskId::new();
        let task = self.foreground.spawn(async move {
            Timer::after(repaint_at.saturating_duration_since(Instant::now())).await;
            if let Some(app) = weak_app.upgrade() {
                let mut app = app.borrow_mut();

                // If the timer is no longer in repaint_tasks, it was cancelled.
                if app.repaint_tasks.remove(&task_id).is_some() {
                    app.window_invalidations
                        .entry(window_id)
                        .or_default()
                        .redraw_requested = true;
                    app.update_windows();
                }
            }
        });

        self.repaint_tasks.insert(
            task_id,
            RepaintTask {
                task,
                window_id,
                repaint_trigger: RepaintTrigger::Timer {
                    instant: repaint_at,
                },
            },
        );
    }

    /// Schedules an asynchronous task for each delayed notify. Removes any existing notify jobs that've been cancelled.
    pub fn manage_pending_assets(
        &mut self,
        window_id: WindowId,
        pending_assets: HashSet<AssetHandle>,
    ) {
        pending_assets.into_iter().for_each(|pending_asset| {
            // Avoid creating new repaint tasks if this (window, asset) pairing already has a future.
            if self.repaint_tasks.iter().any(|(_, task)| {
                task.window_id == window_id && matches!(&task.repaint_trigger, RepaintTrigger::AssetLoaded { asset_handle } if asset_handle == &pending_asset)
            }) {
                return;
            }

            let asset_cache = AssetCache::as_ref(self);
            let Some(asset_loaded_future) = pending_asset.when_loaded(asset_cache) else {
                return;
            };
            let weak_app = self
                .weak_self
                .clone();

            let task_id = TaskId::new();

            let task = self.foreground.spawn(async move {
                asset_loaded_future.await;

                let Some(app) = weak_app.upgrade() else {
                    return;
                };
                let mut app = app.borrow_mut();

                // If the timer is no longer in repaint_tasks, it was cancelled.
                if app.repaint_tasks.remove(&task_id).is_some() {
                    app.window_invalidations
                        .entry(window_id)
                        .or_default()
                        .redraw_requested = true;
                    app.update_windows();
                }
            });

            self.repaint_tasks.insert(
                task_id,
                RepaintTask {
                    task,
                    window_id,
                    repaint_trigger: RepaintTrigger::AssetLoaded {
                        asset_handle: pending_asset,
                    },
                },
            );
        })
    }

    fn emit_event(&mut self, entity_id: EntityId, payload: Box<dyn Any>) {
        if let Some(subscriptions) = self.subscriptions.remove(&entity_id) {
            // Start tracking unsubscribes for this entity. Unsubscribes called from inside
            // callbacks are deferred and processed at the end, avoiding O(N²) tombstone scanning.
            //
            // Note: All callbacks that existed when the event started processing will be called.
            // Unsubscribe only prevents re-insertion (i.e., it affects future events, not the
            // current one).
            debug_assert!(
                self.pending_unsubscribes.is_none(),
                "pending_unsubscribes should be None at start of emit_event"
            );
            self.pending_unsubscribes = Some(PendingUnsubscribes {
                entity_id,
                keys: HashSet::new(),
            });

            let mut to_reinsert = Vec::new();

            for mut subscription in subscriptions {
                let alive = match &mut subscription {
                    Subscription::FromModel { model_id, callback } => {
                        if let Some(mut model) = self.models.remove(model_id) {
                            callback(model.as_any_mut(), payload.as_ref(), self, *model_id);
                            self.models.insert(*model_id, model);
                            true
                        } else {
                            false
                        }
                    }
                    Subscription::FromView {
                        window_id: stored_window_id,
                        view_id,
                        callback,
                    } => {
                        let current_window_id = self
                            .view_to_window
                            .get(view_id)
                            .copied()
                            .unwrap_or(*stored_window_id);
                        if let Some(mut view) = self
                            .windows
                            .get_mut(&current_window_id)
                            .and_then(|window| window.views.remove(view_id))
                        {
                            callback(
                                view.as_any_mut(),
                                payload.as_ref(),
                                self,
                                current_window_id,
                                *view_id,
                            );

                            // XXX We need to check whether window is None
                            // once again because callback could
                            // potentially erase the window (i.e. if we
                            // handle the Terminal exit event)
                            if let Some(window) = self.windows.get_mut(&current_window_id) {
                                window.views.insert(*view_id, view);
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                    Subscription::FromApp { callback } => {
                        callback(payload.as_ref(), self, entity_id);
                        true
                    }
                };

                if alive {
                    to_reinsert.push(subscription);
                }
            }

            // Process any pending unsubscribes collected during callback execution.
            let pending = self.pending_unsubscribes.take().unwrap();

            // Remove unsubscribed entries from the subscriptions that existed when this event started.
            if !pending.keys.is_empty() {
                to_reinsert.retain(|sub| {
                    sub.subscription_key()
                        .is_none_or(|key| !pending.keys.contains(&key))
                });
            }

            // Collect any new subscriptions added during callback execution (e.g., a callback
            // that subscribes a new handler to this same entity).
            let mut final_subs = self.subscriptions.remove(&entity_id).unwrap_or_default();
            final_subs.extend(to_reinsert);

            // Re-insert surviving subscriptions.
            if !final_subs.is_empty() {
                self.subscriptions.insert(entity_id, final_subs);
            }
        }
    }

    fn notify_model_observers(&mut self, observed_id: EntityId) {
        // TODO: Apply the same deferred unsubscribe pattern used in `emit_event` to support
        // unobserving from inside an observation callback.
        if let Some(observations) = self.observations.remove(&observed_id) {
            if self.models.contains_key(&observed_id) {
                for mut observation in observations {
                    let alive = match &mut observation {
                        Observation::FromModel { model_id, callback } => {
                            if let Some(mut model) = self.models.remove(model_id) {
                                callback(model.as_any_mut(), observed_id, self, *model_id);
                                self.models.insert(*model_id, model);
                                true
                            } else {
                                false
                            }
                        }
                        Observation::FromView {
                            window_id: stored_window_id,
                            view_id,
                            callback,
                        } => {
                            let current_window_id = self
                                .view_to_window
                                .get(view_id)
                                .copied()
                                .unwrap_or(*stored_window_id);
                            if let Some(mut view) = self
                                .windows
                                .get_mut(&current_window_id)
                                .and_then(|w| w.views.remove(view_id))
                            {
                                callback(
                                    view.as_any_mut(),
                                    observed_id,
                                    self,
                                    current_window_id,
                                    *view_id,
                                );
                                if let Some(window) = self.windows.get_mut(&current_window_id) {
                                    window.views.insert(*view_id, view);
                                }
                                true
                            } else {
                                false
                            }
                        }
                        Observation::FromApp { callback } => {
                            callback(observed_id, self);
                            true
                        }
                    };

                    if alive {
                        self.observations
                            .entry(observed_id)
                            .or_default()
                            .push(observation);
                    }
                }
            }
        }
    }

    fn notify_view_observers(&mut self, window_id: WindowId, view_id: EntityId) {
        self.window_invalidations
            .entry(window_id)
            .or_default()
            .updated
            .insert(view_id);
    }

    fn focus(&mut self, window_id: WindowId, focused_id: EntityId) {
        if self.windows.get(&window_id).and_then(|w| w.focused_view) == Some(focused_id) {
            return;
        }

        if self.suppress_focus_for_window == Some(window_id) {
            return;
        }
        self.pending_flushes += 1;

        if let Some((blurred_id, mut blurred)) = self.windows.get_mut(&window_id).and_then(|w| {
            let blurred_view = w.focused_view;
            w.focused_view = Some(focused_id);
            blurred_view.and_then(|id| w.views.remove(&id).map(|view| (id, view)))
        }) {
            blurred.on_blur(&BlurContext::SelfBlurred, self, window_id, blurred_id);
            self.windows
                .get_mut(&window_id)
                .unwrap()
                .views
                .insert(blurred_id, blurred);

            let blur_ctx = BlurContext::DescendentBlurred(blurred_id);
            // Skip the last entry, it is the blurred view itself.
            for view_id in self
                .view_ancestors(window_id, blurred_id)
                .into_iter()
                .rev()
                .skip(1)
            {
                if let Some(mut view) = self
                    .windows
                    .get_mut(&window_id)
                    .and_then(|w| w.views.remove(&view_id))
                {
                    view.on_blur(&blur_ctx, self, window_id, view_id);
                    self.windows
                        .get_mut(&window_id)
                        .and_then(|w| w.views.insert(view_id, view));
                }
            }
        }

        // Close the IME if it was open since the view that was focused has changed.
        // It's important for us to do this asynchronously: since we are in a method
        // that's called from UI framework code, we don't want to trigger platform
        // side effects that could cause the AppContext to be borrowed in
        // the same callstack.
        self.platform_delegate.close_ime_async(window_id);

        if let Some(mut focused) = self
            .windows
            .get_mut(&window_id)
            .and_then(|w| w.views.remove(&focused_id))
        {
            focused.on_focus(&FocusContext::SelfFocused, self, window_id, focused_id);
            self.windows
                .get_mut(&window_id)
                .unwrap()
                .views
                .insert(focused_id, focused);

            let focus_ctx = FocusContext::DescendentFocused(focused_id);
            // Skip the last entry, it is the focused view itself.
            for view_id in self
                .view_ancestors(window_id, focused_id)
                .into_iter()
                .rev()
                .skip(1)
            {
                if let Some(mut view) = self
                    .windows
                    .get_mut(&window_id)
                    .and_then(|w| w.views.remove(&view_id))
                {
                    view.on_focus(&focus_ctx, self, window_id, view_id);
                    self.windows
                        .get_mut(&window_id)
                        .and_then(|w| w.views.insert(view_id, view));
                }
            }
        }

        self.flush_effects();
    }

    pub(super) fn spawn_local<F>(&mut self, future: F) -> usize
    where
        F: 'static + Future,
    {
        let task_id = post_inc(&mut self.next_task_id);
        let app = self.weak_self.clone();
        self.foreground
            .spawn_boxed(
                async move {
                    let output = future.await;
                    if let Some(app) = app.upgrade() {
                        // Ignore any errors that may occur when relaying task output,
                        // as there's nothing we can do about the entity no longer
                        // existing.
                        let _ = app
                            .borrow_mut()
                            .relay_task_output(task_id, Box::new(output));
                    }
                }
                .boxed_local(),
            )
            .detach();
        task_id
    }

    pub(super) fn spawn_stream_local<F>(
        &mut self,
        stream: F,
        done_tx: futures::channel::oneshot::Sender<()>,
    ) -> usize
    where
        F: 'static + crate::r#async::Stream,
        F::Item: SpawnableOutput,
    {
        // Spawn a background task to poll the stream, and forward items to the
        // main thread (foreground executor).
        let (tx, rx) = async_channel::unbounded();
        self.background
            .spawn(async move {
                let mut stream = pin!(stream);
                while let Some(item) = stream.next().await {
                    // If we fail to send the item, the foreground task has dropped the receiver,
                    // meaning the entity that spawned the stream no longer exists, and we can
                    // stop polling the stream.
                    if tx.send(item).await.is_err() {
                        break;
                    }
                }
            })
            .detach();

        // Spawn a task on the foreground executor to invoke the provided on_item and on_done
        // callbacks on the main thread.
        let task_id = post_inc(&mut self.next_task_id);
        let app = self.weak_self.clone();
        self.foreground
            .spawn(async move {
                let mut stream = pin!(rx);
                loop {
                    match stream.next().await {
                        Some(item) => {
                            if let Some(app) = app.upgrade() {
                                let mut app = app.borrow_mut();

                                // If the entity that spawned the stream no longer exists, terminate
                                // the stream.
                                if app.relay_task_output(task_id, Box::new(item)).is_err() {
                                    app.stream_completed(task_id);
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        None => {
                            if let Some(app) = app.upgrade() {
                                let mut app = app.borrow_mut();
                                app.stream_completed(task_id);
                            }
                            let _ = done_tx.send(());
                            break;
                        }
                    }
                }
            })
            .detach();
        task_id
    }

    /// Opens the file path using the default application configured to handle the given filetype.
    pub fn open_file_path(&mut self, path: &Path) {
        self.platform_delegate.open_file_path(path);
    }

    /// Opens the given file path in an explorer view. On MacOS this will open the file in finder.
    pub fn open_file_path_in_explorer(&mut self, path: &Path) {
        self.platform_delegate.open_file_path_in_explorer(path);
    }

    /// Prompt the user to pick file path(s) in the OS native file picker.
    pub fn open_file_picker(
        &mut self,
        callback: impl FnOnce(Result<Vec<String>, FilePickerError>, &mut AppContext)
            + Send
            + Sync
            + 'static,
        config: FilePickerConfiguration,
    ) {
        self.platform_delegate
            .open_file_picker(Box::new(callback), config)
    }

    /// Prompt the user to save a file with the OS native save file dialog.
    pub fn open_save_file_picker(
        &mut self,
        callback: impl FnOnce(Option<String>, &mut AppContext) + Send + Sync + 'static,
        config: SaveFilePickerConfiguration,
    ) {
        self.platform_delegate
            .open_save_file_picker(Box::new(callback), config)
    }

    pub fn application_bundle_info(
        &self,
        bundle_identifier: &str,
    ) -> Option<ApplicationBundleInfo<'_>> {
        self.platform_delegate
            .application_bundle_info(bundle_identifier)
    }

    /// Triggers termination of the app and optionally takes in a TerminationResult, which will be
    /// printed when the program exits.
    pub fn terminate_app(
        &mut self,
        termination_mode: TerminationMode,
        termination_result: Option<TerminationResult>,
    ) {
        if let Some(termination_result) = termination_result {
            #[cfg(debug_assertions)]
            self.termination_result
                .set(termination_result)
                .expect("Termination result should not have been set already");
            #[cfg(not(debug_assertions))]
            let _ = self.termination_result.set(termination_result);
        }
        self.platform_delegate.terminate_app(termination_mode);
    }

    pub fn check_view_focused(&self, window_id: WindowId, view_id: &EntityId) -> bool {
        let focused_view_id = match self.focused_view_id(window_id) {
            Some(id) => id,
            None => return false,
        };
        focused_view_id == *view_id
    }

    pub fn check_view_or_child_focused(&self, window_id: WindowId, view_id: &EntityId) -> bool {
        let focused_view_id = match self.focused_view_id(window_id) {
            Some(id) => id,
            None => return false,
        };
        self.view_ancestors(window_id, focused_view_id)
            .contains(view_id)
    }

    fn relay_task_output(&mut self, task_id: usize, output: Box<dyn Any>) -> Result<()> {
        self.pending_flushes += 1;
        let Some(task_callback) = self.task_callbacks.remove(&task_id) else {
            return Err(anyhow!("Unable to retrieve task callback."));
        };

        let mut result = Ok(());

        match task_callback {
            TaskCallback::ModelFromFuture { model_id, callback } => {
                if let Some(mut model) = self.models.remove(&model_id) {
                    callback(model.as_any_mut(), output, self, model_id);
                    self.models.insert(model_id, model);
                }
                self.task_done(task_id);
            }
            TaskCallback::ModelFromStream {
                model_id,
                mut on_item,
                on_done,
            } => {
                if let Some(mut model) = self.models.remove(&model_id) {
                    on_item(model.as_any_mut(), output, self, model_id);
                    self.models.insert(model_id, model);
                } else {
                    result = Err(anyhow!(
                        "Unable to retrieve model when relaying task output from stream"
                    ));
                }
                // Streams go through different code paths compared to Futures.
                // Even if the stream halts after this call, we still need to
                // refer to the task callback in stream completed.
                self.task_callbacks.insert(
                    task_id,
                    TaskCallback::ModelFromStream {
                        model_id,
                        on_item,
                        on_done,
                    },
                );
            }
            TaskCallback::ViewFromFuture {
                window_id,
                view_id,
                callback,
            } => {
                if let Some(mut view) = self
                    .windows
                    .get_mut(&window_id)
                    .and_then(|w| w.views.remove(&view_id))
                {
                    callback(view.as_any_mut(), output, self, window_id, view_id);
                    self.windows
                        .get_mut(&window_id)
                        .ok_or_else(|| anyhow!("Unable to retrieve window for view"))?
                        .views
                        .insert(view_id, view);
                }
                self.task_done(task_id);
            }
            TaskCallback::ViewFromStream {
                window_id,
                view_id,
                mut on_item,
                on_done,
            } => {
                if let Some(mut view) = self
                    .windows
                    .get_mut(&window_id)
                    .and_then(|w| w.views.remove(&view_id))
                {
                    on_item(view.as_any_mut(), output, self, window_id, view_id);
                    self.windows
                        .get_mut(&window_id)
                        .ok_or_else(|| anyhow!("Unable to retrieve window for view"))?
                        .views
                        .insert(view_id, view);
                } else {
                    result = Err(anyhow!(
                        "Unable to retrieve view when relaying task output from stream"
                    ));
                }
                // Streams go through different code paths compared to Futures.
                // Even if the stream halts after this call, we still need to
                // refer to the task callback in stream completed.
                self.task_callbacks.insert(
                    task_id,
                    TaskCallback::ViewFromStream {
                        window_id,
                        view_id,
                        on_item,
                        on_done,
                    },
                );
            }
        };
        self.flush_effects();
        result
    }

    fn stream_completed(&mut self, task_id: usize) {
        self.pending_flushes += 1;
        let Some(task_callback) = self.task_callbacks.remove(&task_id) else {
            return;
        };
        match task_callback {
            TaskCallback::ModelFromStream {
                model_id, on_done, ..
            } => {
                if let Some(mut model) = self.models.remove(&model_id) {
                    on_done(model.as_any_mut(), self, model_id);
                    self.models.insert(model_id, model);
                }
            }
            TaskCallback::ViewFromStream {
                window_id,
                view_id,
                on_done: callback,
                ..
            } => {
                if let Some(mut view) = self
                    .windows
                    .get_mut(&window_id)
                    .and_then(|w| w.views.remove(&view_id))
                {
                    callback(view.as_any_mut(), self, window_id, view_id);
                    self.windows
                        .get_mut(&window_id)
                        .expect("Window should exist.")
                        .views
                        .insert(view_id, view);
                }
            }
            _ => {}
        };
        self.flush_effects();
        self.task_done(task_id);
    }

    fn task_done(&self, _task_id: usize) {
        // If the receiver has been dropped, the app is likely terminating,
        // so ignore the error.  Additionally, we only ever consume items
        // from this queue in tests, so if this isn't a test, don't stick
        // things into the channel, otherwise it will grow forever without
        // bound.
        #[cfg(test)]
        let _ = block_on(self.task_done.0.send(_task_id));
    }

    pub fn set_a11y_verbosity(&mut self, verbosity: AccessibilityVerbosity) {
        self.a11y_verbosity = verbosity;
    }

    #[cfg(test)]
    pub fn finish_pending_tasks(&self) -> impl Future<Output = ()> {
        let mut pending_tasks = self.task_callbacks.keys().cloned().collect::<HashSet<_>>();
        let task_done = self.task_done.1.clone();

        async move {
            while !pending_tasks.is_empty() {
                if let Ok(task_id) = task_done.recv().await {
                    pending_tasks.remove(&task_id);
                } else {
                    break;
                }
            }
        }
    }

    pub fn record_app_focus(&mut self, user_id: Option<String>, anonymous_id: String) {
        self.app_focus_info.record_app_focus(user_id, anonymous_id);
    }

    pub fn record_app_blur(&mut self, user_id: Option<String>, anonymous_id: String) {
        self.app_focus_info.record_app_blur(user_id, anonymous_id);
    }

    pub fn try_record_daily_app_focus_duration(
        &mut self,
        user_id: Option<String>,
        anonymous_id: String,
    ) {
        self.app_focus_info
            .try_record_daily_app_focus_duration(user_id, anonymous_id);
    }

    pub fn is_screen_reader_enabled(&self) -> Option<bool> {
        self.platform_delegate.is_screen_reader_enabled()
    }

    /// A way for applications to specify custom fallback fonts.
    ///
    /// Takes a function that maps characters to external font families, each
    /// containing URLs to the font files for that family.
    ///
    /// If a character cannot be rendered using the existing loaded fonts, and
    /// the fallback font function specifies a fallback font family, the font
    /// family will be lazy-loaded and the character will be re-rendered with
    /// the specified font.
    pub fn set_fallback_font_fn(
        &mut self,
        f: impl Fn(char) -> Option<fonts::ExternalFontFamily> + Send + Sync + 'static,
    ) {
        fonts::Cache::handle(self).update(self, |cache, _| {
            cache.set_fallback_font_fn(Box::new(f));
        });
    }
}

impl UpdateModel for AppContext {
    fn update_model<T, F, S>(&mut self, handle: &ModelHandle<T>, update: F) -> S
    where
        T: Entity,
        F: FnOnce(&mut T, &mut ModelContext<T>) -> S,
    {
        if let Some(mut model) = self.models.remove(&handle.id()) {
            self.pending_flushes += 1;
            let mut ctx = ModelContext::new(self, handle.id());
            let result = update(
                model
                    .as_any_mut()
                    .downcast_mut()
                    .expect("Downcast is type safe"),
                &mut ctx,
            );
            self.models.insert(handle.id(), model);
            self.flush_effects();
            result
        } else {
            panic!("Circular model update");
        }
    }
}

impl UpdateView for AppContext {
    fn update_view<T, F, S>(&mut self, handle: &ViewHandle<T>, update: F) -> S
    where
        T: View,
        F: FnOnce(&mut T, &mut ViewContext<T>) -> S,
    {
        self.pending_flushes += 1;
        let window_id = handle.window_id(self);
        let mut view = if let Some(window) = self.windows.get_mut(&window_id) {
            if let Some(view) = window.views.remove(&handle.id()) {
                view
            } else {
                panic!("Circular view update");
            }
        } else {
            panic!("Window does not exist");
        };

        let mut ctx = ViewContext::new(self, window_id, handle.id());
        let result = update(
            view.as_any_mut()
                .downcast_mut()
                .expect("Downcast is type safe"),
            &mut ctx,
        );
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.views.insert(handle.id(), view);
        }
        self.flush_effects();
        result
    }
}

impl AddSingletonModel for AppContext {
    fn add_singleton_model<T, F>(&mut self, build_model: F) -> ModelHandle<T>
    where
        T: SingletonEntity,
        F: FnOnce(&mut super::ModelContext<T>) -> T,
    {
        AppContext::add_singleton_model(self, build_model)
    }
}

pub struct ClosedWindowData {
    pub window_id: WindowId,
    window: Window,
    subscriptions: HashMap<EntityId, Vec<Subscription>>,
    observations: HashMap<EntityId, Vec<Observation>>,
    view_to_window: HashMap<EntityId, WindowId>,
    // TODO(vorporeal): why is AppContext.window_bounds holding an option?
    bounds: Option<RectF>,
    fullscreen_state: FullscreenState,
}

impl AppContext {
    pub fn font_cache(&self) -> &fonts::Cache {
        fonts::Cache::as_ref(self)
    }

    pub fn root_view_id(&self, window_id: WindowId) -> Option<EntityId> {
        self.windows
            .get(&window_id)
            .and_then(|window| window.root_view.as_ref().map(|v| v.id()))
    }

    pub fn focused_view_id(&self, window_id: WindowId) -> Option<EntityId> {
        self.windows
            .get(&window_id)
            .and_then(|window| window.focused_view)
    }

    pub fn view_name(&self, window_id: WindowId, view_id: EntityId) -> Option<&str> {
        self.windows
            .get(&window_id)
            .and_then(|window| window.views.get(&view_id))
            .map(|view| view.ui_name())
    }

    /// Returns all the views of type `T` within `window_id`.
    pub fn views_of_type<T: View>(&self, window_id: WindowId) -> Option<Vec<ViewHandle<T>>> {
        let ref_counts = &self.ref_counts;
        self.windows.get(&window_id).map(|window| {
            window
                .views
                .iter()
                .filter(|(_, v)| (*v).as_any().type_id() == TypeId::of::<T>())
                .map(|(view_id, _)| ViewHandle::new(window_id, *view_id, ref_counts))
                .collect::<Vec<ViewHandle<T>>>()
        })
    }

    /// Returns the view of type `T` within `window_id` with the given `entity_id`.
    pub fn view_with_id<T: View>(
        &self,
        window_id: WindowId,
        entity_id: EntityId,
    ) -> Option<ViewHandle<T>> {
        let ref_counts = &self.ref_counts;
        self.windows.get(&window_id).and_then(|window| {
            window
                .views
                .get(&entity_id)
                .filter(|view| (*view).as_any().type_id() == TypeId::of::<T>())
                .map(|_| ViewHandle::new(window_id, entity_id, ref_counts))
        })
    }

    /// Opens the given URL in the default application configured to handle the URL.
    pub fn open_url(&self, url: &str) {
        let effective_url = (self.before_open_url_callback)(url, self);
        self.platform_delegate.open_url(&effective_url);
    }

    pub fn system_theme(&self) -> SystemTheme {
        self.platform_delegate.system_theme()
    }

    pub fn is_headless(&self) -> bool {
        self.platform_delegate.is_headless()
    }

    pub fn microphone_access_state(&self) -> MicrophoneAccessState {
        self.platform_delegate.microphone_access_state()
    }

    pub fn windows(&self) -> &WindowManager {
        WindowManager::as_ref(self)
    }

    pub fn window_ids(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.windows.keys().cloned()
    }

    pub fn is_wayland(&self) -> bool {
        matches!(
            self.windows().windowing_system(),
            Some(windowing::System::Wayland)
        )
    }

    /// Returns all view IDs registered in the given window.
    pub fn view_ids_for_window(&self, window_id: WindowId) -> Vec<EntityId> {
        self.windows
            .get(&window_id)
            .map(|window| window.views.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Renders the given view to the active backend's [`RenderOutput`], tracking
    /// any `Tracked` reads as rendering dependencies.
    pub fn render_view(&self, window_id: WindowId, view_id: EntityId) -> Result<RenderOutput> {
        // surfacing the error of a missing window earlier
        let window = self
            .windows
            .get(&window_id)
            .ok_or_else(|| anyhow!("window not found"))?;
        window
            .views
            .get(&view_id)
            .map(|view| autotracking::render_view(window_id, view_id, || view.render(self)))
            .ok_or_else(|| anyhow!("view not found"))
    }

    pub fn render_views(&self, window_id: WindowId) -> Result<HashMap<EntityId, RenderOutput>> {
        self.windows
            .get(&window_id)
            .map(|w| {
                w.views
                    .iter()
                    .map(|(id, view)| (*id, view.render(self)))
                    .collect::<HashMap<_, _>>()
            })
            .ok_or_else(|| anyhow!("window not found"))
    }
}

impl AsRef<AppContext> for AppContext {
    fn as_ref(&self) -> &AppContext {
        self
    }
}

impl ModelAsRef for AppContext {
    fn model<T: Entity>(&self, handle: &ModelHandle<T>) -> &T {
        if let Some(model) = self.models.get(&handle.id()) {
            model
                .as_any()
                .downcast_ref()
                .expect("downcast should be type safe")
        } else {
            panic!(
                "circular model reference for model type {}",
                std::any::type_name::<T>()
            );
        }
    }
}

impl ReadModel for AppContext {
    fn read_model<T, F, S>(&self, handle: &ModelHandle<T>, read: F) -> S
    where
        T: Entity,
        F: FnOnce(&T, &AppContext) -> S,
    {
        read(self.model(handle), self)
    }
}

impl ViewAsRef for AppContext {
    fn view<T: View>(&self, handle: &ViewHandle<T>) -> &T {
        let window_id = handle.window_id(self);
        if let Some(window) = self.windows.get(&window_id) {
            if let Some(view) = window.views.get(&handle.id()) {
                view.as_any()
                    .downcast_ref()
                    .expect("downcast should be type safe")
            } else {
                panic!(
                    "circular view reference for view type {}",
                    std::any::type_name::<T>()
                );
            }
        } else {
            panic!("window does not exist");
        }
    }

    /// Returns the backing view, or None if materializing the view would
    /// otherwise produce a circular view reference.
    fn try_view<T: View>(&self, handle: &ViewHandle<T>) -> Option<&T> {
        let window_id = handle.window_id(self);
        self.windows
            .get(&window_id)?
            .views
            .get(&handle.id())?
            .as_any()
            .downcast_ref()
    }
}

impl ReadView for AppContext {
    fn read_view<T, F, S>(&self, handle: &ViewHandle<T>, read: F) -> S
    where
        T: View,
        F: FnOnce(&T, &AppContext) -> S,
    {
        read(self.view(handle), self)
    }
}

impl GetSingletonModelHandle for AppContext {
    fn get_singleton_model_handle<T: SingletonEntity>(&self) -> ModelHandle<T> {
        match self.singleton_models.get(&std::any::TypeId::of::<T>()) {
            Some(model_handle) => model_handle
                .clone()
                .downcast()
                .expect("a registered singleton model should never have a refcount of 0"),
            None => {
                panic!(
                    "Cannot get singleton model of type {:?} that was never registered",
                    std::any::type_name::<T>()
                );
            }
        }
    }
}

impl AppContext {
    pub(super) fn get_singleton_model_as_ref<T: SingletonEntity>(&self) -> &T {
        match self.singleton_models.get(&std::any::TypeId::of::<T>()) {
            Some(model_handle) => model_handle
                .downcast_ref(self)
                .expect("downcast should be type safe"),
            None => {
                panic!(
                    "Cannot get singleton model of type {:?} that was never registered",
                    std::any::type_name::<T>()
                );
            }
        }
    }
}
