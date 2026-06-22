use warpui::integration::TestStep;
use warpui::{async_assert, SingletonEntity};

use crate::integration_testing::view_getters::workspace_view;
use crate::undo_close::UndoCloseStack;
use crate::workspace::Workspace;

/// Mock pressing a button on the Warp-native quit modal. Note that this modal is currently only
/// used on Linux, not macOS.
pub fn press_native_modal_button(button_index: usize) -> TestStep {
    TestStep::new("Press a native modal button")
        .with_action(move |app, _, _data| {
            let active_window = app
                .read(|ctx| ctx.windows().active_window())
                .expect("no active window");
            let workspace = workspace_view(app, active_window);
            app.update(|ctx| {
                assert!(
                    workspace.as_ref(ctx).is_native_quit_modal_open(ctx),
                    "Native modal should be open"
                );
                Workspace::press_native_modal_button(&workspace, button_index, ctx);
            });
        })
        .add_assertion(|app, window_id| {
            let workspace = workspace_view(app, window_id);
            workspace.read(app, |workspace, ctx| {
                async_assert!(
                    !workspace.is_native_quit_modal_open(ctx),
                    "Native modal is still open"
                )
            })
        })
}

/// Trigger undo close (restore closed pane/tab/window) action.
pub fn trigger_undo_close() -> TestStep {
    TestStep::new("Trigger undo close").with_action(move |app, _, _data| {
        app.update(|ctx| {
            UndoCloseStack::handle(ctx).update(ctx, |stack, model_ctx| {
                stack.undo_close(model_ctx);
            });
        });
    })
}
