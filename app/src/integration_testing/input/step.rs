use pathfinder_geometry::vector::Vector2F;
use warpui::integration::TestStep;
use warpui::windowing::WindowManager;
use warpui::SingletonEntity;

use crate::ai::blocklist::agent_view::AgentInputFooterEvent;
use crate::integration_testing::input::{inline_model_selector_is_open, input_is_empty};
use crate::integration_testing::step::new_step_with_default_assertions;
use crate::integration_testing::terminal::assert_context_menu_is_open;
use crate::integration_testing::view_getters::{single_input_view_for_tab, single_terminal_view};
use crate::terminal::input::models::InlineModelSelectorTab;
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

/// Toggles the inline model selector by emitting the same footer event the model
/// chip emits when clicked, exercising the real `Input` event-handling path.
pub fn toggle_inline_model_selector_from_chip() -> TestStep {
    new_step_with_default_assertions("Toggle inline model selector from model chip").with_action(
        |app, window_id, _| {
            let input = single_input_view_for_tab(app, window_id, 0);
            let footer = input.read(app, |view, _| view.agent_input_footer().clone());
            footer.update(app, |_, ctx| {
                ctx.emit(AgentInputFooterEvent::ToggleInlineModelSelector {
                    initial_tab: InlineModelSelectorTab::BaseAgent,
                });
            });
        },
    )
}

/// Opens the inline model selector from the model chip and asserts it opened with
/// a cleared input buffer (so the input can be used to search models).
pub fn open_inline_model_selector_from_chip() -> TestStep {
    toggle_inline_model_selector_from_chip()
        .add_named_assertion(
            "Inline model selector is open",
            inline_model_selector_is_open(0),
        )
        .add_named_assertion("Prompt is cleared for model search", input_is_empty(0))
}
