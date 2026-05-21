use pathfinder_geometry::rect::RectF;
use std::{
    collections::VecDeque,
    fmt::{Display, Formatter},
};

use crate::{
    geometry,
    platform::{self, FullscreenState, TerminationMode, WindowFocusBehavior},
    scene::{CornerRadius, Radius},
    windowing, DisplayId, DisplayIdx, Entity, ModelContext, OptionalPlatformWindow,
    SingletonEntity, WindowId,
};

use super::WindowCallbacks;

/// Description of the current stage in the lifecycle of the app.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ApplicationStage {
    #[default]
    /// The application is starting. We move from `Starting` to `Active` when we process the
    /// lifecycle event when we open the first window.
    Starting,
    /// The app is currently the active application.
    Active,
    /// Some other application is currently active.
    Inactive,
    /// The application is in the process of terminating.
    Terminating,
}

impl Display for ApplicationStage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplicationStage::Starting => write!(f, "Starting"),
            ApplicationStage::Active => write!(f, "Active"),
            ApplicationStage::Inactive => write!(f, "Inactive"),
            ApplicationStage::Terminating => write!(f, "Terminating"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct State {
    /// The current stage of the app.
    pub stage: ApplicationStage,
    /// The [`WindowId`] of the currently active (frontmost) window. If Warp goes out of focus,
    /// this will go back to None.
    pub active_window: Option<WindowId>,
    /// A stack of [`WindowId`]s which had been active before.
    active_window_stack: VecDeque<WindowId>,
    /// Whether the active window is fullscreen.
    pub is_active_window_fullscreen: Option<bool>,
}

/// Struct that enumerates windowing-related state within the application.
pub struct WindowManager {
    state: State,
    platform: Box<dyn platform::WindowManager>,
}

impl WindowManager {
    /// Constructs a new [`State`] in an inactive state.
    /// NOTE the `State` begins with no windows and an inactive state--it is updated by platform
    /// code when the application first becomes active.
    pub(crate) fn new(window_manager: Box<dyn platform::WindowManager>) -> Self {
        Self {
            state: Default::default(),
            platform: window_manager,
        }
    }

    #[cfg(any(test, feature = "test-util", feature = "integration_tests"))]
    pub fn overwrite_for_test(&mut self, stage: ApplicationStage, active_window: Option<WindowId>) {
        self.state.stage = stage;
        self.state.active_window = active_window;
        self.state.active_window_stack = active_window.into_iter().collect();
        self.state.is_active_window_fullscreen = Some(false);
    }

    pub fn platform_window(&self, window_id: WindowId) -> OptionalPlatformWindow {
        self.platform.platform_window(window_id)
    }

    pub fn app_is_active(&self) -> bool {
        self.platform.app_is_active()
    }

    pub fn hide_window(&self, window_id: WindowId) {
        self.platform.hide_window(window_id)
    }

    pub fn set_window_bounds(&self, window_id: WindowId, bound: RectF) {
        self.platform.set_window_bounds(window_id, bound)
    }

    /// Sets the per-window opacity, where `1.0` is fully opaque and `0.0` is fully
    /// transparent. Cheap alternative to `hide_window` for cases where the window
    /// only needs to disappear visually (e.g. tab drag preview) without changing
    /// focus, key state, or z-order.
    pub fn set_window_alpha(&self, window_id: WindowId, alpha: f32) {
        self.platform.set_window_alpha(window_id, alpha)
    }

    pub fn cancel_synthetic_drag(&self, window_id: WindowId) {
        self.platform.cancel_synthetic_drag(window_id)
    }

    #[cfg(target_os = "macos")]
    pub fn show_window_and_focus_app_without_ordering_front(&self, window_id: WindowId) {
        self.platform
            .show_window_and_focus_app(window_id, WindowFocusBehavior::RetainZIndex)
    }

    pub fn show_window_and_focus_app(&self, window_id: WindowId) {
        self.platform
            .show_window_and_focus_app(window_id, WindowFocusBehavior::default())
    }

    pub fn activate_app(&self) -> Option<WindowId> {
        self.platform.activate_app(self.frontmost_window_id())
    }

    pub fn hide_app(&self) {
        self.platform.hide_app()
    }

    pub fn set_all_windows_background_blur_radius(&self, blur_radius_pixels: u8) {
        self.platform
            .set_all_windows_background_blur_radius(blur_radius_pixels)
    }

    pub fn set_all_windows_background_blur_texture(&self, use_blur_texture: bool) {
        self.platform
            .set_all_windows_background_blur_texture(use_blur_texture)
    }

    pub fn set_window_title(&self, window_id: WindowId, title: &str) {
        self.platform.set_window_title(window_id, title)
    }

    pub fn key_window_is_modal_panel(&self) -> bool {
        self.platform.key_window_is_modal_panel()
    }

    pub fn close_window(&self, window_id: WindowId, termination_mode: TerminationMode) {
        self.platform
            .close_window_async(window_id, termination_mode)
    }

    pub fn active_cursor_position_updated(&self) {
        self.platform.active_cursor_position_updated();
    }

    pub fn active_window(&self) -> Option<WindowId> {
        self.platform.active_window_id()
    }

    // Get the rect of the current active screen. We need the bound instead of just
    // the size of the screen because Mac has a global coordination system containing
    // all user's screens. So the active screen may not have a origin of (0, 0).
    pub fn active_display_bounds(&self) -> geometry::rect::RectF {
        self.platform.active_display_bounds()
    }

    pub fn active_display_id(&self) -> DisplayId {
        self.platform.active_display_id()
    }

    pub fn bounds_for_display_idx(&self, idx: DisplayIdx) -> Option<RectF> {
        self.platform.bounds_for_display_idx(idx)
    }

    pub fn display_count(&self) -> usize {
        self.platform.display_count()
    }

    pub fn is_tiling_window_manager(&self) -> bool {
        self.platform.is_tiling_window_manager()
    }

    pub fn os_window_manager_name(&self) -> Option<String> {
        self.platform.os_window_manager_name()
    }

    pub fn did_window_change_focus(window_id: WindowId, current: &State, previous: &State) -> bool {
        let current_window_is_active = current.active_window == Some(window_id);
        let previous_window_was_active = previous.active_window == Some(window_id);
        current_window_is_active != previous_window_was_active
    }

    /// The window itself usually has rounded corners, except when running in a tiling window
    /// manager or when on Windows. We don't need to specify a custom window corner radius on
    /// Windows because we use OS APIs to round the corners of the window.
    pub fn window_corner_radius(&self) -> CornerRadius {
        let radius = if self.is_tiling_window_manager() || cfg!(windows) {
            0.
        } else {
            8.
        };
        CornerRadius::with_all(Radius::Pixels(radius))
    }

    pub(crate) fn open_window(
        &mut self,
        window_id: WindowId,
        window_options: platform::WindowOptions,
        callbacks: WindowCallbacks,
    ) -> anyhow::Result<()> {
        self.platform
            .open_window(window_id, window_options, callbacks)
    }

    pub(crate) fn remove_window(&mut self, window_id: WindowId, ctx: &mut ModelContext<Self>) {
        self.update(
            |state| {
                state.active_window_stack.retain(|id| *id != window_id);
            },
            ctx,
        );
        self.platform.remove_window(window_id)
    }

    pub(crate) fn close_window_async(
        &self,
        window_id: WindowId,
        termination_mode: TerminationMode,
    ) {
        self.platform
            .close_window_async(window_id, termination_mode)
    }

    /// Sets the current active window to `window_id`.
    pub(crate) fn set_active_window(
        &mut self,
        window_id: Option<WindowId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update(
            |state| {
                state.active_window = window_id;
                if let Some(window_id) = window_id {
                    state.active_window_stack.retain(|id| *id != window_id);
                    state.active_window_stack.push_back(window_id);
                    state.stage = ApplicationStage::Active;
                } else {
                    state.stage = ApplicationStage::Inactive;
                }
            },
            ctx,
        );
    }

    pub fn toggle_fullscreen(&mut self, window_id: WindowId, ctx: &mut ModelContext<Self>) {
        if let Some(window) = self.platform_window(window_id) {
            window.toggle_fullscreen();
        }
        if self.state.active_window == Some(window_id) {
            self.update_is_active_window_fullscreen(ctx);
        }
    }

    /// Updates saved state corresponding to whether the active window is fullscreen.
    pub(crate) fn update_is_active_window_fullscreen(&mut self, ctx: &mut ModelContext<Self>) {
        let is_active_window_fullscreen = self.state.active_window.map(|id| {
            self.platform_window(id)
                .map(|window| window.fullscreen_state() == FullscreenState::Fullscreen)
                .unwrap_or(false)
        });

        self.update(
            |state| state.is_active_window_fullscreen = is_active_window_fullscreen,
            ctx,
        );
    }

    /// Sets the current stage of the application to `stage`.
    pub(crate) fn set_stage(&mut self, stage: ApplicationStage, ctx: &mut ModelContext<Self>) {
        self.update(|state| state.stage = stage, ctx);
    }

    /// Returns the current [`ApplicationStage`] of the application.
    pub fn stage(&self) -> ApplicationStage {
        self.state.stage
    }

    pub fn frontmost_window_id(&self) -> Option<WindowId> {
        self.state.active_window_stack.back().cloned()
    }

    /// Returns boolean representing whether the active window is fullscreen. Returns `false` if
    /// no window is currently active.
    pub fn is_active_window_fullscreen(&self) -> bool {
        self.state.is_active_window_fullscreen.unwrap_or(false)
    }

    pub fn ordered_window_ids(&self) -> Vec<WindowId> {
        self.platform.ordered_window_ids()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn windowing_system(&self) -> Option<windowing::System> {
        self.platform.windowing_system()
    }

    /// Helper function used to ensure that updates to [`State`] end up triggering the proper event
    /// updates.
    fn update(&mut self, update_fn: impl FnOnce(&mut State), ctx: &mut ModelContext<Self>) {
        let previous = self.state.clone();
        update_fn(&mut self.state);
        ctx.emit(StateEvent::ValueChanged {
            previous,
            current: self.state.clone(),
        });
        ctx.notify();
    }
}

/// The set of events that are emitted by the [`crate::windowing::State`] model.
pub enum StateEvent {
    /// The state changed from `previous` to `current`.
    ValueChanged { current: State, previous: State },
}

impl Entity for WindowManager {
    type Event = StateEvent;
}

/// Mark [`State`] as global application state.
impl SingletonEntity for WindowManager {}
