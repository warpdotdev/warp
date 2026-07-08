//! TUI implementation of [`InputModePolicy`].

use warp::tui_export::{
    AISettingsChangedEvent, ConversationSelectionEvent, InputConfig, InputModePolicy, InputType,
    PolicyConfigUpdate,
};
use warpui_core::AppContext;

/// TUI input-mode policy: the input is agent-first and deterministic. It
/// starts locked to AI, may always be locked to AI, has no autodetection (yet),
/// and conversation/settings transitions never rewrite the mode — only
/// explicit user actions (e.g. the `!` shell prefix) change it.
pub(crate) struct TuiInputModePolicy;

impl InputModePolicy for TuiInputModePolicy {
    fn initial_config(&self, _app: &AppContext) -> InputConfig {
        InputConfig {
            input_type: InputType::AI,
            is_locked: true,
        }
    }

    fn allows_locked_ai_input(&self, _app: &AppContext) -> bool {
        true
    }

    fn is_autodetection_enabled(&self, _app: &AppContext) -> bool {
        false
    }

    fn config_on_conversation_selection_changed(
        &self,
        _event: &ConversationSelectionEvent,
        _current: InputConfig,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }

    fn config_on_ai_settings_changed(
        &self,
        _event: &AISettingsChangedEvent,
        _current: InputConfig,
        _is_autodetection_enabled_for_current_context: bool,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }
}
