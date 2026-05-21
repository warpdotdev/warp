use std::borrow::Cow;
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::path::PathBuf;

use cocoa::appkit::NSApp;
use cocoa::base::{id, nil};
use cocoa::foundation::{NSArray, NSAutoreleasePool, NSData, NSString, NSUInteger, NSURL};
use futures_util::future::LocalBoxFuture;
use objc::runtime::{Object, Sel, BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl};
use warpui_core::assets::AssetProvider;
use warpui_core::integration::TestDriver;
use warpui_core::keymap::{Keystroke, Trigger};
use warpui_core::modals::{AlertDialog, ModalId};
use warpui_core::platform::app::{AppCallbackDispatcher, ApproveTerminateResult};
use warpui_core::platform::menu::{Menu, MenuBar};
use warpui_core::platform::{self, FilePickerCallback, SaveFilePickerCallback};
use warpui_core::{AppContext, Event};

use super::keycode::{Keycode, CMD_KEY, CONTROL_KEY, OPTION_KEY, SHIFT_KEY};
use super::make_nsstring;
use super::menus::{make_dock_menu, make_main_menu};
use super::window::{get_window_state, IntegrationTestWindowManager, Window, WindowManager};
use crate::platform::app::{AppBackend, AppBuilder};
use crate::platform::AsInnerMut;

pub trait NSAlert: Sized {
    unsafe fn alloc(_: Self) -> id {
        msg_send![class!(NSAlert), alloc]
    }

    unsafe fn init(self) -> id;
    unsafe fn autorelease(self) -> id;
    unsafe fn set_message_text(self, message_text: id);
    unsafe fn set_informative_text(self, informative_text: id);
    unsafe fn add_button_with_title(self, title: id);
}

impl NSAlert for id {
    unsafe fn init(self) -> id {
        msg_send![self, init]
    }

    unsafe fn autorelease(self) -> id {
        msg_send![self, autorelease]
    }

    unsafe fn set_message_text(self, message_text: id) {
        msg_send![self, setMessageText: message_text]
    }

    unsafe fn set_informative_text(self, informative_text: id) {
        msg_send![self, setInformativeText: informative_text]
    }

    unsafe fn add_button_with_title(self, title: id) {
        msg_send![self, addButtonWithTitle: title]
    }
}

pub fn create_native_platform_modal(dialog: AlertDialog) -> id {
    unsafe {
        let alert = NSAlert::autorelease(NSAlert::init(NSAlert::alloc(nil)));
        alert.set_informative_text(make_nsstring(&dialog.info_text));
        alert.set_message_text(make_nsstring(&dialog.message_text));
        for title in dialog.buttons {
            alert.add_button_with_title(make_nsstring(&title));
        }
        alert
    }
}

const RUST_WRAPPER_IVAR_NAME: &str = "rustWrapper";

extern "C" {
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

    /// Sets the macOS status item menu constructor function.
    fn set_status_item_menu_builder(
        &mut self,
        value: impl FnOnce(&mut AppContext) -> Menu + 'static,
    );

    /// Sets whether the application should show its Dock icon on launch.
    fn set_show_dock_icon_on_launch(&mut self, value: bool);
}

type MenuBarBuilderFn = Box<dyn FnOnce(&mut AppContext) -> MenuBar>;
type DockMenuBuilderFn = Box<dyn FnOnce(&mut AppContext) -> Menu>;
type StatusItemMenuBuilderFn = Box<dyn FnOnce(&mut AppContext) -> Menu>;

/// The actual application, from the perspective of the platform and the
/// main event loop.  This is the true owner of all application state.
pub struct App {
    callbacks: AppCallbackDispatcher,
    activate_on_launch: bool,
    dev_icon: Option<Cow<'static, [u8]>>,
    show_dock_icon_on_launch: bool,
    menu_bar_builder: Option<MenuBarBuilderFn>,
    dock_menu_builder: Option<DockMenuBuilderFn>,
    status_item_menu_builder: Option<StatusItemMenuBuilderFn>,
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
            status_item_menu_builder: None,
            init_fn: None,
        }
    }

    pub(in crate::platform) fn run(
        mut self,
        init_fn: impl FnOnce(&mut AppContext, LocalBoxFuture<'static, crate::App>) + 'static,
    ) {
        self.init_fn = Some(Box::new(init_fn));

        unsafe {
            let pool = NSAutoreleasePool::new(nil);

            // Get (and create, if necessary) the underlying NSApplication.
            let app: id = get_warp_app();

            let running_app: id = msg_send![class!(NSRunningApplication), currentApplication];
            let bundle_id: id = msg_send![running_app, bundleIdentifier];
            let dev_icon = if bundle_id.is_null() {
                self.dev_icon.as_ref().map(|dev_icon| {
                    let data: id = msg_send![class!(NSData), alloc];
                    let data: id = data.initWithBytes_length_(
                        dev_icon.as_ptr() as *const c_void,
                        dev_icon.len() as u64,
                    );
                    let image: id = msg_send![class!(NSImage), alloc];
                    image.initWithData_(data)
                })
            } else {
                None
            };

            let app_delegate: id = msg_send![app, delegate];

            let self_ptr = Box::into_raw(Box::new(self));
            (*app).set_ivar(RUST_WRAPPER_IVAR_NAME, self_ptr as *mut c_void);
            (*app_delegate).set_ivar(RUST_WRAPPER_IVAR_NAME, self_ptr as *mut c_void);

            if let Some(dev_icon) = dev_icon {
                let _: () = msg_send![app, setApplicationIconImage: dev_icon];
            }

            let _: () = msg_send![app, run];
            let _: () = msg_send![pool, drain];

            // App is done running when we get here, so we can reinstantiate the Box and drop it.
            drop(Box::from_raw(self_ptr));
        }
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

    fn set_status_item_menu_builder(
        &mut self,
        value: impl FnOnce(&mut AppContext) -> Menu + 'static,
    ) {
        match self.as_inner_mut() {
            AppBackend::CurrentPlatform(app) => {
                app.status_item_menu_builder = Some(Box::new(value))
            }
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
    let wrapper_ptr: *mut c_void = *object.get_ivar(RUST_WRAPPER_IVAR_NAME);
    &mut *(wrapper_ptr as *mut App)
}

pub(super) fn callback_dispatcher() -> &'static mut AppCallbackDispatcher {
    unsafe {
        let app = get_warp_app();
        let app = get_app(&mut *app);
        &mut app.callbacks
    }
}

#[no_mangle]
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

#[no_mangle]
pub unsafe extern "C-unwind" fn warp_app_will_finish_launching(this: &mut Object) {
    log::info!("application will finish launching");

    let app = get_app(this);

    if app.activate_on_launch {
        let _: () = msg_send![NSApp(), activateIgnoringOtherApps: YES];
    }

    if let Some(init_fn) = app.init_fn.take() {
        app.callbacks.initialize_app(init_fn);
    }

    let app_delegate: id = msg_send![NSApp(), delegate];

    if app.callbacks.has_internet_reachability_changed_callback() {
        let _: () = msg_send![app_delegate, setReachabilityListener];
    }

    if let Some(menu_bar_builder) = app.menu_bar_builder.take() {
        let menu_bar = app.callbacks.with_mutable_app_context(menu_bar_builder);
        let nsmenu = make_main_menu(menu_bar);
        let () = msg_send![NSApp(), setMainMenu: nsmenu];
    }

    if let Some(dock_menu_builder) = app.dock_menu_builder.take() {
        let dock_menu = app.callbacks.with_mutable_app_context(dock_menu_builder);
        let nsmenu = make_dock_menu(dock_menu);
        let _: () = msg_send![app_delegate, setDockMenu: nsmenu];
    }

    if let Some(status_item_menu_builder) = app.status_item_menu_builder.take() {
        let status_item_menu = app
            .callbacks
            .with_mutable_app_context(status_item_menu_builder);
        let nsmenu = make_dock_menu(status_item_menu);
        let _: () = msg_send![app_delegate, setStatusItemMenu: nsmenu];
    }

    let show_dock_icon = if app.show_dock_icon_on_launch {
        YES
    } else {
        NO
    };
    let _: BOOL = msg_send![app_delegate, setDockIconVisible: show_dock_icon];
}

#[no_mangle]
pub(crate) extern "C-unwind" fn warp_app_did_become_active(this: &mut Object, _: Sel, _: id) {
    let app = unsafe { get_app(this) };
    app.callbacks.app_became_active();
}

#[no_mangle]
pub(crate) extern "C-unwind" fn warp_app_internet_reachability_changed(
    this: &mut Object,
    can_reach: u8,
) {
    let is_reachable = can_reach != 0;

    let app = unsafe { get_app(this) };
    app.callbacks.internet_reachability_changed(is_reachable);
}

/// Returns whether or not we can proceed with termination.
#[no_mangle]
pub(crate) extern "C-unwind" fn warp_app_should_terminate_app(this: &mut Object) -> BOOL {
    let app = unsafe { get_app(this) };

    match app.callbacks.should_terminate_app() {
        ApproveTerminateResult::Terminate => YES,
        ApproveTerminateResult::Cancel => NO,
    }
}

/// Returns a NSAlert object if we want to show a dialog for users to confirm or
/// nil for closing the window immediately.
#[no_mangle]
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

#[no_mangle]
pub(crate) extern "C-unwind" fn warp_app_are_key_bindings_disabled_for_window(
    this: &mut Object,
    window_id: &mut Object,
) -> BOOL {
    let app = unsafe { get_app(this) };
    let window = unsafe { get_window_state(window_id) };

    let disabled = app
        .callbacks
        .with_mutable_app_context(|ctx| !ctx.key_bindings_enabled(window.id()));

    if disabled {
        YES
    } else {
        NO
    }
}

#[no_mangle]
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

    if has_binding {
        YES
    } else {
        NO
    }
}

#[no_mangle]
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

    if has_binding {
        YES
    } else {
        NO
    }
}

#[no_mangle]
pub(crate) extern "C-unwind" fn warp_app_disable_warning_modal(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.warning_modal_disabled();
}

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
extern "C-unwind" fn warp_app_did_resign_active(this: &mut Object, _: Sel, _: id) {
    let app = unsafe { get_app(this) };
    app.callbacks.app_resigned_active();
}

#[no_mangle]
extern "C-unwind" fn warp_app_will_terminate(this: &mut Object, _: Sel, _: id) {
    let app = unsafe { get_app(this) };
    app.callbacks.app_will_terminate();
}

#[no_mangle]
extern "C-unwind" fn warp_app_new_window(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.open_new_window();
}

#[no_mangle]
extern "C-unwind" fn warp_app_active_window_changed(this: &mut Object) {
    let app = unsafe { get_app(this) };
    Window::close_ime_on_active_window();
    app.callbacks
        .active_window_changed(Window::active_window_id());
}

#[no_mangle]
extern "C-unwind" fn warp_app_window_did_resize(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.window_resized();
}

#[no_mangle]
extern "C-unwind" fn warp_app_window_did_move(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.window_moved();
}

#[no_mangle]
extern "C-unwind" fn warp_app_window_will_close(this: &mut Object, window: &mut Object) {
    let app = unsafe { get_app(this) };
    let window_state = unsafe { get_window_state(window) };
    app.callbacks.window_will_close(window_state.id());
}

#[no_mangle]
extern "C-unwind" fn warp_app_screen_did_change(this: &mut Object) {
    log::info!("received NSApplicationDidChangeScreenParametersNotification");
    let app = unsafe { get_app(this) };
    app.callbacks.screen_changed();
}

#[no_mangle]
extern "C-unwind" fn cpu_awakened(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.cpu_awakened();
}

#[no_mangle]
extern "C-unwind" fn cpu_will_sleep(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.cpu_will_sleep();
}

#[no_mangle]
extern "C-unwind" fn warp_app_open_files(this: &mut Object, paths: id) {
    let paths = unsafe {
        (0..paths.count())
            .filter_map(|i| {
                let path = paths.objectAtIndex(i);
                match CStr::from_ptr(path.UTF8String() as *mut c_char).to_str() {
                    Ok(string) => Some(PathBuf::from(string)),
                    Err(err) => {
                        log::error!("error converting path to string: {err}");
                        None
                    }
                }
            })
            .collect::<Vec<_>>()
    };
    let app = unsafe { get_app(this) };
    app.callbacks.open_files(paths);
}

#[no_mangle]
extern "C-unwind" fn warp_app_open_urls(this: &mut Object, urls: id) {
    let urls = unsafe {
        (0..urls.count())
            .filter_map(|i| {
                let url = urls.objectAtIndex(i).absoluteString();
                match CStr::from_ptr(url.UTF8String() as *mut c_char).to_str() {
                    Ok(string) => Some(string.to_string()),
                    Err(err) => {
                        log::error!("error converting url to string: {err}");
                        None
                    }
                }
            })
            .collect::<Vec<_>>()
    };

    let app = unsafe { get_app(this) };
    app.callbacks.open_urls(urls);
}

#[no_mangle]
extern "C-unwind" fn warp_app_os_appearance_changed(this: &mut Object) {
    let app = unsafe { get_app(this) };
    app.callbacks.os_appearance_changed();
}

// Calls the callback with None if no file was selected
#[no_mangle]
pub(crate) extern "C-unwind" fn warp_open_panel_file_selected(urls: id, callback: *mut c_void) {
    // Start by converting the callback from a raw pointer back into a Box, to
    // avoid the memory leak that would occur if we left it in raw pointer form.
    let callback = unsafe { Box::from_raw(callback as *mut FilePickerCallback) };

    let paths = unsafe {
        (0..urls.count())
            .map(|i| {
                let file_url = urls.objectAtIndex(i);
                let file_path: id = msg_send![file_url, path];
                let slice = std::slice::from_raw_parts(
                    file_path.UTF8String() as *const std::ffi::c_uchar,
                    file_path.len(),
                );
                std::str::from_utf8_unchecked(slice).to_string()
            })
            .collect::<Vec<_>>()
    };

    if paths.is_empty() {
        log::info!("No file was selected. Dialog was cancelled.")
    }

    let app = unsafe { get_app(&mut *get_warp_app()) };
    app.callbacks.with_mutable_app_context(move |ctx| {
        callback(Ok(paths), ctx);
    });
}

// Calls the save callback with the selected path or None if cancelled
#[no_mangle]
pub(crate) extern "C-unwind" fn warp_save_panel_file_selected(url: id, callback: *mut c_void) {
    let callback = unsafe { Box::from_raw(callback as *mut SaveFilePickerCallback) };

    let path = if url.is_null() {
        None
    } else {
        unsafe {
            let file_path: id = msg_send![url, path];
            let slice = std::slice::from_raw_parts(
                file_path.UTF8String() as *const std::ffi::c_uchar,
                file_path.len(),
            );
            Some(std::str::from_utf8_unchecked(slice).to_string())
        }
    };

    if path.is_none() {
        log::info!("Save dialog was cancelled.");
    }

    let app = unsafe { get_app(&mut *get_warp_app()) };
    app.callbacks.with_mutable_app_context(move |ctx| {
        callback(path, ctx);
    });
}
