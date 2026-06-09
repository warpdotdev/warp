//! Platform delegate for the TUI backend.
//!
//! Copied from the `headless` backend, except that [`AppDelegate::is_headless`]
//! returns `false`: the TUI presents a visible, interactive surface.

use std::mem::ManuallyDrop;
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::thread;

use parking_lot::Mutex;

use super::event_loop::AppEvent;
use crate::clipboard::InMemoryClipboard;
use crate::notification::{NotificationSendError, RequestPermissionsOutcome};
use crate::platform::{self, Cursor};

/// Stores the ID of the application's main thread, which we can reference
/// to determine if a given thread is the main thread or not.
static MAIN_THREAD_ID: OnceLock<thread::ThreadId> = OnceLock::new();

/// Marks the current thread as the application's main thread.
///
/// Panics if called more than once.
pub(super) fn mark_current_thread_as_main() {
    MAIN_THREAD_ID
        .set(thread::current().id())
        .expect("should only call mark_current_thread_as_main once!");
}

pub struct AppDelegate {
    clipboard: InMemoryClipboard,
    cursor_shape: Mutex<Cursor>,
    event_sender: Sender<AppEvent>,
}

impl AppDelegate {
    pub(super) fn new(event_sender: Sender<AppEvent>) -> Self {
        Self {
            clipboard: InMemoryClipboard::default(),
            cursor_shape: Mutex::new(Cursor::Arrow),
            event_sender,
        }
    }

    fn send_event(&self, event: AppEvent) {
        if self.event_sender.send(event).is_err() {
            log::warn!("Tried to send event, but event loop is no longer running");
        }
    }
}

impl platform::Delegate for AppDelegate {
    fn dispatch_delegate(&self) -> Arc<dyn platform::DispatchDelegate> {
        Arc::new(DispatchDelegate {
            event_sender: self.event_sender.clone(),
        })
    }

    fn request_user_attention(&self, _window_id: crate::WindowId) {
        // Unsupported.
    }

    fn clipboard(&mut self) -> &mut dyn crate::Clipboard {
        &mut self.clipboard
    }

    fn system_theme(&self) -> platform::SystemTheme {
        // The TUI surface uses a dark palette, so report a dark system theme.
        platform::SystemTheme::Dark
    }

    fn open_url(&self, url: &str) {
        #[cfg(target_os = "macos")]
        {
            crate::platform::mac::Window::open_url(url);
        }
        #[cfg(not(target_os = "macos"))]
        {
            crate::windowing::winit::delegate::open_url_in_system(url);
        }
    }

    fn open_file_path(&self, _path: &std::path::Path) {
        // Unsupported.
    }

    fn open_file_path_in_explorer(&self, _path: &std::path::Path) {
        // Unsupported.
    }

    fn open_file_picker(
        &self,
        callback: platform::FilePickerCallback,
        _file_picker_config: platform::FilePickerConfiguration,
    ) {
        self.send_event(AppEvent::RunCallback(Box::new(move |ctx| {
            callback(Ok(vec![]), ctx);
        })));
    }

    fn open_save_file_picker(
        &self,
        callback: platform::SaveFilePickerCallback,
        _config: platform::SaveFilePickerConfiguration,
    ) {
        self.send_event(AppEvent::RunCallback(Box::new(move |ctx| {
            callback(None, ctx);
        })));
    }

    fn application_bundle_info(
        &self,
        _bundle_identifier: &str,
    ) -> Option<crate::ApplicationBundleInfo<'_>> {
        None
    }

    fn show_native_platform_modal(
        &self,
        _id: crate::modals::ModalId,
        _modal: crate::modals::AlertDialog,
    ) {
        // Unsupported.
    }

    fn request_desktop_notification_permissions(
        &self,
        on_completion: platform::RequestNotificationPermissionsCallback,
    ) {
        self.send_event(AppEvent::RunCallback(Box::new(move |ctx| {
            on_completion(RequestPermissionsOutcome::PermissionsDenied, ctx);
        })));
    }

    fn send_desktop_notification(
        &self,
        _notification_content: crate::notification::UserNotification,
        _window_id: crate::WindowId,
        on_error: platform::SendNotificationErrorCallback,
    ) {
        self.send_event(AppEvent::RunCallback(Box::new(move |ctx| {
            on_error(NotificationSendError::PermissionsDenied, ctx);
        })));
    }

    fn set_cursor_shape(&self, cursor: Cursor) {
        *self.cursor_shape.lock() = cursor;
    }

    #[cfg(feature = "test-util")]
    fn get_cursor_shape(&self) -> Cursor {
        *self.cursor_shape.lock()
    }

    fn close_ime_async(&self, _window_id: crate::WindowId) {
        // Unsupported.
    }

    fn is_ime_open(&self) -> bool {
        false
    }

    fn open_character_palette(&self) {
        // Unsupported.
    }

    fn set_accessibility_contents(&self, _content: crate::accessibility::AccessibilityContent) {
        // Unsupported.
    }

    fn register_global_shortcut(&self, _shortcut: crate::keymap::Keystroke) {
        // Unsupported.
    }

    fn unregister_global_shortcut(&self, _shortcut: &crate::keymap::Keystroke) {
        // Unsupported.
    }

    fn terminate_app(&self, termination_mode: platform::TerminationMode) {
        self.send_event(AppEvent::Terminate(termination_mode));
    }

    fn is_screen_reader_enabled(&self) -> Option<bool> {
        None
    }

    fn microphone_access_state(&self) -> platform::MicrophoneAccessState {
        platform::MicrophoneAccessState::Denied
    }

    /// The TUI presents a visible, interactive surface, so it is not headless.
    fn is_headless(&self) -> bool {
        false
    }
}

struct DispatchDelegate {
    event_sender: Sender<AppEvent>,
}

impl platform::DispatchDelegate for DispatchDelegate {
    fn is_main_thread(&self) -> bool {
        thread::current().id()
            == *MAIN_THREAD_ID
                .get()
                .expect("should have marked a thread as the main thread")
    }

    fn run_on_main_thread(&self, task: async_task::Runnable) {
        if self
            .event_sender
            .send(AppEvent::RunTask(ManuallyDrop::new(task)))
            .is_err()
        {
            log::warn!("Tried to send event, but event loop is no longer running");
        }
    }
}
