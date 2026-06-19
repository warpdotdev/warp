use warpui::keymap::{EditableBinding, Keystroke, Trigger};
use warpui::platform::OperatingSystem;
use warpui::App;

use crate::terminal;
use crate::util::bindings::{keybinding_name_to_display_string, trigger_to_keystroke};
use crate::workspace::WorkspaceAction;

#[test]
fn test_keybinding_name_to_display_string() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.register_editable_bindings([
                EditableBinding::new(
                    "workspace:show_settings",
                    "Open settings",
                    WorkspaceAction::ShowSettings,
                )
                .with_key_binding("cmd-,"),
                EditableBinding::new(
                    "workspace:toggle_resource_center",
                    "Toggle Resource Center",
                    WorkspaceAction::ToggleResourceCenter,
                ),
            ]);

            let displayed_keybinding = if OperatingSystem::get().is_mac() {
                "⌘,"
            } else {
                "Logo ,"
            };
            assert_eq!(
                Some(displayed_keybinding),
                keybinding_name_to_display_string("workspace:show_settings", ctx).as_deref()
            );

            assert_eq!(
                None,
                keybinding_name_to_display_string("workspace:toggle_resource_center", ctx)
            );

            ctx.set_custom_trigger(
                "workspace:show_settings".to_owned(),
                Trigger::Keystrokes(vec![Keystroke::parse("cmd-shift-<").unwrap()]),
            );

            let displayed_keybinding = if OperatingSystem::get().is_mac() {
                "⇧⌘<"
            } else {
                "Shift Logo <"
            };
            assert_eq!(
                Some(displayed_keybinding),
                keybinding_name_to_display_string("workspace:show_settings", ctx).as_deref()
            );

            ctx.set_custom_trigger(
                "workspace:toggle_resource_center".to_owned(),
                Trigger::Keystrokes(vec![Keystroke::parse("cmd-alt-/").unwrap()]),
            );

            let expected_keybinding = if OperatingSystem::get().is_mac() {
                "⌥⌘/"
            } else {
                "Alt Logo /"
            };
            assert_eq!(
                Some(expected_keybinding),
                keybinding_name_to_display_string("workspace:toggle_resource_center", ctx)
                    .as_deref()
            );
        });
    });
}

#[test]
fn test_orchestration_cycle_bindings_are_editable() {
    App::test((), |mut app| async move {
        app.update(terminal::init);

        app.update(|ctx| {
            let next = ctx
                .editable_bindings()
                .find(|binding| binding.name == "terminal:cycle_next_orchestration_child_agent")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));
            let previous = ctx
                .editable_bindings()
                .find(|binding| binding.name == "terminal:cycle_previous_orchestration_child_agent")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));

            assert_eq!(next, Keystroke::parse("ctrl-alt-]").ok());
            assert_eq!(previous, Keystroke::parse("ctrl-alt-[").ok());
        });
    });
}

#[test]
fn test_toggle_maximize_pane_binding_is_editable() {
    App::test((), |mut app| async move {
        app.update(crate::pane_group::init);

        app.update(|ctx| {
            // The maximize / minimize pane action is registered as an editable binding so
            // it can be assigned a shortcut in Settings → Keyboard shortcuts.
            assert!(
                ctx.editable_bindings()
                    .any(|binding| binding.name == "pane_group:toggle_maximize_pane"),
                "pane_group:toggle_maximize_pane should be registered as an editable binding"
            );

            // It ships without a default keystroke, so nothing is shown next to the menu
            // item until the user assigns one.
            assert_eq!(
                None,
                keybinding_name_to_display_string("pane_group:toggle_maximize_pane", ctx)
            );

            // Once a shortcut is assigned, it resolves to a display string that the pane
            // header menu item surfaces.
            ctx.set_custom_trigger(
                "pane_group:toggle_maximize_pane".to_owned(),
                Trigger::Keystrokes(vec![Keystroke::parse("cmd-shift-m").unwrap()]),
            );

            let displayed_keybinding = if OperatingSystem::get().is_mac() {
                "⇧⌘M"
            } else {
                "Shift Logo M"
            };
            assert_eq!(
                Some(displayed_keybinding),
                keybinding_name_to_display_string("pane_group:toggle_maximize_pane", ctx)
                    .as_deref()
            );
        });
    });
}

#[test]
fn test_terminal_page_scroll_bindings_are_editable() {
    App::test((), |mut app| async move {
        app.update(terminal::init);

        app.update(|ctx| {
            let page_up = ctx
                .editable_bindings()
                .find(|binding| binding.name == "terminal:scroll_up_one_page")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));
            let page_down = ctx
                .editable_bindings()
                .find(|binding| binding.name == "terminal:scroll_down_one_page")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));

            assert_eq!(page_up, Keystroke::parse("pageup").ok());
            assert_eq!(page_down, Keystroke::parse("pagedown").ok());
        });
    });
}
