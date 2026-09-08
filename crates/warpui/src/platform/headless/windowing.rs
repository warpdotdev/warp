use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Result;

use super::event_loop::{AppEvent, EventSender};
use crate::WindowId;
use crate::geometry::rect::RectF;
use crate::geometry::vector::{Vector2F, vec2f};
use crate::platform::{self, WindowOptions};
use crate::windowing::WindowCallbacks;

pub struct WindowManager {
    windows: HashMap<WindowId, Rc<Window>>,
    active_window: RefCell<Option<WindowId>>,
    event_sender: EventSender,
}

impl WindowManager {
    pub(super) fn new(event_sender: EventSender) -> Self {
        Self {
            windows: HashMap::new(),
            active_window: RefCell::new(None),
            event_sender,
        }
    }

    fn set_active_window(&self, window_id: Option<WindowId>) {
        *self.active_window.borrow_mut() = window_id;

        if self
            .event_sender
            .send(AppEvent::ActiveWindowChanged(window_id))
            .is_err()
        {
            log::warn!(
                "Tried to send ActiveWindowChanged event, but event loop is no longer running"
            );
        }
    }
}

impl warpui_core::platform::WindowManager for WindowManager {
    fn open_window(
        &mut self,
        window_id: WindowId,
        window_options: WindowOptions,
        callbacks: WindowCallbacks,
    ) -> Result<()> {
        let window = Rc::new(Window::new(window_options, callbacks));
        self.windows.insert(window_id, window);
        self.set_active_window(Some(window_id));
        Ok(())
    }

    fn platform_window(&self, window_id: WindowId) -> warpui_core::OptionalPlatformWindow {
        self.windows
            .get(&window_id)
            .map(Rc::clone)
            .map(|inner| inner as Rc<dyn crate::platform::Window>)
    }

    fn remove_window(&mut self, window_id: WindowId) {
        self.windows.remove(&window_id);
        if *self.active_window.borrow() == Some(window_id) {
            self.set_active_window(None);
        }
    }

    fn active_window_id(&self) -> Option<WindowId> {
        *self.active_window.borrow()
    }

    fn key_window_is_modal_panel(&self) -> bool {
        false
    }

    fn app_is_active(&self) -> bool {
        true
    }

    fn activate_app(&self, last_active_window: Option<WindowId>) -> Option<WindowId> {
        self.set_active_window(last_active_window);
        last_active_window
    }

    fn show_window_and_focus_app(
        &self,
        window_id: WindowId,
        _behavior: platform::WindowFocusBehavior,
    ) {
        self.set_active_window(Some(window_id));
    }

    fn hide_app(&self) {
        // No-op.
    }

    fn hide_window(&self, window_id: WindowId) {
        // If hiding the active window, clear focus.
        if *self.active_window.borrow() == Some(window_id) {
            self.set_active_window(None);
        }
    }

    fn set_window_bounds(&self, window_id: WindowId, bound: RectF) {
        if let Some(window) = self.windows.get(&window_id) {
            window.set_bounds(bound);
        }
    }

    fn set_all_windows_background_blur_radius(&self, _blur_radius_pixels: u8) {
        // No-op for headless.
    }

    fn set_all_windows_background_blur_texture(&self, _use_blur_texture: bool) {
        // No-op for headless.
    }

    fn set_window_title(&self, _window_id: WindowId, _title: &str) {
        // No-op for headless.
    }

    fn close_window_async(
        &self,
        window_id: WindowId,
        _termination_mode: platform::TerminationMode,
    ) {
        // In headless mode, always force-close the window since there's no confirmation dialog.
        if self
            .event_sender
            .send(AppEvent::CloseWindow(window_id))
            .is_err()
        {
            log::warn!("Tried to send event, but event loop is no longer running");
        }
    }

    fn active_display_bounds(&self) -> RectF {
        // A single default display.
        Default::default()
    }

    fn active_display_id(&self) -> crate::DisplayId {
        crate::DisplayId::from(0)
    }

    fn display_count(&self) -> usize {
        1
    }

    fn bounds_for_display_idx(&self, _idx: crate::DisplayIdx) -> Option<RectF> {
        Default::default()
    }

    fn active_cursor_position_updated(&self) {
        // No-op.
    }

    fn windowing_system(&self) -> Option<crate::windowing::System> {
        None
    }

    fn os_window_manager_name(&self) -> Option<String> {
        None
    }

    fn is_tiling_window_manager(&self) -> bool {
        false
    }
}

pub struct Window {
    callbacks: WindowCallbacks,
    bounds: RefCell<RectF>,
    fullscreen_state: RefCell<platform::FullscreenState>,
}

impl Window {
    fn new(options: WindowOptions, callbacks: WindowCallbacks) -> Self {
        let bounds = match options.bounds {
            platform::WindowBounds::Default => RectF::new(vec2f(0.0, 0.0), vec2f(1024.0, 768.0)),
            platform::WindowBounds::ExactSize(size) => RectF::new(vec2f(0.0, 0.0), size),
            platform::WindowBounds::ExactPosition(rect) => rect,
        };
        Self {
            callbacks,
            bounds: RefCell::new(bounds),
            fullscreen_state: RefCell::new(options.fullscreen_state),
        }
    }

    fn set_bounds(&self, rect: RectF) {
        *self.bounds.borrow_mut() = rect;
    }
}

impl platform::Window for Window {
    fn minimize(&self) {}

    fn toggle_maximized(&self) {}

    fn toggle_fullscreen(&self) {}

    fn fullscreen_state(&self) -> platform::FullscreenState {
        *self.fullscreen_state.borrow()
    }

    fn set_titlebar_height(&self, _height: f64) {}

    fn supports_transparency(&self) -> bool {
        false
    }

    fn graphics_backend(&self) -> platform::GraphicsBackend {
        platform::GraphicsBackend::Empty
    }

    fn supported_backends(&self) -> Vec<platform::GraphicsBackend> {
        vec![]
    }

    fn uses_native_window_decorations(&self) -> bool {
        false
    }

    fn as_ctx(&self) -> &dyn platform::WindowContext {
        self
    }

    fn callbacks(&self) -> &crate::windowing::WindowCallbacks {
        &self.callbacks
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl platform::WindowContext for Window {
    fn size(&self) -> Vector2F {
        self.bounds.borrow().size()
    }

    fn origin(&self) -> Vector2F {
        self.bounds.borrow().origin()
    }

    fn backing_scale_factor(&self) -> f32 {
        1.0
    }

    fn max_texture_dimension_2d(&self) -> Option<u32> {
        Some(2048)
    }

    fn render_scene(&self, _scene: Rc<crate::Scene>) {}

    fn request_redraw(&self) {}

    fn request_frame_capture(
        &self,
        _callback: Box<dyn FnOnce(platform::CapturedFrame) + Send + 'static>,
    ) {
        // no-op for headless
    }
}
