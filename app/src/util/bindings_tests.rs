use warpui::App;
use warpui::keymap::{EditableBinding, Keystroke, Trigger};
use warpui::platform::OperatingSystem;

use crate::terminal;
use crate::util::bindings::{
    TAB_SWITCH_SHORTCUT_BINDING_NAMES, keybinding_name_to_display_string, tab_switch_shortcut_hint,
    trigger_to_keystroke,
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

/// Registers the eight `TAB_SWITCH_SHORTCUT_BINDING_NAMES` as editable bindings
/// bound to `cmdorctrl-1` .. `cmdorctrl-8`, mirroring their real registration in
/// `workspace::init` without pulling in that function's other side effects.
fn register_tab_switch_shortcut_bindings(ctx: &mut warpui::AppContext) {
    ctx.register_editable_bindings(TAB_SWITCH_SHORTCUT_BINDING_NAMES.iter().enumerate().map(
        |(position, name)| {
            EditableBinding::new(
                name,
                format!("Switch to tab {}", position + 1),
                WorkspaceAction::ActivateTabByNumber(position + 1),
            )
            .with_key_binding(format!("cmdorctrl-{}", position + 1))
        },
    ));
}

#[test]
fn test_tab_switch_shortcut_hint_resolves_each_of_the_first_eight_positions() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_tab_switch_shortcut_bindings(ctx);

            for position in 0..8 {
                let expected = if OperatingSystem::get().is_mac() {
                    format!("⌘{}", position + 1)
                } else {
                    format!("Ctrl {}", position + 1)
                };
                assert_eq!(
                    Some(expected),
                    tab_switch_shortcut_hint(position, ctx),
                    "position {position}"
                );
            }
        });
    });
}

#[test]
fn test_tab_switch_shortcut_hint_is_none_past_the_eighth_position() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_tab_switch_shortcut_bindings(ctx);

            assert_eq!(None, tab_switch_shortcut_hint(8, ctx));
            assert_eq!(None, tab_switch_shortcut_hint(100, ctx));
        });
    });
}

#[test]
fn test_tab_switch_shortcut_hint_is_none_when_unbound() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_tab_switch_shortcut_bindings(ctx);

            // The user removed the binding for the third tab entirely.
            ctx.set_custom_trigger(
                TAB_SWITCH_SHORTCUT_BINDING_NAMES[2].to_owned(),
                Trigger::Empty,
            );

            assert_eq!(None, tab_switch_shortcut_hint(2, ctx));
            // Other positions are unaffected.
            let expected = if OperatingSystem::get().is_mac() {
                "⌘4"
            } else {
                "Ctrl 4"
            };
            assert_eq!(Some(expected.to_owned()), tab_switch_shortcut_hint(3, ctx));
        });
    });
}

#[test]
fn test_tab_switch_shortcut_hint_is_none_for_a_multi_keystroke_chord() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_tab_switch_shortcut_bindings(ctx);

            // The user reassigned the fifth tab's shortcut to a two-key chord, which
            // can't be rendered as a single keystroke hint.
            ctx.set_custom_trigger(
                TAB_SWITCH_SHORTCUT_BINDING_NAMES[4].to_owned(),
                Trigger::Keystrokes(vec![
                    Keystroke::parse("ctrl-k").unwrap(),
                    Keystroke::parse("b").unwrap(),
                ]),
            );

            assert_eq!(None, tab_switch_shortcut_hint(4, ctx));
        });
    });
}

#[test]
fn test_tab_switch_shortcut_hint_reflects_a_remap_to_a_different_single_keystroke() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_tab_switch_shortcut_bindings(ctx);

            let default_expected = if OperatingSystem::get().is_mac() {
                "⌘3".to_owned()
            } else {
                "Ctrl 3".to_owned()
            };
            assert_eq!(Some(default_expected), tab_switch_shortcut_hint(2, ctx));

            // The user remaps the third tab's shortcut away from `cmdorctrl-3` to a
            // different single keystroke: the displayed hint must follow the remap
            // immediately (the same lookup is redone on every render), and other
            // positions must stay on their own bindings.
            ctx.set_custom_trigger(
                TAB_SWITCH_SHORTCUT_BINDING_NAMES[2].to_owned(),
                Trigger::Keystrokes(vec![Keystroke::parse("cmd-shift-9").unwrap()]),
            );

            let remapped_expected = if OperatingSystem::get().is_mac() {
                "⇧⌘9"
            } else {
                "Shift Logo 9"
            };
            assert_eq!(
                Some(remapped_expected.to_owned()),
                tab_switch_shortcut_hint(2, ctx)
            );

            let position_four_expected = if OperatingSystem::get().is_mac() {
                "⌘4"
            } else {
                "Ctrl 4"
            };
            assert_eq!(
                Some(position_four_expected.to_owned()),
                tab_switch_shortcut_hint(3, ctx)
            );
        });
    });
}
