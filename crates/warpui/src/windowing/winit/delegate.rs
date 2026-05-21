#![allow(unused)]

#[cfg(not(target_family = "wasm"))]
mod global_hotkey;

use std::mem::ManuallyDrop;
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    thread::{self, panicking},
};

use anyhow::Result;
use geometry::rect::RectF;
use itertools::Itertools;
use parking_lot::Mutex;
use serde::de::IntoDeserializer;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};

use crate::platform::MicrophoneAccessState;
use crate::platform::{
    file_picker::{
        FilePickerCallback, FilePickerError, SaveFilePickerCallback, SaveFilePickerConfiguration,
    },
    Cursor, RequestNotificationPermissionsCallback, SendNotificationErrorCallback,
};
use crate::windowing::winit::app::CustomEvent::UpdateUIApp;
use crate::windowing::WindowManager;
use crate::Effect::Event;
use crate::{
    accessibility,
    clipboard::{self, ClipboardContent, InMemoryClipboard},
    geometry, keymap,
    modals::{AlertDialog, ModalId},
    notification, platform,
    platform::file_picker::{FilePickerConfiguration, FileType},
    windowing::{self, WindowCallbacks},
    AppContext, ApplicationBundleInfo, Clipboard, DisplayId, DisplayIdx, WindowId,
};
use crate::{
    notification::{NotificationSendError, RequestPermissionsOutcome},
    platform::TerminationMode,
};

use super::{notifications, CustomEvent};

#[cfg(not(target_family = "wasm"))]
use self::global_hotkey::GlobalHotKeyHandler;

// No-op on WASM since the browser cannot provide this functionality.
#[cfg(target_family = "wasm")]
struct GlobalHotKeyHandler {}

#[cfg(target_family = "wasm")]
impl GlobalHotKeyHandler {
    fn register(&self, _: keymap::Keystroke) {}
    fn unregister(&self, _: &keymap::Keystroke) {}
}

/// Stores the ID of the application's main thread, which we can reference
/// to determine if a given thread is the main thread or not.
static MAIN_THREAD_ID: OnceLock<thread::ThreadId> = OnceLock::new();

/// Open a URL using the platform's default handler.
pub fn open_url_in_system(url: &str) {
    #[cfg(target_family = "wasm")]
    if let Some(window) = web_sys::window() {
        // Try to open the URL in a new tab.
        let _ = window.open_with_url_and_target(url, "_blank");
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        // Opening in WSL is complicated for a few reasons
        // 1. By default, wsl does not have an awareness of browsers installed in windows.
        //    We either need to have wslu installed for wslview, or we need
        // 2. We do not necessarily have things like xdg-utils installed, so relying on
        //    "native" opening of files is not necessarily going to work.
        // We choose to do the following:
        // 1. First attempt to open with `wslview`, since that is basically made to open stuff in wsl
        // 2. Use `cmd.exe /c start {url}` to open in the user's default windows browser
        //    - If a user does not want this behavior, and wants all opening to go through
        //      WSL, they can set the env variable WARP_FORCE_WSL_BROWSER.
        // 3. Fall back to default linux url opening behavior.
        if platform::linux::is_wsl() {
            match open::with_detached(url, "wslview") {
                Ok(_) => return,
                Err(e) => log::info!(
                    "Failed to open url with wslview {e:?}, falling back to another method"
                ),
            };

            // Attempt to open by
            if !use_wsl_browser() {
                let mut cmd = command::blocking::Command::new("cmd.exe");
                cmd.args(["/c", "start", url]);

                // Note: Ideally, we would be calling detached like open::that_detached does.
                // However, it is probably fine.
                match cmd
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                {
                    Ok(_) => return,
                    Err(e) => log::info!(
                        "Failed to open url with cmd.exe {e:?}, falling back to another method"
                    ),
                }
            }
        }
        if let Err(e) = open::that_detached(url) {
            log::warn!("Unable to open url {e:?}");
        }
    }

    #[cfg(windows)]
    {
        if let Err(e) = open::that_detached(url) {
            log::warn!("Unable to open url {e:?}");
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn use_wsl_browser() -> bool {
    static USE_WSL_BROWSER: OnceLock<bool> = OnceLock::new();
    USE_WSL_BROWSER
        .get_or_init(|| std::env::var("WARP_FORCE_WSL_BROWSER").is_ok())
        .to_owned()
}

/// Marks the current thread as the application's main thread.
///
/// # Panics
///
/// Panics if called more than once.
pub(super) fn mark_current_thread_as_main() {
    MAIN_THREAD_ID
        .set(thread::current().id())
        .expect("should only call mark_current_thread_as_main once!");
}

pub struct DispatchDelegate {
    event_loop_proxy: Mutex<EventLoopProxy<super::CustomEvent>>,
}

impl platform::DispatchDelegate for DispatchDelegate {
    fn is_main_thread(&self) -> bool {
        thread::current().id()
            == *MAIN_THREAD_ID
                .get()
                .expect("should have marked a thread as the main thread")
    }

    fn run_on_main_thread(&self, task: async_task::Runnable) {
        // Surround the `task` in a `ManuallyDrop` so we can control when the task gets dropped.
        // If the event loop is no longer running, sending the task over a channel will fail which
        // causes the `task` to be dropped by _this_ thread. This in turns triggers a panic in
        // `async-task` since the future is dropped by a different thread than what spawned it.
        // In the case the event loop is no longer running, we will end up leaking the task until
        // the process exits (which should happen imminently given the event loop has terminated).
        self.event_loop_proxy
            .lock()
            .send_event(super::CustomEvent::RunTask(ManuallyDrop::new(task)));
    }
}

pub struct AppDelegate {
    /// A handle for enqueueing [`CustomEvent`]s into the main event loop.
    pub(super) event_loop_proxy: EventLoopProxy<super::CustomEvent>,

    clipboard: Box<dyn Clipboard>,

    /// Responsible for registering the global hotkeys in the platform's desktop environment. Will
    /// be `None` for platforms that can't support global hotkeys.
    global_hotkey_handler: Option<GlobalHotKeyHandler>,

    #[cfg(feature = "test-util")]
    last_known_cursor: RefCell<Cursor>,
}

impl AppDelegate {
    pub fn new(event_loop_proxy: EventLoopProxy<super::CustomEvent>) -> Result<Self> {
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                let global_hotkey_handler = None;
            } else {
                let global_hotkey_handler = match GlobalHotKeyHandler::new(event_loop_proxy.clone()) {
                    Ok(handler) => Some(handler),
                    Err(err) => {
                        log::error!("Error creating global hotkey handler: {err:?}");
                        None
                    }
                };
            }
        }
        Ok(Self {
            event_loop_proxy,
            clipboard: Box::<InMemoryClipboard>::default(),
            global_hotkey_handler,
            #[cfg(feature = "test-util")]
            last_known_cursor: RefCell::new(Cursor::Arrow),
        })
    }

    /// The way copy-paste is handled depends on the specific windowing system. As winit is
    /// abstracting the windowing system, we need to ask it which one is running. We can do that by
    /// matching against the display server raw handle.
    pub fn use_platform_clipboard(&mut self) {
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                self.clipboard = Box::new(super::wasm::WebClipboard::new());
            } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
                match super::linux::LinuxClipboard::new() {
                    Ok(clipboard) => self.clipboard = Box::new(clipboard),
                    Err(err) => {
                        log::error!("Error creating Linux clipboard: {err:?}");
                    }
                }
            } else if #[cfg(target_os = "windows")] {
                match super::windows::WindowsClipboard::new() {
                    Ok(clipboard) => self.clipboard = Box::new(clipboard),
                    Err(err) => {
                        log::error!("Error creating Windows clipboard: {err:?}");
                    }
                }
            }
        }
    }
}

impl platform::Delegate for AppDelegate {
    fn dispatch_delegate(&self) -> Arc<dyn platform::DispatchDelegate> {
        Arc::new(DispatchDelegate {
            event_loop_proxy: Mutex::new(self.event_loop_proxy.clone()),
        })
    }

    fn request_user_attention(&self, window_id: WindowId) {
        self.event_loop_proxy
            .send_event(CustomEvent::RequestUserAttention { window_id });
    }

    fn clipboard(&mut self) -> &mut dyn crate::Clipboard {
        self.clipboard.as_mut()
    }

    #[cfg(not(target_family = "wasm"))]
    fn system_theme(&self) -> platform::SystemTheme {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        match super::linux::get_system_theme() {
            Ok(system_theme) => {
                return system_theme;
            }
            Err(err) => {
                log::info!("Unable to fetch Linux system color scheme: {err:#}");
            }
        }

        #[cfg(target_os = "windows")]
        match super::windows::get_system_theme() {
            Ok(system_theme) => {
                return system_theme;
            }
            Err(err) => {
                log::warn!("Unable to fetch Windows system color scheme: {err:#?}");
            }
        }

        platform::SystemTheme::Light
    }

    #[cfg(target_family = "wasm")]
    fn system_theme(&self) -> platform::SystemTheme {
        // To determine dark mode versus light mode, we check the CSS media query string "prefers-color-scheme". According
        // to StackOverflow, this is the current consensus solution.
        // See https://stackoverflow.com/questions/56393880/how-do-i-detect-dark-mode-using-javascript.
        if let Ok(Some(media_query_list)) =
            gloo::utils::window().match_media("(prefers-color-scheme: dark)")
        {
            if media_query_list.matches() {
                return platform::SystemTheme::Dark;
            }
        }
        platform::SystemTheme::Light
    }

    fn open_url(&self, url: &str) {
        open_url_in_system(url);
    }

    fn open_file_path(&self, path: &Path) {
        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
                let _ = command::blocking::Command::new("xdg-open")
                    .arg(path)
                    .spawn();
            } else if #[cfg(target_family = "wasm")] {
                if let Some(window) = web_sys::window() {
                    if let Some(path) = path.to_str() {
                        // Try to open the path via a file:// URL.
                        let url = format!("file://{path}");
                        let _ = window.open_with_url(&url);
                    }
                }
            } else if #[cfg(windows)] {
                if let Err(e) = open::that_detached(path) {
                    log::warn!("Unable to open path {e:?}");
                }
            }
        }
    }

    fn open_file_picker(
        &self,
        callback: FilePickerCallback,
        file_picker_config: FilePickerConfiguration,
    ) {
        // TODO(wasm): Investigate implementing this by creating a <input> element
        // and calling `click` on it.

        #[cfg(not(target_family = "wasm"))]
        {
            // This callback is called either on the “File Picker” background thread or, if starting
            // that thread fails, on this thread. Wrap this type in order to make ownership work.
            let callback = Arc::new(takecell::TakeOwnCell::new(callback));
            let callback_clone = callback.clone();

            // Since native_dialog::FileDialog blocks while waiting for the user to select a file,
            // put it in its own thread to avoid blocking the rest of the app.
            let event_loop_proxy = self.event_loop_proxy.clone();
            let thread_result = std::thread::Builder::new()
                .name("File Picker".to_string())
                .spawn(move || {
                    let file_type_names = file_picker_config
                        .file_types()
                        .iter()
                        .map(|file_type| file_type.display_name())
                        .join(", ");
                    let allowed_extensions = file_picker_config
                        .file_types()
                        .iter()
                        .map(|file_type| file_type.extensions())
                        .collect_vec()
                        .concat();

                    // native-dialog doesn't support file-or-directory or multi-directory pickers,
                    // so if folders are allowed, it can only show a directory picker.
                    let result = if file_picker_config.allows_folder() {
                        native_dialog::FileDialog::new()
                            .set_title("Choose directory...")
                            .show_open_single_dir()
                            .map(|opt| opt.into_iter().collect())
                            .map_err(|e| FilePickerError::DialogFailed(e.to_string()))
                    } else {
                        let mut file_dialog =
                            native_dialog::FileDialog::new().set_title("Choose file...");
                        if !allowed_extensions.is_empty() {
                            file_dialog = file_dialog.add_filter(
                                file_type_names.as_str(),
                                allowed_extensions.as_slice(),
                            );
                        }
                        if file_picker_config.allows_multi_select() {
                            file_dialog
                                .show_open_multiple_file()
                                .map_err(|e| FilePickerError::DialogFailed(e.to_string()))
                        } else {
                            file_dialog
                                .show_open_single_file()
                                .map(|opt| opt.into_iter().collect())
                                .map_err(|e| FilePickerError::DialogFailed(e.to_string()))
                        }
                    };

                    let result =
                        result.and_then(|file_result| {
                            file_result
                                .iter()
                                .map(|path_buf| {
                                    path_buf.as_os_str().to_str().map(String::from).ok_or_else(
                                        || {
                                            FilePickerError::DialogFailed(format!(
                                                "Invalid path encoding: {:?}",
                                                path_buf
                                            ))
                                        },
                                    )
                                })
                                .collect::<Result<Vec<_>, _>>()
                        });

                    event_loop_proxy.send_event(CustomEvent::UpdateUIApp(Box::new(move |app| {
                        if let Some(callback) = callback_clone.take() {
                            callback(result, app);
                        }
                    })));
                });
            if let Err(e) = thread_result {
                self.event_loop_proxy
                    .send_event(CustomEvent::UpdateUIApp(Box::new(move |app| {
                        if let Some(callback) = callback.take() {
                            callback(Err(FilePickerError::ThreadSpawnFailed(Arc::new(e))), app);
                        }
                    })));
            }
        }
    }

    fn open_save_file_picker(
        &self,
        callback: SaveFilePickerCallback,
        config: SaveFilePickerConfiguration,
    ) {
        #[cfg(not(target_family = "wasm"))]
        {
            let event_loop_proxy = self.event_loop_proxy.clone();
            std::thread::Builder::new()
                .name("Save File Picker".to_string())
                .spawn(move || {
                    let mut file_dialog =
                        native_dialog::FileDialog::new().set_title("Save file as...");

                    if let Some(default_filename) = config.default_filename.as_ref() {
                        file_dialog = file_dialog.set_filename(default_filename);
                    }

                    if let Some(default_directory) = config.default_directory.as_ref() {
                        file_dialog = file_dialog.set_location(default_directory);
                    }

                    let file_result = file_dialog.show_save_single_file().unwrap_or_else(|err| {
                        log::error!("unable to show save file dialog: {err:?}");
                        None
                    });

                    let path = file_result
                        .and_then(|path_buf| path_buf.as_os_str().to_str().map(String::from));

                    event_loop_proxy.send_event(CustomEvent::UpdateUIApp(Box::new(|app| {
                        callback(path, app);
                    })));
                });
        }
    }

    fn application_bundle_info(
        &self,
        bundle_identifier: &str,
    ) -> Option<ApplicationBundleInfo<'_>> {
        None
    }

    fn request_desktop_notification_permissions(
        &self,
        on_completion: RequestNotificationPermissionsCallback,
    ) {
        notifications::request_desktop_notification_permissions(
            on_completion,
            &self.event_loop_proxy,
        );
    }

    #[cfg(feature = "test-util")]
    fn get_cursor_shape(&self) -> Cursor {
        *self.last_known_cursor.borrow()
    }

    fn send_desktop_notification(
        &self,
        notification_content: notification::UserNotification,
        window_id: WindowId,
        on_error: SendNotificationErrorCallback,
    ) {
        notifications::send_desktop_notification(
            notification_content,
            window_id,
            on_error,
            &self.event_loop_proxy,
        )
    }

    fn set_cursor_shape(&self, cursor: Cursor) {
        #[cfg(test)]
        {
            *self.last_known_cursor.borrow_mut() = cursor;
        }
        self.event_loop_proxy
            .send_event(CustomEvent::SetCursorShape(cursor));
    }

    fn close_ime_async(&self, _window_id: WindowId) {
        // TODO(wasm): implement this.
    }

    fn is_ime_open(&self) -> bool {
        // TODO(wasm): implement this.
        false
    }

    fn open_character_palette(&self) {
        // TODO(wasm): Implement this.
    }

    fn set_accessibility_contents(&self, content: accessibility::AccessibilityContent) {
        // TODO(wasm): Implement this.
    }

    fn register_global_shortcut(&self, shortcut: keymap::Keystroke) {
        if let Some(handler) = &self.global_hotkey_handler {
            handler.register(shortcut);
        }
    }

    fn unregister_global_shortcut(&self, shortcut: &keymap::Keystroke) {
        if let Some(handler) = &self.global_hotkey_handler {
            handler.unregister(shortcut);
        }
    }

    fn terminate_app(&self, terminaton_mode: TerminationMode) {
        self.event_loop_proxy
            .send_event(CustomEvent::Terminate(terminaton_mode));
    }

    fn is_screen_reader_enabled(&self) -> Option<bool> {
        // TODO(wasm): Implement this.
        None
    }

    fn microphone_access_state(&self) -> MicrophoneAccessState {
        // Note that for voice input, we can actually detect microphone access state
        // in the course of trying to start voice input, but we don't have a way to do
        // it at arbitrary times, so we just return NotDetermined here.
        MicrophoneAccessState::NotDetermined
    }

    fn open_file_path_in_explorer(&self, path: &Path) {
        if path.is_dir() {
            self.open_file_path(path);
        } else if let Some(parent_path) = path.parent() {
            if parent_path.is_dir() {
                self.open_file_path(parent_path);
            } else {
                log::info!("Parent directory is not a valid directory, not opening file")
            }
        } else {
            log::info!("Neither file nor parent was a valid directory, not opening file");
        }
    }

    fn show_native_platform_modal(&self, _id: ModalId, _modal: AlertDialog) {
        // TODO
    }
}

pub struct IntegrationTestDelegate {
    app_delegate: AppDelegate,
    clipboard: InMemoryClipboard,
}

impl IntegrationTestDelegate {
    pub fn new(event_loop_proxy: EventLoopProxy<super::CustomEvent>) -> Result<Self> {
        Ok(IntegrationTestDelegate {
            app_delegate: AppDelegate::new(event_loop_proxy)?,
            clipboard: InMemoryClipboard::default(),
        })
    }
}

impl platform::Delegate for IntegrationTestDelegate {
    fn dispatch_delegate(&self) -> Arc<dyn platform::DispatchDelegate> {
        self.app_delegate.dispatch_delegate()
    }

    fn request_user_attention(&self, _window_id: WindowId) {
        // no-op
    }

    fn clipboard(&mut self) -> &mut dyn crate::Clipboard {
        &mut self.clipboard
    }

    fn system_theme(&self) -> platform::SystemTheme {
        self.app_delegate.system_theme()
    }

    fn open_url(&self, _: &str) {
        // no-op
    }

    fn open_file_path(&self, _: &Path) {
        // no-op
    }

    fn open_file_picker(
        &self,
        _callback: FilePickerCallback,
        _file_picker_config: FilePickerConfiguration,
    ) {
        // no-op
    }

    fn open_save_file_picker(
        &self,
        _callback: SaveFilePickerCallback,
        _config: SaveFilePickerConfiguration,
    ) {
        // no-op
    }

    fn application_bundle_info(&self, _: &str) -> Option<ApplicationBundleInfo<'_>> {
        None
    }

    fn microphone_access_state(&self) -> MicrophoneAccessState {
        MicrophoneAccessState::NotDetermined
    }

    fn request_desktop_notification_permissions(
        &self,
        _on_completion: RequestNotificationPermissionsCallback,
    ) {
        // no-op
    }

    fn send_desktop_notification(
        &self,
        _notification_content: notification::UserNotification,
        _window_id: WindowId,
        _on_error: SendNotificationErrorCallback,
    ) {
        // no-op
    }

    #[cfg(feature = "test-util")]
    fn get_cursor_shape(&self) -> platform::Cursor {
        self.app_delegate.get_cursor_shape()
    }

    fn set_cursor_shape(&self, cursor: platform::Cursor) {
        self.app_delegate.set_cursor_shape(cursor)
    }

    fn close_ime_async(&self, _window_id: WindowId) {
        // no-op
    }

    fn is_ime_open(&self) -> bool {
        false
    }

    fn open_character_palette(&self) {
        // no-op
    }

    fn set_accessibility_contents(&self, _: accessibility::AccessibilityContent) {
        // no-op
    }

    fn register_global_shortcut(&self, shortcut: keymap::Keystroke) {
        self.app_delegate.register_global_shortcut(shortcut)
    }

    fn unregister_global_shortcut(&self, shortcut: &keymap::Keystroke) {
        self.app_delegate.unregister_global_shortcut(shortcut)
    }

    fn terminate_app(&self, termination_mode: TerminationMode) {
        self.app_delegate.terminate_app(termination_mode);
    }

    fn is_screen_reader_enabled(&self) -> Option<bool> {
        self.app_delegate.is_screen_reader_enabled()
    }

    fn open_file_path_in_explorer(&self, path: &Path) {
        // no-op
    }

    fn show_native_platform_modal(&self, _id: ModalId, _modal: AlertDialog) {
        // no-op
    }
}
