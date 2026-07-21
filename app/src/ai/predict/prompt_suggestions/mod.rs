use std::sync::LazyLock;

use warpui::AppContext;
use warpui::keymap::Keystroke;

use crate::terminal::TerminalModel;
use crate::util::bindings::keybinding_name_to_keystroke;

pub const ACCEPT_PROMPT_SUGGESTION_KEYBINDING: &str = "terminal:accept_prompt_suggestions";

pub static REJECT_PROMPT_SUGGESTION_KEYSTROKE: LazyLock<Keystroke> = LazyLock::new(|| Keystroke {
    ctrl: true,
    key: "c".to_owned(),
    ..Default::default()
});

pub fn is_accept_prompt_suggestion_bound_to_cmd_enter(app: &AppContext) -> bool {
    static CMD_ENTER_KEYSTROKE: LazyLock<Keystroke> = LazyLock::new(|| Keystroke {
        cmd: true,
        key: "enter".to_owned(),
        ..Default::default()
    });
    keybinding_name_to_keystroke(ACCEPT_PROMPT_SUGGESTION_KEYBINDING, app)
        .is_some_and(|keystroke| keystroke == *CMD_ENTER_KEYSTROKE)
}

pub fn is_accept_prompt_suggestion_bound_to_ctrl_enter(app: &AppContext) -> bool {
    static CTRL_ENTER_KEYSTROKE: LazyLock<Keystroke> = LazyLock::new(|| Keystroke {
        ctrl: true,
        key: "enter".to_owned(),
        ..Default::default()
    });
    keybinding_name_to_keystroke(ACCEPT_PROMPT_SUGGESTION_KEYBINDING, app)
        .is_some_and(|keystroke| keystroke == *CTRL_ENTER_KEYSTROKE)
}

/// Returns `true` if the last AI block in the blocklist has a pending suggested diff or unit test
/// suggestion.
pub fn has_pending_code_or_unit_test_prompt_suggestion(
    terminal_model: &TerminalModel,
    app: &AppContext,
) -> bool {
    terminal_model
        .block_list()
        .last_non_hidden_ai_block_handle(app)
        .is_some_and(|ai_block| {
            let block = ai_block.as_ref(app);
            if !block.is_passive_conversation(app) || block.is_hidden(app) {
                return false;
            }
            // Check for pending passive code diff
            let has_code_diff = block.find_undismissed_code_diff(app).is_some();
            // Check for pending MAA suggested prompt (e.g., unit test suggestion)
            let has_suggested_prompt = block.pending_unit_test_suggestion(app).is_some();
            has_code_diff || has_suggested_prompt
        })
}
