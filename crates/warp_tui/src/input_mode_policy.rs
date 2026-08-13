//! TUI implementation of [`InputModePolicy`].

use warp::settings::{AISettings, AISettingsChangedEvent};
use warp::tui_export::{
    BlocklistAIInputModel, ConversationSelectionEvent, InputConfig, InputModePolicy, InputType,
    PolicyConfigUpdate,
};
use warpui_core::{AppContext, SingletonEntity};

/// The TUI's agent config when autodetection is disabled or explicitly overridden.
pub(crate) const AI_LOCKED_CONFIG: InputConfig = InputConfig {
    input_type: InputType::AI,
    is_locked: true,
};
/// The TUI's default agent config while autodetection is enabled.
pub(crate) const AI_UNLOCKED_CONFIG: InputConfig = InputConfig {
    input_type: InputType::AI,
    is_locked: false,
};

/// The config for `!` shell mode.
pub(crate) const SHELL_LOCKED_CONFIG: InputConfig = InputConfig {
    input_type: InputType::Shell,
    is_locked: true,
};
fn agent_config_for_autodetection(is_autodetection_enabled: bool) -> InputConfig {
    if is_autodetection_enabled {
        AI_UNLOCKED_CONFIG
    } else {
        AI_LOCKED_CONFIG
    }
}

fn config_on_autodetection_setting_changed(
    current: InputConfig,
    is_autodetection_enabled: bool,
) -> Option<InputConfig> {
    // Keep the explicit `!` override until the user exits it. Other states
    // return to the setting-derived, agent-first default.
    (current != SHELL_LOCKED_CONFIG)
        .then(|| agent_config_for_autodetection(is_autodetection_enabled))
}

/// Whether the shared input mode is shell input, detected or explicitly locked.
/// The single definition of "in shell mode" for every TUI read site.
pub(crate) fn is_shell_mode(input_mode: &BlocklistAIInputModel) -> bool {
    input_mode.input_type() == InputType::Shell
}

/// TUI input-mode policy: the input is agent-first, with autodetection driven
/// by the shared AI setting. Conversation transitions do not rewrite the mode;
/// explicit user actions may still lock the input to Agent or Shell.
pub(crate) struct TuiInputModePolicy;

impl InputModePolicy for TuiInputModePolicy {
    fn initial_config(&self, app: &AppContext) -> InputConfig {
        agent_config_for_autodetection(AISettings::as_ref(app).is_ai_autodetection_enabled(app))
    }

    fn allows_locked_ai_input(&self, _app: &AppContext) -> bool {
        true
    }

    fn is_autodetection_enabled(&self, app: &AppContext) -> bool {
        AISettings::as_ref(app).is_ai_autodetection_enabled(app)
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
        event: &AISettingsChangedEvent,
        current: InputConfig,
        is_autodetection_enabled_for_current_context: bool,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        match event {
            AISettingsChangedEvent::AIAutoDetectionEnabled { .. } => {
                config_on_autodetection_setting_changed(
                    current,
                    is_autodetection_enabled_for_current_context,
                )
                .map(PolicyConfigUpdate::new)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "input_mode_policy_tests.rs"]
mod tests;
