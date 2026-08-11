use std::boxed::Box;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;

use cocoa::base::{id, nil};
use lazy_static::lazy_static;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{
    NSApplication, NSControlStateValue, NSDownArrowFunctionKey, NSEndFunctionKey,
    NSEventModifierFlags, NSF1FunctionKey, NSF2FunctionKey, NSF3FunctionKey, NSF4FunctionKey,
    NSF5FunctionKey, NSF6FunctionKey, NSF7FunctionKey, NSF8FunctionKey, NSF9FunctionKey,
    NSF10FunctionKey, NSF11FunctionKey, NSF12FunctionKey, NSF13FunctionKey, NSF14FunctionKey,
    NSF15FunctionKey, NSF16FunctionKey, NSF17FunctionKey, NSF18FunctionKey, NSF19FunctionKey,
    NSF20FunctionKey, NSHomeFunctionKey, NSInsertFunctionKey, NSLeftArrowFunctionKey, NSMenu,
    NSMenuItem, NSPageDownFunctionKey, NSPageUpFunctionKey, NSRightArrowFunctionKey,
    NSUpArrowFunctionKey,
};
use objc2_foundation::{NSInteger, NSString, ns_string};
use warpui_core::actions::StandardAction;
use warpui_core::keymap::Keystroke;
use warpui_core::platform::menu::{
    ItemTriggeredCallback, Menu, MenuBar, MenuItem, MenuItemProperties, MenuItemPropertyChanges,
    UpdateMenuItemCallback,
};

use super::app::callback_dispatcher;

lazy_static! {
    /// A mac-menu-specific map of key names to special characters used for the keyboard shortcuts
    /// in the mac menus
    static ref MENU_KEY_EQUIVALENTS: HashMap<&'static str, char> = {
        fn to_char(key: u32) -> char {
            char::from_u32(key).unwrap()
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
#[unsafe(no_mangle)]
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

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_menu_item_triggered(_item: id, ctx: *mut c_void) {
    let func = &MenuItemData::read_context(ctx).triggered;
    callback_dispatcher().menu_item_triggered(func);
}

#[unsafe(no_mangle)]
extern "C-unwind" fn warp_menu_item_deallocated(ctx: *mut c_void) {
    MenuItemData::consume_context(ctx)
}

// Declarations of functions implemented in ObjC files.
// These signatures must be manually synced - there's no type checking here.
unsafe extern "C" {
    fn make_delegated_menu(title: id) -> id;
    fn make_warp_custom_menu_item(ctx: *mut c_void) -> id;
    fn set_menu_item_submenu(item: id, submenu: id);
    fn make_services_menu_item() -> id;
}

struct StandardMenuItemProperties {
    /// The menu item title.
    title: &'static NSString,
    /// The selector to invoke.
    action: Sel,
    /// The key equivalent string, or empty for none.
    shortcut: &'static NSString,
    modifiers: NSEventModifierFlags,
}

enum KeyEquivalent {
    Static(&'static NSString),
    Dynamic(Retained<NSString>),
}

impl KeyEquivalent {
    fn as_nsstring(&self) -> &NSString {
        match self {
            Self::Static(value) => value,
            Self::Dynamic(value) => value,
        }
    }
}

// Get properties from a standard action.
fn resolve_standard_action(action: StandardAction) -> StandardMenuItemProperties {
    let cmd = NSEventModifierFlags::Command;
    let option = NSEventModifierFlags::Option;
    let ctrl = NSEventModifierFlags::Control;
    let none = NSEventModifierFlags::empty();

    fn make(
        title: &'static NSString,
        action: Sel,
        modifiers: NSEventModifierFlags,
        shortcut: &'static NSString,
    ) -> StandardMenuItemProperties {
        StandardMenuItemProperties {
            title,
            action,
            shortcut,
            modifiers,
        }
    }

    match action {
        StandardAction::Close => make(
            ns_string!("Close Window"),
            sel!(performClose:),
            none,
            ns_string!(""),
        ),
        StandardAction::Quit => make(
            ns_string!("Quit Warp"),
            sel!(terminate:),
            cmd,
            ns_string!("q"),
        ),
        StandardAction::Hide => make(ns_string!("Hide Warp"), sel!(hide:), cmd, ns_string!("h")),
        StandardAction::HideOtherApps => make(
            ns_string!("Hide Others"),
            sel!(hideOtherApplications:),
            cmd | option,
            ns_string!("h"),
        ),
        StandardAction::ShowAllApps => make(
            ns_string!("Show All"),
            sel!(unhideAllApplications:),
            none,
            ns_string!(""),
        ),
        StandardAction::Minimize => make(
            ns_string!("Minimize"),
            sel!(performMiniaturize:),
            cmd,
            ns_string!("m"),
        ),
        StandardAction::Zoom => make(ns_string!("Zoom"), sel!(performZoom:), none, ns_string!("")),
        StandardAction::BringAllToFront => make(
            ns_string!("Bring All to Front"),
            sel!(arrangeInFront:),
            none,
            ns_string!(""),
        ),
        StandardAction::ToggleFullScreen => make(
            ns_string!("ToggleFullScreen"),
            sel!(toggleFullScreen:),
            cmd | ctrl,
            ns_string!("f"),
        ),
        StandardAction::Paste => make(ns_string!("Paste"), sel!(paste:), none, ns_string!("")),
    }
}

/// Determine the key equivalent for the given keystroke
fn resolve_key_equivalent(keystroke: Option<&Keystroke>) -> (KeyEquivalent, NSEventModifierFlags) {
    let mut flags = NSEventModifierFlags::empty();

    let keystroke = match keystroke {
        Some(value) => value,
        None => return (KeyEquivalent::Static(ns_string!("")), flags),
    };

    let key_equivalent = match MENU_KEY_EQUIVALENTS.get(keystroke.key.as_str()) {
        Some(c) => KeyEquivalent::Dynamic(NSString::from_str(&String::from(*c))),
        None => KeyEquivalent::Dynamic(NSString::from_str(&keystroke.key)),
    };

    for (is_set, flag) in [
        (keystroke.cmd, NSEventModifierFlags::Command),
        (keystroke.alt, NSEventModifierFlags::Option),
        (keystroke.shift, NSEventModifierFlags::Shift),
        (keystroke.ctrl, NSEventModifierFlags::Control),
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
    // so this is a hot path. A local pool bounds peak memory for the temporaries AppKit
    // produces here (e.g. inside `setTitle:`/`setKeyEquivalent:`) without relying on the
    // outer AppKit pool.
    autoreleasepool(|_| unsafe {
        let menu_item = &*item.cast::<NSMenuItem>();
        if let Some(name) = changes.name {
            menu_item.setTitle(&NSString::from_str(&name));
        }
        if let Some(keystroke) = changes.keystroke {
            let (key_equivalent, modifiers) = resolve_key_equivalent(keystroke.as_ref());
            menu_item.setKeyEquivalent(key_equivalent.as_nsstring());
            menu_item.setKeyEquivalentModifierMask(modifiers);
        }
        if let Some(disabled) = changes.disabled {
            menu_item.setEnabled(!disabled);
        }
        if let Some(checked) = changes.checked {
            // NSControlStateValue has Off as 0, On as 1, Mixed as -1.
            let control_state = i64::from(checked) as NSControlStateValue;
            menu_item.setState(control_state);
        }
        if let Some(submenu) = changes.submenu {
            let nsmenu = match submenu {
                Some(menu_items) => make_submenu(menu_items),
                None => nil,
            };
            set_menu_item_submenu(item, nsmenu);
        }
    });
}

unsafe fn make_submenu(menu_items: Vec<MenuItem>) -> id {
    unsafe {
        let nsmenu = make_delegated_menu(ns_string!("") as *const NSString as id);
        let nsmenu_ref = &*nsmenu.cast::<NSMenu>();
        for menu_item in menu_items {
            nsmenu_ref.addItem(&*make_menu_item(menu_item).cast::<NSMenuItem>());
        }
        nsmenu
    }
}

unsafe fn make_menu_item(menu_item: MenuItem) -> id {
    unsafe {
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
                let mtm = MainThreadMarker::new_unchecked();
                let properties = resolve_standard_action(standard_action);
                let nsmenu_item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    properties.title,
                    Some(properties.action),
                    properties.shortcut,
                );
                nsmenu_item.setKeyEquivalentModifierMask(properties.modifiers);
                nsmenu_item.setTag(standard_action as NSInteger);
                Retained::autorelease_ptr(nsmenu_item) as id
            }
            MenuItem::Separator => Retained::autorelease_ptr(NSMenuItem::separatorItem(
                MainThreadMarker::new_unchecked(),
            )) as id,
            MenuItem::Services => make_services_menu_item(),
        }
    }
}

/// \return an autoreleased NSMenuItem with a submenu represented by \p menu.
// This supports creating the top-level menu bar.
unsafe fn make_top_level_menu_item(menu: Menu) -> id {
    unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        let nsmenu = make_delegated_menu(Retained::as_ptr(&NSString::from_str(&menu.title)) as id);
        let nsmenu = &*nsmenu.cast::<NSMenu>();

        if menu.is_window_menu() {
            // `setWindowsMenu` gives us all the default window menu items like
            // 'Enter Full Screen' and 'Tile Window to Left of Screen'.
            NSApplication::sharedApplication(mtm).setWindowsMenu(Some(nsmenu));
        }

        for menu_item in menu.menu_items {
            nsmenu.addItem(&*make_menu_item(menu_item).cast::<NSMenuItem>());
        }

        let menuitem = NSMenuItem::new(mtm);
        menuitem.setSubmenu(Some(nsmenu));
        Retained::autorelease_ptr(menuitem) as id
    }
}

/// \return an NSMenu representing the given menu bar.
pub unsafe fn make_main_menu(menubar: MenuBar) -> Retained<NSMenu> {
    unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        let main_menu = NSMenu::new(mtm);
        for menu in menubar.menus {
            main_menu.addItem(&*make_top_level_menu_item(menu).cast::<NSMenuItem>());
        }
        main_menu
    }
}

/// \return an NSMenu representing the given dock menu.
pub unsafe fn make_dock_menu(menu: Menu) -> Retained<NSMenu> {
    unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        let dock_menu = NSMenu::new(mtm);
        for item in menu.menu_items {
            dock_menu.addItem(&*make_menu_item(item).cast::<NSMenuItem>());
        }
        dock_menu
    }
}

#[cfg(test)]
#[path = "menus_tests.rs"]
mod tests;
