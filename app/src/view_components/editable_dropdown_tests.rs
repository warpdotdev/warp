#[cfg(feature = "voice_input")]
use voice_input::VoiceInput;
use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::text_layout::TextAlignment;
use warpui::{App, View};

use super::EditableDropdown;
use crate::auth::AuthStateProvider;
use crate::editor::{EditOrigin, Event as EditorEvent};
use crate::menu::Event as MenuEvent;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::view_components::DropdownItem;
use crate::vim_registers::VimRegisters;
use crate::workspace::ToastStack;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

#[derive(Clone, Debug, PartialEq)]
struct TestAction(u16);

fn initialize_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ToastStack);
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(VoiceInput::new);
    app.add_singleton_model(UserWorkspaces::default_mock);
}

fn add_dropdown(app: &mut App) -> warpui::ViewHandle<EditableDropdown<TestAction>> {
    let (_, dropdown) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let mut dropdown = EditableDropdown::new(ctx);
        dropdown.set_validation(
            |input| input.parse::<u16>().ok().map(TestAction),
            |_| "100".to_string(),
        );
        dropdown
    });
    dropdown
}

#[test]
fn typed_values_validate_commit_and_revert() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let dropdown = add_dropdown(&mut app);

        dropdown.update(&mut app, |dropdown, ctx| {
            dropdown.set_editor_text("175", ctx);
            dropdown.handle_editor_event(&EditorEvent::Edited(EditOrigin::UserTyped), ctx);
            assert!(dropdown.is_valid);
            dropdown.handle_editor_event(&EditorEvent::Enter, ctx);
            assert_eq!(dropdown.last_dispatched_action(), Some(&TestAction(175)));

            dropdown.set_editor_text("225", ctx);
            dropdown.handle_editor_event(&EditorEvent::Edited(EditOrigin::UserTyped), ctx);
            dropdown.handle_editor_event(&EditorEvent::Blurred, ctx);
            assert_eq!(dropdown.last_dispatched_action(), Some(&TestAction(225)));

            dropdown.set_editor_text("invalid", ctx);
            dropdown.handle_editor_event(&EditorEvent::Edited(EditOrigin::UserTyped), ctx);
            assert!(!dropdown.is_valid);
            assert!(dropdown.validation_border_fill().is_some());
            dropdown.handle_editor_event(&EditorEvent::Enter, ctx);
            assert_eq!(dropdown.editor_text(ctx), "100");
            assert!(dropdown.is_valid);
            assert!(dropdown.validation_border_fill().is_none());
            assert_eq!(dropdown.last_dispatched_action(), Some(&TestAction(225)));

            dropdown.set_editor_text("300", ctx);
            dropdown.handle_editor_event(&EditorEvent::Escape, ctx);
            assert_eq!(dropdown.editor_text(ctx), "100");
            assert_eq!(dropdown.last_dispatched_action(), Some(&TestAction(225)));
            dropdown.handle_editor_event(&EditorEvent::Blurred, ctx);
            assert_eq!(dropdown.last_dispatched_action(), Some(&TestAction(225)));

            dropdown.set_editor_text("invalid", ctx);
            dropdown.handle_editor_event(&EditorEvent::Edited(EditOrigin::UserTyped), ctx);
            dropdown.handle_editor_event(&EditorEvent::Blurred, ctx);
            assert_eq!(dropdown.editor_text(ctx), "100");
            assert_eq!(dropdown.last_dispatched_action(), Some(&TestAction(225)));
        });
    })
}

#[test]
fn editor_text_is_center_aligned() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let dropdown = add_dropdown(&mut app);

        dropdown.read(&app, |dropdown, ctx| {
            assert_eq!(
                dropdown.editor.as_ref(ctx).text_alignment(),
                TextAlignment::Center
            );
        });
    })
}

#[test]
fn presets_and_external_values_stay_synchronized() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let dropdown = add_dropdown(&mut app);

        dropdown.update(&mut app, |dropdown, ctx| {
            let values = [
                50, 60, 70, 80, 90, 100, 110, 125, 150, 175, 200, 225, 250, 300, 350,
            ];
            dropdown.set_items(
                values
                    .into_iter()
                    .map(|value| DropdownItem::new(format!("{value}%"), TestAction(value)))
                    .collect(),
                ctx,
            );
            assert_eq!(dropdown.items_len(ctx), values.len());
            assert_eq!(
                dropdown.item_labels(ctx),
                values
                    .into_iter()
                    .map(|value| format!("{value}%"))
                    .collect::<Vec<_>>()
            );

            dropdown.set_selected_by_action(TestAction(175), ctx);
            assert_eq!(dropdown.selected_item_label().as_deref(), Some("175%"));
            assert_eq!(dropdown.editor_text(ctx), "175%");
            dropdown.select_action_and_close(&TestAction(350), ctx);
            assert_eq!(dropdown.last_dispatched_action(), Some(&TestAction(350)));

            dropdown.set_value_text("142%", ctx);
            dropdown.set_selected_by_action(TestAction(142), ctx);
            assert_eq!(dropdown.selected_item_label(), None);
            assert_eq!(dropdown.editor_text(ctx), "142%");

            dropdown.dropdown.update(ctx, |menu, ctx| {
                menu.set_selected_by_index(0, ctx);
            });
            dropdown.handle_menu_event(&MenuEvent::ItemSelected, ctx);
            assert_eq!(dropdown.editor_text(ctx), "142%");
        });

        dropdown.read(&app, |dropdown, ctx| {
            dropdown.render(ctx);
        });
    })
}
