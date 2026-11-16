/// A StandardAction is one that corresponds to an action that
/// must be dispatched and handled natively by NSApp (e.g. terminate:)
/// Use CustomActions for handling Warp specific actions.
///
/// Set a 'repr' here as we store these values as tags in menu items.
#[derive(Copy, Clone, Debug, PartialEq, Eq, FromPrimitive, ToPrimitive, Hash)]
#[repr(isize)]
pub enum StandardAction {
    Close,
    Hide,
    HideOtherApps,
    ShowAllApps,
    Quit,
    Zoom,
    Minimize,
    BringAllToFront,
    ToggleFullScreen,
    Paste,
}
