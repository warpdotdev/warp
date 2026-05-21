pub mod state;

mod system;

use std::rc::Rc;

use pathfinder_geometry::rect::RectF;
pub use state::{State, StateEvent, WindowManager};
pub use system::{CreateWindowingSystemError, System};

use crate::{
    actions::StandardAction, platform::WindowContext, AppContext, AppContextRefMut, CursorInfo,
    Event, Scene,
};

/// Result of dispatching an event through the UI framework.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventDispatchResult {
    /// Whether the event was handled by the UI framework.
    pub handled: bool,
    /// Whether the soft keyboard should be shown (mobile WASM only).
    pub soft_keyboard_requested: bool,
}

pub(crate) type EventCallback = Box<dyn Fn(Event, &mut AppContext) -> EventDispatchResult>;
pub(crate) type ResizeCallback = Box<dyn Fn(&dyn WindowContext, &mut AppContext)>;
pub(crate) type StandardActionCallback = Box<dyn Fn(StandardAction, &mut AppContext)>;
pub(crate) type ActiveCursorPositionCallback = Box<dyn Fn(&mut AppContext) -> Option<CursorInfo>>;
pub(crate) type MoveCallback = Box<dyn Fn(RectF, &mut AppContext)>;
pub(crate) type FrameCallback = Box<dyn Fn(&mut AppContext)>;
pub(crate) type BuildSceneCallback = Box<dyn Fn(&dyn WindowContext, &mut AppContext) -> Rc<Scene>>;

/// A collection of callbacks that are used to update UI framework state in
/// response to platform-level events.
pub struct WindowCallbacks {
    /// Dispatches a [`StandardAction`].
    pub standard_action_callback: StandardActionCallback,
    /// Dispatches an [`Event`].
    pub event_callback: EventCallback,
    /// Notifies the UI framework that the window was resized.
    pub resize_callback: ResizeCallback,
    /// Requests that the UI framework construct a scene to render.
    pub build_scene_callback: BuildSceneCallback,
    /// Notifies the UI framework that a frame was rendered.
    pub frame_callback: FrameCallback,
    /// Notifies the UI framework that a frame failed to draw.
    pub draw_frame_error_callback: FrameCallback,
    /// Notifies the UI framework that the window moved.
    pub move_callback: MoveCallback,
    /// Returns the current location of the text editing cursor, if an
    /// editable text field is focused.
    pub active_cursor_position_callback: ActiveCursorPositionCallback,
}

/// A helper structure to simplify and standardize the act of making calls from
/// platform code into platform-independent UI framework code.
pub struct WindowCallbackDispatcher<'a> {
    callbacks: &'a WindowCallbacks,
    ctx: AppContextRefMut<'a>,
}

impl<'a> WindowCallbackDispatcher<'a> {
    pub(crate) fn new(callbacks: &'a WindowCallbacks, ctx: AppContextRefMut<'a>) -> Self {
        Self { callbacks, ctx }
    }

    pub fn dispatch_event(&mut self, event: Event) -> EventDispatchResult {
        (self.callbacks.event_callback)(event, &mut self.ctx)
    }

    pub fn window_resized(&mut self, window: &dyn WindowContext) {
        (self.callbacks.resize_callback)(window, &mut self.ctx)
    }

    pub fn frame_drawn(&mut self) {
        (self.callbacks.frame_callback)(&mut self.ctx)
    }

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn frame_failed_to_draw(&mut self) {
        (self.callbacks.draw_frame_error_callback)(&mut self.ctx)
    }

    pub fn get_active_cursor_position(&mut self) -> Option<CursorInfo> {
        (self.callbacks.active_cursor_position_callback)(&mut self.ctx)
    }

    pub fn window_moved(&mut self, bounds: RectF) {
        (self.callbacks.move_callback)(bounds, &mut self.ctx)
    }

    pub fn build_scene(&mut self, window: &dyn WindowContext) -> Rc<Scene> {
        (self.callbacks.build_scene_callback)(window, &mut self.ctx)
    }
}

// Functions in WindowCallbackDispatcher that relate to application menus.
//
// This is marked as `allow(dead_code)` on Linux and wasm, as they do not
// support application menus, so these never get called.
// TODO(CORE-2691): implement native Windows OS app menus
#[cfg_attr(
    any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows",
        target_family = "wasm"
    ),
    allow(dead_code)
)]
impl WindowCallbackDispatcher<'_> {
    pub fn dispatch_standard_action(&mut self, action: StandardAction) {
        (self.callbacks.standard_action_callback)(action, &mut self.ctx)
    }
}
