use std::borrow::Cow;
use std::ffi::CStr;
use std::os::raw::c_void;
use std::path::PathBuf;

use cocoa::base::id;
use futures_util::future::LocalBoxFuture;
use objc::runtime::{BOOL, NO, Object, Sel, YES};
use objc2::rc::{Retained, autoreleasepool};
use objc2::{AnyThread, MainThreadMarker, msg_send};
use objc2_app_kit::{NSAlert, NSApplication, NSImage, NSRunningApplication};
use objc2_foundation::{NSArray, NSData, NSString, NSUInteger, NSURL};
use warp_errors::report_error;
use warpui_core::assets::AssetProvider;
use warpui_core::integration::TestDriver;
use warpui_core::keymap::{Keystroke, Trigger};
use warpui_core::modals::{AlertDialog, ModalId};
use warpui_core::platform::app::{
    AppCallbackDispatcher, ApproveTerminateResult, TerminationRequestSource,
};
use warpui_core::platform::menu::{Menu, MenuBar};
use warpui_core::platform::{self, FilePickerCallback, SaveFilePickerCallback};
use warpui_core::{AppContext, Event};

use super::keycode::{CMD_KEY, CONTROL_KEY, Keycode, OPTION_KEY, SHIFT_KEY};
use super::menus::{make_dock_menu, make_main_menu};
use super::window::{IntegrationTestWindowManager, Window, WindowManager, get_window_state};
use crate::platform::AsInnerMut;
use crate::platform::app::{AppBackend, AppBuilder};

/// Builds a native macOS alert dialog from an [`AlertDialog`].
pub fn create_native_platform_modal(dialog: AlertDialog) -> Retained<NSAlert> {
    // SAFETY: native modals are constructed on the main thread.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let alert = NSAlert::new(mtm);
    alert.setInformativeText(&NSString::from_str(&dialog.info_text));
    alert.setMessageText(&NSString::from_str(&dialog.message_text));
    for title in dialog.buttons {
        alert.addButtonWithTitle(&NSString::from_str(&title));
    }
    alert
}

const RUST_WRAPPER_IVAR_NAME: &str = "rustWrapper";

unsafe extern "C" {
    // Implemented in ObjC to get the warp NSApplication subclass.
    pub(super) fn get_warp_app() -> id;
}

/// An extension trait defining additional configurability for
/// applications when running on macOS.
pub trait AppExt {
    /// Sets whether or not the application should be activated
    /// when it is launched.
    fn set_activate_on_launch(&mut self, value: bool);

    /// Sets the application icon which should be used when running
    /// without an application bundle.
    fn set_dev_icon(&mut self, value: Cow<'static, [u8]>);

    /// Sets the main menu bar constructor function.
    fn set_menu_bar_builder(&mut self, value: impl FnOnce(&mut AppContext) -> MenuBar + 'static);

    /// Sets the macOS dock menu constructor function.
    fn set_dock_menu_builder(&mut self, value: impl FnOnce(&mut AppContext) -> Menu + 'static);

    /// Sets whether the application should show its Dock icon on launch.
    fn set_show_dock_icon_on_launch(&mut self, value: bool);
}

type MenuBarBuilderFn = Box<dyn FnOnce(&mut AppContext) -> MenuBar>;
type DockMenuBuilderFn = Box<dyn FnOnce(&mut AppContext) -> Menu>;

/// The actual application, from the perspective of the platform and the
/// main event loop.  This is the true owner of all application state.
pub struct App {
    callbacks: AppCallbackDispatcher,
    activate_on_launch: bool,
    dev_icon: Option<Cow<'static, [u8]>>,
    show_dock_icon_on_launch: bool,
    menu_bar_builder: Option<MenuBarBuilderFn>,
    dock_menu_builder: Option<DockMenuBuilderFn>,
    init_fn: Option<platform::app::AppInitCallbackFn>,
}

impl App {
    pub(in crate::platform) fn new(
        callbacks: platform::app::AppCallbacks,
        assets: Box<dyn AssetProvider>,
        test_driver: Option<&TestDriver>,
    ) -> Self {
        let platform_delegate: Box<dyn platform::Delegate> = if test_driver.is_some() {
            Box::new(
                super::delegate::IntegrationTestDelegate::new()
                    .expect("should not fail to create platform delegate"),
            )
        } else {
            Box::new(
                super::delegate::AppDelegate::new()
                    .expect("should not fail to create platform delegate"),
            )
        };

        let window_manager: Box<dyn platform::WindowManager> = if test_driver.is_some() {
            Box::new(IntegrationTestWindowManager::new())
        } else {
            Box::new(WindowManager::new())
        };

        let ui_app = crate::App::new(
            platform_delegate,
            window_manager,
            Box::new(super::fonts::FontDB::new()),
            assets,
        )
        .expect("should not fail to construct application");

        Self {
            callbacks: AppCallbackDispatcher::new(callbacks, ui_app),
            activate_on_launch: true,
            dev_icon: None,
            show_dock_icon_on_launch: true,
            menu_bar_builder: None,
            dock_menu_builder: None,
            init_fn: None,
        }
    }

    pub(in crate::platform) fn run(
        mut self,
        init_fn: impl FnOnce(&mut AppContext, LocalBoxFuture<'static, crate::App>) + 'static,
    ) {
        self.init_fn = Some(Box::new(init_fn));

        // The autorelease pool stays open for the whole app lifetime (`run` blocks
        // until termination).
        autoreleasepool(|_| {
            // Get (and create, if necessary) the underlying NSApplication.
            // SAFETY: `get_warp_app()` returns the warp NSApplication subclass instance.
            let app_ptr = unsafe { get_warp_app() };
            let app = unsafe { &*app_ptr.cast::<NSApplication>() };

            // When running without an application bundle (dev builds), install the
            // provided dev icon as the app icon. This is a dev-only path: if the icon
            // bytes fail to decode we skip the call below and leave the default icon.
            let running_app = NSRunningApplication::currentApplication();
            let dev_icon: Option<Retained<NSImage>> = if running_app.bundleIdentifier().is_none() {
                self.dev_icon.as_ref().and_then(|dev_icon| {
                    let data = NSData::with_bytes(dev_icon);
                    NSImage::initWithData(NSImage::alloc(), &data)
                })
            } else {
                None
            };

            // SAFETY: the app and its delegate are exclusively owned here, so writing
            // the `rustWrapper` ivar and messaging them is sound.
            unsafe {
                let app_delegate = app.delegate().expect("the warp app always has a delegate");

                let self_ptr = Box::into_raw(Box::new(self));
                (*app_ptr).set_ivar(RUST_WRAPPER_IVAR_NAME, self_ptr as *mut c_void);
                (*Retained::as_ptr(&app_delegate).cast::<Object>().cast_mut())
                    .set_ivar(RUST_WRAPPER_IVAR_NAME, self_ptr as *mut c_void);

                if let Some(dev_icon) = dev_icon {
                    app.setApplicationIconImage(Some(&dev_icon));
                }

                app.run();

                // App is done running when we get here, so we can reinstantiate the Box and drop it.
                drop(Box::from_raw(self_ptr));
            }
        });
    }
}

impl AppExt for AppBuilder {
    fn set_activate_on_launch(&mut self, value: bool) {
        match self.as_inner_mut() {
            AppBackend::CurrentPlatform(app) => app.activate_on_launch = value,
            AppBackend::Headless(_) => (),
        }
    }

    fn set_dev_icon(&mut self, value: Cow<'static, [u8]>) {
        match self.as_inner_mut() {
            AppBackend::CurrentPlatform(app) => app.dev_icon = Some(value),
            AppBackend::Headless(_) => (),
        }
    }

    fn set_menu_bar_builder(&mut self, value: impl FnOnce(&mut AppContext) -> MenuBar + 'static) {
        match self.as_inner_mut() {
            AppBackend::CurrentPlatform(app) => app.menu_bar_builder = Some(Box::new(value)),
            AppBackend::Headless(_) => (),
        }
    }

    fn set_dock_menu_builder(&mut self, value: impl FnOnce(&mut AppContext) -> Menu + 'static) {
        match self.as_inner_mut() {
            AppBackend::CurrentPlatform(app) => app.dock_menu_builder = Some(Box::new(value)),
            AppBackend::Headless(_) => (),
        }
    }

    fn set_show_dock_icon_on_launch(&mut self, value: bool) {
        match self.as_inner_mut() {
            AppBackend::CurrentPlatform(app) => app.show_dock_icon_on_launch = value,
            AppBackend::Headless(_) => (),
        }
    }
}

unsafe fn get_app(object: &mut Object) -> &mut App {
    unsafe {
        let wrapper_ptr: *mut c_void = *object.get_ivar(RUST_WRAPPER_IVAR_NAME);
        &mut *(wrapper_ptr as *mut App)
    }
}

pub(super) fn callback_dispatcher() -> &'static mut AppCallbackDispatcher {
    unsafe {
        let app = get_warp_app();
        let app = get_app(&mut *app);
        &mut app.callbacks
    }
}

#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_send_global_keybinding(
    this: &mut Object,
    modifiers: NSUInteger,
    key_code: NSUInteger,
) {
    let keystroke = {
        let modifiers = modifiers as u16;
        let shift_key_pressed = (modifiers & SHIFT_KEY) > 0;
        Keycode(key_code as u16)
            .try_to_key_name(shift_key_pressed)
            .map(|key| Keystroke {
                ctrl: (modifiers & CONTROL_KEY) > 0,
                alt: (modifiers & OPTION_KEY) > 0,
                shift: shift_key_pressed,
                cmd: (modifiers & CMD_KEY) > 0,
                meta: false,
                key,
            })
    };

    if let Some(keystroke) = keystroke {
        let app = unsafe { get_app(this) };
        app.callbacks.global_shortcut_triggered(keystroke);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn warp_app_will_finish_launching(this: &mut Object) {
    unsafe {
        log::info!("application will finish launching");

        let app = get_app(this);

        // SAFETY: this delegate callback runs on the main thread.
        let mtm = MainThreadMarker::new_unchecked();
        let ns_app = NSApplication::sharedApplication(mtm);

        if app.activate_on_launch {
            ns_app.activateIgnoringOtherApps(true);
        }

        if let Some(init_fn) = app.init_fn.take() {
            app.callbacks.initialize_app(init_fn);
        }

        let app_delegate = ns_app
            .delegate()
            .expect("the warp app always has a delegate");

        if app.callbacks.has_internet_reachability_changed_callback() {
            // `setReachabilityListener` is a custom warp app-delegate selector.
            let _: () = msg_send![&*app_delegate, setReachabilityListener];
        }

        if let Some(menu_bar_builder) = app.menu_bar_builder.take() {
            let menu_bar = app.callbacks.with_mutable_app_context(menu_bar_builder);
            let nsmenu = make_main_menu(menu_bar);
            ns_app.setMainMenu(Some(&nsmenu));
        }

        if let Some(dock_menu_builder) = app.dock_menu_builder.take() {
            let dock_menu = app.callbacks.with_mutable_app_context(dock_menu_builder);
            let nsmenu = make_dock_menu(dock_menu);
            // `setDockMenu:` is a custom warp app-delegate selector.
            let _: () = msg_send![&*app_delegate, setDockMenu: &*nsmenu];
        }

        let show_dock_icon = if app.show_dock_icon_on_launch {
            YES
        } else {
            NO
        };
        // `setDockIconVisible:` is a custom warp app-delegate selector.
        let _: BOOL = msg_send![&*app_delegate, setDockIconVisible: show_dock_icon];
    }
}

#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_did_become_active(this: &mut Object, _: Sel, _: id) {
    let app = unsafe { get_app(this) };
    app.callbacks.app_became_active();
}

#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_internet_reachability_changed(
    this: &mut Object,
    can_reach: u8,
) {
    let is_reachable = can_reach != 0;

    let app = unsafe { get_app(this) };
    app.callbacks.internet_reachability_changed(is_reachable);
}

/// Returns whether or not we can proceed with termination.
#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_should_terminate_app(
    this: &mut Object,
    system_initiated: BOOL,
) -> BOOL {
    let app = unsafe { get_app(this) };

    let source = if system_initiated != NO {
        TerminationRequestSource::System
    } else {
        TerminationRequestSource::User
    };
    match app.callbacks.should_terminate_app(source) {
        ApproveTerminateResult::Terminate => YES,
        ApproveTerminateResult::Cancel => NO,
    }
}

/// Returns a NSAlert object if we want to show a dialog for users to confirm or
/// nil for closing the window immediately.
#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_should_close_window(
    this: &mut Object,
    window_id: &mut Object,
) -> BOOL {
    let app = unsafe { get_app(this) };
    let window = unsafe { get_window_state(window_id) };

    match app.callbacks.should_close_window(window.id()) {
        ApproveTerminateResult::Terminate => YES,
        ApproveTerminateResult::Cancel => NO,
    }
}

#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_are_key_bindings_disabled_for_window(
    this: &mut Object,
    window_id: &mut Object,
) -> BOOL {
    let app = unsafe { get_app(this) };
    let window = unsafe { get_window_state(window_id) };

    let disabled = app
        .callbacks
        .with_mutable_app_context(|ctx| !ctx.key_bindings_enabled(window.id()));

    if disabled { YES } else { NO }
}

#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_has_binding_for_keystroke(
    this: &mut Object,
    event: id,
) -> BOOL {
    let app = unsafe { get_app(this) };
    let warp_event = unsafe { super::event::from_native(event, None, false) };

    let Some(Event::KeyDown { keystroke, .. }) = warp_event else {
        return NO;
    };
    let has_binding = app.callbacks.with_mutable_app_context(|ctx| {
        ctx.get_key_bindings().any(|binding| {
            if let Trigger::Keystrokes(keystrokes) = binding.trigger {
                keystrokes.len() == 1 && keystrokes[0] == keystroke
            } else {
                false
            }
        })
    });

    if has_binding { YES } else { NO }
}

#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_has_custom_action_for_keystroke(
    this: &mut Object,
    event: id,
) -> BOOL {
    let app = unsafe { get_app(this) };
    let warp_event = unsafe { super::event::from_native(event, None, false) };

    let Some(Event::KeyDown { keystroke, .. }) = warp_event else {
        return NO;
    };
    let has_binding = app.callbacks.with_mutable_app_context(|ctx| {
        ctx.custom_action_bindings()
            .any(|binding| match binding.trigger {
                Trigger::Keystrokes(keystrokes) => {
                    keystrokes.len() == 1 && keystrokes[0] == keystroke
                }
                Trigger::Custom(tag) => ctx
                    .default_keystroke_trigger_for_custom_action(*tag)
                    .is_some_and(|k| k == keystroke),
                _ => false,
            })
    });

    if has_binding { YES } else { NO }
}

#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_disable_warning_modal(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.warning_modal_disabled();
}

#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_process_modal_response(
    this: &mut Object,
    modal_id: ModalId,
    response: usize,
    disable_modal: bool,
) {
    let app = unsafe { get_app(this) };
    app.callbacks
        .process_platform_modal_response(modal_id, response, disable_modal);
}

#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_app_notification_clicked(
    this: &mut Object,
    date: f64,
    data: id,
) {
    let app = unsafe { get_app(this) };
    if let Ok(notification_response) =
        unsafe { super::notification::response_from_native(date as i32, data) }
    {
        app.callbacks.notification_clicked(notification_response);
    }
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_did_resign_active(this: &mut Object, _: Sel, _: id) {
    let app = unsafe { get_app(this) };
    app.callbacks.app_resigned_active();
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_will_terminate(this: &mut Object, _: Sel, _: id) {
    let app = unsafe { get_app(this) };
    app.callbacks.app_will_terminate();
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_new_window(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.open_new_window();
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_active_window_changed(this: &mut Object) {
    let app = unsafe { get_app(this) };
    Window::close_ime_on_active_window();
    app.callbacks
        .active_window_changed(Window::active_window_id());
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_window_did_resize(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.window_resized();
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_window_did_move(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.window_moved();
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_window_will_close(this: &mut Object, window: &mut Object) {
    let app = unsafe { get_app(this) };
    let window_state = unsafe { get_window_state(window) };
    app.callbacks.window_will_close(window_state.id());
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_screen_did_change(this: &mut Object) {
    log::info!("received NSApplicationDidChangeScreenParametersNotification");
    let app = unsafe { get_app(this) };
    app.callbacks.screen_changed();
}

#[unsafe(no_mangle)]
extern "C-unwind" fn cpu_awakened(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.cpu_awakened();
}

#[unsafe(no_mangle)]
extern "C-unwind" fn cpu_will_sleep(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.cpu_will_sleep();
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_open_files(this: &mut Object, paths: id) {
    // SAFETY: `paths` is an `NSArray<NSString>` of file paths.
    let paths = unsafe {
        let paths = &*paths.cast::<NSArray<NSString>>();
        (0..paths.count())
            .filter_map(|i| {
                let path = paths.objectAtIndex(i);
                match CStr::from_ptr(path.UTF8String()).to_str() {
                    Ok(string) => Some(PathBuf::from(string)),
                    Err(err) => {
                        report_error!(
                            anyhow::Error::new(err).context("error converting path to string")
                        );
                        None
                    }
                }
            })
            .collect::<Vec<_>>()
    };
    let app = unsafe { get_app(this) };
    app.callbacks.open_files(paths);
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_open_urls(this: &mut Object, urls: id) {
    // SAFETY: `urls` is an `NSArray<NSURL>`.
    let urls = unsafe {
        let urls = &*urls.cast::<NSArray<NSURL>>();
        (0..urls.count())
            .filter_map(|i| {
                let url = urls.objectAtIndex(i).absoluteString()?;
                match CStr::from_ptr(url.UTF8String()).to_str() {
                    Ok(string) => Some(string.to_string()),
                    Err(err) => {
                        report_error!(
                            anyhow::Error::new(err).context("error converting url to string")
                        );
                        None
                    }
                }
            })
            .collect::<Vec<_>>()
    };

    let app = unsafe { get_app(this) };
    app.callbacks.open_urls(urls);
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_app_os_appearance_changed(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.os_appearance_changed();
}

// Calls the callback with None if no file was selected
#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_open_panel_file_selected(urls: id, callback: *mut c_void) {
    // Start by converting the callback from a raw pointer back into a Box, to
    // avoid the memory leak that would occur if we left it in raw pointer form.
    let callback = unsafe { Box::from_raw(callback as *mut FilePickerCallback) };

    // SAFETY: `urls` is an `NSArray<NSURL>` of selected files.
    let paths = unsafe {
        let urls = &*urls.cast::<NSArray<NSURL>>();
        (0..urls.count())
            .map(|i| {
                urls.objectAtIndex(i)
                    .path()
                    .map(|file_path| file_path.to_string())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
    };

    if paths.is_empty() {
        log::info!("No file was selected. Dialog was cancelled.")
    }

    // SAFETY: `get_warp_app()` returns the warp NSApplication subclass instance.
    let app = unsafe { get_app(&mut *get_warp_app()) };
    app.callbacks.with_mutable_app_context(move |ctx| {
        callback(Ok(paths), ctx);
    });
}

// Calls the save callback with the selected path or None if cancelled
#[unsafe(no_mangle)]
pub(crate) extern "C-unwind" fn warp_save_panel_file_selected(url: id, callback: *mut c_void) {
    let callback = unsafe { Box::from_raw(callback as *mut SaveFilePickerCallback) };

    // SAFETY: `url` is null or a valid `NSURL`.
    let path = unsafe {
        url.cast::<NSURL>()
            .as_ref()
            .and_then(|url| url.path())
            .map(|file_path| file_path.to_string())
    };

    if path.is_none() {
        log::info!("Save dialog was cancelled.");
    }

    // SAFETY: `get_warp_app()` returns the warp NSApplication subclass instance.
    let app = unsafe { get_app(&mut *get_warp_app()) };
    app.callbacks.with_mutable_app_context(move |ctx| {
        callback(path, ctx);
    });
}
