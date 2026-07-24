use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;
use pathfinder_geometry::rect::{RectF, RectI};
use pathfinder_geometry::vector::{Vector2F, Vector2I, vec2f, vec2i};

use crate::accessibility::AccessibilityContent;
use crate::clipboard::InMemoryClipboard;
use crate::fonts::FamilyId;
use crate::keymap::Keystroke;
use crate::modals::{AlertDialog, ModalId};
use crate::notification::UserNotification;
use crate::platform::file_picker::{FilePickerCallback, FilePickerConfiguration};
use crate::platform::{
    self, Cursor, MicrophoneAccessState, RequestNotificationPermissionsCallback,
    SendNotificationErrorCallback, TerminationMode, TextLayoutSystem, WindowFocusBehavior,
    WindowOptions,
};
use crate::text_layout::TextAlignment;
use crate::windowing::WindowCallbacks;
use crate::{
    ApplicationBundleInfo, DisplayId, DisplayIdx, OptionalPlatformWindow, Scene, WindowId, geometry,
};

pub struct AppDelegate {
    clipboard: InMemoryClipboard,
    cursor_shape: Mutex<Cursor>,
}

// Dummy IntegrationTestDelegate implementation so the integration test code
// builds on non-mac platforms (even though running them there is a no-op for now).
// This is relevant to build on Linux for GitHub Actions.
pub struct IntegrationTestDelegate {
    clipboard: InMemoryClipboard,
    cursor_shape: Mutex<Cursor>,
}

pub struct Window {
    callbacks: WindowCallbacks,
}

impl AppDelegate {
    pub fn new() -> Result<Self> {
        Ok(Self {
            clipboard: InMemoryClipboard::default(),
            cursor_shape: Mutex::new(Cursor::Arrow),
        })
    }
}

impl IntegrationTestDelegate {
    pub fn new() -> Result<Self> {
        Ok(Self {
            clipboard: InMemoryClipboard::default(),
            cursor_shape: Mutex::new(Cursor::Arrow),
        })
    }
}

#[derive(Default)]
pub(crate) struct WindowManager {
    windows: HashMap<WindowId, Rc<Window>>,
}

impl WindowManager {
    pub(crate) fn new() -> Self {
        Default::default()
    }
}

impl platform::WindowManager for WindowManager {
    fn open_window(
        &mut self,
        window_id: WindowId,
        _window_options: WindowOptions,
        callbacks: WindowCallbacks,
    ) -> Result<()> {
        self.windows
            .insert(window_id, Rc::new(Window { callbacks }));
        Ok(())
    }

    fn platform_window(&self, window_id: WindowId) -> OptionalPlatformWindow {
        self.windows
            .get(&window_id)
            .map(Rc::clone)
            .map(|window| window as Rc<dyn platform::Window>)
    }

    fn remove_window(&mut self, window_id: WindowId) {
        self.windows.remove(&window_id);
    }

    fn active_window_id(&self) -> Option<WindowId> {
        None
    }

    fn key_window_is_modal_panel(&self) -> bool {
        false
    }

    fn app_is_active(&self) -> bool {
        true
    }

    fn activate_app(&self, _last_active_window: Option<WindowId>) -> Option<WindowId> {
        // no-op for tests
        None
    }

    fn show_window_and_focus_app(&self, _window_id: WindowId, _behavior: WindowFocusBehavior) {
        // no-op for tests
    }

    fn hide_app(&self) {
        // no-op for tests
    }

    fn hide_window(&self, _window_id: WindowId) {
        // no-op for tests
    }

    fn set_window_bounds(&self, _window_id: WindowId, _bound: RectF) {
        // no-op for tests
    }

    fn set_all_windows_background_blur_radius(&self, _blur_radius_pixels: u8) {
        // no-op for tests
    }

    fn set_all_windows_background_blur_texture(&self, _use_blur_texture: bool) {
        // no-op for tests
    }

    fn set_window_title(&self, _window_id: WindowId, _title: &str) {
        // no-op for tests
    }

    fn close_window_async(&self, _window_id: WindowId, _termination_mode: TerminationMode) {
        // no-op for tests
    }

    fn active_display_bounds(&self) -> geometry::rect::RectF {
        Default::default()
    }

    fn active_display_id(&self) -> DisplayId {
        DisplayId::from(0)
    }

    fn display_count(&self) -> usize {
        1
    }

    fn bounds_for_display_idx(&self, _idx: DisplayIdx) -> Option<RectF> {
        Default::default()
    }

    fn active_cursor_position_updated(&self) {
        // no-op for tests
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

impl platform::Delegate for AppDelegate {
    #[cfg(feature = "test-util")]
    fn get_cursor_shape(&self) -> Cursor {
        *self.cursor_shape.lock()
    }

    fn set_cursor_shape(&self, cursor: Cursor) {
        *self.cursor_shape.lock() = cursor;
    }

    fn open_url(&self, _: &str) {
        // no-op for tests
    }

    fn close_ime_async(&self, _window_id: WindowId) {
        // no-op for tests
    }

    fn open_character_palette(&self) {
        // no-op for tests
    }

    fn open_file_path(&self, _: &Path) {
        // no-op for tests
    }

    fn open_file_path_in_explorer(&self, _: &Path) {
        // no-op for tests
    }

    fn open_file_picker(
        &self,
        _callback: FilePickerCallback,
        _file_picker_config: FilePickerConfiguration,
    ) {
        // no-op for tests
    }

    fn open_save_file_picker(
        &self,
        _callback: platform::SaveFilePickerCallback,
        _config: platform::SaveFilePickerConfiguration,
    ) {
        // no-op for tests
    }

    fn application_bundle_info(&self, _: &str) -> Option<ApplicationBundleInfo<'_>> {
        None
    }

    fn is_ime_open(&self) -> bool {
        false
    }

    fn set_accessibility_contents(&self, _: AccessibilityContent) {
        // no-op for tests
    }

    fn request_user_attention(&self, _window_id: WindowId) {
        // no-op for tests
    }

    fn request_desktop_notification_permissions(
        &self,
        _on_completion: RequestNotificationPermissionsCallback,
    ) {
        // no-op for tests
    }

    fn send_desktop_notification(
        &self,
        _notification_content: UserNotification,
        _window_id: WindowId,
        _on_error: SendNotificationErrorCallback,
    ) {
        // no-op for tests
    }

    fn clipboard(&mut self) -> &mut dyn crate::Clipboard {
        &mut self.clipboard
    }

    fn system_theme(&self) -> platform::SystemTheme {
        platform::SystemTheme::Light
    }

    fn dispatch_delegate(&self) -> Arc<dyn platform::DispatchDelegate> {
        Arc::new(DispatchDelegate)
    }

    fn register_global_shortcut(&self, _: Keystroke) {
        // no-op for tests
    }

    fn unregister_global_shortcut(&self, _: &Keystroke) {
        // no-op for tests
    }

    fn terminate_app(&self, _termination_mode: TerminationMode) {
        // no-op for tests
    }

    fn is_screen_reader_enabled(&self) -> Option<bool> {
        None
    }

    fn microphone_access_state(&self) -> MicrophoneAccessState {
        MicrophoneAccessState::NotDetermined
    }

    fn show_native_platform_modal(&self, _id: ModalId, _modal: AlertDialog) {
        // no-op
    }
}

impl platform::Delegate for IntegrationTestDelegate {
    #[cfg(feature = "test-util")]
    fn get_cursor_shape(&self) -> Cursor {
        *self.cursor_shape.lock()
    }

    fn set_cursor_shape(&self, cursor: Cursor) {
        *self.cursor_shape.lock() = cursor;
    }

    fn open_url(&self, _: &str) {
        // no-op for tests
    }

    fn close_ime_async(&self, _window_id: WindowId) {
        // no-op for tests
    }

    fn open_character_palette(&self) {
        // no-op for tests
    }

    fn open_file_path(&self, _: &Path) {
        // no-op for tests
    }

    fn open_file_path_in_explorer(&self, _: &Path) {
        // no-op for tests
    }

    fn open_file_picker(
        &self,
        _callback: FilePickerCallback,
        _file_picker_config: FilePickerConfiguration,
    ) {
        // no-op for tests
    }

    fn open_save_file_picker(
        &self,
        _callback: platform::SaveFilePickerCallback,
        _config: platform::SaveFilePickerConfiguration,
    ) {
        // no-op for tests
    }

    fn application_bundle_info(&self, _: &str) -> Option<ApplicationBundleInfo<'_>> {
        None
    }

    fn is_ime_open(&self) -> bool {
        false
    }

    fn set_accessibility_contents(&self, _: AccessibilityContent) {
        // no-op for tests
    }

    fn request_user_attention(&self, _window_id: WindowId) {
        // no-op for tests
    }

    fn request_desktop_notification_permissions(
        &self,
        _on_completion: RequestNotificationPermissionsCallback,
    ) {
        // no-op for tests
    }

    fn send_desktop_notification(
        &self,
        _notification_content: UserNotification,
        _window_id: WindowId,
        _on_error: SendNotificationErrorCallback,
    ) {
        // no-op for tests
    }

    fn clipboard(&mut self) -> &mut dyn crate::Clipboard {
        &mut self.clipboard
    }

    fn system_theme(&self) -> platform::SystemTheme {
        platform::SystemTheme::Light
    }

    fn dispatch_delegate(&self) -> Arc<dyn platform::DispatchDelegate> {
        Arc::new(DispatchDelegate)
    }

    fn register_global_shortcut(&self, _: Keystroke) {
        // no-op for tests
    }

    fn unregister_global_shortcut(&self, _: &Keystroke) {
        // no-op for tests
    }

    fn terminate_app(&self, _termination_mode: TerminationMode) {
        // no-op for tests
    }

    fn is_screen_reader_enabled(&self) -> Option<bool> {
        None
    }

    fn microphone_access_state(&self) -> MicrophoneAccessState {
        MicrophoneAccessState::NotDetermined
    }

    fn show_native_platform_modal(&self, _id: ModalId, _modal: AlertDialog) {
        // no-op
    }
}

impl platform::Window for Window {
    fn callbacks(&self) -> &crate::windowing::WindowCallbacks {
        &self.callbacks
    }

    fn minimize(&self) {}

    fn toggle_maximized(&self) {}

    fn toggle_fullscreen(&self) {}

    fn fullscreen_state(&self) -> platform::FullscreenState {
        platform::FullscreenState::Normal
    }

    fn set_titlebar_height(&self, _height: f64) {}

    fn as_ctx(&self) -> &dyn platform::WindowContext {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supports_transparency(&self) -> bool {
        true
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
}

impl platform::WindowContext for Window {
    fn size(&self) -> Vector2F {
        vec2f(1024.0, 768.0)
    }

    fn origin(&self) -> Vector2F {
        vec2f(0., 0.)
    }

    fn backing_scale_factor(&self) -> f32 {
        2.0
    }

    fn max_texture_dimension_2d(&self) -> Option<u32> {
        // For tests, choose a limit so low that it can run on any device.
        // https://github.com/gfx-rs/wgpu/blob/3b6112d45de8da75e47270fe3b0329e5d5166585/wgpu-types/src/lib.rs#L1278
        Some(2048)
    }

    fn render_scene(&self, _scene: Rc<Scene>) {}

    fn request_redraw(&self) {}

    fn request_frame_capture(
        &self,
        _callback: Box<dyn FnOnce(platform::CapturedFrame) + Send + 'static>,
    ) {
        // no-op for tests
    }
}

struct DispatchDelegate;

impl platform::DispatchDelegate for DispatchDelegate {
    fn is_main_thread(&self) -> bool {
        todo!()
    }

    fn run_on_main_thread(&self, _task: async_task::Runnable) {
        todo!()
    }
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct LoadedSystemFonts;
impl platform::LoadedSystemFonts for LoadedSystemFonts {
    fn as_any(self: Box<Self>) -> Box<dyn Any> {
        self as Box<dyn Any>
    }
}

/// A no-op font cache for use in tests that don't want to use full platform
/// functionality.
#[derive(Default)]
pub struct FontDB;

impl FontDB {
    pub fn new() -> Self {
        Self
    }
}

impl platform::FontDB for FontDB {
    fn load_from_bytes(&mut self, _name: &str, _bytes: Vec<Vec<u8>>) -> Result<FamilyId> {
        Ok(FamilyId(0))
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_from_system(&mut self, _font_family: &str) -> Result<FamilyId> {
        Ok(FamilyId(0))
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_all_system_fonts(
        &self,
    ) -> futures::future::BoxFuture<'static, Box<dyn platform::LoadedSystemFonts>> {
        use futures::FutureExt as _;

        futures::future::ready(Box::new(LoadedSystemFonts) as Box<dyn platform::LoadedSystemFonts>)
            .boxed()
    }

    #[cfg(not(target_family = "wasm"))]
    fn process_loaded_system_fonts(
        &mut self,
        loaded_system_fonts: Box<dyn platform::LoadedSystemFonts>,
    ) -> Vec<(Option<FamilyId>, crate::fonts::FontInfo)> {
        let _loaded_system_fonts: Box<LoadedSystemFonts> = loaded_system_fonts
            .as_any()
            .downcast()
            .expect("should not fail to downcast to concrete type");
        vec![]
    }

    fn fallback_fonts(
        &self,
        _ch: char,
        _font_id: crate::fonts::FontId,
    ) -> Vec<crate::fonts::FontId> {
        vec![]
    }

    fn select_font(
        &self,
        _family_id: crate::fonts::FamilyId,
        _properties: crate::fonts::Properties,
    ) -> crate::fonts::FontId {
        crate::fonts::FontId(0)
    }

    fn font_metrics(&self, _font_id: crate::fonts::FontId) -> crate::fonts::Metrics {
        crate::fonts::Metrics {
            units_per_em: 2048,
            ascent: 1901_i16,
            descent: (-483_i16),
            line_gap: 0_i16,
        }
    }

    fn glyph_advance(
        &self,
        _font_id: crate::fonts::FontId,
        _glyph_id: crate::fonts::GlyphId,
    ) -> Result<Vector2I> {
        Ok(Vector2I::zero())
    }

    fn load_family_name_from_id(&self, _id: crate::fonts::FamilyId) -> Option<String> {
        None
    }

    fn glyph_raster_bounds(
        &self,
        _font_id: crate::fonts::FontId,
        _size: f32,
        _glyph_id: crate::fonts::GlyphId,
        _scale: Vector2F,
        _glyph_config: &crate::rendering::GlyphConfig,
    ) -> Result<pathfinder_geometry::rect::RectI> {
        Ok(pathfinder_geometry::rect::RectI::default())
    }

    fn glyph_typographic_bounds(
        &self,
        _font_id: crate::fonts::FontId,
        _glyph_id: crate::fonts::GlyphId,
    ) -> Result<RectI> {
        Ok(RectI::default())
    }

    fn rasterize_glyph(
        &self,
        _font_id: crate::fonts::FontId,
        _size: f32,
        _glyph_id: crate::fonts::GlyphId,
        _scale: Vector2F,
        _subpixel_alignment: crate::fonts::SubpixelAlignment,
        _glyph_config: &crate::rendering::GlyphConfig,
        _format: crate::fonts::canvas::RasterFormat,
    ) -> Result<crate::fonts::RasterizedGlyph> {
        Ok(crate::fonts::RasterizedGlyph {
            canvas: crate::fonts::canvas::Canvas {
                pixels: vec![],
                size: vec2i(0, 0),
                row_stride: 0,
                format: crate::fonts::canvas::RasterFormat::Rgba32,
            },
            is_emoji: false,
        })
    }

    fn glyph_for_char(
        &self,
        _font_id: crate::fonts::FontId,
        _char: char,
    ) -> Option<crate::fonts::GlyphId> {
        Some(0)
    }

    fn family_id_for_name(&self, _name: &str) -> Option<FamilyId> {
        None
    }

    fn text_layout_system(&self) -> &dyn TextLayoutSystem {
        self
    }
}

impl platform::TextLayoutSystem for FontDB {
    fn layout_line(
        &self,
        _text: &str,
        line_style: platform::LineStyle,
        _style_runs: &[(std::ops::Range<usize>, crate::text_layout::StyleAndFont)],
        _max_width: f32,
        _clip_config: crate::text_layout::ClipConfig,
    ) -> crate::text_layout::Line {
        crate::text_layout::Line::empty(line_style.font_size, line_style.line_height_ratio, 0)
    }

    fn layout_text(
        &self,
        _text: &str,
        line_style: platform::LineStyle,
        _style_runs: &[(std::ops::Range<usize>, crate::text_layout::StyleAndFont)],
        _max_width: f32,
        _max_height: f32,
        _alignment: TextAlignment,
        _first_line_head_indent: Option<f32>,
    ) -> crate::text_layout::TextFrame {
        crate::text_layout::TextFrame::empty(line_style.font_size, line_style.line_height_ratio)
    }
}
