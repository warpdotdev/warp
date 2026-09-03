use settings::Setting as _;
use warpui::integration::TestStep;
use warpui::{SingletonEntity, ViewHandle};

use super::assert_git_operation_state_menu_is_open;
use crate::context_chips::ContextChipKind;
use crate::context_chips::display_chip::{DisplayChip, DisplayChipAction};
use crate::context_chips::prompt::{PromptConfiguration, PromptSelection};
use crate::integration_testing::step::new_step_with_default_assertions;
use crate::settings::WarpPromptSeparator;
use crate::terminal::session_settings::SessionSettings;

/// Switches the prompt to a custom chip selection that includes
/// `GitOperationState`, exercising the same settings path a user hits when
/// customizing their prompt chips. `GitOperationState` is opt-in (not part
/// of the default prompt), so tests that need to observe it must enable it
/// explicitly.
pub fn enable_git_operation_state_chip() -> TestStep {
    new_step_with_default_assertions("Enable Git Operation State chip").with_action(|app, _, _| {
        SessionSettings::handle(app).update(app, |session_settings, ctx| {
            let config = PromptConfiguration::from_chips(
                [
                    ContextChipKind::WorkingDirectory,
                    ContextChipKind::ShellGitBranch,
                    ContextChipKind::GitOperationState,
                ],
                false,
                WarpPromptSeparator::None,
            );
            let _ = session_settings
                .saved_prompt
                .set_value(PromptSelection::CustomChipSelection(config), ctx);
        });
    })
}

/// Opens the `GitOperationState` chip's dropdown menu by dispatching the same
/// `DisplayChipAction::ToggleMenu` action a real click on the chip sends,
/// exercising the real production click-handling path rather than computing
/// pixel coordinates for a dynamically-positioned chip.
pub fn open_git_operation_state_chip_menu() -> TestStep {
    new_step_with_default_assertions("Open Git Operation State chip menu")
        .with_action(|app, window_id, _| {
            let chips: Vec<ViewHandle<DisplayChip>> =
                app.views_of_type(window_id).unwrap_or_default();
            let chip_id = chips.iter().find_map(|chip| {
                let is_git_operation_state = chip.read(app, |chip, _ctx| {
                    *chip.chip_kind() == ContextChipKind::GitOperationState
                });
                is_git_operation_state.then(|| chip.id())
            });
            if let Some(chip_id) = chip_id {
                app.dispatch_typed_action(window_id, &[chip_id], &DisplayChipAction::ToggleMenu);
            }
        })
        .add_assertion(assert_git_operation_state_menu_is_open(true))
}
