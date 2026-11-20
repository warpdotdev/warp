use warp_core::ui::appearance::Appearance;
use warpui::App;
use warpui::platform::WindowStyle;

use super::FilterableDropdown;
use crate::auth::AuthStateProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::view_components::DropdownItem;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Hovering a pinned footer (the repo picker's "+ Add new repo...") calls
/// `reset_selection` so no row stays highlighted, then `restore_selection`
/// when the pointer leaves. Neither call may change the cached selection
/// shown in the closed dropdown.
#[test]
fn test_reset_and_restore_selection_keep_cached_item() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SyncedInputState::mock());
        app.add_singleton_model(|_| VimRegisters::new());
        app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
        app.add_singleton_model(UserWorkspaces::default_mock);

        let (_, dropdown) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut dropdown = FilterableDropdown::<()>::new(ctx);
            dropdown.set_items(
                vec![
                    DropdownItem::new("alpha", ()),
                    DropdownItem::new("beta", ()),
                    DropdownItem::new("gamma", ()),
                ],
                ctx,
            );
            dropdown.set_selected_by_name("beta", ctx);
            dropdown
        });

        dropdown.update(&mut app, |dropdown, ctx| {
            assert_eq!(dropdown.menu_selected_index_for_test(ctx), Some(1));

            dropdown.reset_selection(ctx);
            assert_eq!(dropdown.menu_selected_index_for_test(ctx), None);
            assert_eq!(dropdown.selected_item_label().as_deref(), Some("beta"));

            dropdown.restore_selection(ctx);
            assert_eq!(dropdown.menu_selected_index_for_test(ctx), Some(1));
            assert_eq!(dropdown.selected_item_label().as_deref(), Some("beta"));
        });
    })
}
