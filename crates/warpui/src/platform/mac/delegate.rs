use super::app::create_native_platform_modal;
use super::keycode::{modifier_code, Keycode};
use super::utils::nsstring_as_str;
use super::{app, make_nsstring, Clipboard, Window};
use anyhow::Result;
use cocoa::base::{BOOL, NO, YES};
use cocoa::foundation::NSUInteger;
use cocoa::{
    appkit::{NSApp, NSRequestUserAttentionType},
    base::{id, nil},
};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;
use warpui_core::clipboard::InMemoryClipboard;
use warpui_core::keymap::Keystroke;
use warpui_core::modals::{AlertDialog, ModalId};
use warpui_core::notification::{NotificationSendError, RequestPermissionsOutcome};
use warpui_core::platform::{
    Cursor, FilePickerCallback, FilePickerConfiguration, MicrophoneAccessState,
    SendNotificationErrorCallback, TerminationMode,
};
use warpui_core::ApplicationBundleInfo;
use warpui_core::{
    accessibility::AccessibilityContent, notification::UserNotification, platform, WindowId,
};

// Functions implemented in objC files.
extern "C" {
    // Requests permissions to send desktop notifications.
    fn requestNotificationPermissions(on_completion_callback: *const c_void);
    // Sends a desktop notification.
    fn sendNotification(
        title: id,
        body: id,
        data: id,
        on_error_callback: *const c_void,
        play_sound: BOOL,
    );
    fn isDarkMode() -> BOOL;
    fn registerGlobalHotkey(key_code: NSUInteger, modifiers_key: NSUInteger);
    fn unregisterGlobalHotkey(key_code: NSUInteger, modifiers_key: NSUInteger);
    fn executableInApplicationBundleWithIdentifier(bundle_path: id) -> id;
    fn absolutePathForApplicationBundleWithIdentifier(bundle_identifier: id) -> id;
    fn isVoiceOverEnabled() -> BOOL;
}

type RequestNotificationPermissionsCallback = Box<dyn FnOnce(RequestPermissionsOutcome) + Send>;
type NotificationSendErrorCallback = Box<dyn FnOnce(NotificationSendError) + Send>;

/// Delegator that wraps platform-specific calls in a common API.
pub struct AppDelegate {
    clipboard: Clipboard,
    dispatch_delegate: Arc<DispatchDelegate>,
}

pub struct IntegrationTestDelegate {
    app_delegate: AppDelegate,
    clipboard: InMemoryClipboard,
}

impl IntegrationTestDelegate {
    pub fn new() -> Result<Self> {
        Ok(IntegrationTestDelegate {
            app_delegate: AppDelegate::new()?,
            clipboard: InMemoryClipboard::default(),
        })
    }
}

impl platform::Delegate for IntegrationTestDelegate {
    #[cfg(feature = "test-util")]
    fn get_cursor_shape(&self) -> Cursor {
        self.app_delegate.get_cursor_shape()
    }

    fn set_cursor_shape(&self, cursor: Cursor) {
        self.app_delegate.set_cursor_shape(cursor)
    }

    fn open_url(&self, _: &str) {
        // no-op
    }

    fn open_file_path(&self, _: &Path) {
        // no-op
    }

    fn open_file_path_in_explorer(&self, _: &Path) {
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
        _callback: platform::SaveFilePickerCallback,
        _config: platform::SaveFilePickerConfiguration,
    ) {
        // no-op
    }

    fn application_bundle_info(&self, _: &str) -> Option<ApplicationBundleInfo<'_>> {
        None
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

    fn set_accessibility_contents(&self, _: AccessibilityContent) {
        // no-op
    }

    fn request_user_attention(&self, _window_id: WindowId) {
        // no-op
    }

    fn clipboard(&mut self) -> &mut dyn crate::Clipboard {
        &mut self.clipboard
    }

    fn request_desktop_notification_permissions(
        &self,
        _on_completion: platform::RequestNotificationPermissionsCallback,
    ) {
        // no-op
    }

    fn send_desktop_notification(
        &self,
        _notification_content: UserNotification,
        _window_id: WindowId,
        _on_error: SendNotificationErrorCallback,
    ) {
    }

    fn system_theme(&self) -> platform::SystemTheme {
        self.app_delegate.system_theme()
    }

    fn dispatch_delegate(&self) -> Arc<dyn platform::DispatchDelegate> {
        self.app_delegate.dispatch_delegate()
    }

    fn register_global_shortcut(&self, shortcut: Keystroke) {
        self.app_delegate.register_global_shortcut(shortcut)
    }

    fn unregister_global_shortcut(&self, shortcut: &Keystroke) {
        self.app_delegate.unregister_global_shortcut(shortcut)
    }

    fn terminate_app(&self, termination_mode: TerminationMode) {
        self.app_delegate.terminate_app(termination_mode);
    }

    fn is_screen_reader_enabled(&self) -> Option<bool> {
        self.app_delegate.is_screen_reader_enabled()
    }

    fn microphone_access_state(&self) -> MicrophoneAccessState {
        self.app_delegate.microphone_access_state()
    }

    fn show_native_platform_modal(&self, _id: ModalId, _modal: AlertDialog) {
        // no-op
    }
}

pub struct DispatchDelegate;

impl AppDelegate {
    pub fn new() -> Result<Self> {
        Ok(AppDelegate {
            clipboard: Clipboard::new()?,
            dispatch_delegate: Arc::new(DispatchDelegate),
        })
    }
}

impl platform::Delegate for AppDelegate {
    /// Sets the cursor shape
    /// See https://developer.apple.com/documentation/appkit/nscursor?language=objc
    fn set_cursor_shape(&self, cursor: Cursor) {
        unsafe {
            let cursor: id = match cursor {
                Cursor::Arrow => msg_send![class!(NSCursor), arrowCursor],
                Cursor::IBeam => msg_send![class!(NSCursor), IBeamCursor],
                Cursor::Crosshair => msg_send![class!(NSCursor), crosshairCursor],
                Cursor::OpenHand => msg_send![class!(NSCursor), openHandCursor],
                Cursor::NotAllowed => msg_send![class!(NSCursor), operationNotAllowedCursor],
                Cursor::PointingHand => msg_send![class!(NSCursor), pointingHandCursor],
                Cursor::ResizeLeftRight => msg_send![class!(NSCursor), resizeLeftRightCursor],
                Cursor::ResizeUpDown => msg_send![class!(NSCursor), resizeUpDownCursor],
                Cursor::ClosedHand => msg_send![class!(NSCursor), closedHandCursor],
                Cursor::DragCopy => msg_send![class!(NSCursor), dragCopyCursor],
            };
            let () = msg_send![cursor, set];
        }
    }

    #[cfg(feature = "test-util")]
    fn get_cursor_shape(&self) -> Cursor {
        unimplemented!("only implemented in tests")
    }
    fn open_url(&self, url: &str) {
        Window::open_url(url);
    }

    fn open_file_path(&self, path: &Path) {
        Window::open_file_path(path);
    }

    fn open_file_path_in_explorer(&self, path: &Path) {
        Window::open_file_path_in_explorer(path);
    }

    fn open_file_picker(
        &self,
        callback: FilePickerCallback,
        file_picker_config: FilePickerConfiguration,
    ) {
        Window::open_file_picker(callback, file_picker_config);
    }

    fn open_save_file_picker(
        &self,
        callback: platform::SaveFilePickerCallback,
        config: platform::SaveFilePickerConfiguration,
    ) {
        Window::open_save_file_picker(callback, config);
    }

    fn application_bundle_info(
        &self,
        bundle_identifier: &str,
    ) -> Option<ApplicationBundleInfo<'_>> {
        let bundle_path = unsafe {
            let nsstring =
                absolutePathForApplicationBundleWithIdentifier(make_nsstring(bundle_identifier));

            if nsstring == nil {
                return None;
            }

            nsstring_as_str(nsstring).ok()?
        };

        let executable_path = unsafe {
            let nsstring = executableInApplicationBundleWithIdentifier(make_nsstring(bundle_path));

            if nsstring == nil {
                None
            } else {
                nsstring_as_str(nsstring).map(Path::new).ok()
            }
        };

        Some(ApplicationBundleInfo {
            path: Path::new(bundle_path),
            executable: executable_path,
        })
    }

    /// Open the macOS character palette.
    fn open_character_palette(&self) {
        // Open the character palette in a async task on the main thread to
        // ensure we don't double-borrow the app.
        dispatch::Queue::main().exec_async(move || unsafe {
            // See https://developer.apple.com/documentation/appkit/nsapplication/1428455-orderfrontcharacterpalette.
            // If the `sender` argument is nil, the palette is shown relative to the
            // first responder's cursor location. In our case, that will be the Warp
            // host view, with a location set via the `active_cursor_position` API.
            let () = msg_send![NSApp(), orderFrontCharacterPalette: nil];
        });
    }

    fn close_ime_async(&self, window_id: WindowId) {
        Window::close_ime_async(window_id);
    }

    fn is_ime_open(&self) -> bool {
        Window::is_ime_open()
    }

    fn set_accessibility_contents(&self, content: AccessibilityContent) {
        Window::set_accessibility_contents(content);
    }

    fn request_user_attention(&self, _window_id: WindowId) {
        unsafe {
            let () = msg_send![
                NSApp(),
                requestUserAttention: NSRequestUserAttentionType::NSInformationalRequest
            ];
        }
    }

    fn request_desktop_notification_permissions(
        &self,
        on_completion_callback: platform::RequestNotificationPermissionsCallback,
    ) {
        unsafe {
            let callback: RequestNotificationPermissionsCallback = Box::new(|outcome| {
                app::callback_dispatcher().with_mutable_app_context(|ctx| {
                    on_completion_callback(outcome, ctx);
                })
            });
            requestNotificationPermissions(Box::into_raw(Box::new(callback)) as *const c_void);
        };
    }

    fn send_desktop_notification(
        &self,
        notification_content: UserNotification,
        _window_id: WindowId,
        on_error_callback: SendNotificationErrorCallback,
    ) {
        unsafe {
            let callback: NotificationSendErrorCallback = Box::new(|error| {
                app::callback_dispatcher().with_mutable_app_context(|ctx| {
                    on_error_callback(error, ctx);
                })
            });
            sendNotification(
                make_nsstring(notification_content.title()),
                make_nsstring(notification_content.body()),
                make_nsstring(notification_content.data().unwrap_or_default()),
                Box::into_raw(Box::new(callback)) as *const c_void,
                if notification_content.play_sound() {
                    YES
                } else {
                    NO
                },
            );
        };
    }

    fn clipboard(&mut self) -> &mut dyn crate::Clipboard {
        &mut self.clipboard
    }

    fn system_theme(&self) -> platform::SystemTheme {
        unsafe {
            let dark_mode = isDarkMode();
            if dark_mode == YES {
                platform::SystemTheme::Dark
            } else {
                platform::SystemTheme::Light
            }
        }
    }

    fn dispatch_delegate(&self) -> Arc<dyn platform::DispatchDelegate> {
        self.dispatch_delegate.clone()
    }

    fn show_native_platform_modal(&self, id: ModalId, modal: AlertDialog) {
        let alert = create_native_platform_modal(modal);
        unsafe {
            let _: () = msg_send![app::get_warp_app(), showModal: alert modalId: id];
        }
    }

    fn register_global_shortcut(&self, shortcut: Keystroke) {
        unsafe {
            for shortcut_key in Keycode::keycodes_from_key_name(&shortcut.key) {
                registerGlobalHotkey(shortcut_key.0.into(), modifier_code(&shortcut).into());
            }
        }
    }

    fn unregister_global_shortcut(&self, shortcut: &Keystroke) {
        unsafe {
            for shortcut_key in Keycode::keycodes_from_key_name(&shortcut.key) {
                unregisterGlobalHotkey(shortcut_key.0.into(), modifier_code(shortcut).into());
            }
        }
    }

    fn set_dock_icon_visible(&self, visible: bool) {
        // NSApplicationActivationPolicyRegular = 0, NSApplicationActivationPolicyAccessory = 1.
        let policy: i64 = if visible { 0 } else { 1 };
        dispatch::Queue::main().exec_async(move || unsafe {
            let _: BOOL = msg_send![NSApp(), setActivationPolicy: policy];
        });
    }

    fn terminate_app(&self, termination_mode: TerminationMode) {
        // Execute `[NSApp terminate]` asynchronously on the main thread to
        // ensure we don't accidentally run into any double-borrow errors.
        dispatch::Queue::main().exec_async(move || unsafe {
            match termination_mode {
                // ContentTransferred windows have already moved their content to another
                // window (e.g. during tab drag), so they can close immediately without
                // prompting the user for confirmation.
                TerminationMode::ForceTerminate | TerminationMode::ContentTransferred => {
                    let _: () = msg_send![NSApp(), setForceTermination];
                }
                TerminationMode::Cancellable => {}
            }
            let _: () = msg_send![NSApp(), terminate: nil];
        });
    }

    fn is_screen_reader_enabled(&self) -> Option<bool> {
        unsafe { Some(isVoiceOverEnabled() == YES) }
    }

    fn microphone_access_state(&self) -> MicrophoneAccessState {
        unsafe {
            let cls = class!(AVCaptureDevice);
            // "soun" is not a typo, it's the correct constant name.
            let media_type_audio = make_nsstring("soun");

            // AVAuthorizationStatus constants:
            // 0 = AVAuthorizationStatusNotDetermined - User has not yet made a choice
            // 1 = AVAuthorizationStatusRestricted - Restricted by system settings/parental controls
            // 2 = AVAuthorizationStatusDenied - User explicitly denied access
            // 3 = AVAuthorizationStatusAuthorized - User granted access
            let status: i32 = msg_send![cls, authorizationStatusForMediaType: media_type_audio];
            match status {
                0 => MicrophoneAccessState::NotDetermined,
                1 => MicrophoneAccessState::Restricted,
                2 => MicrophoneAccessState::Denied,
                3 => MicrophoneAccessState::Authorized,
                _ => MicrophoneAccessState::NotDetermined, // fallback
            }
        }
    }
}

#[no_mangle]
/// # Safety
/// This function is marked unsafe because it retrieves the pointer to the callback
/// function that we sent down to the Objective-C code.
pub unsafe extern "C-unwind" fn warp_on_request_notification_permissions_completed(
    result_type: NSUInteger,
    result_msg: id,
    callback: *mut c_void,
) {
    let outcome =
        super::notification::request_permissions_outcome_from_native(result_type, result_msg);
    if let Ok(outcome) = outcome {
        let callback = Box::from_raw(callback as *mut RequestNotificationPermissionsCallback);
        callback(outcome);
    }
}

#[no_mangle]
/// # Safety
/// This function is marked unsafe because it retrieves the pointer to the callback
/// function that we sent down to the Objective-C code.
pub unsafe extern "C-unwind" fn warp_on_notification_send_error(
    error_type: NSUInteger,
    error_msg: id,
    callback: *mut c_void,
) {
    let notification_error = super::notification::send_error_from_native(error_type, error_msg);
    if let Ok(notification_error) = notification_error {
        let callback = Box::from_raw(callback as *mut NotificationSendErrorCallback);
        callback(notification_error);
    }
}

impl platform::DispatchDelegate for DispatchDelegate {
    fn is_main_thread(&self) -> bool {
        let is_main_thread: BOOL = unsafe { msg_send![class!(NSThread), isMainThread] };
        is_main_thread == YES
    }

    fn run_on_main_thread(&self, task: async_task::Runnable) {
        dispatch::Queue::main().exec_async(move || {
            task.run();
        });
    }
}
