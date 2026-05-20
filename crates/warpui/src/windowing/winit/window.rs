#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod x11;

#[cfg(windows)]
mod windows_wm;

use std::collections::HashMap;
use std::sync::Arc;
#[cfg(windows)]
use std::sync::LazyLock;
use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::Rc,
};

use anyhow::{Context as _, Result};
use itertools::Itertools;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use wgpu::rwh::HasDisplayHandle;
use wgpu::{AdapterInfo, CompositeAlphaMode};
use winit::dpi::PhysicalPosition;
use winit::error::ExternalError;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy, OwnedDisplayHandle};
#[cfg(not(target_family = "wasm"))]
use winit::monitor::MonitorHandle;
#[cfg(windows)]
use winit::platform::windows::{BackdropType, WindowExtWindows};
use winit::window::{CursorIcon, ResizeDirection, UserAttentionType, WindowLevel};
use winit::{
    dpi::{LogicalPosition, LogicalSize, PhysicalSize, Position, Size},
    window::Fullscreen,
};

#[cfg(not(target_family = "wasm"))]
use crate::platform::WindowBounds;
use crate::platform::{
    self, Cursor, FullscreenState, GraphicsBackend, TerminationMode, WindowFocusBehavior,
    WindowOptions, WindowStyle,
};
use crate::rendering::{
    wgpu::{
        adapter_has_rendering_offset_bug, from_wgpu_backend, renderer, to_wgpu_backend, Renderer,
        Resources,
    },
    GPUPowerPreference, GlyphConfig, OnGPUDeviceSelected,
};
use crate::windowing::WindowCallbacks;
use crate::{fonts, geometry, Scene};
use crate::{DisplayId, DisplayIdx, OptionalPlatformWindow, WindowId};

use super::app::CustomEvent;

#[cfg(windows)]
use super::windows::{get_system_caption_button_bounds, set_window_attribute, WindowAttributeErr};
#[cfg(windows)]
use windows::Win32::Graphics::Dwm;

/// The inner margin from the edges of the window within which the mouse can drag to resize the
/// window. Note that this value is a logical size, not a physical size. It can be converted to a
/// physical size by multiplying by the scale factor.
const DRAG_RESIZE_MARGIN: f32 = 4.0;

/// This must match the ID in the embedded resource file in `app\build.rs`
#[cfg(windows)]
const IDI_ICON: u16 = 0x101;

cfg_if::cfg_if! {
    if #[cfg(any(test, feature = "integration_tests"))] {
        /// The window cannot be resized smaller than this.
        /// TODO(CORE-1891) Instead of being hard-coded, this should be configurable by the user via
        /// [`crate::platform::WindowOptions`].
        #[cfg_attr(target_family = "wasm", allow(dead_code))]
        pub(in crate::windowing::winit) const MIN_WINDOW_SIZE: LogicalSize<f64> =
            LogicalSize::new(124., 34.);
    } else {
        #[cfg_attr(target_family = "wasm", allow(dead_code))]
        pub(in crate::windowing::winit) const MIN_WINDOW_SIZE: LogicalSize<f64> =
            LogicalSize::new(480., 192.);
    }
}

lazy_static! {
    static ref DEFAULT_WINDOW_SIZE: Vector2F = Vector2F::new(1280., 800.);
}

pub(crate) struct WindowManager {
    windows: HashMap<WindowId, Rc<Window>>,
    event_loop_proxy: EventLoopProxy<CustomEvent>,
    window_ordering: Mutex<WindowOrderingState>,
    /// We assume this won't change throughout the life of the Warp process.
    os_window_manager_name: OnceCell<Option<String>>,
    /// This is a client for talking to the Xorg server directly instead of through winit.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    x11_manager: Option<x11::X11Manager>,
    display_handle: OwnedDisplayHandle,
}

/// Manually-tracked window z-ordering. Winit has no native z-order API, so
/// `WindowManager` keeps this list in sync from its create / focus / hide /
/// remove callbacks and exposes it via `ordered_window_ids()`.
#[derive(Default)]
struct WindowOrderingState {
    front_to_back_window_ids: Vec<WindowId>,
    window_styles: HashMap<WindowId, WindowStyle>,
}

impl WindowOrderingState {
    fn note_window_created(&mut self, window_id: WindowId, style: WindowStyle) {
        self.window_styles.insert(window_id, style);
        if style != WindowStyle::NotStealFocus {
            self.move_to_front(window_id);
        }
    }

    fn move_to_front(&mut self, window_id: WindowId) {
        self.front_to_back_window_ids.retain(|id| *id != window_id);
        self.front_to_back_window_ids.insert(0, window_id);
    }

    fn note_window_hidden(&mut self, window_id: WindowId) {
        self.front_to_back_window_ids.retain(|id| *id != window_id);
    }

    fn note_window_removed(&mut self, window_id: WindowId) {
        self.note_window_hidden(window_id);
        self.window_styles.remove(&window_id);
    }

    fn has_positioned_no_focus_window(&self) -> bool {
        self.front_to_back_window_ids
            .iter()
            .any(|id| self.window_styles.get(id) == Some(&WindowStyle::PositionedNoFocus))
    }
}

impl WindowManager {
    pub(crate) fn new(
        event_loop_proxy: EventLoopProxy<CustomEvent>,
        display_handle: OwnedDisplayHandle,
    ) -> Self {
        Self {
            windows: Default::default(),
            event_loop_proxy,
            window_ordering: Default::default(),
            os_window_manager_name: Default::default(),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            x11_manager: match x11::X11Manager::new() {
                Ok(x11_manager) => Some(x11_manager),
                Err(err) => {
                    log::error!("error creating connection to Xorg server: {err:?}");
                    None
                }
            },
            display_handle,
        }
    }

    /// Get winit's determination of the scale factor.
    ///
    /// In X11, the scale factor is a per-screen setting. Note that a "screen" in X11 is not the
    /// same thing as a physical monitor, but a grouping of monitors into a single coordinate
    /// space. All our app's windows must be on the same screen, and hence will have the same scale
    /// factor. For more in-depth explanation:
    /// https://github.com/warpdotdev/warp-internal/pull/8431#discussion_r1460629912
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn get_x11_backing_scale_factor(&self) -> f32 {
        use crate::platform::WindowContext;

        self.windows
            .values()
            .next()
            .map(|window| window.backing_scale_factor())
            .unwrap_or(1.)
    }
}

impl platform::WindowManager for WindowManager {
    fn open_window(
        &mut self,
        window_id: WindowId,
        window_options: WindowOptions,
        callbacks: WindowCallbacks,
    ) -> Result<()> {
        let style = window_options.style;
        self.event_loop_proxy.send_event(CustomEvent::OpenWindow {
            window_id,
            window_options,
        })?;
        self.windows
            .insert(window_id, Rc::new(super::window::Window::new(callbacks)));
        self.window_ordering
            .lock()
            .note_window_created(window_id, style);
        Ok(())
    }

    fn platform_window(&self, window_id: WindowId) -> OptionalPlatformWindow {
        self.windows
            .get(&window_id)
            .map(Rc::clone)
            .map(|inner| inner as Rc<dyn crate::platform::Window>)
    }

    fn remove_window(&mut self, window_id: WindowId) {
        self.windows.remove(&window_id);
        self.window_ordering.lock().note_window_removed(window_id);
    }

    fn active_window_id(&self) -> Option<WindowId> {
        // Weirdly, it is possible for "has_focus" to return `true` for hidden windows, so also
        // check "is_visible".
        self.windows
            .iter()
            .find_map(|(id, window)| (window.has_focus() && window.is_visible()).then_some(*id))
    }

    fn key_window_is_modal_panel(&self) -> bool {
        false
    }

    fn app_is_active(&self) -> bool {
        self.active_window_id().is_some()
    }

    /// Activate the app as defined in MacOS.
    ///
    /// The concept of "activating" an app doesn't really exist on non-Mac platforms. This is a
    /// concept which we've borrowed from MacOS AppKit. Activating means:
    ///
    /// 1. Make all windows visible if they were hidden.
    /// 2. Stack all windows on top of other apps' windows.
    /// 3. Give focus to the frontmost window in this app.
    ///
    /// See the AppKit docs for more details:
    /// https://developer.apple.com/documentation/appkit/nsapplication/1428468-activateignoringotherapps?language=objc
    fn activate_app(&self, last_active_window: Option<WindowId>) -> Option<WindowId> {
        let mut next_active_window: Option<WindowId> = None;

        // We want to loop through all windows and stack each one on top of other windows.
        for (id, window) in &self.windows {
            // We want to be sure that the window which ends up with the focus in the end is the
            // "frontmost" window, or the one which was most recently active/focused. Therefore,
            // the last active window needs to be focused last. So, skip that window in this loop.
            if last_active_window.is_some_and(|window_id| window_id == *id) {
                continue;
            }

            window.focus();
            next_active_window = Some(*id);
        }

        // Finally, go back and focus the last active window to make sure it ends up having the
        // focus.
        if let Some(window_id) = last_active_window {
            if let Some(window) = self.windows.get(&window_id) {
                window.focus();
                next_active_window = Some(window_id);
            }
        }

        if let Some(window_id) = next_active_window {
            self.window_ordering.lock().move_to_front(window_id);
        }

        next_active_window
    }

    fn show_window_and_focus_app(&self, window_id: WindowId, behavior: WindowFocusBehavior) {
        debug_assert!(matches!(behavior, WindowFocusBehavior::BringToFront));
        if let Some(window) = self.windows.get(&window_id) {
            window.focus();
        }
        self.window_ordering.lock().move_to_front(window_id);
    }

    fn hide_app(&self) {
        for window in self.windows.values() {
            window.set_visible(false);
        }
    }

    fn hide_window(&self, window_id: WindowId) {
        if let Some(window) = self.windows.get(&window_id) {
            window.set_visible(false);
        }
        self.window_ordering.lock().note_window_hidden(window_id);
    }

    fn set_window_bounds(&self, window_id: WindowId, bound: RectF) {
        if let Some(window) = self.windows.get(&window_id) {
            window.set_bounds(bound);
        }
    }

    fn set_all_windows_background_blur_radius(&self, _blur_radius_pixels: u8) {
        // unsupported on Linux and Windows
        // https://docs.rs/winit/latest/winit/window/struct.Window.html#method.set_blur
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    fn set_all_windows_background_blur_texture(&self, use_blur_texture: bool) {
        #[cfg(windows)]
        {
            let new_backdrop_texture = if use_blur_texture {
                BackdropType::TransientWindow
            } else {
                BackdropType::None
            };
            for window in self.windows.values() {
                if let Some(inner) = window.inner.borrow().as_ref() {
                    inner.window.set_system_backdrop(new_backdrop_texture);
                }
            }
        }
    }

    fn set_window_title(&self, window_id: WindowId, title: &str) {
        if let Some(window) = self.windows.get(&window_id) {
            window.set_title(title);
        }
    }

    fn close_window_async(&self, window_id: WindowId, termination_mode: TerminationMode) {
        self.event_loop_proxy
            .send_event(CustomEvent::CloseWindow {
                window_id,
                termination_mode,
            })
            .expect("event loop should still exist");
    }

    fn active_display_bounds(&self) -> RectF {
        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
                self.x11_manager
                    .as_ref()
                    .and_then(|x11_manager| match x11_manager.get_active_monitor() {
                        Ok(result) => Some(result),
                        Err(err) => {
                            log::warn!("Error getting active display bounds: {err:?}");
                            None
                        }
                    })
                    .map(|(_, monitor_bounds)| x11::physical_bounds_to_rect(&monitor_bounds, self.get_x11_backing_scale_factor()))
                    .unwrap_or(RectF::new(Vector2F::zero(), *DEFAULT_WINDOW_SIZE))
            } else if #[cfg(windows)] {
                self.get_active_monitor_logical_bounds().unwrap_or(RectF::new(Vector2F::zero(), *DEFAULT_WINDOW_SIZE))
            } else {
                RectF::new(Vector2F::zero(), *DEFAULT_WINDOW_SIZE)
            }
        }
    }

    fn active_display_id(&self) -> DisplayId {
        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
                self.x11_manager
                    .as_ref()
                    .and_then(|x11_manager| match x11_manager.get_active_monitor() {
                        Ok(result) => Some(result),
                        Err(err) => {
                            log::warn!("Error getting active display ID: {err:?}");
                            None
                        }
                    })
                    .map(|(i, _)| DisplayId::from(i))
                    .unwrap_or(0.into())
            } else if #[cfg(windows)] {
                self.get_current_monitor_id().unwrap_or(0.into())
            } else {
                0.into()
            }
        }
    }

    fn display_count(&self) -> usize {
        // Although winit provides a `Window::available_monitors` method, it caches the result and
        // never invalidates the cache. We need to drop down to X11 directly to ensure we read a
        // fresh value.
        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
                self.x11_manager
                    .as_ref()
                    .and_then(|x11_manager| x11_manager.list_monitor_bounds().ok())
                    .as_ref()
                    .map(|monitors| monitors.len())
                    .unwrap_or(1)
            } else if #[cfg(windows)] {
                self.get_available_monitor_count().unwrap_or(1_usize)
            } else {
                1
            }
        }
    }

    fn bounds_for_display_idx(&self, display_idx: DisplayIdx) -> Option<RectF> {
        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
                let idx = match display_idx {
                    DisplayIdx::Primary => 0,
                    DisplayIdx::External(idx) => idx + 1,
                };
                self.x11_manager
                    .as_ref()
                    .and_then(|x11_manager| x11_manager.list_monitor_bounds().ok())
                    .as_ref()
                    .and_then(|monitors| monitors.get(idx))
                    .map(|monitor| x11::physical_bounds_to_rect(monitor, self.get_x11_backing_scale_factor()))
            } else if #[cfg(windows)] {
                self.get_monitor_bounds_for_display_idx(display_idx).ok()
            } else {
                let _ = display_idx;
                None
            }
        }
    }

    fn active_cursor_position_updated(&self) {
        self.event_loop_proxy
            .send_event(CustomEvent::ActiveCursorPositionUpdated)
            .expect("event loop should still exist");
    }

    fn windowing_system(&self) -> Option<crate::windowing::System> {
        self.display_handle
            .display_handle()
            .ok()?
            .as_raw()
            .try_into()
            .ok()
    }

    fn is_tiling_window_manager(&self) -> bool {
        self.os_window_manager_name()
            .map(|name| is_tiling_window_manager(name.as_str()))
            .unwrap_or(false)
    }

    fn ordered_window_ids(&self) -> Vec<WindowId> {
        let mut window_ordering = self.window_ordering.lock();
        // Winit has no native z-order API, so we maintain this list manually.
        // Prune windows that have since been hidden.
        window_ordering
            .front_to_back_window_ids
            .retain(|window_id| {
                self.windows
                    .get(window_id)
                    .is_some_and(|window| window.is_visible())
            });
        // Keep the list current by promoting the focused window to the front.
        // Skip when a PositionedNoFocus (drag preview) window is present: it
        // was moved to the front on creation and must stay there so that
        // `cross_window_attach_target` can find it at index 0.
        if !window_ordering.has_positioned_no_focus_window() {
            if let Some(active_window_id) = self.active_window_id() {
                window_ordering.move_to_front(active_window_id);
            }
        }
        window_ordering.front_to_back_window_ids.clone()
    }

    fn os_window_manager_name(&self) -> Option<String> {
        self.os_window_manager_name
            .get_or_init(|| {
                cfg_if::cfg_if! {
                    if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
                        get_os_window_manager_name_internal(self.x11_manager.as_ref())
                    } else {
                        None
                    }
                }
            })
            .clone()
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn get_os_window_manager_name() -> Option<String> {
    get_os_window_manager_name_internal(x11::X11Manager::new().ok().as_ref())
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn get_os_window_manager_name_internal(x11_manager: Option<&x11::X11Manager>) -> Option<String> {
    super::linux::look_for_wayland_compositor()
        .or_else(|| x11_manager.and_then(|manager| manager.os_window_manager_name().ok()))
}

fn is_tiling_window_manager(name: &str) -> bool {
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            super::linux::is_tiling_window_manager(name)
        } else {
            let _ = name;
            false
        }
    }
}

/// Some additional state we need to track in memory for integration tests.
struct IntegrationTestAppState {
    /// A list of window IDs representing the order of visible windows, with
    /// the frontmost window at the start of the list.
    window_id_stack: Vec<WindowId>,
}

/// Running the integration tests cases in parallel causes some conflicts in the window state which
/// breaks the tests. In order to keep the state for each "instance" of the
/// [`platform::current::App`] separate, we track it separately here to scope it down.
pub(crate) struct IntegrationTestWindowManager {
    window_manager: WindowManager,
    app_state: Mutex<IntegrationTestAppState>,
}

impl IntegrationTestWindowManager {
    pub(crate) fn new(
        event_loop_proxy: EventLoopProxy<CustomEvent>,
        display_handle: OwnedDisplayHandle,
    ) -> Self {
        Self {
            window_manager: WindowManager::new(event_loop_proxy, display_handle),
            app_state: Mutex::new(IntegrationTestAppState {
                window_id_stack: Default::default(),
            }),
        }
    }
}

impl platform::WindowManager for IntegrationTestWindowManager {
    fn open_window(
        &mut self,
        window_id: WindowId,
        window_options: WindowOptions,
        callbacks: WindowCallbacks,
    ) -> Result<()> {
        let window_should_be_tracked = window_options.style != platform::WindowStyle::NotStealFocus;
        self.window_manager
            .open_window(window_id, window_options, callbacks)?;
        if window_should_be_tracked {
            let mut app_state = self.app_state.lock();
            app_state.window_id_stack.retain(|id| *id != window_id);
            app_state.window_id_stack.insert(0, window_id);
        }
        Ok(())
    }

    fn platform_window(&self, window_id: WindowId) -> OptionalPlatformWindow {
        self.window_manager.platform_window(window_id)
    }

    fn remove_window(&mut self, window_id: WindowId) {
        self.window_manager.remove_window(window_id)
    }

    fn active_window_id(&self) -> Option<WindowId> {
        self.app_is_active()
            .then(|| self.app_state.lock().window_id_stack.first().cloned())
            .flatten()
    }

    fn key_window_is_modal_panel(&self) -> bool {
        self.window_manager.key_window_is_modal_panel()
    }

    fn app_is_active(&self) -> bool {
        // always assume active for tests.
        true
    }

    fn activate_app(&self, last_active_window: Option<WindowId>) -> Option<WindowId> {
        self.window_manager.activate_app(last_active_window)
    }

    fn show_window_and_focus_app(&self, window_id: WindowId, behavior: WindowFocusBehavior) {
        debug_assert!(matches!(behavior, WindowFocusBehavior::BringToFront));
        self.window_manager
            .show_window_and_focus_app(window_id, behavior);

        let mut app_state = self.app_state.lock();

        // Move the window to the top of the stack.
        app_state.window_id_stack.retain(|id| *id != window_id);
        app_state.window_id_stack.insert(0, window_id);
    }

    fn hide_app(&self) {
        self.window_manager.hide_app();
    }

    fn hide_window(&self, window_id: WindowId) {
        self.window_manager.hide_window(window_id);
        // Remove the hidden window from the window stack.
        self.app_state
            .lock()
            .window_id_stack
            .retain(|id| *id != window_id);
    }

    fn set_window_bounds(&self, window_id: WindowId, bound: RectF) {
        self.window_manager.set_window_bounds(window_id, bound)
    }

    fn set_all_windows_background_blur_radius(&self, blur_radius_pixels: u8) {
        self.window_manager
            .set_all_windows_background_blur_radius(blur_radius_pixels)
    }

    fn set_all_windows_background_blur_texture(&self, use_blur_texture: bool) {
        self.window_manager
            .set_all_windows_background_blur_texture(use_blur_texture)
    }

    fn set_window_title(&self, window_id: WindowId, title: &str) {
        self.window_manager.set_window_title(window_id, title)
    }

    fn close_window_async(&self, window_id: WindowId, termination_mode: TerminationMode) {
        self.window_manager
            .close_window_async(window_id, termination_mode);
        // Remove the closed window from the window stack.
        self.app_state
            .lock()
            .window_id_stack
            .retain(|id| *id != window_id);
    }

    fn active_display_bounds(&self) -> geometry::rect::RectF {
        self.window_manager.active_display_bounds()
    }

    fn active_display_id(&self) -> DisplayId {
        self.window_manager.active_display_id()
    }

    fn display_count(&self) -> usize {
        1
    }

    fn bounds_for_display_idx(&self, idx: DisplayIdx) -> Option<RectF> {
        self.window_manager.bounds_for_display_idx(idx)
    }

    fn active_cursor_position_updated(&self) {
        // no-op
    }

    fn windowing_system(&self) -> Option<crate::windowing::System> {
        self.window_manager.windowing_system()
    }

    fn os_window_manager_name(&self) -> Option<String> {
        self.window_manager.os_window_manager_name()
    }

    fn is_tiling_window_manager(&self) -> bool {
        self.window_manager.is_tiling_window_manager()
    }

    fn ordered_window_ids(&self) -> Vec<WindowId> {
        self.app_state.lock().window_id_stack.clone()
    }
}

fn window_level_for_style(style: WindowStyle) -> WindowLevel {
    match style {
        WindowStyle::NotStealFocus => WindowLevel::AlwaysOnBottom,
        WindowStyle::Pin => WindowLevel::AlwaysOnTop,
        _ => WindowLevel::Normal,
    }
}

/// If the selected adapter has a known rendering offset bug, enable native window decorations
/// to work around it. See: https://github.com/warpdotdev/Warp/issues/6120
fn enable_decorations_if_needed(window: &winit::window::Window, adapter_info: &AdapterInfo) {
    if adapter_has_rendering_offset_bug(adapter_info) {
        log::warn!(
            "Enabling native window decorations to work around a rendering offset bug in the \
            selected GPU adapter ({}). See: https://github.com/warpdotdev/Warp/issues/6120",
            adapter_info.name,
        );
        window.set_decorations(true);
    }
}

struct RenderingResources {
    resources: Resources,
    renderer: Renderer,
}

struct Inner {
    window: Arc<winit::window::Window>,
    #[cfg(windows)]
    is_cloaked: bool,
    #[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
    gpu_power_preference: GPUPowerPreference,
    backend_preference: Option<wgpu::Backend>,
    rendering_resources: Option<RenderingResources>,
    /// Callback that reports when a GPU device is selected. We need to store this on the window
    /// because we may attempt to recreate resources (which in turns reselects a GPU device) to
    /// handle cases where wgpu treats the device as "lost" when the system is waking up from sleep.
    on_gpu_device_selected: Box<OnGPUDeviceSelected>,
    active_drag_resize_direction: Option<ResizeDirection>,
    surface_size: Vector2F,
    surface_requires_reconfiguration: bool,
    /// The window level isn't just needed at window creation. The window level may get un-set in
    /// some desktop environments, e.g. when the window is hidden and un-hidden. We don't want this
    /// behavior, and so we must re-set the window level. In order to re-set it, we must store the
    /// intended window level.
    level: WindowLevel,
}

impl Inner {
    /// Returns the physical size of the current window.
    fn physical_size(&self) -> PhysicalSize<u32> {
        self.window.inner_size()
    }
}

pub(super) const DEFAULT_TITLEBAR_HEIGHT: f32 = 35.0;

/// A one-shot callback invoked with a captured frame. See [`platform::WindowContext::request_frame_capture`].
type FrameCaptureCallback = Box<dyn FnOnce(platform::CapturedFrame) + Send + 'static>;

pub(super) struct Window {
    pub(super) callbacks: WindowCallbacks,
    inner: RefCell<Option<Inner>>,
    scene: RefCell<Option<Rc<Scene>>>,
    /// The height, in logical pixels, of the "titlebar region" at the top of the window.
    ///
    /// The "titlebar region" is a fixed region at the top of the window which is treated as an
    /// invisible title bar insofar as it triggers window dragging when clicked-and-dragged, or
    /// maximize/restore when double-clicked.
    titlebar_height: Cell<f32>,
    capture_callback: RefCell<Option<FrameCaptureCallback>>,
}

impl Window {
    pub fn new(callbacks: WindowCallbacks) -> Self {
        Self {
            callbacks,
            inner: Default::default(),
            scene: Default::default(),
            titlebar_height: Cell::new(DEFAULT_TITLEBAR_HEIGHT),
            capture_callback: RefCell::new(None),
        }
    }

    pub fn titlebar_height(&self) -> f32 {
        self.titlebar_height.get()
    }

    pub fn open_window(
        &self,
        window_target: &ActiveEventLoop,
        window_options: WindowOptions,
        window_class: &Option<String>,
        tiling_window_manager: bool,
        downrank_non_nvidia_vulkan_adapters: bool,
    ) -> Result<winit::window::WindowId> {
        let window = create_window(
            window_target,
            &window_options,
            window_class,
            tiling_window_manager,
        )?;

        let window = Arc::new(window);

        // Use the window's size as the initial surface size, ensuring that the
        // surface has a minimum size of 1 along each dimension.
        let initial_surface_size = window.inner_size().to_vec2f().max(Vector2F::splat(1.));

        let gpu_power_preference = window_options.gpu_power_preference;
        let backend_preference = window_options.backend_preference.map(to_wgpu_backend);
        let resources = Resources::new(
            window.clone(),
            gpu_power_preference,
            backend_preference,
            &window_options.on_gpu_device_info_reported,
            initial_surface_size,
            downrank_non_nvidia_vulkan_adapters,
        )?;

        enable_decorations_if_needed(&window, &resources.adapter.get_info());

        let renderer = Renderer::new(&resources, GlyphConfig::default());

        let window_id = window.id();
        self.inner.replace(Some(Inner {
            window,
            #[cfg(windows)]
            is_cloaked: true,
            gpu_power_preference,
            backend_preference,
            rendering_resources: Some(RenderingResources {
                resources,
                renderer,
            }),
            on_gpu_device_selected: window_options.on_gpu_device_info_reported,
            active_drag_resize_direction: None,
            surface_size: initial_surface_size,
            surface_requires_reconfiguration: false,
            level: window_level_for_style(window_options.style),
        }));
        Ok(window_id)
    }

    pub fn update_size_if_needed(&self) -> Result<(), renderer::Error> {
        let mut inner = self.inner.borrow_mut();
        let Some(inner) = inner.as_mut() else {
            log::warn!("Tried to render a window before it had been fully initialized");
            return Ok(());
        };

        let window_size = inner.physical_size().to_vec2f();

        let Some(RenderingResources { resources, .. }) = inner.rendering_resources.as_mut() else {
            return Ok(());
        };

        // This log exists to let us know if we can eliminate the size
        // comparison and rely solely on the boolean flag, which is set to true
        // when we receive a window resize event.  We're logging this in debug
        // builds only so that we can (hopefully) understand when this occurs
        // and why, but it's not something we need to log in production.
        #[cfg(debug_assertions)]
        if inner.surface_size != window_size && !inner.surface_requires_reconfiguration {
            log::info!("surface size changed but does not require reconfiguration!");
        }

        // If the window size has changed since we last configured the
        // underlying surface, reconfigure the surface with the new size.
        if inner.surface_requires_reconfiguration || inner.surface_size != window_size {
            resources.update_surface_size(window_size)?;
            inner.surface_size = window_size;
            inner.surface_requires_reconfiguration = false;
        }

        Ok(())
    }

    pub fn render(
        &self,
        new_scene: Option<Rc<Scene>>,
        font_cache: &fonts::Cache,
    ) -> Result<(), renderer::Error> {
        let mut scene = self.scene.borrow_mut();

        if scene.is_none() {
            *scene = new_scene;
        }

        let Some(scene) = scene.clone() else {
            log::error!(
                "A redraw of the window was requested but no scene was available to render"
            );
            return Ok(());
        };

        let mut inner = self.inner.borrow_mut();
        let Some(inner) = inner.as_mut() else {
            log::warn!("Tried to render a window before it had been fully initialized");
            return Ok(());
        };

        let Some(RenderingResources {
            resources,
            renderer,
        }) = inner.rendering_resources.as_mut()
        else {
            return Ok(());
        };

        let capture_callback = self.capture_callback.borrow_mut().take();
        let window = &inner.window;
        renderer.render(
            scene.as_ref(),
            resources,
            &|glyph_key, scale, subpixel_alignment, glyph_config, format| {
                font_cache.rasterized_glyph(
                    glyph_key,
                    scale,
                    subpixel_alignment,
                    glyph_config,
                    format,
                )
            },
            &|glyph_key, scale, alignment| {
                font_cache.glyph_raster_bounds(glyph_key, scale, alignment)
            },
            inner.surface_size,
            Some(Box::new(|| {
                window.pre_present_notify();
            })),
            capture_callback,
        )?;

        #[cfg(windows)]
        {
            use crate::windowing::winit::windows::WindowExt;

            // Uncloak the window upon successfully drawing a frame.
            if inner.is_cloaked {
                match inner.window.set_cloaked(false) {
                    Ok(_) => {
                        inner.is_cloaked = false;
                    }
                    Err(e) => {
                        log::warn!("Failed to uncloak window: {e:#?}");
                    }
                }
            }
        }
        Ok(())
    }

    /// Drops the window's renderer and all associated resources.
    #[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
    pub fn drop_renderer(&self, display_handle: Box<dyn wgpu::wgt::WgpuHasDisplayHandle>) {
        let mut inner = self.inner.borrow_mut();
        let Some(inner) = inner.as_mut() else {
            log::warn!("Tried to drop a window's renderer before it had been fully initialized");
            return;
        };

        let _ = inner.rendering_resources.take();

        // Forceably drop and recreate our cached `wgpu::Instance` after dropping the renderer.
        // Without this, certain NVIDIA drivers deadlock upon recreating the resources with the
        // existing `Instance`.
        crate::rendering::wgpu::reset_wgpu_instance(display_handle);
    }

    /// Recreates the window's renderer and all associated resources.
    #[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
    pub fn recreate_renderer(&self, downrank_non_nvidia_vulkan_adapters: bool) {
        let mut inner = self.inner.borrow_mut();
        let Some(inner) = inner.as_mut() else {
            log::warn!(
                "Tried to recreate a window's renderer before it had been fully initialized"
            );
            return;
        };

        let resources = match Resources::new(
            inner.window.clone(),
            inner.gpu_power_preference,
            inner.backend_preference,
            &inner.on_gpu_device_selected,
            inner.surface_size,
            downrank_non_nvidia_vulkan_adapters,
        )
        .context("Failed to recreate window renderer")
        {
            Ok(resources) => resources,
            Err(err) => {
                log::error!("{err:#}");
                return;
            }
        };

        enable_decorations_if_needed(&inner.window, &resources.adapter.get_info());

        let renderer = Renderer::new(&resources, GlyphConfig::default());

        let _ = inner.rendering_resources.insert(RenderingResources {
            resources,
            renderer,
        });
    }

    pub fn has_scene(&self) -> bool {
        self.scene.borrow().is_some()
    }

    pub fn handle_resize(&self) {
        let mut inner = self.inner.borrow_mut();
        let Some(inner) = inner.as_mut() else {
            return;
        };

        // Force a recreation of the swap chain, in case it became outdated by
        // the resize.
        inner.surface_requires_reconfiguration = true;
    }

    /// This method updates window state derived from the cursor position. It keeps track of
    /// whether the cursor is in a position to initiate drag-resizing.
    pub fn update_drag_resize_state(&self, position: LogicalPosition<f32>) {
        let mut inner = self.inner.borrow_mut();
        let Some(inner) = inner.as_mut() else {
            return;
        };

        // Windows are not drag-resizable when maximized.
        let drag_resize_direction = if inner.window.is_maximized() {
            None
        } else {
            let scale_factor = inner.window.scale_factor();
            Self::drag_resize_direction_at_position(
                inner.physical_size().to_logical(scale_factor),
                position,
                DRAG_RESIZE_MARGIN,
            )
        };

        if let Some(resize_direction) = &drag_resize_direction {
            inner
                .window
                .set_cursor(winit::window::CursorIcon::from(*resize_direction));
        }

        // Reset the cursor icon if we stopped resizing.
        if inner.active_drag_resize_direction.is_some() && drag_resize_direction.is_none() {
            inner.window.set_cursor(winit::window::Cursor::default());
        }

        inner.active_drag_resize_direction = drag_resize_direction;
    }

    /// Start a drag-resize iff the cursor is in a position to do this. That positioning should
    /// already have been determined by [`Self::update_drag_resize_state`]. Returns `true` if
    /// drag-resizing was successfully initiated.
    pub fn try_drag_resize(&self) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some(inner) = inner.as_mut() else {
            return false;
        };
        let Some(direction) = inner.active_drag_resize_direction else {
            return false;
        };
        inner.window.drag_resize_window(direction).is_ok()
    }

    fn drag_resize_direction_at_position(
        window_size: LogicalSize<u32>,
        cursor_position: LogicalPosition<f32>,
        margin: f32,
    ) -> Option<ResizeDirection> {
        enum XDirection {
            West,
            East,
            None,
        }

        enum YDirection {
            North,
            South,
            None,
        }

        let xdir = if cursor_position.x < margin {
            XDirection::West
        } else if cursor_position.x > (window_size.width as f32 - margin) {
            XDirection::East
        } else {
            XDirection::None
        };

        let ydir = if cursor_position.y < margin {
            YDirection::North
        } else if cursor_position.y > (window_size.height as f32 - margin) {
            YDirection::South
        } else {
            YDirection::None
        };

        let dir = match (ydir, xdir) {
            (YDirection::North, XDirection::West) => ResizeDirection::NorthWest,
            (YDirection::North, XDirection::East) => ResizeDirection::NorthEast,
            (YDirection::North, XDirection::None) => ResizeDirection::North,
            (YDirection::South, XDirection::West) => ResizeDirection::SouthWest,
            (YDirection::South, XDirection::East) => ResizeDirection::SouthEast,
            (YDirection::South, XDirection::None) => ResizeDirection::South,
            (YDirection::None, XDirection::West) => ResizeDirection::West,
            (YDirection::None, XDirection::East) => ResizeDirection::East,
            (YDirection::None, XDirection::None) => return None,
        };
        Some(dir)
    }

    pub fn is_decorated(&self) -> bool {
        self.inner
            .borrow()
            .as_ref()
            .map(|inner| inner.window.is_decorated())
            .unwrap_or(false)
    }

    /// Requests user attention. If the window is in focus, this is a noop.
    pub fn request_user_attention(&self) {
        // Determine which level of user attention urgency to request from the OS.
        // On Windows, we don't use the OS-provided title bar, and the
        // Informational level doesn't seem to flash the taskbar icon,
        // so we use the Critical level instead.
        let user_attention_urgency = if cfg!(windows) {
            UserAttentionType::Critical
        } else {
            UserAttentionType::Informational
        };

        if let Some(inner) = self.inner.borrow().as_ref() {
            inner
                .window
                .request_user_attention(Some(user_attention_urgency));
        };
    }

    /// Stops requesting user attention for the current window. If window has not previously requested user attention,
    /// this is a noop.
    pub fn stop_requesting_user_attention(&self) {
        if let Some(inner) = self.inner.borrow().as_ref() {
            inner.window.request_user_attention(None);
        };
    }

    pub fn set_ime_position<P, S>(&self, position: P, size: S)
    where
        P: Into<Position>,
        S: Into<Size>,
    {
        let inner = self.inner.borrow_mut();
        if let Some(inner) = inner.as_ref() {
            inner.window.set_ime_cursor_area(position, size);
        }
    }

    pub fn drag_window(&self) -> Result<(), ExternalError> {
        let inner = self.inner.borrow();
        let Some(inner) = inner.as_ref() else {
            return Ok(());
        };
        inner.window.drag_window()
    }

    pub fn outer_position(&self) -> Option<PhysicalPosition<i32>> {
        self.inner
            .borrow()
            .as_ref()
            .and_then(|inner| inner.window.outer_position().ok())
    }

    pub fn set_outer_position(&self, position: PhysicalPosition<i32>) {
        if let Some(inner) = self.inner.borrow().as_ref() {
            inner.window.set_outer_position(position);
        }
    }

    pub fn has_focus(&self) -> bool {
        self.inner
            .borrow()
            .as_ref()
            .map(|inner| inner.window.has_focus())
            .unwrap_or(false)
    }

    pub fn focus(&self) {
        if let Some(Inner { window, level, .. }) = self.inner.borrow().as_ref() {
            // Winit is a bit quirky here. Trying to focus a window which isn't visible will not
            // make it visible. So, make it visible first if needed, then explicitly focus it.
            if window.is_visible().unwrap_or(true) {
                window.set_minimized(false);
            } else {
                window.set_visible(true);
            }
            window.focus_window();
            window.set_window_level(*level);
        }
    }

    /// Sets whether or not the window is visible.
    ///
    /// The definition of "visibility" depends on the platform. On X11 this is referring to the
    /// concept of "mapping" a window. If a window is "unmapped" it is hidden, i.e. unviewable. The
    /// window is removed from the screen and, in some desktop environments, removed from window-
    /// switchers like alt-tab.
    /// See the Xlib docs:
    /// https://tronche.com/gui/x/xlib/window/XMapWindow.html
    ///
    /// On Wayland, setting visibility is unsupported.
    fn set_visible(&self, visible: bool) {
        if let Some(Inner { window, level, .. }) = self.inner.borrow().as_ref() {
            window.set_visible(visible);
            window.set_window_level(*level);
        }
    }

    /// Reads whether or not the window is visible.
    ///
    /// See [`Self::set_visible`] for an explanation of what "visible" means in winit. For
    /// platforms where setting visibility is unsupported, e.g. Wayland, always return `true`.
    #[cfg(not(target_family = "wasm"))]
    fn is_visible(&self) -> bool {
        self.inner
            .borrow()
            .as_ref()
            .and_then(|inner| inner.window.is_visible())
            .unwrap_or(true)
    }

    /// Intended for reading whether or not the window is visible. Always returns true.
    ///
    /// winit does not support is_visible on wasm. See: https://docs.rs/winit/latest/winit/window/struct.Window.html#method.is_visible
    #[cfg(target_family = "wasm")]
    fn is_visible(&self) -> bool {
        true
    }

    fn set_bounds(&self, bounds: RectF) {
        if let Some(Inner { window, .. }) = self.inner.borrow().as_ref() {
            let origin = bounds.origin();
            window.set_outer_position(LogicalPosition::new(origin.x(), origin.y()));
            let size = bounds.size();
            let requested_size = LogicalSize::new(size.x() as u32, size.y() as u32);
            let resize_result = window.request_inner_size(requested_size);
            match resize_result {
                Some(resulting_size) => {
                    if resulting_size == requested_size.to_physical(window.scale_factor()) {
                        log::debug!("resize request fulfilled synchronously");
                    } else {
                        log::info!(
                            "resizing unallowed by windowing system. resize request ignored"
                        );
                    }
                }
                None => log::debug!("resize request sent asynchronously"),
            };
        }
    }

    fn set_title(&self, title: &str) {
        if let Some(Inner { window, .. }) = self.inner.borrow().as_ref() {
            window.set_title(title)
        }
    }

    pub(super) fn set_cursor_icon(&self, cursor: Cursor) {
        if let Some(Inner { window, .. }) = self.inner.borrow().as_ref() {
            let icon = match cursor {
                Cursor::Arrow => CursorIcon::Default,
                Cursor::IBeam => CursorIcon::Text,
                Cursor::Crosshair => CursorIcon::Crosshair,
                Cursor::OpenHand => CursorIcon::Grab,
                Cursor::ClosedHand => CursorIcon::Grabbing,
                Cursor::NotAllowed => CursorIcon::NotAllowed,
                Cursor::PointingHand => CursorIcon::Pointer,
                Cursor::ResizeLeftRight => CursorIcon::ColResize,
                Cursor::ResizeUpDown => CursorIcon::RowResize,
                Cursor::DragCopy => CursorIcon::Copy,
            };

            window.set_cursor(winit::window::Cursor::Icon(icon));
        }
    }
}

#[cfg(target_family = "wasm")]
fn create_window(
    window_target: &ActiveEventLoop,
    _window_options: &WindowOptions,
    _window_class: &Option<String>,
    _tiling_window_manager: bool,
) -> Result<winit::window::Window> {
    use winit::platform::web::WindowAttributesExtWebSys;
    use winit::platform::web::WindowExtWebSys;

    use crate::platform::current::add_prevent_default_listener;

    let window_attributes = winit::window::WindowAttributes::default().with_prevent_default(false);

    let window = window_target.create_window(window_attributes)?;
    let canvas = window
        .canvas()
        .ok_or(anyhow::anyhow!("Failed to find canvas element"))?;

    if let Some(element) = gloo::utils::document().get_element_by_id("wasm-container") {
        log::info!("Attaching canvas element \"{canvas:?}\" to the wasm-container element");
        element.replace_children_with_node_1(&canvas);
    } else {
        log::info!("Attaching canvas element \"{canvas:?}\" to the document body");
        gloo::utils::body()
            .append_child(&canvas)
            .map_err(|_| anyhow::anyhow!("Failed to append canvas element to <body>"))?;
    }

    add_prevent_default_listener(&canvas);
    let _ = canvas.focus();

    Ok(window)
}

#[cfg(not(target_family = "wasm"))]
fn create_window(
    window_target: &ActiveEventLoop,
    window_options: &WindowOptions,
    _window_class: &Option<String>,
    tiling_window_manager: bool,
) -> Result<winit::window::Window> {
    let decorations = !window_options.hide_title_bar;

    let mut window_attributes = winit::window::WindowAttributes::default()
        .with_min_inner_size(MIN_WINDOW_SIZE)
        .with_decorations(decorations)
        .with_window_level(window_level_for_style(window_options.style))
        .with_transparent(true);

    if let Some(title) = &window_options.title {
        window_attributes.title = title.to_owned();
    }

    #[cfg_attr(
        not(windows),
        expect(
            unused_mut,
            reason = "Windows may need to ignore the requested bounds if they are off any of the \
                displays. Linux will adjust the bounds automatically."
        )
    )]
    let mut window_bounds = window_options.bounds;

    // The monitor which has the most overlap with the new window. Will only be Some if
    // `window_bounds` is a WindowBounds::ExactPosition.
    #[cfg_attr(
        not(windows),
        expect(
            unused,
            reason = "Both uses of this variable are for Windows-only workarounds."
        )
    )]
    let mut most_overlapping_monitor: Option<MonitorHandle> = None;

    #[cfg(windows)]
    {
        // WARNING: Do not use [`WindowAttributes::with_no_redirection_bitmap`] as that caused:
        // https://github.com/warpdotdev/Warp/issues/8935

        use winit::platform::windows::{IconExtWindows, WindowAttributesExtWindows};

        let background_texture = if window_options.background_blur_texture {
            BackdropType::TransientWindow
        } else {
            BackdropType::None
        };
        window_attributes = window_attributes.with_system_backdrop(background_texture);

        // On Windows, don't set the window to be visible until after it has been marked as
        // "cloaked". Winit doesn't support initializing a window as cloaked--so we temporarily set
        // the window to be invisible to ensure the window doesn't flash in between the window being
        // created and the window being marked as cloaked.
        window_attributes.visible = false;

        let icon = winit::window::Icon::from_resource(IDI_ICON, None);
        window_attributes.window_icon = icon.as_ref().ok().cloned();
        window_attributes = window_attributes.with_taskbar_icon(icon.ok());

        // This is to make sure the bounds the caller is requesting intersects with any of the
        // monitors. We only do this on Windows b/c Windows happily renders a window outside of any
        // monitor, whereas Linux window managers automatically correct this.
        if let WindowBounds::ExactPosition(bound_rect) = window_options.bounds {
            most_overlapping_monitor = window_target
                .available_monitors()
                .filter_map(|monitor| {
                    get_monitor_logical_bounds(&monitor)
                        .intersection(bound_rect)
                        .map(|rect| (monitor, rect.width() * rect.height()))
                })
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|pair| pair.0);
            if most_overlapping_monitor.is_none() {
                window_bounds = WindowBounds::Default;
            }
        }
    }

    let (size, origin) = if tiling_window_manager && window_options.style != WindowStyle::Pin {
        (None, None)
    } else {
        match window_bounds {
            // If we weren't passed specific bounds for the new window, use a
            // reasonable initial size but don't specify an origin - let the window
            // manager decide the window's position.
            WindowBounds::Default => (Some(*DEFAULT_WINDOW_SIZE), None),
            WindowBounds::ExactSize(size) => (Some(size), None),
            WindowBounds::ExactPosition(bounds) => (Some(bounds.size()), Some(bounds.origin())),
        }
    };

    if let Some(origin) = origin {
        let mut position =
            Position::Logical(LogicalPosition::new(origin.x() as f64, origin.y() as f64));
        // Manually convert logical position to physical. Normally, winit does this for us.
        // However, this conversion is failing on Windows so we do it ourselves.
        if let (Some(monitor), true) = (most_overlapping_monitor, cfg!(windows)) {
            position = Position::Physical(position.to_physical(monitor.scale_factor()));
        }
        window_attributes.position = Some(position);
    }

    if let Some(size) = size {
        window_attributes.inner_size = Some(Size::Logical(LogicalSize::new(
            size.x() as f64,
            size.y() as f64,
        )));
    }

    match window_options.fullscreen_state {
        FullscreenState::Fullscreen => {
            window_attributes.fullscreen = Some(Fullscreen::Borderless(None));
        }
        FullscreenState::Maximized => {
            window_attributes.maximized = true;
        }
        FullscreenState::Normal => {}
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if let Some(window_class) = _window_class.as_deref() {
        use winit::platform::x11::{WindowAttributesExtX11, WindowType};

        window_attributes = window_attributes.with_name(
            window_class,
            window_options
                .window_instance
                .as_deref()
                .unwrap_or(window_class),
        );

        if tiling_window_manager && window_options.style == WindowStyle::Pin {
            window_attributes = window_attributes.with_x11_window_type(vec![WindowType::Dialog]);
        }
    }

    #[allow(clippy::let_and_return)]
    let created_window = window_target
        .create_window(window_attributes)
        .map_err(Into::into);

    #[cfg(windows)]
    {
        use super::windows::WindowExt;
        if let Ok(window) = created_window.as_ref() {
            // Mark the window as cloaked when created to prevent a white flash from occurring in
            // the time between the window being created and us drawing to the window. On Windows, a
            // "cloaked" window is one that is not visible but can still be composited / drawn to.
            // This differs from `visible` which is not composited.
            // The window is uncloaked after the drawing the first frame.
            if let Err(e) = window.set_cloaked(true) {
                log::error!("Failed to mark window as cloaked: {e:#?}");
            };

            if let Some(adjustment) = maybe_adjust_window_vertically(window) {
                let direction = if adjustment > 0 { "down" } else { "up" };
                log::info!(
                    "Window launched partially offsceen. Moving the window {direction} by {} pixels",
                    adjustment.abs()
                );
            }

            window.set_visible(true);
            window.set_ime_allowed(true);

            // When launching a window from windows file explorer, it isn't given focus. We're considering
            // this a winit quirk and forcing it to be focused.
            if !matches!(
                window_options.style,
                WindowStyle::NotStealFocus | WindowStyle::PositionedNoFocus
            ) {
                window.focus_window();
            }

            let rounded_corner_result = set_window_attribute(
                window,
                Dwm::DWMWA_WINDOW_CORNER_PREFERENCE,
                Dwm::DWMWCP_ROUND,
            );

            static WINDOWS_VERSION: LazyLock<windows_version::OsVersion> =
                LazyLock::new(windows_version::OsVersion::current);

            if let Err(err) = rounded_corner_result {
                match err {
                    WindowAttributeErr::Win32Error(_) if WINDOWS_VERSION.build < 22000 => {
                        log::info!("Rounded window corners not supported on Windows 10");
                    }
                    _ => {
                        log::error!("Error setting rounded window corners: {err:#}");
                    }
                }
            }

            let caption_button_result = get_system_caption_button_bounds(window);
            match caption_button_result {
                Ok(_caption_button_location) => {
                    // TODO: use location to actually draw buttons
                }
                Err(err) => {
                    log::warn!("Couldn't retrieve system caption button bounds: {err:?}");
                }
            };
        }
    }

    created_window
}

#[cfg(windows)]
/// Moves the new window up if it was positioned vertically offscreen. This only checks for the window
/// being too low vertically. We have this additional check because winit doesn't handle the case of us
/// adjusting the default window size (DEFAULT_WINDOW_SIZE) without setting a window position particularly
/// well.
///
/// Returns the vertical difference of the adjustment, or None.
fn maybe_adjust_window_vertically(window: &winit::window::Window) -> Option<i32> {
    if window.is_maximized() || window.fullscreen().is_some() {
        return None;
    }
    let window_position = window.outer_position().ok()?;
    let window_size = window.outer_size();
    let bottom_of_window = window_position.y + window_size.height as i32;

    let current_monitor = window.current_monitor()?;
    let monitor_position = current_monitor.position();
    let bottom_of_monitor = monitor_position.y + current_monitor.size().height as i32;

    let mut adjustment = 0;
    if window_position.y < monitor_position.y {
        adjustment = monitor_position.y - window_position.y;
    } else if bottom_of_window > bottom_of_monitor {
        adjustment = bottom_of_monitor - bottom_of_window;
    }

    if adjustment.unsigned_abs() <= current_monitor.size().height {
        window.set_outer_position(winit::dpi::PhysicalPosition::new(
            window_position.x,
            window_position.y + adjustment,
        ));
        Some(adjustment)
    } else {
        None
    }
}

impl crate::platform::Window for Window {
    fn callbacks(&self) -> &WindowCallbacks {
        &self.callbacks
    }

    fn minimize(&self) {
        if let Some(Inner { window, .. }) = self.inner.borrow().as_ref() {
            window.set_minimized(true);
        }
    }

    fn toggle_maximized(&self) {
        if let Some(Inner { window, .. }) = self.inner.borrow().as_ref() {
            window.set_maximized(!window.is_maximized());
        }
    }

    fn toggle_fullscreen(&self) {
        if let Some(Inner { window, .. }) = self.inner.borrow().as_ref() {
            match window.fullscreen() {
                Some(_) => window.set_fullscreen(None),
                None => window.set_fullscreen(Some(Fullscreen::Borderless(None))),
            };
        }
    }

    fn fullscreen_state(&self) -> FullscreenState {
        self.inner
            .borrow()
            .as_ref()
            .and_then(|Inner { window, .. }| {
                window
                    .fullscreen()
                    .map(|_| FullscreenState::Fullscreen)
                    .or(window.is_maximized().then_some(FullscreenState::Maximized))
            })
            .unwrap_or_default()
    }

    fn supports_transparency(&self) -> bool {
        self.inner
            .borrow()
            .as_ref()
            .and_then(|inner| inner.rendering_resources.as_ref())
            .is_some_and(|resources| {
                let surface = &resources.resources.surface;
                let adapter = &resources.resources.adapter;
                surface
                    .get_capabilities(adapter)
                    .alpha_modes
                    .iter()
                    .any(|&mode| {
                        matches!(
                            mode,
                            CompositeAlphaMode::PreMultiplied
                                | CompositeAlphaMode::PostMultiplied
                                | CompositeAlphaMode::Inherit
                        )
                    })
            })
    }

    fn graphics_backend(&self) -> GraphicsBackend {
        self.inner
            .borrow()
            .as_ref()
            .and_then(|inner| inner.rendering_resources.as_ref())
            .map(|resources| from_wgpu_backend(resources.resources.adapter.get_info().backend))
            .unwrap_or(GraphicsBackend::Empty)
    }

    fn supported_backends(&self) -> Vec<GraphicsBackend> {
        self.inner
            .borrow()
            .as_ref()
            .and_then(|inner| inner.rendering_resources.as_ref())
            .map(|resources| {
                resources
                    .resources
                    .supported_backends
                    .iter()
                    .map(|backend| from_wgpu_backend(*backend))
                    .collect_vec()
            })
            .unwrap_or_default()
    }

    fn uses_native_window_decorations(&self) -> bool {
        self.is_decorated()
    }

    fn as_ctx(&self) -> &dyn platform::WindowContext {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn set_titlebar_height(&self, height: f64) {
        self.titlebar_height.set(height as f32);
    }
}

impl platform::WindowContext for Window {
    fn size(&self) -> Vector2F {
        let scale_factor = self.backing_scale_factor() as f64;
        self.inner
            .borrow()
            .as_ref()
            .map_or(Vector2F::zero(), |inner| {
                let size = inner.window.inner_size().to_logical::<f32>(scale_factor);
                Vector2F::new(size.width, size.height)
            })
    }

    fn origin(&self) -> Vector2F {
        let scale_factor = self.backing_scale_factor() as f64;
        self.inner
            .borrow()
            .as_ref()
            .map_or(Vector2F::zero(), |inner| {
                // [`winit::window::Window::outer_position`] is returning weird results on Windows.
                // When maximized, it shows (physical) position (-13, -13) as the origin.
                // `inner_position` seems to be correct on Windows, whereas `outer_position` seems
                // correct on Linux.
                let position_res = if cfg!(windows) {
                    inner.window.inner_position()
                } else {
                    inner.window.outer_position()
                };
                let Ok(position) = position_res else {
                    return Vector2F::zero();
                };
                let position = position.to_logical::<f32>(scale_factor);
                Vector2F::new(position.x, position.y)
            })
    }

    fn backing_scale_factor(&self) -> f32 {
        self.inner
            .borrow()
            .as_ref()
            .map_or(1., |inner| inner.window.scale_factor() as f32)
    }

    fn max_texture_dimension_2d(&self) -> Option<u32> {
        self.inner
            .borrow()
            .as_ref()
            .and_then(|inner| inner.rendering_resources.as_ref())
            .map(|resources| resources.resources.device.limits().max_texture_dimension_2d)
    }

    fn render_scene(&self, scene: Rc<Scene>) {
        self.scene.borrow_mut().replace(scene);
        if let Some(inner) = self.inner.borrow_mut().as_mut() {
            inner.window.request_redraw();
        }
    }

    fn request_redraw(&self) {
        let _ = self.scene.borrow_mut().take();
        if let Some(inner) = self.inner.borrow_mut().as_mut() {
            inner.window.request_redraw();
        }
    }

    fn request_frame_capture(
        &self,
        callback: Box<dyn FnOnce(platform::CapturedFrame) + Send + 'static>,
    ) {
        *self.capture_callback.borrow_mut() = Some(callback);
        if let Some(inner) = self.inner.borrow_mut().as_mut() {
            inner.window.request_redraw();
        }
    }
}

/// An extension trait to add helpful methods to [`PhysicalSize`].
trait PhysicalSizeExt {
    fn to_vec2f(&self) -> Vector2F;
}

impl PhysicalSizeExt for PhysicalSize<u32> {
    fn to_vec2f(&self) -> Vector2F {
        let physical_size = self.cast::<f32>();
        vec2f(physical_size.width, physical_size.height)
    }
}

#[cfg(windows)]
fn get_monitor_logical_bounds(monitor: &MonitorHandle) -> RectF {
    let scale_factor = monitor.scale_factor();
    let logical_size = monitor.size().to_logical(scale_factor);
    let logical_position = monitor.position().to_logical(scale_factor);
    RectF::new(
        Vector2F::new(logical_position.x, logical_position.y),
        Vector2F::new(logical_size.width, logical_size.height),
    )
}
