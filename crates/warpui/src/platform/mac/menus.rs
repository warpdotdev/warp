use std::boxed::Box;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;

use cocoa::appkit::{
    NSApp, NSDownArrowFunctionKey, NSEndFunctionKey, NSEventModifierFlags, NSF10FunctionKey,
    NSF11FunctionKey, NSF12FunctionKey, NSF13FunctionKey, NSF14FunctionKey, NSF15FunctionKey,
    NSF16FunctionKey, NSF17FunctionKey, NSF18FunctionKey, NSF19FunctionKey, NSF1FunctionKey,
    NSF20FunctionKey, NSF2FunctionKey, NSF3FunctionKey, NSF4FunctionKey, NSF5FunctionKey,
    NSF6FunctionKey, NSF7FunctionKey, NSF8FunctionKey, NSF9FunctionKey, NSHomeFunctionKey,
    NSInsertFunctionKey, NSLeftArrowFunctionKey, NSMenu, NSMenuItem, NSPageDownFunctionKey,
    NSPageUpFunctionKey, NSRightArrowFunctionKey, NSUpArrowFunctionKey,
};
use cocoa::base::{id, nil, selector};
use cocoa::foundation::{NSArray, NSAutoreleasePool, NSInteger};
use lazy_static::lazy_static;
use objc::runtime::{NO, YES};
use objc::{msg_send, sel, sel_impl};
use warpui_core::actions::StandardAction;
use warpui_core::keymap::Keystroke;
use warpui_core::platform::menu::{
    ItemTriggeredCallback, Menu, MenuBar, MenuItem, MenuItemProperties, MenuItemPropertyChanges,
    UpdateMenuItemCallback,
};

use super::app::callback_dispatcher;
use super::make_nsstring;

lazy_static! {
    /// A mac-menu-specific map of key names to special characters used for the keyboard shortcuts
    /// in the mac menus
    static ref MENU_KEY_EQUIVALENTS: HashMap<&'static str, char> = {
        fn to_char(key: u16) -> char {
            char::from_u32(key.into()).unwrap()
        }

        HashMap::from([
            ("up", to_char(NSUpArrowFunctionKey)),
            ("down", to_char(NSDownArrowFunctionKey)),
            ("left", to_char(NSLeftArrowFunctionKey)),
            ("right", to_char(NSRightArrowFunctionKey)),
            ("home", to_char(NSHomeFunctionKey)),
            ("end", to_char(NSEndFunctionKey)),
            ("pageup", to_char(NSPageUpFunctionKey)),
            ("pagedown", to_char(NSPageDownFunctionKey)),
            ("enter", '\n'),
            ("tab", '\t'),
            ("insert", to_char(NSInsertFunctionKey)),
            ("f1", to_char(NSF1FunctionKey)),
            ("f2", to_char(NSF2FunctionKey)),
            ("f3", to_char(NSF3FunctionKey)),
            ("f4", to_char(NSF4FunctionKey)),
            ("f5", to_char(NSF5FunctionKey)),
            ("f6", to_char(NSF6FunctionKey)),
            ("f7", to_char(NSF7FunctionKey)),
            ("f8", to_char(NSF8FunctionKey)),
            ("f9", to_char(NSF9FunctionKey)),
            ("f10", to_char(NSF10FunctionKey)),
            ("f11", to_char(NSF11FunctionKey)),
            ("f12", to_char(NSF12FunctionKey)),
            ("f13", to_char(NSF13FunctionKey)),
            ("f14", to_char(NSF14FunctionKey)),
            ("f15", to_char(NSF15FunctionKey)),
            ("f16", to_char(NSF16FunctionKey)),
            ("f17", to_char(NSF17FunctionKey)),
            ("f18", to_char(NSF18FunctionKey)),
            ("f19", to_char(NSF19FunctionKey)),
            ("f20", to_char(NSF20FunctionKey)),
            // The following values are the inverse of `ui/src/platform/mac/event.rs` mappings
            ("numpadenter", to_char(0x03)),
            ("escape", to_char(0x1b)),
            // Note: Backspace and Delete have different characters for the menu key equivalents
            // than they send when they are pressed. See the discussion in the Apple docs:
            // https://developer.apple.com/documentation/appkit/nsmenuitem/1514842-keyequivalent?language=objc
            ("backspace", to_char(0x08)),
            ("delete", to_char(0x7F)),
        ])
    };
}

/// Data associated with a custom NSMenuItem.
struct MenuItemData {
    /// Properties of the menu item.
    /// These could be computed from the menu item but we trust AppKit does not change them.
    props: RefCell<MenuItemProperties>,

    /// Callback when the menu item is triggered by the user.
    triggered: ItemTriggeredCallback,

    /// Callback when the menu item needs updating.
    update: UpdateMenuItemCallback,
}

impl MenuItemData {
    /// Convert self to a Cocoa context pointer, including the refcount.
    /// This should be balanced by consume_cocoa_context.
    fn into_context(self: Rc<MenuItemData>) -> *mut c_void {
        Box::into_raw(Box::new(self)) as *mut c_void
    }

    /// Read out from the Cocoa context pointer, without consuming its refcount.
    fn read_context(ctx: *const c_void) -> Rc<MenuItemData> {
        unsafe {
            let ptr = &*(ctx as *const Rc<MenuItemData>);
            ptr.clone()
        }
    }

    /// Balances a call from to_cocoa_context.
    fn consume_context(ctx: *mut c_void) {
        unsafe { std::mem::drop(Box::from_raw(ctx as *mut Rc<MenuItemData>)) }
    }
}

/// We hand Cocoa a void* which is really an unwrapped Box<Rc<MenuItemData>>.
/// The NSMenuItem logically holds a reference count on this Rc, which is balanced in our dealloc callback below.
/// The following functions are invoked from Cocoa.
#[no_mangle]
extern "C-unwind" fn warp_menu_item_needs_update(item: id, ctx: *mut c_void) {
    let ctx = MenuItemData::read_context(ctx);
    let props: MenuItemProperties = ctx.props.borrow().clone();
    let func = &ctx.update;

    let mut updated_properties = callback_dispatcher().update_menu_item(|ctx| func(&props, ctx));

    // Always re-apply the disabled state even when the updater has no opinion.
    // AppKit's modal sessions (e.g. [NSAlert runModal]) can externally disable
    // menu items, and items whose updaters return `disabled: None` would never
    // call setEnabled: to restore the correct state. On macOS with the quake
    // mode (non-activating panel) window, this results in permanently disabled
    // items after a modal is dismissed. Default to enabled — updaters that want
    // an item disabled must say so explicitly.
    if updated_properties.disabled.is_none() {
        updated_properties.disabled = Some(false);
    }

    // Update any changed properties.
    ctx.props.borrow_mut().apply(&updated_properties);
    unsafe { apply_changes(updated_properties, item) };
}

#[no_mangle]
extern "C-unwind" fn warp_menu_item_triggered(_item: id, ctx: *mut c_void) {
    let func = &MenuItemData::read_context(ctx).triggered;
    callback_dispatcher().menu_item_triggered(func);
}

#[no_mangle]
extern "C-unwind" fn warp_menu_item_deallocated(ctx: *mut c_void) {
    MenuItemData::consume_context(ctx)
}

// Declarations of functions implemented in ObjC files.
// These signatures must be manually synced - there's no type checking here.
extern "C" {
    fn make_delegated_menu(title: id) -> id;
    fn make_warp_custom_menu_item(ctx: *mut c_void) -> id;
    fn set_menu_item_submenu(item: id, submenu: id);
    fn make_services_menu_item() -> id;
}

struct StandardMenuItemProperties {
    title: &'static str,    // menu item title
    action: &'static str,   // the selector name
    shortcut: &'static str, // the key equivalent string, or empty for none
    modifiers: NSEventModifierFlags,
}

// Get properties from a standard action.
fn resolve_standard_action(action: StandardAction) -> StandardMenuItemProperties {
    let cmd = NSEventModifierFlags::NSCommandKeyMask;
    let option = NSEventModifierFlags::NSAlternateKeyMask;
    let ctrl = NSEventModifierFlags::NSControlKeyMask;
    let none = NSEventModifierFlags::empty();

    fn make(
        title: &'static str,
        action: &'static str,
        modifiers: NSEventModifierFlags,
        shortcut: &'static str,
    ) -> StandardMenuItemProperties {
        StandardMenuItemProperties {
            title,
            action,
            shortcut,
            modifiers,
        }
    }

    match action {
        StandardAction::Close => make("Close Window", "performClose:", none, ""),
        StandardAction::Quit => make("Quit Warp", "terminate:", cmd, "q"),
        StandardAction::Hide => make("Hide Warp", "hide:", cmd, "h"),
        StandardAction::HideOtherApps => {
            make("Hide Others", "hideOtherApplications:", cmd | option, "h")
        }
        StandardAction::ShowAllApps => make("Show All", "unhideAllApplications:", none, ""),
        StandardAction::Minimize => make("Minimize", "performMiniaturize:", cmd, "m"),
        StandardAction::Zoom => make("Zoom", "performZoom:", none, ""),
        StandardAction::BringAllToFront => make("Bring All to Front", "arrangeInFront:", none, ""),
        StandardAction::ToggleFullScreen => {
            make("ToggleFullScreen", "toggleFullScreen:", cmd | ctrl, "f")
        }
        StandardAction::Paste => make("Paste", "paste:", none, ""),
    }
}

/// Determine the key equivalent for the given keystroke
fn resolve_key_equivalent(keystroke: Option<&Keystroke>) -> (id, NSEventModifierFlags) {
    let mut flags = NSEventModifierFlags::empty();

    let keystroke = match keystroke {
        Some(value) => value,
        None => return (make_nsstring(""), flags),
    };

    let key_equivalent = match MENU_KEY_EQUIVALENTS.get(keystroke.key.as_str()) {
        Some(c) => make_nsstring(String::from(*c)),
        None => make_nsstring(&keystroke.key),
    };

    for (is_set, flag) in [
        (keystroke.cmd, NSEventModifierFlags::NSCommandKeyMask),
        (keystroke.alt, NSEventModifierFlags::NSAlternateKeyMask),
        (keystroke.shift, NSEventModifierFlags::NSShiftKeyMask),
        (keystroke.ctrl, NSEventModifierFlags::NSControlKeyMask),
    ] {
        if is_set {
            flags |= flag
        }
    }

    (key_equivalent, flags)
}

// Apply any differences between the two states to the menu item.
unsafe fn apply_changes(changes: MenuItemPropertyChanges, item: id) {
    // Wrap in a local autorelease pool: AppKit invokes `warp_menu_item_needs_update`
    // on every menu validation (per menu open and per keystroke for shortcut matching),
    // so this is a hot path. A local pool bounds peak memory for the NSString temporaries
    // created here (item title, key equivalent) without relying on the outer AppKit pool.
    let pool = NSAutoreleasePool::new(nil);
    if let Some(name) = changes.name {
        let _: () = msg_send![item, setTitle: make_nsstring(name)];
    }
    if let Some(keystroke) = changes.keystroke {
        let (key_equivalent, modifiers) = resolve_key_equivalent(keystroke.as_ref());
        let _: () = msg_send![item, setKeyEquivalent: key_equivalent];
        let _: () = msg_send![item, setKeyEquivalentModifierMask: modifiers];
    }
    if let Some(disabled) = changes.disabled {
        let enabled = if disabled { NO } else { YES };
        let _: () = msg_send![item, setEnabled: enabled];
    }
    if let Some(checked) = changes.checked {
        // NSControlStateValue has Off as 0, On as 1, Mixed as -1.
        let control_state: NSInteger = i64::from(checked);
        let _: () = msg_send![item, setState: control_state];
    }
    if let Some(submenu) = changes.submenu {
        let nsmenu = submenu
            .map(|menu_items| make_submenu(menu_items))
            .unwrap_or(nil);
        set_menu_item_submenu(item, nsmenu);
    }
    pool.drain();
}

unsafe fn make_submenu(menu_items: Vec<MenuItem>) -> id {
    let nsmenu = make_delegated_menu(make_nsstring(""));
    for menu_item in menu_items {
        nsmenu.addItem_(make_menu_item(menu_item));
    }
    nsmenu
}

unsafe fn make_menu_item(menu_item: MenuItem) -> id {
    match menu_item {
        MenuItem::Custom(custom_menu_item) => {
            let props = custom_menu_item.properties;
            let data = Rc::new(MenuItemData {
                props: RefCell::new(props.clone()),
                triggered: custom_menu_item.callback,
                update: custom_menu_item.updater,
            });

            let nsmenu_item = make_warp_custom_menu_item(MenuItemData::into_context(data));

            // Set initial properties for the item.
            apply_changes(
                MenuItemPropertyChanges::for_new_item(props, custom_menu_item.submenu),
                nsmenu_item,
            );

            nsmenu_item
        }
        MenuItem::Standard(standard_action) => {
            let properties = resolve_standard_action(standard_action);
            let nsmenu_item = NSMenuItem::alloc(nil)
                .initWithTitle_action_keyEquivalent_(
                    make_nsstring(properties.title),
                    selector(properties.action),
                    make_nsstring(properties.shortcut),
                )
                .autorelease();
            nsmenu_item.setKeyEquivalentModifierMask_(properties.modifiers);
            let _: id = msg_send![nsmenu_item, setTag: standard_action as libc::c_long];
            nsmenu_item
        }
        MenuItem::Separator => NSMenuItem::separatorItem(nil),
        MenuItem::Services => make_services_menu_item(),
    }
}

/// \return an autoreleased NSMenuItem with a submenu represented by \p menu.
// This supports creating the top-level menu bar.
unsafe fn make_top_level_menu_item(menu: Menu) -> id {
    let nsmenu = make_delegated_menu(make_nsstring(&menu.title));

    if menu.is_window_menu() {
        // `setWindowsMenu` gives us all the default window menu items like
        // 'Enter Full Screen' and 'Tile Window to Left of Screen'.
        let () = msg_send![NSApp(), setWindowsMenu: nsmenu];
    }

    for menu_item in menu.menu_items {
        nsmenu.addItem_(make_menu_item(menu_item));
    }

    let menuitem = NSMenuItem::alloc(nil).init().autorelease();
    menuitem.setSubmenu_(nsmenu);
    menuitem
}

/// \return an autoreleased NSMenu representing the given menu bar.
pub unsafe fn make_main_menu(menubar: MenuBar) -> id {
    let main_menu = NSMenu::alloc(nil).init().autorelease();
    for menu in menubar.menus {
        main_menu.addItem_(make_top_level_menu_item(menu));
    }
    main_menu
}

/// \return an autoreleased NSMenu representing the given dock menu.
pub unsafe fn make_dock_menu(menu: Menu) -> id {
    let dock_menu = NSMenu::alloc(nil).init().autorelease();
    for item in menu.menu_items {
        dock_menu.addItem_(make_menu_item(item));
    }
    dock_menu
}

#[cfg(test)]
#[path = "menus_tests.rs"]
mod tests;
