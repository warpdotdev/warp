//! Window management for the TUI backend.
//!
//! Initially copied from the `headless` backend. The `eventloop` child agent
//! extends [`Window`] so that:
//!
//! * it holds the [`AppEvent`] sender and `request_redraw()` sends
//!   [`AppEvent::Redraw`], and
//! * `size()` returns the live terminal size in cells (so the scene is laid out
//!   at one cell == one "pixel").

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;

use anyhow::Result;

use super::event_loop::AppEvent;
use crate::geometry::rect::RectF;
use crate::geometry::vector::{vec2f, Vector2F};
use crate::platform::{self, WindowOptions};
use crate::windowing::WindowCallbacks;
use crate::WindowId;

pub(super) struct WindowManager {
    windows: HashMap<WindowId, Rc<Window>>,
    active_window: RefCell<Option<WindowId>>,
    event_sender: mpsc::Sender<AppEvent>,
}

impl WindowManager {
    pub(super) fn new(event_sender: mpsc::Sender<AppEvent>) -> Self {
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
        let window = Rc::new(Window::new(
            window_options,
            callbacks,
            self.event_sender.clone(),
        ));
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
        if *self.active_window.borrow() == Some(window_id) {
            self.set_active_window(None);
        }
    }

    fn set_window_bounds(&self, _window_id: WindowId, _bound: RectF) {
        // The terminal dictates the window size; app-requested bounds are ignored.
    }

    fn set_all_windows_background_blur_radius(&self, _blur_radius_pixels: u8) {
        // No-op for the TUI backend.
    }

    fn set_all_windows_background_blur_texture(&self, _use_blur_texture: bool) {
        // No-op for the TUI backend.
    }

    fn set_window_title(&self, _window_id: WindowId, _title: &str) {
        // No-op for the TUI backend.
    }

    fn close_window_async(
        &self,
        window_id: WindowId,
        _termination_mode: platform::TerminationMode,
    ) {
        if self
            .event_sender
            .send(AppEvent::CloseWindow(window_id))
            .is_err()
        {
            log::warn!("Tried to send event, but event loop is no longer running");
        }
    }

    fn active_display_bounds(&self) -> RectF {
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

pub(super) struct Window {
    callbacks: WindowCallbacks,
    fullscreen_state: RefCell<platform::FullscreenState>,
    /// Used by [`request_redraw`](Window::request_redraw) to enqueue a repaint.
    event_sender: mpsc::Sender<AppEvent>,
    /// The current terminal size in `(cols, rows)`. One cell maps to one WarpUI
    /// "pixel", so this is also the window's logical size.
    cell_size: Cell<(u16, u16)>,
    /// Whether an [`AppEvent::Redraw`] is already queued, used to coalesce
    /// redundant redraw requests into a single frame.
    redraw_queued: Cell<bool>,
}

impl Window {
    fn new(
        options: WindowOptions,
        callbacks: WindowCallbacks,
        event_sender: mpsc::Sender<AppEvent>,
    ) -> Self {
        // The terminal owns the window size; seed it from the live terminal.
        let cell_size = crossterm::terminal::size().unwrap_or((80, 24));
        Self {
            callbacks,
            fullscreen_state: RefCell::new(options.fullscreen_state),
            event_sender,
            cell_size: Cell::new(cell_size),
            redraw_queued: Cell::new(false),
        }
    }

    /// Updates the stored terminal size after a resize event.
    pub(super) fn set_cell_size(&self, size: (u16, u16)) {
        self.cell_size.set(size);
    }

    /// Returns the current terminal size in `(cols, rows)`.
    pub(super) fn cell_size(&self) -> (u16, u16) {
        self.cell_size.get()
    }

    /// Clears the coalescing flag so the next call to
    /// [`request_redraw`](Window::request_redraw) enqueues a fresh frame.
    /// Called by the event loop as it begins a redraw.
    pub(super) fn clear_redraw_queued(&self) {
        self.redraw_queued.set(false);
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
        let (cols, rows) = self.cell_size.get();
        vec2f(cols as f32, rows as f32)
    }

    fn origin(&self) -> Vector2F {
        vec2f(0.0, 0.0)
    }

    fn backing_scale_factor(&self) -> f32 {
        1.0
    }

    fn max_texture_dimension_2d(&self) -> Option<u32> {
        Some(2048)
    }

    fn render_scene(&self, _scene: Rc<crate::Scene>) {}

    fn request_redraw(&self) {
        // Coalesce: if a redraw is already queued, don't enqueue another.
        if self.redraw_queued.replace(true) {
            return;
        }
        if self.event_sender.send(AppEvent::Redraw).is_err() {
            self.redraw_queued.set(false);
            log::warn!("Tried to request a redraw, but event loop is no longer running");
        }
    }

    fn request_frame_capture(
        &self,
        _callback: Box<dyn FnOnce(platform::CapturedFrame) + Send + 'static>,
    ) {
        // No-op for the TUI backend.
    }
}
