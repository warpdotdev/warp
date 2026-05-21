use super::delegate::DispatchDelegate;
use super::{
    app, make_nsstring,
    rendering::{is_integrated_gpu, Device, RendererManager},
    RectFExt as _,
};
use anyhow::{anyhow, Result};
use cocoa::appkit::{NSWindowButton, NSWindowStyleMask};
use cocoa::foundation::{NSArray, NSPoint, NSRange, NSString, NSUInteger};
use cocoa::{
    appkit::{NSApp, NSView, NSWindow},
    base::{id, nil},
    foundation::{NSAutoreleasePool, NSInteger, NSRect, NSSize},
};
use foreign_types::ForeignType;
use itertools::Itertools;
use num_traits::FromPrimitive;
use objc::class;
use objc::{
    msg_send,
    runtime::{Object, BOOL, NO, YES},
    sel, sel_impl,
};
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use warpui_core::event::ModifiersState;
use warpui_core::platform::{
    file_picker, FilePickerCallback, FilePickerConfiguration, FullscreenState, GraphicsBackend,
    TerminationMode, WindowFocusBehavior,
};
use warpui_core::r#async::Timer;
use warpui_core::rendering::GPUPowerPreference;
use warpui_core::windowing::WindowCallbacks;
use warpui_core::{
    accessibility::AccessibilityContent,
    actions::StandardAction,
    platform::{self, WindowBounds, WindowOptions, WindowStyle},
    r#async::executor,
    Event, OptionalPlatformWindow, Scene, WindowId,
};
use warpui_core::{DisplayId, DisplayIdx};

use instant::Instant;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use std::{cell::Cell, os::raw::c_uchar, panic, path::Path, ptr};
use std::{cell::RefCell, ffi::c_void, rc::Rc};
use std::{slice, str};

extern "C" {
    fn screenFrame() -> NSRect;
    fn activeScreenId() -> NSUInteger;
}

pub const WINDOW_STATE_IVAR: &str = "windowState";
const INITIAL_WINDOW_WIDTH: f32 = 1280.;
const INITIAL_WINDOW_HEIGHT: f32 = 800.;
const DEFAULT_WINDOW_BACKGROUND_BLUR_RADIUS: u8 = 1;

// A mac::window::Window holds a reference to a WindowState.
// The NSWindow also holds a reference, so that either may be deallocated first.
pub struct Window(Rc<WindowState>);

pub(crate) struct WindowManager {
    windows: HashMap<WindowId, Rc<Window>>,
    renderer_manager: Rc<RefCell<RendererManager>>,
}

impl WindowManager {
    pub(crate) fn new() -> Self {
        Self {
            windows: Default::default(),
            renderer_manager: Rc::new(RefCell::new(RendererManager::new())),
        }
    }
}

impl platform::WindowManager for WindowManager {
    fn platform_window(&self, window_id: WindowId) -> OptionalPlatformWindow {
        self.windows
            .get(&window_id)
            .map(Rc::clone)
            .map(|inner| inner as Rc<dyn crate::platform::Window>)
    }

    fn open_window(
        &mut self,
        window_id: WindowId,
        options: WindowOptions,
        callbacks: WindowCallbacks,
    ) -> Result<()> {
        let executor = Rc::new(executor::Foreground::platform(Arc::new(DispatchDelegate))?);
        let window = Rc::new(Window::open(
            options,
            window_id,
            callbacks,
            executor,
            Rc::clone(&self.renderer_manager),
        )?);
        self.windows.insert(window_id, Rc::clone(&window));
        Ok(())
    }

    fn remove_window(&mut self, window_id: WindowId) {
        self.windows.remove(&window_id);
    }

    fn active_window_id(&self) -> Option<WindowId> {
        Window::active_window_id()
    }

    fn key_window_is_modal_panel(&self) -> bool {
        Window::key_window_is_modal_panel()
    }

    fn app_is_active(&self) -> bool {
        let res: BOOL = unsafe { msg_send![app::get_warp_app(), isActive] };
        res == YES
    }

    fn hide_app(&self) {
        unsafe {
            hide_app();
        }
    }

    fn activate_app(&self, _last_active_window: Option<WindowId>) -> Option<WindowId> {
        unsafe {
            activate_app();
        }
        Window::frontmost_window_id()
    }

    fn show_window_and_focus_app(&self, window_id: WindowId, behavior: WindowFocusBehavior) {
        if matches!(behavior, WindowFocusBehavior::BringToFront) {
            Window::show_window_and_focus_app(window_id, true)
        } else {
            Window::show_window_and_focus_app(window_id, false)
        }
    }

    fn hide_window(&self, window_id: WindowId) {
        Window::hide_window(window_id)
    }

    fn set_window_bounds(&self, window_id: WindowId, bound: RectF) {
        Window::set_window_bounds(window_id, bound)
    }

    fn set_window_alpha(&self, window_id: WindowId, alpha: f32) {
        Window::set_window_alpha(window_id, alpha)
    }

    fn set_all_windows_background_blur_radius(&self, blur_radius_pixels: u8) {
        Window::set_all_windows_background_blur_radius(blur_radius_pixels)
    }

    fn set_all_windows_background_blur_texture(&self, _use_blur_texture: bool) {
        // no-op on MacOS. This is only available on Windows.
    }

    fn set_window_title(&self, window_id: WindowId, title: &str) {
        Window::set_window_title(window_id, title)
    }

    fn close_window_async(&self, window_id: WindowId, termination_mode: TerminationMode) {
        Window::close_window_async(window_id, termination_mode);
    }

    fn display_count(&self) -> usize {
        unsafe {
            let screens: id = msg_send![class!(NSScreen), screens];
            screens.count() as usize
        }
    }

    fn active_display_bounds(&self) -> RectF {
        let rect = unsafe { screenFrame() };
        let point = Vector2F::new(rect.origin.x as f32, rect.origin.y as f32);
        let size = Vector2F::new(rect.size.width as f32, rect.size.height as f32);
        RectF::new(
            transform_origin_from_frame_coord_to_rect_coord(point, size),
            size,
        )
    }

    fn active_display_id(&self) -> DisplayId {
        let id = unsafe { activeScreenId() };
        (id as usize).into()
    }

    fn bounds_for_display_idx(&self, display_idx: DisplayIdx) -> Option<RectF> {
        let rect: NSRect = unsafe {
            let screens: id = msg_send![class!(NSScreen), screens];

            let idx: u64 = match display_idx {
                DisplayIdx::Primary => 0,
                DisplayIdx::External(idx) => (idx + 1) as u64,
            };

            if idx >= screens.count() {
                return None;
            }

            let screen: id = screens.objectAtIndex(idx);
            msg_send![screen, frame]
        };

        let point = Vector2F::new(rect.origin.x as f32, rect.origin.y as f32);
        let size = Vector2F::new(rect.size.width as f32, rect.size.height as f32);

        Some(RectF::new(
            transform_origin_from_frame_coord_to_rect_coord(point, size),
            size,
        ))
    }

    fn active_cursor_position_updated(&self) {
        // no-op on macOS
    }

    fn windowing_system(&self) -> Option<crate::windowing::System> {
        Some(crate::windowing::System::AppKit)
    }

    fn os_window_manager_name(&self) -> Option<String> {
        None
    }

    fn is_tiling_window_manager(&self) -> bool {
        false
    }

    fn ordered_window_ids(&self) -> Vec<WindowId> {
        unsafe {
            let ordered_windows: id = msg_send![NSApp(), orderedWindows];
            let count = ordered_windows.count();
            let mut result = Vec::with_capacity(count as usize);
            for i in 0..count {
                let window: id = ordered_windows.objectAtIndex(i);
                if is_warp_window(window) == YES {
                    result.push(get_window_state(&*window).window_id);
                }
            }
            result
        }
    }

    /// Cancels any in-flight synthetic mouse-drag event loop for the given window.
    ///
    /// During tab drag, we run a synthetic drag loop (pumping mouse-moved events)
    /// to track the cursor across windows. When a tab is handed off to another
    /// window or the drag is finalized, we need to stop the old loop so stale
    /// events don't continue driving the source window's drag state.
    /// Incrementing the drag ID causes the running loop to see a mismatched ID
    /// and exit on its next iteration.
    fn cancel_synthetic_drag(&self, window_id: WindowId) {
        if let Some(window) = self.windows.get(&window_id) {
            window.0.next_synthetic_drag_id();
        }
    }
}

pub(crate) struct IntegrationTestWindowManager {
    window_manager: WindowManager,
}

impl IntegrationTestWindowManager {
    pub(crate) fn new() -> Self {
        Self {
            window_manager: WindowManager::new(),
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
        self.window_manager
            .open_window(window_id, window_options, callbacks)
    }

    fn platform_window(&self, window_id: WindowId) -> OptionalPlatformWindow {
        self.window_manager.platform_window(window_id)
    }

    fn remove_window(&mut self, window_id: WindowId) {
        self.window_manager.remove_window(window_id)
    }

    fn active_window_id(&self) -> Option<WindowId> {
        // Pretend that the frontmost window is the active one. This aligns with forcing
        // `app_is_active()` to always return `true`.
        Window::frontmost_window_id()
    }

    fn key_window_is_modal_panel(&self) -> bool {
        false
    }

    fn app_is_active(&self) -> bool {
        // always assume active for tests.
        true
    }

    fn activate_app(&self, last_active_window: Option<WindowId>) -> Option<WindowId> {
        self.window_manager.activate_app(last_active_window)
    }

    fn show_window_and_focus_app(&self, window_id: WindowId, behavior: WindowFocusBehavior) {
        self.window_manager
            .show_window_and_focus_app(window_id, behavior)
    }

    fn hide_app(&self) {
        self.window_manager.hide_app()
    }

    fn hide_window(&self, window_id: WindowId) {
        self.window_manager.hide_window(window_id)
    }

    fn set_window_bounds(&self, window_id: WindowId, bound: RectF) {
        self.window_manager.set_window_bounds(window_id, bound)
    }

    fn set_window_alpha(&self, window_id: WindowId, alpha: f32) {
        self.window_manager.set_window_alpha(window_id, alpha)
    }

    fn set_all_windows_background_blur_radius(&self, _blur_radius_pixels: u8) {
        // no-op for tests
    }

    fn set_all_windows_background_blur_texture(&self, _use_blur_texture: bool) {
        // no-op for tests
    }

    fn set_window_title(&self, window_id: WindowId, title: &str) {
        self.window_manager.set_window_title(window_id, title)
    }

    fn close_window_async(&self, window_id: WindowId, termination_mode: TerminationMode) {
        self.window_manager
            .close_window_async(window_id, termination_mode)
    }

    fn active_display_bounds(&self) -> RectF {
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
        // no-op on macOS
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

    fn ordered_window_ids(&self) -> Vec<WindowId> {
        self.window_manager.ordered_window_ids()
    }

    fn cancel_synthetic_drag(&self, window_id: WindowId) {
        self.window_manager.cancel_synthetic_drag(window_id)
    }
}

// We put a pointer type into NSWindow and sometimes NSView ivars.
// The pointer type is a Box<Rc<WindowState>>, into raw.
// Note that the ivar holds a strong reference to the window state.
#[allow(non_snake_case)]
mod Ivar {
    use super::*;
    type WindowStatePtr = *mut Rc<WindowState>;

    // Convert an Rc<WindowState> into our ivar pointer type.
    // This increments the reference count.
    pub fn from_state(ws: &Rc<WindowState>) -> *const c_void {
        // Note this increments the reference count.
        let reference = ws.clone();
        Box::into_raw(Box::new(reference)) as *const c_void
    }

    // Get a reference to the window state from an ivar pointer.
    // Note the lifetime here is suspicious: the caller must ensure that the object
    // is not deallocated while the reference is alive.
    pub fn get_state<'a>(ptr: *mut c_void) -> &'a Rc<WindowState> {
        unsafe { (ptr as WindowStatePtr).as_ref().expect("Pointer was null") }
    }

    // Extract the window state from an ivar pointer.
    // After calling this, the pointer is now dangling, and the ivar should be cleared.
    pub fn take_state(ptr: *mut c_void) -> Rc<WindowState> {
        // Note dereferencing a box consumes it, because Rust is confused.
        unsafe { *Box::from_raw(ptr as WindowStatePtr) }
    }
}

// Declarations of functions implemented in ObjC files.
// These signatures must be manually synced - there's no type checking here.
extern "C" {
    fn create_warp_nswindow(
        contentRect: NSRect,
        metalDevice: id,
        hideTitleBar: BOOL,
        backgroundBlurRadiusPixels: u8,
        testMode: BOOL,
    ) -> id;
    fn create_warp_nspanel(
        contentRect: NSRect,
        metalDevice: id,
        hideTitleBar: BOOL,
        backgroundBlurRadiusPixels: u8,
        testMode: BOOL,
    ) -> id;
    fn is_warp_window(window: id) -> BOOL;
    fn get_frontmost_window() -> id;
    fn set_accessibility_contents(
        window: id,
        value: id,    /* NSString */
        help: id,     /* NSString */
        warpRole: id, /* NSString */
        setFrame: BOOL,
        frame: NSRect,
    );

    fn hide_app();
    fn activate_app();
    fn show_window_and_focus_app(window: id, bringToFront: BOOL);
    fn hide_window(window: id);
    fn set_window_alpha(window: id, alpha: f64);
    fn position_and_order_front(window: id);
    fn position_at_given_location(window: id, origin: NSPoint);
    fn order_front_without_focus(window: id, origin: NSPoint);
    fn set_window_title(window: id, title: id);
    fn set_window_bounds(window: id, bound: NSRect);
    fn set_window_background_blur_radius(window: id, blurRadiusPixels: u8);
    fn open_file_path(pathString: id);
    fn open_file_path_in_explorer(pathString: id);
    fn open_file_picker(
        callback: *mut c_void,
        file_types: id,
        allow_files: BOOL,
        allow_folders: BOOL,
        allow_multi_selection: BOOL,
    );
    fn open_save_file_picker(callback: *mut c_void, default_filename: id, default_directory: id);
    fn open_url(urlString: id);
    fn set_titlebar_height(window: id, height: f64);
}

pub type FrameCaptureCallback = Box<dyn FnOnce(platform::CapturedFrame) + Send + 'static>;

pub struct WindowState {
    native_window: id,
    window_id: WindowId,
    callbacks: WindowCallbacks,
    next_scene: RefCell<Option<Rc<Scene>>>,
    device: Option<Device>,
    renderer_manager: Option<Rc<RefCell<RendererManager>>>,
    synthetic_drag_counter: Cell<usize>,
    executor: Rc<executor::Foreground>,
    ime_active: Cell<bool>,
    pub(super) capture_callback: RefCell<Option<FrameCaptureCallback>>,
}

impl Window {
    pub fn open(
        options: WindowOptions,
        window_id: WindowId,
        callbacks: WindowCallbacks,
        executor: Rc<executor::Foreground>,
        renderer_manager: Rc<RefCell<RendererManager>>,
    ) -> Result<Self> {
        log::info!("Opening window with id {window_id}");
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let frame = match options.bounds {
                WindowBounds::ExactPosition(bounds) => RectF::new(
                    transform_origin_from_rect_coord_to_frame_coord(bounds.origin(), bounds.size()),
                    bounds.size(),
                )
                .to_ns_rect(),
                WindowBounds::ExactSize(size) => RectF::new(Vector2F::zero(), size).to_ns_rect(),
                WindowBounds::Default => RectF::new(
                    Vector2F::zero(),
                    Vector2F::new(INITIAL_WINDOW_WIDTH, INITIAL_WINDOW_HEIGHT),
                )
                .to_ns_rect(),
            };

            let test_mode = cfg!(feature = "integration_tests")
                && std::env::var("WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS").is_err();

            let (metal_device, metal_device_ptr) = if test_mode {
                (None, nil)
            } else {
                // TODO: device appears to be leaked here.
                let metal_device = match options.gpu_power_preference {
                    GPUPowerPreference::LowPower => metal::Device::all()
                        .into_iter()
                        .find(is_integrated_gpu)
                        .or_else(metal::Device::system_default),
                    _ => metal::Device::system_default(),
                }
                .ok_or_else(|| anyhow!("could not obtain metal device"))?;
                log::info!(
                    "Using {} GPU for rendering new window.",
                    if is_integrated_gpu(&metal_device) {
                        "integrated"
                    } else {
                        "discrete"
                    }
                );

                let ptr = metal_device.as_ptr() as id;
                (Some(metal_device), ptr)
            };

            let native_window: id = match options.style {
                WindowStyle::Pin => {
                    let panel: id = create_warp_nspanel(
                        frame,
                        metal_device_ptr,
                        options.hide_title_bar as BOOL,
                        options
                            .background_blur_radius_pixels
                            .unwrap_or(DEFAULT_WINDOW_BACKGROUND_BLUR_RADIUS),
                        test_mode as BOOL,
                    );

                    let _: () = msg_send![panel, positionPinnedPanel];
                    panel
                }
                _ => create_warp_nswindow(
                    frame,
                    metal_device_ptr,
                    options.hide_title_bar as BOOL,
                    options
                        .background_blur_radius_pixels
                        .unwrap_or(DEFAULT_WINDOW_BACKGROUND_BLUR_RADIUS),
                    test_mode as BOOL,
                ),
            };
            if native_window == nil {
                return Err(anyhow!("WarpWindow returned nil from initializer"));
            }

            if options.fullscreen_state == FullscreenState::Fullscreen {
                // Instead of directly calling toggleFullScreen, we call a wrapper method that
                // ensures MacOS window animations don't overlap.
                let _: () = msg_send![native_window, enqueueFullscreenTransition];
            }

            let native_view: id = msg_send![native_window, contentView];

            let device = metal_device.map(move |metal_device| {
                Device::new(
                    metal_device,
                    native_view as id,
                    native_window as id,
                    options.gpu_power_preference,
                    options.on_gpu_device_info_reported,
                )
            });

            let window_state = Rc::new(WindowState {
                native_window,
                window_id,
                callbacks,
                next_scene: Default::default(),
                renderer_manager: Some(renderer_manager),
                device,
                synthetic_drag_counter: Cell::new(0),
                executor,
                ime_active: Cell::new(false),
                capture_callback: RefCell::new(None),
            });

            (*native_window).set_ivar(WINDOW_STATE_IVAR, Ivar::from_state(&window_state));
            (*native_view).set_ivar(WINDOW_STATE_IVAR, Ivar::from_state(&window_state));

            let native_window_delegate: id = msg_send![native_window, delegate];
            (*native_window_delegate).set_ivar(WINDOW_STATE_IVAR, Ivar::from_state(&window_state));

            // Set the initial scale properly.
            warp_view_did_change_backing_properties(&*native_view, true);

            match options.style {
                WindowStyle::Normal | WindowStyle::Pin => {
                    match options.bounds {
                        WindowBounds::ExactPosition(_) => {
                            // If specfied, we should set the window to the exact position.
                            // Note that the final position could be different from the set one as the original
                            // frame may no longer exist (e.g. user unplugs the monitor).
                            position_at_given_location(native_window, frame.origin)
                        }
                        WindowBounds::ExactSize(_) | WindowBounds::Default => {
                            // Otherwise we put it in the center of the window or cascade from the previous window.
                            position_and_order_front(native_window)
                        }
                    }
                }
                WindowStyle::Cascade => position_and_order_front(native_window),
                WindowStyle::NotStealFocus => (),
                WindowStyle::PositionedNoFocus => {
                    order_front_without_focus(native_window, frame.origin)
                }
            }

            pool.drain();

            Ok(Self(window_state))
        }
    }

    /// Returns the key window, if any.
    pub fn key_window() -> Option<id> {
        unsafe {
            let native_window: id = msg_send![NSApp(), keyWindow];
            if native_window == nil {
                None
            } else {
                Some(native_window)
            }
        }
    }

    pub fn active_window_id() -> Option<WindowId> {
        unsafe {
            let native_window = Self::key_window()?;
            let is_ours: bool = is_warp_window(native_window) == YES;
            if is_ours {
                Some(get_window_state(&*native_window).window_id)
            } else {
                None
            }
        }
    }

    pub fn frontmost_window_id() -> Option<WindowId> {
        unsafe {
            let native_window: id = get_frontmost_window();
            let is_ours: bool = is_warp_window(native_window) == YES;
            if is_ours {
                Some(get_window_state(&*native_window).window_id)
            } else {
                None
            }
        }
    }

    pub fn key_window_is_modal_panel() -> bool {
        unsafe {
            let Some(native_window) = Self::key_window() else {
                return false;
            };
            let is_modal_panel: BOOL = msg_send![native_window, isModalPanel];
            is_modal_panel == YES
        }
    }

    pub fn close_ime_on_active_window() {
        unsafe {
            let Some(native_window) = Self::key_window() else {
                return;
            };
            let is_ours: bool = is_warp_window(native_window) == YES;
            if is_ours {
                Self::send_close_ime_msg(&*native_window);
            }
        }
    }

    fn send_close_ime_msg(native_window: &Object) {
        unsafe {
            let view = get_window_state(native_window).native_window.contentView();
            let _: () = msg_send![view, closeIMEAsync];
        }
    }

    pub fn close_ime_async(window_id: WindowId) {
        unsafe {
            if let Some(native_window) = Self::find_window_with_id(window_id) {
                Self::send_close_ime_msg(&*native_window);
            }
        }
    }

    pub fn open_url(url: &str) {
        unsafe {
            open_url(make_nsstring(url));
        }
    }

    pub fn open_file_path(path: &Path) {
        unsafe {
            if let Some(path_string) = path.to_str() {
                open_file_path(make_nsstring(path_string));
            }
        }
    }

    pub fn open_file_path_in_explorer(path: &Path) {
        unsafe {
            if let Some(path_string) = path.to_str() {
                open_file_path_in_explorer(make_nsstring(path_string));
            }
        }
    }

    pub fn open_file_picker(callback: FilePickerCallback, config: FilePickerConfiguration) {
        let file_types = config
            .file_types()
            .iter()
            .map(|file_type| make_nsstring(file_type.to_string()))
            .collect_vec();
        unsafe {
            open_file_picker(
                Box::into_raw(Box::new(callback)) as *mut c_void,
                NSArray::arrayWithObjects(nil, &file_types),
                config.allows_files() as BOOL,
                config.allows_folder() as BOOL,
                config.allows_multi_select() as BOOL,
            );
        }
    }

    pub fn open_save_file_picker(
        callback: file_picker::SaveFilePickerCallback,
        config: file_picker::SaveFilePickerConfiguration,
    ) {
        let default_directory = config
            .default_directory
            .map(|path| make_nsstring(path.display().to_string()))
            .unwrap_or_else(|| make_nsstring(""));
        let default_filename = config
            .default_filename
            .map(make_nsstring)
            .unwrap_or_else(|| make_nsstring(""));
        unsafe {
            open_save_file_picker(
                Box::into_raw(Box::new(callback)) as *mut c_void,
                default_filename,
                default_directory,
            );
        }
    }

    pub fn is_ime_open() -> bool {
        unsafe {
            let Some(native_window) = Self::key_window() else {
                return false;
            };
            let is_ours: bool = is_warp_window(native_window) == YES;
            if is_ours {
                get_window_state(&*native_window).ime_active.get()
            } else {
                false
            }
        }
    }

    pub fn set_accessibility_contents(content: AccessibilityContent) {
        unsafe {
            let Some(native_window) = Self::key_window() else {
                return;
            };
            if is_warp_window(native_window) == YES {
                let frame = if let Some(frame) = content.frame {
                    RectF::new(
                        transform_origin_from_rect_coord_to_frame_coord(
                            frame.origin(),
                            frame.size(),
                        ),
                        frame.size(),
                    )
                    .to_ns_rect()
                } else {
                    RectF::default().to_ns_rect()
                };
                // Wrap in a local autorelease pool: under VoiceOver this is invoked
                // from `AppContext::handle_action` on every user action, so it's a hot
                // path. The ObjC `set_accessibility_contents` callee retains the strings,
                // so draining after the call safely bounds peak memory.
                let pool = NSAutoreleasePool::new(nil);
                set_accessibility_contents(
                    native_window,
                    make_nsstring(&content.value),
                    make_nsstring(content.help.unwrap_or_default()),
                    make_nsstring(content.role.to_string()),
                    content.frame.is_some() as BOOL,
                    frame,
                );
                pool.drain();
            }
        }
    }

    /// Sets the background blur radius for all active windows to `blur_radius_pixels`.
    pub fn set_all_windows_background_blur_radius(blur_radius_pixels: u8) {
        unsafe {
            let windows: id = msg_send![NSApp(), windows];
            for i in 0..windows.count() {
                let window: id = windows.objectAtIndex(i);
                if is_warp_window(window) == YES {
                    set_window_background_blur_radius(window, blur_radius_pixels)
                }
            }
        }
    }

    pub fn show_window_and_focus_app(window_id: WindowId, bring_to_front: bool) {
        unsafe {
            if let Some(window) = Self::find_window_with_id(window_id) {
                show_window_and_focus_app(window, bring_to_front as BOOL);
            }
        }
    }

    pub fn hide_window(window_id: WindowId) {
        unsafe {
            if let Some(window) = Self::find_window_with_id(window_id) {
                hide_window(window)
            }
        }
    }

    pub fn set_window_alpha(window_id: WindowId, alpha: f32) {
        unsafe {
            if let Some(window) = Self::find_window_with_id(window_id) {
                set_window_alpha(window, alpha as f64)
            }
        }
    }

    /// Returns a reference to a `WarpWindow` identified by `window_id`, if any.
    ///
    /// # Safety
    /// This code is unsafe since it requires interfacing with platform code.
    unsafe fn find_window_with_id(window_id: WindowId) -> Option<id> {
        let windows: id = msg_send![NSApp(), windows];
        (0..windows.count())
            .find(|i| {
                let window: id = windows.objectAtIndex(*i);
                let is_ours: bool = is_warp_window(window) == YES;

                is_ours && get_window_state(&*window).window_id == window_id
            })
            .map(|idx| windows.objectAtIndex(idx))
    }

    pub fn close_window_async(window_id: WindowId, termination_mode: TerminationMode) {
        let force_terminate = match termination_mode {
            TerminationMode::Cancellable => NO,
            TerminationMode::ForceTerminate | TerminationMode::ContentTransferred => YES,
        };
        unsafe {
            if let Some(window) = Self::find_window_with_id(window_id) {
                let _: id = msg_send![window, closeWindowAsync: force_terminate];
            }
        }
    }

    pub fn set_window_bounds(window_id: WindowId, bound: RectF) {
        unsafe {
            if let Some(window) = Self::find_window_with_id(window_id) {
                set_window_bounds(
                    window,
                    RectF::new(
                        // Transform the bound from the UI internal coordinate system
                        // to Cocoa's coordinate system.
                        transform_origin_from_rect_coord_to_frame_coord(
                            bound.origin(),
                            bound.size(),
                        ),
                        bound.size(),
                    )
                    .to_ns_rect(),
                );
            }
        }
    }

    pub fn set_window_title(window_id: WindowId, title: &str) {
        unsafe {
            if let Some(window) = Self::find_window_with_id(window_id) {
                set_window_title(window, make_nsstring(title));
            }
        }
    }
}

impl platform::Window for Window {
    fn minimize(&self) {
        let native_window = self.0.native_window;
        unsafe {
            let _: () = msg_send![native_window, miniaturize: nil];
        }
    }

    fn fullscreen_state(&self) -> FullscreenState {
        let native_window = self.0.native_window;
        let zoomed: BOOL = unsafe { msg_send![native_window, isZoomed] };
        if unsafe {
            NSWindow::styleMask(native_window).contains(NSWindowStyleMask::NSFullScreenWindowMask)
        } {
            FullscreenState::Fullscreen
        } else if zoomed == YES {
            FullscreenState::Maximized
        } else {
            FullscreenState::Normal
        }
    }

    fn toggle_fullscreen(&self) {
        let native_window = self.0.native_window;
        unsafe {
            let _: () = msg_send![native_window, enqueueFullscreenTransition];
        }
    }

    fn toggle_maximized(&self) {
        let native_window = self.0.native_window;
        unsafe {
            let _: () = msg_send![native_window, zoomAsync: nil];
        }
    }

    fn as_ctx(&self) -> &dyn platform::WindowContext {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn callbacks(&self) -> &WindowCallbacks {
        &self.0.callbacks
    }

    fn supports_transparency(&self) -> bool {
        true
    }

    fn graphics_backend(&self) -> GraphicsBackend {
        GraphicsBackend::Metal
    }

    fn supported_backends(&self) -> Vec<GraphicsBackend> {
        vec![GraphicsBackend::Metal]
    }

    /// We never use the MacOS native window frame.
    fn uses_native_window_decorations(&self) -> bool {
        false
    }

    fn set_titlebar_height(&self, height: f64) {
        self.0.set_titlebar_height(height);
    }
}

impl platform::WindowContext for Window {
    fn size(&self) -> Vector2F {
        self.0.logical_size()
    }

    fn origin(&self) -> Vector2F {
        self.0.origin()
    }

    fn backing_scale_factor(&self) -> f32 {
        self.0.backing_scale_factor() as f32
    }

    fn max_texture_dimension_2d(&self) -> Option<u32> {
        self.0.max_texture_dimension_2d()
    }

    fn render_scene(&self, scene: Rc<Scene>) {
        self.0.render_scene(scene)
    }

    fn request_redraw(&self) {
        self.0.request_redraw();
    }

    fn request_frame_capture(
        &self,
        callback: Box<dyn FnOnce(platform::CapturedFrame) + Send + 'static>,
    ) {
        self.0.request_frame_capture(callback);
    }
}

impl WindowState {
    pub fn id(&self) -> WindowId {
        self.window_id
    }

    /// Returns the logical (resolution-agnostic) size of the current window.
    pub fn logical_size(&self) -> Vector2F {
        let view_frame = unsafe { NSView::frame(self.native_window.contentView()) };
        vec2f(view_frame.size.width as f32, view_frame.size.height as f32)
    }

    /// Returns the physical (resolution-aware) size of the current window.
    pub fn physical_size(&self) -> Vector2F {
        self.logical_size() * self.backing_scale_factor() as f32
    }

    pub fn backing_scale_factor(&self) -> f64 {
        unsafe { NSWindow::backingScaleFactor(self.native_window) }
    }

    fn next_synthetic_drag_id(&self) -> usize {
        let next_id = self.synthetic_drag_counter.get() + 1;
        self.synthetic_drag_counter.set(next_id);
        next_id
    }

    /// Attempts to resize the renderer with the new physical size of the window. Noops if there is
    /// no device or the renderer manager is unset.
    fn resize_renderer(&self) {
        if let Some((renderer_manager, device)) = self.renderer_manager.as_ref().zip(self.device())
        {
            let mut renderer_manager = renderer_manager.borrow_mut();
            let renderer = renderer_manager.renderer_for_device(device, self.physical_size());

            renderer.resize(self);
        }
    }

    /// Returns an `id` to the current `NSView` of the window.
    pub(super) fn native_view(&self) -> id {
        unsafe { msg_send![self.native_window, contentView] }
    }

    /// Returns the current [`Device`] for rendering. `None` if the window was configured with no
    /// display.
    pub fn device(&self) -> Option<&Device> {
        self.device.as_ref()
    }

    fn has_window_buttons(&self) -> bool {
        unsafe {
            // Use the close button as a proxy, since we modify all standard buttons together.
            let button = NSWindow::standardWindowButton_(
                self.native_window,
                NSWindowButton::NSWindowCloseButton,
            );
            let is_hidden: BOOL = msg_send![button, isHidden];
            is_hidden == NO
        }
    }

    fn set_window_buttons(&self, show_window_buttons: bool) {
        let hide_buttons = !show_window_buttons as BOOL;
        unsafe {
            let close_button = NSWindow::standardWindowButton_(
                self.native_window,
                NSWindowButton::NSWindowCloseButton,
            );
            let _: () = msg_send![close_button, setHidden: hide_buttons];
            let miniaturize_button = NSWindow::standardWindowButton_(
                self.native_window,
                NSWindowButton::NSWindowMiniaturizeButton,
            );
            let _: () = msg_send![miniaturize_button, setHidden: hide_buttons];
            let zoom_button = NSWindow::standardWindowButton_(
                self.native_window,
                NSWindowButton::NSWindowZoomButton,
            );
            let _: () = msg_send![zoom_button, setHidden: hide_buttons];
        }
    }

    fn set_titlebar_height(&self, height: f64) {
        unsafe {
            set_titlebar_height(self.native_window, height);
        }
    }
}

impl platform::WindowContext for WindowState {
    fn size(&self) -> Vector2F {
        let view_frame = unsafe { NSView::frame(self.native_window.contentView()) };
        vec2f(view_frame.size.width as f32, view_frame.size.height as f32)
    }

    fn origin(&self) -> Vector2F {
        let view_frame = unsafe { NSWindow::frame(self.native_window) };
        transform_origin_from_frame_coord_to_rect_coord(
            vec2f(view_frame.origin.x as f32, view_frame.origin.y as f32),
            vec2f(view_frame.size.width as f32, view_frame.size.height as f32),
        )
    }

    fn backing_scale_factor(&self) -> f32 {
        unsafe { NSWindow::backingScaleFactor(self.native_window) as f32 }
    }

    fn max_texture_dimension_2d(&self) -> Option<u32> {
        // https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf
        Some(8192)
    }

    fn render_scene(&self, scene: Rc<Scene>) {
        *self.next_scene.borrow_mut() = Some(scene);
        unsafe {
            let _: () = msg_send![self.native_window, setNeedsDisplayAsync];
        }
    }

    fn request_redraw(&self) {
        let _ = self.next_scene.borrow_mut().take();
        unsafe {
            let _: () = msg_send![self.native_window, setNeedsDisplayAsync];
        }
    }

    fn request_frame_capture(
        &self,
        callback: Box<dyn FnOnce(platform::CapturedFrame) + Send + 'static>,
    ) {
        *self.capture_callback.borrow_mut() = Some(callback);
        unsafe {
            let _: () = msg_send![self.native_window, setNeedsDisplayAsync];
        }
    }
}

/// An extension trait defining additional window-management options when running on macOS.
pub trait WindowExt {
    /// Returns whether or not the native macOS window buttons are visible.
    fn has_window_buttons(&self) -> bool;

    /// Sets whether or not to show the native macOS window buttons (traffic lights).
    fn set_window_buttons(&self, window_buttons: bool);
}

/// Utility for interacting with the native [`Window`] implementation. The native window is always
/// available if the app is running, but not in unit tests.
fn native_window(window: &dyn platform::Window) -> Option<&Window> {
    let native_window = window.as_any().downcast_ref::<Window>();
    if native_window.is_none() {
        let is_headless_window = window.as_any().is::<crate::platform::headless::Window>();
        let is_test_window = cfg!(any(test, feature = "test-util"));
        assert!(
            is_headless_window || is_test_window,
            "Should not fail to downcast the platform window to its concrete type"
        );
    }
    native_window
}

impl WindowExt for &dyn platform::Window {
    fn has_window_buttons(&self) -> bool {
        native_window(*self).is_none_or(|window| window.0.has_window_buttons())
    }

    fn set_window_buttons(&self, window_buttons: bool) {
        if let Some(window) = native_window(*self) {
            window.0.set_window_buttons(window_buttons)
        }
    }
}

#[no_mangle]
extern "C-unwind" fn warp_view_did_change_backing_properties(this: &Object, async_callback: bool) {
    let window;
    unsafe {
        window = get_window_state(this);
        let layer: id = msg_send![this, layer];
        let _: () = msg_send![layer, setContentsScale: window.backing_scale_factor()];
        let size = window.logical_size();

        let scale_factor = window.backing_scale_factor();

        if !size.is_zero() {
            // Manually convert the size into the drawable size by multiplying by the scale factor. For
            // some reason using `convertSizeToBacking` incorrectly upscales the drawable size in some
            // cases even when the backing scale factor is 1.0.
            let drawable_size: NSSize = NSSize::new(
                size.x() as f64 * scale_factor,
                size.y() as f64 * scale_factor,
            );
            let _: () = msg_send![layer, setDrawableSize: drawable_size];
        }
    }

    window.resize_renderer();

    if async_callback {
        // Dispatch the callback asynchronously if async_callback is true.
        let weak_window_state = Rc::downgrade(window);
        window
            .executor
            .spawn(async move {
                if let Some(window_state) = weak_window_state.upgrade() {
                    app::callback_dispatcher()
                        .for_window(&Window(window_state.clone()))
                        .window_resized(window_state.as_ref());
                }
            })
            .detach();
    } else {
        app::callback_dispatcher()
            .for_window(&Window(window.clone()))
            .window_resized(window.as_ref());
    }
}

#[no_mangle]
pub extern "C-unwind" fn warp_get_accessibility_contents(object: &mut Object) -> id {
    let state = unsafe { get_window_state(object) };
    let window_id = state.window_id;
    let accessibility_data = app::callback_dispatcher()
        .with_mutable_app_context(|app| app.focused_view_accessibility_data(window_id));

    let accessibility_contents = accessibility_data
        .map(|data| data.content)
        .unwrap_or_default();
    make_nsstring(accessibility_contents.as_str())
}

#[no_mangle]
pub extern "C-unwind" fn warp_ime_position(object: &mut Object, content_rect: NSRect) -> NSRect {
    let state = unsafe { get_window_state(object) };

    let cursor_info = app::callback_dispatcher()
        .for_window(&Window(state.clone()))
        .get_active_cursor_position();

    let size = Vector2F::new(
        content_rect.size.width as f32,
        content_rect.size.height as f32,
    );

    NSRect {
        origin: match cursor_info {
            Some(cursor_info) => NSPoint {
                x: content_rect.origin.x + cursor_info.position.origin_x() as f64,
                y: content_rect.origin.y + (size.y() - cursor_info.position.origin_y()) as f64
                    - (1.2 * cursor_info.font_size) as f64,
            },
            None => NSPoint {
                x: content_rect.origin.x,
                y: content_rect.origin.y + size.y() as f64,
            },
        },
        size: NSSize::new(0., 0.),
    }
}

#[no_mangle]
extern "C-unwind" fn warp_view_set_frame_size(this: &Object, size: NSSize, async_callback: bool) {
    let window;
    unsafe {
        window = get_window_state(this);
        // Manually convert the size into the drawable size by multiplying by the scale factor. For
        // some reason using `convertSizeToBacking` incorrectly upscales the drawable size in some
        // cases even when the backing scale factor is 1.0.
        let scale_factor = window.backing_scale_factor();
        let drawable_size: NSSize = NSSize {
            width: size.width * scale_factor,
            height: size.height * scale_factor,
        };
        let layer: id = msg_send![this, layer];
        let _: () = msg_send![layer, setDrawableSize: drawable_size];
    }

    window.resize_renderer();

    if async_callback {
        // Dispatch the callback asynchronously if async_callback is true.
        let weak_window_state = Rc::downgrade(window);
        window
            .executor
            .spawn(async move {
                if let Some(window_state) = weak_window_state.upgrade() {
                    app::callback_dispatcher()
                        .for_window(&Window(window_state.clone()))
                        .window_resized(window_state.as_ref());
                }
            })
            .detach();
    } else {
        app::callback_dispatcher()
            .for_window(&Window(window.clone()))
            .window_resized(window.as_ref());
    }
}

#[no_mangle]
extern "C-unwind" fn warp_update_layer(this: &Object) {
    if !app::callback_dispatcher().can_borrow_mut() {
        #[cfg(debug_assertions)]
        log::warn!(
            "Tried to update window's backing CAMetalDrawable but app was already mutably borrowed!\nStack trace:\n{:#}",
            std::backtrace::Backtrace::force_capture()
        );
        return;
    }

    unsafe {
        let window = get_window_state(this);

        let scene = {
            if window.next_scene.borrow().is_none() {
                // Do this without holding a mutable borrow on
                // window.next_scene, to ensure that we don't hit BorrowMut
                // errors if `build_scene()` ends up invoking `request_redraw`.
                let scene = app::callback_dispatcher()
                    .for_window(&Window(window.clone()))
                    .build_scene(window.as_ref());
                *window.next_scene.borrow_mut() = Some(scene);
            }

            let Some(scene) = window.next_scene.borrow().clone() else {
                return;
            };

            scene
        };
        debug_assert!(
            window.next_scene.try_borrow_mut().is_ok(),
            "Should not be holding a borrow of the scene RefCell before beginning to render."
        );

        // SAFETY: warp_update_layer should only be invoked for windows
        // created via Window::open(), which always sets a non-None device.
        let device = window
            .device
            .as_ref()
            .expect("warp_update_layer should not be called for a window that has no real display");
        // SAFETY: warp_update_layer is only invoked by the event loop,
        // which should never attempt to draw a window while it is already
        // being drawn.
        let mut renderer_manager = window
            .renderer_manager
            .as_ref()
            .expect("warp_update_layer should never be called twice in parallel")
            .borrow_mut();
        let renderer = renderer_manager.renderer_for_device(device, window.physical_size());

        app::callback_dispatcher().with_mutable_app_context(|ctx| {
            renderer.render(&scene, window.as_ref(), ctx.font_cache());
        });

        app::callback_dispatcher()
            .for_window(&Window(window.clone()))
            .frame_drawn();
    }
}

/// Returns whether this event was handled.
#[no_mangle]
extern "C-unwind" fn warp_handle_view_event(
    this: &Object,
    native_event: id,
    composing_state: bool,
) -> bool {
    let window = unsafe { get_window_state(this) };
    let event = unsafe {
        super::event::from_native(
            native_event,
            Some(window.logical_size().y()),
            false, /* is_first_mouse */
        )
    };
    if let Some(mut event) = event {
        match event {
            Event::LeftMouseDragged {
                position,
                modifiers,
                ..
            } => schedule_synthetic_drag(window, position, modifiers),
            Event::LeftMouseUp { .. } => {
                window.next_synthetic_drag_id();
            }
            Event::KeyDown {
                ref mut is_composing,
                ..
            } => {
                *is_composing = composing_state;
            }
            _ => {}
        }

        return app::callback_dispatcher()
            .for_window(&Window(window.clone()))
            .dispatch_event(event)
            .handled;
    }

    false
}

/// Handles the "first mouse event" - the first mouse event fired that causes an unfocused window to
/// gain focus.
/// Returns whether this event was handled.
#[no_mangle]
extern "C-unwind" fn warp_handle_first_mouse_event(this: &Object, native_event: id) -> bool {
    let window = unsafe { get_window_state(this) };
    let event =
        unsafe { super::event::from_native(native_event, Some(window.logical_size().y()), true) };
    if let Some(event) = event {
        return app::callback_dispatcher()
            .for_window(&Window(window.clone()))
            .dispatch_event(event)
            .handled;
    }
    false
}

#[no_mangle]
extern "C-unwind" fn warp_handle_insert_text(this: &Object, characters: id) {
    let string = unsafe { to_string(characters) };
    let window = unsafe { get_window_state(this) };
    app::callback_dispatcher()
        .for_window(&Window(window.clone()))
        .dispatch_event(Event::TypedCharacters { chars: string });
}

#[no_mangle]
extern "C-unwind" fn warp_handle_drag_and_drop(this: &Object, paths: id, point: NSPoint) {
    let paths = unsafe {
        (0..paths.count())
            .map(|i| {
                let directory = paths.objectAtIndex(i);
                to_string(directory)
            })
            .collect::<Vec<_>>()
    };

    let window = unsafe { get_window_state(this) };
    let location = vec2f(point.x as f32, window.logical_size().y() - point.y as f32);
    app::callback_dispatcher()
        .for_window(&Window(window.clone()))
        .dispatch_event(Event::DragAndDropFiles { paths, location });
}

#[no_mangle]
extern "C-unwind" fn warp_handle_file_drag(this: &Object, point: NSPoint) {
    let window = unsafe { get_window_state(this) };
    let location = vec2f(point.x as f32, window.logical_size().y() - point.y as f32);

    app::callback_dispatcher()
        .for_window(&Window(window.clone()))
        .dispatch_event(Event::DragFiles { location });
}

#[no_mangle]
extern "C-unwind" fn warp_handle_file_drag_exit(this: &Object) {
    let window = unsafe { get_window_state(this) };

    app::callback_dispatcher()
        .for_window(&Window(window.clone()))
        .dispatch_event(Event::DragFileExit);
}

#[no_mangle]
extern "C-unwind" fn warp_update_ime_state(this: &mut Object, ime_active: bool) {
    let state = unsafe { get_window_state(this) };
    state.ime_active.set(ime_active);
}

/// Converts an NSRange to a Rust Range<usize>
/// NSRange has location (start) and length, while Rust Range has start and end
fn nsrange_to_rust_range(ns_range: NSRange) -> std::ops::Range<usize> {
    let start = ns_range.location as usize;
    let end = start + ns_range.length as usize;
    start..end
}

#[no_mangle]
extern "C-unwind" fn warp_marked_text_updated(
    this: &mut Object,
    marked_text: id,
    selected_range: NSRange,
) {
    let state = unsafe { get_window_state(this) };
    let marked_text = unsafe { to_string(marked_text) };
    let selected_range = nsrange_to_rust_range(selected_range);
    app::callback_dispatcher()
        .for_window(&Window(state.clone()))
        .dispatch_event(Event::SetMarkedText {
            marked_text,
            selected_range,
        });
}

#[no_mangle]
extern "C-unwind" fn warp_marked_text_cleared(this: &mut Object) {
    let state = unsafe { get_window_state(&*this) };
    app::callback_dispatcher()
        .for_window(&Window(state.clone()))
        .dispatch_event(Event::ClearMarkedText);
}

#[no_mangle]
pub extern "C-unwind" fn warp_dispatch_standard_action(this: id, tag: NSInteger) {
    if let Some(action) = StandardAction::from_isize(tag as isize) {
        let state = unsafe { get_window_state(&*this) };
        app::callback_dispatcher()
            .for_window(&Window(state.clone()))
            .dispatch_standard_action(action);
    }
}

#[no_mangle]
pub extern "C-unwind" fn warp_app_window_moved(this: id, rect: NSRect) {
    let state = unsafe { get_window_state(&*this) };
    let point = Vector2F::new(rect.origin.x as f32, rect.origin.y as f32);
    let size = Vector2F::new(rect.size.width as f32, rect.size.height as f32);

    let weak_window_state = Rc::downgrade(state);
    state
        .executor
        .spawn(async move {
            if let Some(window) = weak_window_state.upgrade() {
                app::callback_dispatcher()
                    .for_window(&Window(window))
                    .window_moved(RectF::new(
                        transform_origin_from_frame_coord_to_rect_coord(point, size),
                        size,
                    ));
            }
        })
        .detach();
}

/// Removes the WindowState ivar from an Objective-C object, nulls out the ivar
/// pointer within the object, and returns the reference-counted pointer to the
/// state.
unsafe fn remove_state_ivar_from_object(object: &mut Object) -> Rc<WindowState> {
    let wrapper_ptr: *mut c_void = *object.get_ivar(WINDOW_STATE_IVAR);
    let state = Ivar::take_state(wrapper_ptr);
    object.set_ivar(WINDOW_STATE_IVAR, ptr::null::<c_void>());
    state
}

// dealloc is called by AppKit when our NSWindow subclass is deallocating,
// because its retain count has dropped to zero. This is our chance to release
// our Rust resources. Do not call this manually.
#[no_mangle]
pub extern "C-unwind" fn warp_dealloc_window(native_window: &mut Object) {
    log::info!("dealloc native window {native_window:p}");
    let state;
    unsafe {
        // Remove the window state from the content NSView and drop a reference.
        let view_id: id = msg_send![native_window, contentView];
        let native_view = &mut (*view_id);
        let _ = remove_state_ivar_from_object(native_view);

        // Remove the window state from the NSWindowDelegate and drop a reference.
        let delegate_id: id = msg_send![native_window, delegate];
        let native_window_delegate = &mut (*delegate_id);
        let _ = remove_state_ivar_from_object(native_window_delegate);

        // Remove the window state from the NSWindow.
        state = remove_state_ivar_from_object(native_window);
    }

    // Drop the final reference to the `WindowState`, which actually drops the
    // underlying object and frees the memory.
    drop(state);
}

pub unsafe fn get_window_state(object: &Object) -> &Rc<WindowState> {
    let wrapper_ptr: *mut c_void = *object.get_ivar(WINDOW_STATE_IVAR);
    Ivar::get_state(wrapper_ptr)
}

fn schedule_synthetic_drag(
    window_state: &Rc<WindowState>,
    position: Vector2F,
    modifiers: ModifiersState,
) {
    let drag_id = window_state.next_synthetic_drag_id();
    let weak_window_state = Rc::downgrade(window_state);
    let instant = Instant::now() + Duration::from_millis(16);
    window_state
        .executor
        .spawn(async move {
            Timer::at(instant).await;
            if let Some(window_state) = weak_window_state.upgrade() {
                if window_state.synthetic_drag_counter.get() == drag_id {
                    schedule_synthetic_drag(&window_state, position, modifiers);
                    app::callback_dispatcher()
                        .for_window(&Window(window_state))
                        .dispatch_event(Event::LeftMouseDragged {
                            position,
                            modifiers,
                        });
                }
            }
        })
        .detach();
}

// We need this transformation as Cocoa follows the Cartesian coordinate system with rect's
// origin on the lower left corner and positive value extending along the y coordinate
// up, whereas RectF follows the flipped coordinate system with rect's origin on the upper left
// corner and positive value extending along the y coordinate down.
// https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/CocoaDrawingGuide/Transforms/Transforms.html
//
// For example, a rect in Cocoa will look like:
// (0, 100)          (100, 100)
// -------------------------
// |                       |
// |                       |
// |                       |
// |                       |
// |                       |
// -------------------------
// Origin: (0, 0)     (100, 0)
//
// Whereas the same rect represented in RectF should look like:
// Origin: (0, -100)  (100, -100)
// -------------------------
// |                       |
// |                       |
// |                       |
// |                       |
// |                       |
// -------------------------
// (0, 0)             (100, 0)
//
// Note that the y_axis is flipped in RectF.
//
// To transform a Cocoa rect into RectF, we need the following two steps:
// 1. Get the upper left corner coordinate of the rect:
// * upper_left = (cocoa_origin.x(), cocoa_origin.y() + size.y())
// 2. Flip the y coordinate:
// * new_origin = (upper_left.x(), -upper_left.y())
//
// In the reverse transformation, we also need to follow two steps:
// 1. Get the lower left corner coordinate of the rect:
// * lower_left = (rectf_origin.x(), rectf_origin.y() + size.y())
// 2. Flip the y coordinate:
// * new_origin = (lower_left.x(), -lower_left.y())
pub fn transform_origin_from_frame_coord_to_rect_coord(
    origin: Vector2F,
    size: Vector2F,
) -> Vector2F {
    Vector2F::new(origin.x(), -(origin.y() + size.y()))
}

fn transform_origin_from_rect_coord_to_frame_coord(origin: Vector2F, size: Vector2F) -> Vector2F {
    Vector2F::new(origin.x(), -(origin.y() + size.y()))
}

/// Converts an Objective-C `Object` into a `String`
unsafe fn to_string(value: *mut Object) -> String {
    let slice = slice::from_raw_parts(value.UTF8String() as *const c_uchar, value.len());
    str::from_utf8_unchecked(slice).to_string()
}
