pub mod app;
pub mod file_picker;
pub mod keyboard;
pub mod menu;

pub mod test;
#[cfg(target_family = "wasm")]
pub mod wasm;

pub use app::AppCallbacks;
use derivative::Derivative;
pub use file_picker::{
    FilePickerCallback, FilePickerConfiguration, FileType, SaveFilePickerCallback,
    SaveFilePickerConfiguration,
};
use serde::{Deserialize, Serialize};
use warp_util::path::ShellFamily;

use crate::fonts::SubpixelAlignment;
use crate::keymap::Keystroke;
use crate::modals::{AlertDialog, ModalId};
use crate::notification::{NotificationSendError, RequestPermissionsOutcome};

use crate::rendering::{GPUPowerPreference, OnGPUDeviceSelected};
use crate::text_layout::{ClipConfig, StyleAndFont, TextAlignment, TextFrame};
use crate::{
    accessibility::AccessibilityContent,
    fonts::{
        canvas::RasterFormat, FamilyId, FontId, GlyphId, Metrics, Properties, RasterizedGlyph,
    },
    notification::UserNotification,
    text_layout::Line,
    windowing::WindowCallbacks,
    Scene, WindowId,
};
use crate::{
    geometry, rendering, AppContext, ApplicationBundleInfo, Clipboard, DisplayId, DisplayIdx,
    OptionalPlatformWindow,
};
use anyhow::Result;
use async_task::Runnable;
use lazy_static::lazy_static;
use pathfinder_geometry::vector::Vector2I;
use pathfinder_geometry::{
    rect::{RectF, RectI},
    vector::Vector2F,
};
use std::any::Any;
use std::collections::HashSet;
use std::path::Path;
use std::{ops::Range, rc::Rc, sync::Arc};

#[cfg(not(target_family = "wasm"))]
lazy_static! {
    pub static ref KEYS_TO_IGNORE: HashSet<Keystroke> = HashSet::new();
}
#[cfg(target_family = "wasm")]
lazy_static! {
    pub static ref KEYS_TO_IGNORE: HashSet<Keystroke> =
        HashSet::from([Keystroke::parse("cmdorctrl-v").unwrap()]);
}

/// Type of the callback function that provides the result of requesting
/// desktop notification permissions.
pub type RequestNotificationPermissionsCallback =
    Box<dyn FnOnce(RequestPermissionsOutcome, &mut AppContext) + Send + Sync>;
/// Type of the callback function invoked when an error occurs while sending
/// a desktop notification.
pub type SendNotificationErrorCallback =
    Box<dyn FnOnce(NotificationSendError, &mut AppContext) + Send + Sync>;

/// The information needed to send a notification.
#[derive(Derivative)]
#[derivative(Debug)]
pub struct NotificationInfo {
    pub notification_content: UserNotification,
    #[derivative(Debug = "ignore")]
    pub on_error: SendNotificationErrorCallback,
}

// TODO(advait): revisit this to check if there's a better approach.
#[derive(Copy, Clone)]
pub struct LineStyle {
    pub font_size: f32,
    pub line_height_ratio: f32,
    pub baseline_ratio: f32,
    /// Size of tab stops in spaces for fully fixed-width (monospace) text.
    ///
    /// `Some(n)` means `\t` advances to the next stop every `n` spaces. This is intended only for
    /// paragraphs where all runs share the same fixed-width font metrics.
    ///
    /// `None` leaves tab stop behavior up to the backend defaults.
    pub fixed_width_tab_size: Option<u8>,
}

pub struct WindowOptions {
    pub bounds: WindowBounds,
    pub fullscreen_state: FullscreenState,
    pub hide_title_bar: bool,
    pub title: Option<String>,
    pub style: WindowStyle,
    pub background_blur_radius_pixels: Option<u8>,
    pub background_blur_texture: bool,
    pub gpu_power_preference: GPUPowerPreference,
    pub backend_preference: Option<GraphicsBackend>,
    pub on_gpu_device_info_reported: Box<OnGPUDeviceSelected>,
    /// This is an identifier to distinguish different windows among one application. It is a no-op
    /// on all platforms except X11 Linux.
    /// See docs on the "WM_CLASS" property:
    /// https://www.x.org/docs/ICCCM/icccm.pdf
    pub window_instance: Option<String>,
}

impl std::fmt::Debug for WindowOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowOptions")
            .field("bounds", &self.bounds)
            .field("hide_title_bar", &self.hide_title_bar)
            .field("title", &self.title)
            .field("style", &self.style)
            .field(
                "background_blur_radius_pixels",
                &self.background_blur_radius_pixels,
            )
            .field("background_blur_texture", &self.background_blur_texture)
            .field("gpu_power_preference", &self.gpu_power_preference)
            .field("backend_preference", &self.backend_preference)
            .field("window_instance", &self.window_instance)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum WindowStyle {
    #[default]
    Normal,

    /// If the window does not steal focus, the passed bounds won't be applied and the
    /// the window will be set to default size.
    NotStealFocus,

    /// If a window is pinned, it will be positioned above all other apps and steals focus
    /// by default.
    Pin,

    /// A window that needs to cascade in case of opening a new window with ExactPosition
    Cascade,

    /// Position the window at exact bounds and show it, but don't make it key (no focus steal).
    /// Used for drag preview windows that should appear but not interrupt the drag.
    PositionedNoFocus,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum WindowBounds {
    /// The platform chooses the window size and origin.
    #[default]
    Default,
    /// Use an exact size for the window, but let the platform choose its origin.
    ExactSize(Vector2F),
    /// Use an exact size and origin for the window.
    ExactPosition(RectF),
}

impl WindowBounds {
    pub fn new(bounds: Option<RectF>) -> Self {
        match bounds {
            // Make sure the bounds are valid before passing down to platform call.
            Some(bound) if bound.height() > 0. && bound.width() > 0. => {
                WindowBounds::ExactPosition(bound)
            }
            _ => WindowBounds::Default,
        }
    }

    pub fn bounds(&self) -> Option<RectF> {
        match &self {
            Self::Default => None,
            Self::ExactSize(_) => None,
            Self::ExactPosition(bound) => Some(*bound),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MicrophoneAccessState {
    NotDetermined,
    Denied,
    Restricted,
    Authorized,
}

pub trait Delegate: 'static {
    /// Returns a handle to the platform dispatch delegate.
    fn dispatch_delegate(&self) -> Arc<dyn DispatchDelegate>;

    fn request_user_attention(&self, window_id: WindowId);

    fn clipboard(&mut self) -> &mut dyn Clipboard;

    fn system_theme(&self) -> SystemTheme;

    fn open_url(&self, url: &str);

    /// Opens an absolute file path with native system API.
    fn open_file_path(&self, path: &Path);

    /// Opens an absolute file path in the file explorer with native system API.
    fn open_file_path_in_explorer(&self, path: &Path);

    fn open_file_picker(
        &self,
        callback: FilePickerCallback,
        file_picker_config: FilePickerConfiguration,
    );

    fn open_save_file_picker(
        &self,
        callback: file_picker::SaveFilePickerCallback,
        config: file_picker::SaveFilePickerConfiguration,
    );

    /// Retrieve the absolute path of given application's bundle and its executable.
    fn application_bundle_info(&self, bundle_identifier: &str)
        -> Option<ApplicationBundleInfo<'_>>;

    /// Create a window showing a modal dialog native to the platform. The modal will synchronously
    /// block all other interactions with the app until dismissed. The [`ModalId`] is a handle to
    /// map the modal response to the right callback for the [`AppContext`].
    fn show_native_platform_modal(&self, id: ModalId, modal: AlertDialog);

    /// Requests OS permissions for sending desktop notifications.
    fn request_desktop_notification_permissions(
        &self,
        on_completion: RequestNotificationPermissionsCallback,
    );

    /// Sends a desktop notification.
    fn send_desktop_notification(
        &self,
        notification_content: UserNotification,
        window_id: WindowId,
        on_error: SendNotificationErrorCallback,
    );

    /// Sets the cursor pointer
    fn set_cursor_shape(&self, cursor: Cursor);

    /// Returns the current cursor pointer
    #[cfg(feature = "test-util")]
    fn get_cursor_shape(&self) -> Cursor;
    fn close_ime_async(&self, window_id: WindowId);
    fn is_ime_open(&self) -> bool;

    /// Requests that the system character palette (usually an emoji picker)
    /// be shown.
    fn open_character_palette(&self);

    /// Sets the passed string as the content available for a11y tools (such as screen readers).
    fn set_accessibility_contents(&self, content: AccessibilityContent);

    fn register_global_shortcut(&self, shortcut: Keystroke);
    fn unregister_global_shortcut(&self, shortcut: &Keystroke);

    fn terminate_app(&self, termination_mode: TerminationMode);

    /// Returns whether or not a screen reader is enabled, or None if we do not
    /// know for sure.
    fn is_screen_reader_enabled(&self) -> Option<bool>;

    /// Returns the current microphone access state.
    fn microphone_access_state(&self) -> MicrophoneAccessState;

    /// Returns whether the app is running with a headless rendering backend
    /// (no GUI or visible output).
    fn is_headless(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub enum TerminationMode {
    /// The termination can be interrupted. This is the default, and should be used most
    /// of the time.
    Cancellable,
    /// The termination cannot be interrupted. This can be useful when we have received
    /// confirmation from the user that it is ok to terminate, for example.
    ForceTerminate,
    /// The window's content (tab) has been transferred to another window, so the
    /// now-empty source window should close without any confirmation dialogs.
    ContentTransferred,
}

/// A trait for interacting with the main thread.
#[cfg(not(target_family = "wasm"))]
pub trait DispatchDelegate: 'static + Send + Sync {
    fn is_main_thread(&self) -> bool;
    fn run_on_main_thread(&self, task: Runnable);
}

#[cfg(target_family = "wasm")]
pub trait DispatchDelegate: 'static {
    fn is_main_thread(&self) -> bool;
    fn run_on_main_thread(&self, task: Runnable);
}

/// A marker trait for the types that [`FontDB`] implementations return from
/// [`FontDB::load_all_system_fonts`].
pub trait LoadedSystemFonts: 'static + Any + Send + Sync {
    fn as_any(self: Box<Self>) -> Box<dyn Any>;
}

/// Trait that implements text layout. Implementors must be [`Send`] and
/// [`Sync`] so that text can be laid out in a background thread.
pub trait TextLayoutSystem: 'static + Send + Sync {
    /// Lays out a single line of text.
    fn layout_line(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        clip_config: ClipConfig,
    ) -> Line;

    /// Lays out text into a series of lines that fit within the bounding box
    /// defined by `max_width` and `max_height`.
    #[allow(clippy::too_many_arguments)]
    fn layout_text(
        &self,
        text: &str,
        line_style: LineStyle,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        max_height: f32,
        alignment: TextAlignment,
        first_line_head_indent: Option<f32>,
    ) -> TextFrame;
}

/// A trait for working with fonts.
///
/// This interface provides a platform-agnostic API for loading fonts,
/// retrieving font-related metrics, performing text shaping/layout, and
/// rasterizing glyphs.
///
/// Implementations of this trait can rely on callers to cache returned values
/// where appropriate.
pub trait FontDB: 'static {
    /// Loads a font family from the provided set of font data.
    ///
    /// Each bytestring should be decodable as a single font.
    fn load_from_bytes(&mut self, name: &str, bytes: Vec<Vec<u8>>) -> Result<FamilyId>;

    /// Loads a font from the system by family name.
    #[cfg(not(target_family = "wasm"))]
    fn load_from_system(&mut self, font_family: &str) -> Result<FamilyId>;

    /// Returns a background task that produces the set of data the font DB
    /// needs to make all system fonts available to the application.
    #[cfg(not(target_family = "wasm"))]
    fn load_all_system_fonts(
        &self,
    ) -> futures::future::BoxFuture<'static, Box<dyn LoadedSystemFonts>>;

    /// Processes the data produced by [`FontDB::load_all_system_fonts`],
    /// returning the list of system fonts that can be used by the application.
    #[cfg(not(target_family = "wasm"))]
    fn process_loaded_system_fonts(
        &mut self,
        loaded_system_fonts: Box<dyn LoadedSystemFonts>,
    ) -> Vec<(Option<FamilyId>, crate::fonts::FontInfo)>;

    /// Returns the [`FamilyId`] identified by `name`, or [`None`] if no font
    /// with `name` has been inserted into the cache.
    fn family_id_for_name(&self, name: &str) -> Option<FamilyId>;

    /// Gets the name of a font family by ID.
    fn load_family_name_from_id(&self, id: FamilyId) -> Option<String>;

    /// Determines which font from a family should be used to display text with
    /// the given properties.
    fn select_font(&self, family_id: FamilyId, properties: Properties) -> FontId;

    /// Returns the ordered list of fonts which should be checked when the given
    /// font is lacking a glyph for a character.
    fn fallback_fonts(&self, character: char, font_id: FontId) -> Vec<FontId>;

    /// Returns a set of metrics about the font that aren't glyph-dependent.
    fn font_metrics(&self, font_id: FontId) -> Metrics;

    /// Computes the position of a glyph that occurs after this one, relative to
    /// this glyph.
    ///
    /// The `x` position within the resulting `Vector2F` is the horizontal distance to
    /// increment (or decrement, for RTL text) the position after a glyph has been rendered. It is
    /// always positive for horizontal layouts, and 0 for fonts that only support being
    /// rendered vertically.
    ///
    /// The `y` position within the resulting `Vector2F` is the vertical distance to decrement (or
    /// increment for bottom to top writing) the position after a glyph has been rendered. It is
    /// always positive for vertical layouts, and 0 for fonts that only support being rendered
    /// horizontally.
    fn glyph_advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Vector2I>;

    /// Computes the size of the canvas needed to rasterize the glyph.
    fn glyph_raster_bounds(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        glyph_config: &rendering::GlyphConfig,
    ) -> Result<RectI>;

    /// Computes the bounding box of a glyph with respect to surrounding glyphs.
    fn glyph_typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<RectI>;

    /// Rasterizes a single glyph so it can be rendered to the screen.
    #[allow(clippy::too_many_arguments)]
    fn rasterize_glyph(
        &self,
        font_id: FontId,
        size: f32,
        glyph_id: GlyphId,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &rendering::GlyphConfig,
        format: RasterFormat,
    ) -> Result<RasterizedGlyph>;

    /// Returns the ID of the glyph which represents the given character in the
    /// given font.
    fn glyph_for_char(&self, font_id: FontId, char: char) -> Option<GlyphId>;

    fn text_layout_system(&self) -> &dyn TextLayoutSystem;
}

#[derive(Clone, Copy, Debug, Default, num_derive::FromPrimitive, PartialEq, Eq)]
pub enum FullscreenState {
    #[default]
    Normal = 0,
    Fullscreen = 1,
    Maximized = 2,
}

pub trait Window: 'static + WindowContext + std::any::Any {
    fn minimize(&self);
    fn toggle_maximized(&self);
    fn toggle_fullscreen(&self);
    fn fullscreen_state(&self) -> FullscreenState;
    /// Whether the window has the native OS window frame (title bar and buttons).
    fn uses_native_window_decorations(&self) -> bool;
    fn set_titlebar_height(&self, height: f64);

    /// Whether any hardware supports window transparency
    fn supports_transparency(&self) -> bool;
    fn graphics_backend(&self) -> GraphicsBackend;
    fn supported_backends(&self) -> Vec<GraphicsBackend>;

    fn as_ctx(&self) -> &dyn WindowContext;
    fn callbacks(&self) -> &WindowCallbacks;

    fn as_any(&self) -> &dyn std::any::Any;
}

pub trait WindowContext {
    /// Returns the current inner (content) size of the window, in logical
    /// pixels.
    fn size(&self) -> Vector2F;

    /// Returns the position of the window origin (top-left corner) within the
    /// screen, in logical pixels.
    fn origin(&self) -> Vector2F;

    /// Returns the scale factor for the window surface.
    fn backing_scale_factor(&self) -> f32;

    /// The maximum dimension size in pixels, either width or height, for a 2D-texture. `None`
    /// will be treated as unbounded.
    fn max_texture_dimension_2d(&self) -> Option<u32>;

    /// Provides the window the next scene to render and asks it to schedule a
    /// redraw.
    fn render_scene(&self, scene: Rc<Scene>);

    /// Schedules a redraw of the window.
    fn request_redraw(&self);

    /// Requests a frame capture on the next render.
    ///
    /// When the frame is captured, the provided callback will be invoked with the
    /// captured frame data.
    fn request_frame_capture(&self, callback: Box<dyn FnOnce(CapturedFrame) + Send + 'static>);
}

/// Pixel format of the data in a `CapturedFrame`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturedFrameFormat {
    Rgba,
    Bgra,
}

/// A captured frame containing pixel data in the format indicated by `format`.
#[derive(Clone)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub format: CapturedFrameFormat,
}

impl CapturedFrame {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
            format: CapturedFrameFormat::Rgba,
        }
    }

    pub fn new_bgra(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
            format: CapturedFrameFormat::Bgra,
        }
    }

    pub fn ensure_rgba(&mut self) {
        if self.format == CapturedFrameFormat::Bgra {
            for chunk in self.data.chunks_exact_mut(4) {
                chunk.swap(0, 2);
            }
            self.format = CapturedFrameFormat::Rgba;
        }
    }
}

#[derive(Copy, Clone, Default)]

pub enum WindowFocusBehavior {
    /// Brings the window to the front when focusing the app.
    #[default]
    BringToFront,
    /// Retain the window's current position in the z-index when
    /// focusing the app. May not be supported on all platforms.
    RetainZIndex,
}

/// Common interface for abstracting platform-specific windowing logic.
pub trait WindowManager {
    fn open_window(
        &mut self,
        window_id: WindowId,
        window_options: WindowOptions,
        callbacks: WindowCallbacks,
    ) -> Result<()>;

    /// Returns a platform-independent trait-object for the window with the given ID.
    fn platform_window(&self, window_id: WindowId) -> OptionalPlatformWindow;

    /// Drop a window. Note that other pieces of state pointing to this window ID must also be
    /// removed from [`AppContext`].
    fn remove_window(&mut self, window_id: WindowId);

    /// \return the window ID of the window that is active (has typing focus), or None if none.
    fn active_window_id(&self) -> Option<WindowId>;

    /// \return the active window is an alert modal
    fn key_window_is_modal_panel(&self) -> bool;

    /// \return if the app is currently active.
    fn app_is_active(&self) -> bool;

    /// Makes all the app's windows visible, and transfer focus to whichever window most recently
    /// had focus.
    /// \return the window ID of the window that will become active, which may be None if we are on
    /// a platform that allows the app to run without any open windows.
    fn activate_app(&self, last_active_window: Option<WindowId>) -> Option<WindowId>;
    fn show_window_and_focus_app(&self, window_id: WindowId, behavior: WindowFocusBehavior);
    fn hide_app(&self);
    fn hide_window(&self, window_id: WindowId);
    fn set_window_bounds(&self, window_id: WindowId, bound: RectF);

    /// Sets the per-window opacity, where `1.0` is fully opaque and `0.0` is fully
    /// transparent. Unlike `hide_window`, this leaves the window in the window list
    /// and does not change focus, key, or z-order. Useful for cheaply hiding a
    /// window during a drag without triggering AppKit's `orderOut:` machinery.
    /// Default is a no-op on platforms that don't support per-window alpha.
    fn set_window_alpha(&self, _window_id: WindowId, _alpha: f32) {}

    /// Sets the background blur radius for all windows to the given `blur_radius_pixels` value.
    fn set_all_windows_background_blur_radius(&self, blur_radius_pixels: u8);

    /// [Windows only] Sets the background blur texture (Acrylic) for all windows.
    fn set_all_windows_background_blur_texture(&self, use_blur_texture: bool);

    fn set_window_title(&self, window_id: WindowId, title: &str);

    /// Closes a window asynchronously. This is done asynchronously solely because the UI framework
    /// incorrectly assumes that a call to platform code cannot synchronously trigger a callback
    /// back to the UI framework. For example, closing window will also synchronously trigger a
    /// `window_will_close`, which will crash the app with a BorrowMut error. To avoid this error,
    /// we do this asynchronously.
    fn close_window_async(&self, window_id: WindowId, termination_mode: TerminationMode);

    /// Returns the display bound for the current active display.
    fn active_display_bounds(&self) -> geometry::rect::RectF;
    /// Returns the unique identifier for the current active display.
    fn active_display_id(&self) -> DisplayId;
    fn display_count(&self) -> usize;
    fn bounds_for_display_idx(&self, idx: DisplayIdx) -> Option<RectF>;

    fn active_cursor_position_updated(&self);

    fn windowing_system(&self) -> Option<crate::windowing::System>;

    /// The name of the operating system's window server/manager/compositor.
    fn os_window_manager_name(&self) -> Option<String>;
    /// Whether or not this is a tiling window manager.
    fn is_tiling_window_manager(&self) -> bool;

    /// Returns the IDs of all application windows in front-to-back z-order.
    /// An empty vector indicates that z-ordering information is not available
    /// on this platform.
    fn ordered_window_ids(&self) -> Vec<WindowId> {
        vec![]
    }

    fn cancel_synthetic_drag(&self, _window_id: WindowId) {}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SystemTheme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Arrow,
    IBeam,
    Crosshair,
    OpenHand,
    ClosedHand,
    NotAllowed,
    PointingHand,
    ResizeLeftRight,
    ResizeUpDown,
    /// The drag copy cursor, indicating the currently will result in a copy action.
    DragCopy,
}

/// The current operating system in which this library is running. If on the web, this reads the
/// user agent to determine the backing OS, otherwise this is determined at compile time based on
/// the value of `target_arch` (<https://doc.rust-lang.org/reference/conditional-compilation.html#target_arch>).
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum OperatingSystem {
    /// Any distribution of Linux.
    Linux,
    /// MacOS.
    Mac,
    /// Windows.
    Windows,
    /// The operating system is unknown or not one of the ones specified.
    /// Contains the name of the operating system if it is known.
    Other(Option<&'static str>),
}

impl OperatingSystem {
    pub fn get() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                wasm::current_platform()
            } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
                OperatingSystem::Linux
            } else if #[cfg(target_os = "macos")] {
                OperatingSystem::Mac
            } else if #[cfg(windows)] {
                OperatingSystem::Windows
            } else {
                OperatingSystem::Other(None)
            }
        }
    }

    /// Returns true if the current [`OperatingSystem`] is Mac.
    pub fn is_mac(&self) -> bool {
        *self == OperatingSystem::Mac
    }

    /// Returns true if the current [`OperatingSystem`] is Linux.
    pub fn is_linux(&self) -> bool {
        *self == OperatingSystem::Linux
    }

    /// Returns true if the current [`OperatingSystem`] is Windows.
    pub fn is_windows(&self) -> bool {
        *self == OperatingSystem::Windows
    }

    pub fn default_shell_family(&self) -> ShellFamily {
        match self {
            OperatingSystem::Linux | OperatingSystem::Mac | OperatingSystem::Other(_) => {
                ShellFamily::Posix
            }
            OperatingSystem::Windows => ShellFamily::PowerShell,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "schema_gen", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema_gen",
    schemars(
        description = "Graphics rendering backend used for display output.",
        rename_all = "snake_case"
    )
)]
#[cfg_attr(feature = "settings_value", derive(settings_value::SettingsValue))]
pub enum GraphicsBackend {
    /// This maps to [`wgpu::Backend::Empty`].
    #[cfg_attr(
        feature = "schema_gen",
        schemars(description = "No-op backend for testing.")
    )]
    Empty,
    #[cfg_attr(feature = "schema_gen", schemars(description = "DirectX 12."))]
    Dx12,
    #[cfg_attr(feature = "schema_gen", schemars(description = "Vulkan."))]
    Vulkan,
    #[cfg_attr(feature = "schema_gen", schemars(description = "OpenGL."))]
    Gl,
    #[cfg_attr(feature = "schema_gen", schemars(description = "Metal."))]
    Metal,
    #[cfg_attr(feature = "schema_gen", schemars(description = "WebGPU (browser)."))]
    BrowserWebGpu,
}

impl GraphicsBackend {
    pub fn to_label(&self) -> &'static str {
        match self {
            GraphicsBackend::Empty => "",
            GraphicsBackend::Dx12 => "DirectX 12",
            GraphicsBackend::Vulkan => "Vulkan",
            GraphicsBackend::Gl => "OpenGL",
            GraphicsBackend::Metal => "Metal",
            GraphicsBackend::BrowserWebGpu => "WebGPU",
        }
    }
}
