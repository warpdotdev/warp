mod action;
mod app;
mod autotracking;
mod entity;
mod model;
mod view;
mod window;

pub use action::*;
pub use app::*;
pub use autotracking::Tracked;
pub use entity::*;
pub use model::*;
pub use view::*;
pub use window::*;

use crate::platform::{self, FullscreenState, WindowBounds, WindowStyle};
use crate::{keymap, Element};
use anyhow::Error;

use crate::rendering::OnGPUDeviceSelected;
use derivative::Derivative;
use futures_util::future::BoxFuture;
use pathfinder_geometry::rect::RectF;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;
use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
    fmt::{self, Debug},
    hash::Hash,
    mem,
    sync::{atomic::AtomicUsize, atomic::Ordering},
};

/// A unique identifier for a display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayId(usize);

impl From<usize> for DisplayId {
    fn from(value: usize) -> Self {
        DisplayId(value)
    }
}

/// Index of a valid display. Note that this only denotes the index of a display
/// in the list of active displays and is not a unique identifier.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema_gen", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema_gen",
    schemars(
        description = "Which display to use when multiple monitors are connected.",
        rename_all = "snake_case"
    )
)]
#[cfg_attr(feature = "settings_value", derive(settings_value::SettingsValue))]
pub enum DisplayIdx {
    /// The primary display of the user.
    #[cfg_attr(
        feature = "schema_gen",
        schemars(description = "The primary (main) display.")
    )]
    Primary,
    /// An external display at a given index.
    #[cfg_attr(
        feature = "schema_gen",
        schemars(description = "An external display, identified by index.")
    )]
    External(usize),
}

impl DisplayIdx {
    // If the current DisplayIdx is still valid given the number of displays user has.
    pub fn is_valid_given_display_count(&self, display_count: usize) -> bool {
        match self {
            DisplayIdx::Primary => display_count >= 1,
            // Assumption here is we will always have one primary display -- any
            // external display count should be on top of it.
            DisplayIdx::External(idx) => display_count > *idx + 1,
        }
    }
}

impl fmt::Display for DisplayIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayIdx::Primary => write!(f, "Main Screen"),
            // The naming convention here is for the first external display External(0),
            // we should name it "Screen 2" and incrementally for the following displays.
            DisplayIdx::External(idx) => write!(f, "Screen {}", idx + 2),
        }
    }
}

/// Information to display the IME editor near the active cursor.
#[derive(Debug)]
pub struct CursorInfo {
    /// Position of the active cursor.
    pub position: RectF,
    /// The font size tells us how far below the active cursor position we place the IME.
    pub font_size: f32,
}

#[derive(Debug)]
pub struct ApplicationBundleInfo<'a> {
    pub path: &'a Path,
    // Executable path could be None if the application does not have an executable.
    pub executable: Option<&'a Path>,
}

// An TimerId is a globally unique id for a timer associated with a callback
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskId(usize);

impl TaskId {
    /// \return the next view ID. Note the first return is 0.
    #[allow(clippy::new_without_default)]
    pub fn new() -> TaskId {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        TaskId(raw)
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

pub type OptionalPlatformWindow = Option<Rc<dyn platform::Window>>;

type ActionCallback =
    dyn FnMut(&mut dyn AnyView, &dyn Any, &mut AppContext, WindowId, EntityId) -> bool;

type TypedActionCallback =
    dyn FnMut(&mut dyn AnyView, &dyn Any, &mut AppContext, WindowId, EntityId);

type GlobalActionCallback =
    dyn FnMut(&dyn Any, &'static std::panic::Location<'static>, &mut AppContext);

type InvalidationCallback = dyn FnMut(WindowId, &mut AppContext);

#[derive(PartialEq, Eq, Hash, Debug)]
struct ViewType(TypeId);

impl ViewType {
    fn of<T: ?Sized + 'static>() -> Self {
        ViewType(TypeId::of::<T>())
    }
}

// Helper struct for defining actions bound to a global shortcut/hotkey.
struct GlobalShortcut {
    action: &'static str,
    args: Box<dyn Any>,
}

#[derive(Default, Derivative)]
#[derivative(Debug)]
pub struct AddWindowOptions {
    pub background_blur_radius_pixels: Option<u8>,
    pub background_blur_texture: bool,
    pub window_style: WindowStyle,
    pub window_bounds: WindowBounds,
    pub title: Option<String>,
    pub fullscreen_state: FullscreenState,

    /// If true, new windows created immediately after this window is closed
    /// will have the same position and size as this window.
    pub anchor_new_windows_from_closed_position: NextNewWindowsHasThisWindowsBoundsUponClose,
    /// The callback to be called when the GPU driver this window will render to is selected.
    #[derivative(Debug = "ignore")]
    pub on_gpu_driver_selected: Option<Box<OnGPUDeviceSelected>>,
    /// This is a name to distinguish different windows among one application. It is a no-op on all
    /// platforms except X11 Linux. See docs on the "WM_CLASS" property:
    /// https://www.x.org/docs/ICCCM/icccm.pdf
    pub window_instance: Option<String>,
}

#[derive(Debug, Default)]
pub enum NextNewWindowsHasThisWindowsBoundsUponClose {
    /// Create the next new window with the position and size of this window if it's been closed.
    #[default]
    Yes,

    /// Ignore the bounds of this window when creating the next new one.
    No,
}

pub(crate) type SpawnedFuture = BoxFuture<'static, ()>;

#[derive(Debug, Default, Clone)]
pub struct WindowInvalidation {
    pub updated: HashSet<EntityId>,
    pub removed: HashSet<EntityId>,
    /// Stores whether an element in the window needs to be repainted. Currently an
    /// invalidation will repaint the entire element tree for that window, so we
    /// only store a boolean. In the future we can extend this to store entity ids
    /// for specific views that need to be redrawn once we have that capability.
    pub redraw_requested: bool,
}

pub enum Effect {
    Event {
        entity_id: EntityId,
        payload: Box<dyn Any>,
    },
    ModelNotification {
        model_id: EntityId,
    },
    ViewNotification {
        window_id: WindowId,
        view_id: EntityId,
    },
    Focus {
        window_id: WindowId,
        view_id: EntityId,
    },
    TypedAction {
        window_id: WindowId,
        view_id: EntityId,
        action: Box<dyn Action>,
    },
    GlobalAction {
        name: &'static str,
        location: &'static std::panic::Location<'static>,
        arg: Box<dyn Any>,
    },
}

pub trait AnyView {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn ui_name(&self) -> &'static str;
    fn render(&self, app: &AppContext) -> Box<dyn Element>;
    fn on_focus(
        &mut self,
        focus_ctx: &FocusContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    );
    fn on_blur(
        &mut self,
        blur_ctx: &BlurContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    );
    fn keymap_context(&self, app: &AppContext) -> keymap::Context;
    fn active_cursor_position(
        &self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Option<CursorInfo>;
    fn on_window_closed(&mut self, app: &mut AppContext, window_id: WindowId, view_id: EntityId);
    fn on_window_transferred(
        &mut self,
        source_window_id: WindowId,
        target_window_id: WindowId,
        app: &mut AppContext,
        view_id: EntityId,
    );
    fn self_or_child_interacted_with(
        &self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    );
    fn accessibility_data(
        &self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Option<AccessibilityData>;
}

impl<T> AnyView for T
where
    T: View,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn ui_name(&self) -> &'static str {
        T::ui_name()
    }

    fn render<'a>(&self, app: &AppContext) -> Box<dyn Element> {
        View::render(self, app)
    }

    fn active_cursor_position(
        &self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Option<CursorInfo> {
        let ctx = ViewContext::new(app, window_id, view_id);
        View::active_cursor_position(self, &ctx)
    }

    fn on_focus(
        &mut self,
        focus_ctx: &FocusContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        let mut ctx = ViewContext::new(app, window_id, view_id);
        View::on_focus(self, focus_ctx, &mut ctx);
        // Send notification to a11y tools that the view gained focus
        if focus_ctx.is_self_focused() {
            if let Some(accessibility_contents) = View::accessibility_contents(self, app) {
                app.platform_delegate.set_accessibility_contents(
                    accessibility_contents.with_verbosity(app.a11y_verbosity),
                );
            }
        }
    }

    fn on_blur(
        &mut self,
        blur_ctx: &BlurContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        let mut ctx = ViewContext::new(app, window_id, view_id);
        View::on_blur(self, blur_ctx, &mut ctx);
    }

    fn on_window_closed(&mut self, app: &mut AppContext, window_id: WindowId, view_id: EntityId) {
        let mut ctx = ViewContext::new(app, window_id, view_id);
        View::on_window_closed(self, &mut ctx);
    }

    fn on_window_transferred(
        &mut self,
        source_window_id: WindowId,
        target_window_id: WindowId,
        app: &mut AppContext,
        view_id: EntityId,
    ) {
        let mut ctx = ViewContext::new(app, target_window_id, view_id);
        View::on_window_transferred(self, source_window_id, target_window_id, &mut ctx);
    }

    fn keymap_context(&self, app: &AppContext) -> keymap::Context {
        View::keymap_context(self, app)
    }

    fn self_or_child_interacted_with(
        &self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        let mut ctx = ViewContext::new(app, window_id, view_id);
        View::self_or_child_interacted_with(self, &mut ctx)
    }

    fn accessibility_data(
        &self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Option<AccessibilityData> {
        let mut ctx = ViewContext::new(app, window_id, view_id);
        View::accessibility_data(self, &mut ctx)
    }
}

pub trait Handle<T> {
    fn id(&self) -> EntityId;
    fn location(&self) -> EntityLocation;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EntityLocation {
    Model(EntityId),
    View(WindowId, EntityId),
}

#[derive(Default)]
struct RefCounts {
    entity_counts: HashMap<EntityId, usize>,
    dropped: DroppedItems,
}

#[derive(Default)]
struct DroppedItems {
    models: HashSet<EntityId>,
    views: HashSet<(WindowId, EntityId)>,
}

impl RefCounts {
    fn inc_entity(&mut self, entity_id: EntityId) {
        *self.entity_counts.entry(entity_id).or_insert(0) += 1;
    }

    fn dec_model(&mut self, model_id: EntityId) {
        if let Some(count) = self.entity_counts.get_mut(&model_id) {
            *count -= 1;
            if *count == 0 {
                self.entity_counts.remove(&model_id);

                self.dropped.models.insert(model_id);
            }
        } else {
            panic!("Expected ref count to be positive")
        }
    }

    fn dec_view(&mut self, window_id: WindowId, view_id: EntityId) {
        if let Some(count) = self.entity_counts.get_mut(&view_id) {
            *count -= 1;
            if *count == 0 {
                self.entity_counts.remove(&view_id);
                self.dropped.views.insert((window_id, view_id));
            }
        } else {
            panic!("Expected ref count to be positive")
        }
    }

    fn take_dropped(&mut self) -> DroppedItems {
        mem::take(&mut self.dropped)
    }
}

impl DroppedItems {
    fn is_empty(&self) -> bool {
        self.models.is_empty() && self.views.is_empty()
    }
}

type SubscriptionFromModelCallback = dyn FnMut(&mut dyn Any, &dyn Any, &mut AppContext, EntityId);
type SubscriptionFromViewCallback =
    dyn FnMut(&mut dyn Any, &dyn Any, &mut AppContext, WindowId, EntityId);
type SubscriptionFromAppCallback = dyn FnMut(&dyn Any, &mut AppContext, EntityId);

/// Key that uniquely identifies a subscription for deferred unsubscribe tracking.
#[derive(Hash, Eq, PartialEq, Clone, Copy)]
pub(super) enum SubscriptionKey {
    Model(EntityId),
    View(WindowId, EntityId),
}

/// Tracks pending unsubscribes during event emission.
/// When `emit_event` is processing callbacks, unsubscribes are deferred to avoid
/// O(N²) tombstone scanning. This struct collects the unsubscribes, which are then
/// processed in a single pass at the end of event emission.
pub(super) struct PendingUnsubscribes {
    /// The entity we're currently emitting events for.
    pub entity_id: EntityId,
    /// Keys of subscriptions that should be removed after all callbacks complete.
    pub keys: HashSet<SubscriptionKey>,
}

/// Sources from where an [`Entity`] (e.g. View or Model) can be subscribed to for events.
#[allow(clippy::enum_variant_names)]
enum Subscription {
    /// The [`Entity`] is subscribed to from a [`Model`].
    FromModel {
        model_id: EntityId,
        callback: Box<SubscriptionFromModelCallback>,
    },
    /// The [`Entity`] is subscribed to from a [`View`].
    FromView {
        window_id: WindowId,
        view_id: EntityId,
        callback: Box<SubscriptionFromViewCallback>,
    },
    /// The [`Entity`] is subscribed to from the [`App`].
    FromApp {
        callback: Box<SubscriptionFromAppCallback>,
    },
}

impl Subscription {
    /// Returns a key that uniquely identifies this subscription for deferred unsubscribe tracking.
    /// Returns `None` for `FromApp` subscriptions since they cannot be unsubscribed.
    fn subscription_key(&self) -> Option<SubscriptionKey> {
        match self {
            Subscription::FromModel { model_id, .. } => Some(SubscriptionKey::Model(*model_id)),
            Subscription::FromView {
                window_id, view_id, ..
            } => Some(SubscriptionKey::View(*window_id, *view_id)),
            Subscription::FromApp { .. } => None,
        }
    }
}

type ObservationFromModelCallback = dyn FnMut(&mut dyn Any, EntityId, &mut AppContext, EntityId);
type ObservationFromViewCallback =
    dyn FnMut(&mut dyn Any, EntityId, &mut AppContext, WindowId, EntityId);
type ObservationFromAppCallback = dyn FnMut(EntityId, &mut AppContext);

/// Sources from where an [`Entity`] can be observed for invalidations.
#[allow(clippy::enum_variant_names)]
enum Observation {
    /// The [`Entity`] is observed from another [`Model`].
    FromModel {
        model_id: EntityId,
        callback: Box<ObservationFromModelCallback>,
    },
    /// The [`Entity`] is observed from a [`View`].
    FromView {
        window_id: WindowId,
        view_id: EntityId,
        callback: Box<ObservationFromViewCallback>,
    },
    /// The [`Entity`] is observed from the [`App`].
    FromApp {
        callback: Box<ObservationFromAppCallback>,
    },
}

type ModelFromFutureCallback = dyn FnOnce(&mut dyn Any, Box<dyn Any>, &mut AppContext, EntityId);

type ModelFromStreamItemCallback = dyn FnMut(&mut dyn Any, Box<dyn Any>, &mut AppContext, EntityId);
type ModelFromStreamDoneCallback = dyn FnOnce(&mut dyn Any, &mut AppContext, EntityId);

type ViewFromFutureCallback =
    dyn FnOnce(&mut dyn AnyView, Box<dyn Any>, &mut AppContext, WindowId, EntityId);

type ViewFromStreamItemCallback =
    dyn FnMut(&mut dyn AnyView, Box<dyn Any>, &mut AppContext, WindowId, EntityId);

type ViewFromStreamDoneCallback = dyn FnOnce(&mut dyn AnyView, &mut AppContext, WindowId, EntityId);

enum TaskCallback {
    ModelFromFuture {
        model_id: EntityId,
        callback: Box<ModelFromFutureCallback>,
    },
    ModelFromStream {
        model_id: EntityId,
        on_item: Box<ModelFromStreamItemCallback>,
        on_done: Box<ModelFromStreamDoneCallback>,
    },
    ViewFromFuture {
        window_id: WindowId,
        view_id: EntityId,
        callback: Box<ViewFromFutureCallback>,
    },
    ViewFromStream {
        window_id: WindowId,
        view_id: EntityId,
        on_item: Box<ViewFromStreamItemCallback>,
        on_done: Box<ViewFromStreamDoneCallback>,
    },
}

/// Given a duration and a max jitter percentage, returns a duration representing the
/// Duration + random value (0, jitter_percentage * Duration)
pub fn duration_with_jitter(duration: Duration, max_jitter_percentage: f32) -> Duration {
    let max_jitter = duration.mul_f32(max_jitter_percentage);
    let jitter = max_jitter.mul_f32(rand::random());
    duration + jitter
}

/// Configurable retrying option for spawn_with_retry_on_error.
#[derive(Clone, Copy, Debug)]
pub struct RetryOption {
    strategy: RetryStrategy,
    /// Interval until the next retry.
    interval: Duration,
    /// The remaining number of retries left.
    remaining_retry_count: usize,
    /// The maximum jitter percentage to be added to the interval. If this is None, there's no jitter.
    max_jitter_percentage: Option<f32>,
}

impl RetryOption {
    pub const fn linear(interval: Duration, remaining_retry_count: usize) -> Self {
        Self {
            strategy: RetryStrategy::LinearBackoff,
            interval,
            remaining_retry_count,
            max_jitter_percentage: None,
        }
    }

    pub const fn exponential(
        interval: Duration,
        factor: f32,
        remaining_retry_count: usize,
    ) -> Self {
        Self {
            strategy: RetryStrategy::ExponentialBackoff(factor),
            interval,
            remaining_retry_count,
            max_jitter_percentage: None,
        }
    }

    pub const fn with_jitter(mut self, max_jitter_percentage: f32) -> Self {
        self.max_jitter_percentage = Some(max_jitter_percentage);
        self
    }

    /// Advance the retry option after receiving one failure callback.
    pub fn advance(&mut self) {
        self.remaining_retry_count = self.remaining_retry_count.saturating_sub(1);

        if let RetryStrategy::ExponentialBackoff(factor) = self.strategy {
            self.interval = self.interval.mul_f32(factor);
        }
    }

    /// The number of remaining retries, not including previous attempts.
    pub fn remaining_retries(&self) -> usize {
        self.remaining_retry_count
    }

    /// Computes the duration until the next retry.
    pub fn duration(&self) -> Duration {
        match self.max_jitter_percentage {
            Some(max_jitter_percentage) => {
                duration_with_jitter(self.interval, max_jitter_percentage)
            }
            None => self.interval,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum RetryStrategy {
    /// Constant interval between each backoff.
    LinearBackoff,
    /// Exponential backoff with the set multiplication factor.
    ExponentialBackoff(f32),
}

/// State of the resolved future in `spawn_with_retry_on_error`.
#[derive(Debug)]
pub enum RequestState<T> {
    /// Request succeeded with return value T.
    RequestSucceeded(T),
    /// Request failed but there are pending retries.
    RequestFailedRetryPending(Error),
    /// Request failed.
    RequestFailed(Error),
}

impl<T> RequestState<T> {
    pub fn has_pending_retries(&self) -> bool {
        matches!(self, RequestState::RequestFailedRetryPending(_))
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
