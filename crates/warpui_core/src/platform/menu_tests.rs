//! Tests for the `Menu` struct — specifically the `is_window_menu()` marker.
//!
//! Background: `is_window_menu()` used to compare the title against the literal
//! string `"Window"`. Once the macOS Window menu title is localized via
//! `menu_label()` (e.g. `"Окно"`, `"창"`), that comparison silently failed and
//! `NSApplication::setWindowsMenu` was never called, dropping the OS-provided
//! window-menu items (Enter Full Screen, tiling, window list). The fix adds an
//! explicit `is_window_menu` flag set via `Menu::new_window_menu()`, keeping
//! the title-string comparison as a back-compat fallback.

use super::*;

#[test]
fn new_window_menu_marks_is_window_menu() {
    let menu = Menu::new_window_menu("Окно", vec![]);
    assert!(
        menu.is_window_menu(),
        "Window menu constructed via new_window_menu() must be detected even with a localized title"
    );
}

#[test]
fn new_window_menu_keeps_title() {
    let menu = Menu::new_window_menu("Окно", vec![]);
    assert_eq!(menu.title, "Окно");
}

#[test]
fn plain_new_is_not_window_menu() {
    let menu = Menu::new("Edit", vec![]);
    assert!(!menu.is_window_menu());
}

#[test]
fn plain_new_with_localized_title_is_not_window_menu() {
    // A non-Window menu that happens to have a localized title must NOT be
    // mistaken for the Window menu.
    let menu = Menu::new("Файл", vec![]);
    assert!(!menu.is_window_menu());
}

#[test]
fn legacy_window_title_string_fallback_still_detected() {
    // Back-compat: a Window menu constructed the old way (Menu::new("Window", ...))
    // must still be detected, so any caller that hasn't migrated yet keeps working.
    let menu = Menu::new("Window", vec![]);
    assert!(menu.is_window_menu());
}
