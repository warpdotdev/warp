use warpui::App;
use warpui::keymap::{Context, DescriptionContext, EditableBinding, Keystroke, Trigger};
use warpui::platform::OperatingSystem;

use crate::terminal;
use crate::util::bindings::{
    BindingGroup, keybinding_name_to_display_string, trigger_to_keystroke,
};
use crate::workspace::WorkspaceAction;
use crate::workspace::view::tests::initialize_app;

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

#[test]
fn test_cycle_active_tab_color_binding_contract() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.update(|ctx| {
            let binding = ctx
                .editable_bindings()
                .find(|binding| binding.name == "workspace:cycle_active_tab_color")
                .expect("cycle active tab color should be registered as an editable binding");

            assert_eq!(
                binding
                    .description
                    .resolve(ctx, DescriptionContext::Default),
                "Cycle Current Tab Color"
            );
            assert_eq!(binding.group, Some(BindingGroup::Settings.as_str()));
            assert_eq!(binding.trigger, &Trigger::Empty);
            assert!(matches!(
                binding.action.as_any().downcast_ref::<WorkspaceAction>(),
                Some(WorkspaceAction::CycleActiveTabColor)
            ));

            let workspace_context = Context {
                set: ["Workspace"].into(),
                ..Default::default()
            };
            assert!(binding.in_context(&workspace_context));
            assert!(!binding.in_context(&Context::default()));
        });
    });
}
