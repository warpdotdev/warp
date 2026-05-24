use crate::actions::StandardAction;
use crate::keymap::Keystroke;
use crate::AppContext;

pub enum MenuItem {
    Custom(CustomMenuItem),
    Separator,
    Standard(StandardAction),

    /// Services is a system-defined standard menu on macOS.
    #[cfg(target_os = "macos")]
    Services,
}

// We allow dead_code here because the title is only read when compiling the
// Mac bits.
#[allow(dead_code)]
pub struct Menu {
    pub title: String,
    pub menu_items: Vec<MenuItem>,
}

impl Menu {
    pub fn new<S: Into<String>>(title: S, menu_items: Vec<MenuItem>) -> Self {
        Menu {
            title: title.into(),
            menu_items,
        }
    }

    pub fn is_window_menu(&self) -> bool {
        &self.title == "Window"
    }
}

#[allow(dead_code)]
pub struct MenuBar {
    pub menus: Vec<Menu>,
}

impl MenuBar {
    pub fn new(menus: Vec<Menu>) -> Self {
        MenuBar { menus }
    }
}

/// Properties of a menu item.
#[derive(Clone, Debug, Default)]
pub struct MenuItemProperties {
    pub name: String,
    pub keystroke: Option<Keystroke>,
    pub disabled: bool,
    /// If set, the item gets a checkmark.
    pub checked: bool,
}

impl MenuItemProperties {
    pub fn apply(&mut self, changes: &MenuItemPropertyChanges) {
        if let Some(name) = &changes.name {
            self.name.clone_from(name);
        }
        if let Some(keystroke) = changes.keystroke.as_ref() {
            self.keystroke.clone_from(keystroke);
        }
        if let Some(disabled) = changes.disabled {
            self.disabled = disabled;
        }
        if let Some(checked) = changes.checked {
            self.checked = checked;
        }
    }
}

/// Changes to properties of a menu item.
#[derive(Default)]
pub struct MenuItemPropertyChanges {
    pub name: Option<String>,
    pub keystroke: Option<Option<Keystroke>>,
    pub disabled: Option<bool>,
    pub checked: Option<bool>,
    pub submenu: Option<Submenu>,
}

impl MenuItemPropertyChanges {
    /// Returns a struct that unconditionally sets all properties, to be used
    /// when initializing a menu item for the first time.
    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub fn for_new_item(props: MenuItemProperties, submenu: Submenu) -> Self {
        Self {
            name: Some(props.name),
            keystroke: Some(props.keystroke),
            disabled: Some(props.disabled),
            checked: Some(props.checked),
            submenu: Some(submenu),
        }
    }
}

pub type ItemTriggeredCallback = Box<dyn Fn(&mut AppContext)>;

/// A callback function that is invoked when we may want to update
/// a menu item.
///
/// It receives a reference to the current set of properties, and
/// returns a structure indicating which properties should be updated
/// and what the new values should be.
pub type UpdateMenuItemCallback =
    Box<dyn Fn(&MenuItemProperties, &mut AppContext) -> MenuItemPropertyChanges>;

pub type Submenu = Option<Vec<MenuItem>>;

pub struct CustomMenuItem {
    pub properties: MenuItemProperties,
    pub callback: ItemTriggeredCallback,
    pub updater: UpdateMenuItemCallback,
    pub submenu: Submenu,
}

impl CustomMenuItem {
    /// Construct a new CustomMenuItem with the given \p name.
    /// \p callback will be invoked when the user triggers the menu item.
    /// \p updater is invoked when the menu is opened or otherwise needs to be updated.
    /// The function receives a bag of properties and may mutate it.
    /// Any properties that are changed will be reflected in the menu item.
    pub fn new<
        Callback: 'static + Fn(&mut AppContext),
        Updater: 'static + Fn(&MenuItemProperties, &mut AppContext) -> MenuItemPropertyChanges,
    >(
        name: &str,
        callback: Callback,
        updater: Updater,
        keystroke: Option<Keystroke>,
    ) -> Self {
        Self {
            properties: MenuItemProperties {
                name: name.to_string(),
                keystroke,
                ..Default::default()
            },
            callback: Box::new(callback),
            updater: Box::new(updater),
            submenu: None,
        }
    }

    // Constructor that takes in additional submenu argument.
    pub fn new_with_submenu<
        Callback: 'static + Fn(&mut AppContext),
        Updater: 'static + Fn(&MenuItemProperties, &mut AppContext) -> MenuItemPropertyChanges,
    >(
        name: &str,
        callback: Callback,
        updater: Updater,
        keystroke: Option<Keystroke>,
        submenu: Vec<MenuItem>,
    ) -> Self {
        Self {
            properties: MenuItemProperties {
                name: name.to_string(),
                keystroke,
                ..Default::default()
            },
            callback: Box::new(callback),
            updater: Box::new(updater),
            submenu: Some(submenu),
        }
    }
}
