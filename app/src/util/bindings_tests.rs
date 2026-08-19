use warpui::App;
use warpui::keymap::{EditableBinding, Keystroke, Trigger};
use warpui::platform::OperatingSystem;

use crate::terminal;
use crate::util::bindings::{
    CustomAction, keybinding_name_to_display_string, trigger_to_keystroke,
};
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
            use crate::pane_group::TOGGLE_MAXIMIZE_PANE_BINDING_NAME;

            // The toggle-maximize-pane action is registered as an editable binding so
            // it can be assigned a shortcut in Settings → Keyboard shortcuts.
            assert!(
                ctx.editable_bindings()
                    .any(|binding| binding.name == TOGGLE_MAXIMIZE_PANE_BINDING_NAME),
                "{TOGGLE_MAXIMIZE_PANE_BINDING_NAME} should be registered as an editable binding"
            );

            // It ships with a mac-only default shortcut (cmd-shift-enter) via its custom
            // action; other platforms have no default until the user assigns one. Either
            // way, whatever resolves here is what the pane header menu item surfaces.
            let default = keybinding_name_to_display_string(TOGGLE_MAXIMIZE_PANE_BINDING_NAME, ctx);
            if OperatingSystem::get().is_mac() {
                assert_eq!(Some("⇧⌘⏎"), default.as_deref());
            } else {
                assert_eq!(None, default);
            }

            // A reassigned shortcut resolves to its display string on every platform.
            ctx.set_custom_trigger(
                TOGGLE_MAXIMIZE_PANE_BINDING_NAME.to_owned(),
                Trigger::Keystrokes(vec![Keystroke::parse("cmd-shift-M").unwrap()]),
            );

            let displayed_keybinding = if OperatingSystem::get().is_mac() {
                "⇧⌘M"
            } else {
                "Shift Logo M"
            };
            assert_eq!(
                Some(displayed_keybinding),
                keybinding_name_to_display_string(TOGGLE_MAXIMIZE_PANE_BINDING_NAME, ctx)
                    .as_deref()
            );
        });
    });
}

#[test]
fn test_clearing_custom_action_binding_leaves_no_keystroke_behind() {
    // Regression test for GH#15309: `workspace::mod::init` used to also register a
    // non-editable `FixedBinding` for `CustomAction::ToggleProjectExplorer` (for the macOS
    // menu). Because bindings with a `Trigger::Custom` are matched by tag rather than by name,
    // that hidden binding kept resolving a keystroke (alt-1 on non-mac platforms) even after
    // the user cleared the editable `workspace:left_panel_project_explorer` binding in
    // Settings > Keyboard shortcuts, making the shortcut impossible to actually remove.
    //
    // This test exercises the general mechanism: any action exposed via `with_custom_action`
    // must not also be registered as a separate `FixedBinding` sharing the same `CustomAction`,
    // or clearing the editable binding will not actually free up the keystroke.
    App::test((), |mut app| async move {
        app.update(|ctx| {
            const BINDING_NAME: &str = "test:toggle_project_explorer";
            let tag = CustomAction::ToggleProjectExplorer as isize;

            ctx.register_editable_bindings([EditableBinding::new(
                BINDING_NAME,
                "Left Panel: Project explorer",
                WorkspaceAction::ToggleProjectExplorer,
            )
            .with_custom_action(CustomAction::ToggleProjectExplorer)]);

            let expected_default = if OperatingSystem::get().is_mac() {
                "⌃1"
            } else {
                "Alt 1"
            };
            assert_eq!(
                Some(expected_default),
                keybinding_name_to_display_string(BINDING_NAME, ctx).as_deref()
            );

            // Simulate the user clicking "Clear" in Settings > Keyboard shortcuts (see
            // `KeybindingsView::remove_keystroke`).
            ctx.set_custom_trigger(BINDING_NAME.to_owned(), Trigger::Empty);
            assert_eq!(
                None,
                keybinding_name_to_display_string(BINDING_NAME, ctx),
                "clearing the editable binding should leave no default keystroke behind"
            );

            // No binding -- editable or fixed -- should still resolve a keystroke for this
            // custom action once the editable binding has been cleared.
            let leftover_keystrokes: Vec<_> = ctx
                .custom_action_bindings()
                .filter(|binding| {
                    matches!(binding.trigger, Trigger::Custom(t) if *t == tag)
                        || matches!(binding.original_trigger, Some(Trigger::Custom(t)) if *t == tag)
                })
                .filter_map(|binding| trigger_to_keystroke(binding.trigger))
                .collect();
            assert!(
                leftover_keystrokes.is_empty(),
                "expected no binding to still resolve a keystroke for ToggleProjectExplorer \
                 after clearing the editable binding, found: {leftover_keystrokes:?}"
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
