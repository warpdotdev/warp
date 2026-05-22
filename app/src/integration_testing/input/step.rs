use pathfinder_geometry::vector::Vector2F;
use warpui::integration::TestStep;
use warpui::windowing::WindowManager;
use warpui::SingletonEntity;

use crate::integration_testing::step::new_step_with_default_assertions;
use crate::integration_testing::terminal::assert_context_menu_is_open;
use crate::integration_testing::view_getters::single_terminal_view;
use crate::terminal::view::TerminalAction;

pub fn open_input_context_menu() -> TestStep {
    new_step_with_default_assertions("Open input context menu")
        .with_action(move |app, _, _| {
            let window_id = app.read(|ctx| {
                WindowManager::as_ref(ctx)
                    .active_window()
                    .expect("no active window")
            });
            let terminal_view_id = single_terminal_view(app, window_id).id();
            app.dispatch_typed_action(
                window_id,
                &[terminal_view_id],
                &TerminalAction::OpenInputContextMenu {
                    position: Vector2F::new(8.5, 8.5),
                },
            );
        })
        .add_assertion(assert_context_menu_is_open(true))
}
