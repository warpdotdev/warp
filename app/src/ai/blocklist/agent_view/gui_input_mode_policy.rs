//! GUI implementation of [`InputModePolicy`].

use warp_core::features::FeatureFlag;
use warpui::{AppContext, EntityId, ModelHandle, SingletonEntity};

use super::super::conversation_selection::ConversationSelectionEvent;
use super::super::input_mode_policy::{InputModePolicy, PolicyConfigUpdate};
use super::super::input_model::{InputConfig, InputType, InputTypeAutoDetectionSource};
use super::super::{BlocklistAIContextModel, ConversationSelectionHandle};
use super::AgentViewEntryOrigin;
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;

/// GUI input-mode policy. The surface is either a fullscreen agent view or a
/// top-level terminal (when `FeatureFlag::AgentView` is enabled), each with its
/// own autodetection setting, and AI input may only be locked inside an agent
/// view or an open CLI-agent rich input session.
pub(crate) struct GuiInputModePolicy {
    conversation_selection: ConversationSelectionHandle,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    terminal_surface_id: EntityId,
}

impl GuiInputModePolicy {
    /// Creates the GUI policy for a terminal surface.
    pub(crate) fn new(
        conversation_selection: ConversationSelectionHandle,
        ai_context_model: ModelHandle<BlocklistAIContextModel>,
        terminal_surface_id: EntityId,
    ) -> Self {
        Self {
            conversation_selection,
            ai_context_model,
            terminal_surface_id,
        }
    }
}

impl InputModePolicy for GuiInputModePolicy {
    fn initial_config(&self, app: &AppContext) -> InputConfig {
        let is_autodetection_enabled = if FeatureFlag::AgentView.is_enabled() {
            AISettings::as_ref(app).is_nld_in_terminal_enabled(app)
        } else {
            AISettings::as_ref(app).is_ai_autodetection_enabled(app)
        };
        InputConfig {
            input_type: InputType::Shell,
            is_locked: !is_autodetection_enabled,
        }
    }

    fn allows_locked_ai_input(&self, app: &AppContext) -> bool {
        // When `AgentView` is enabled, AI input mode can only be set in the top-level terminal
        // mode via autodetection; it cannot be locked to AI input mode unless there is an active
        // agent view or a CLI agent rich input session is open. In the agent view case, executing
        // autodetected AI input will trigger entering the agent view with that query. In the CLI
        // agent rich input case, the input must be in AI mode to suppress shell decorations
        // (syntax highlighting, error underlining).
        !FeatureFlag::AgentView.is_enabled()
            || self
                .conversation_selection
                .as_ref(app)
                .is_conversation_active(app)
            || CLIAgentSessionsModel::as_ref(app).is_input_open(self.terminal_surface_id)
    }

    fn is_autodetection_enabled(&self, app: &AppContext) -> bool {
        let ai_settings = AISettings::as_ref(app);
        if FeatureFlag::AgentView.is_enabled() {
            if self
                .conversation_selection
                .as_ref(app)
                .is_conversation_fullscreen(app)
            {
                ai_settings.is_ai_autodetection_enabled(app)
            } else {
                ai_settings.is_nld_in_terminal_enabled(app)
            }
        } else {
            // AgentView not enabled: use the main autodetection setting
            ai_settings.is_ai_autodetection_enabled(app)
        }
    }

    fn config_on_conversation_selection_changed(
        &self,
        event: &ConversationSelectionEvent,
        current: InputConfig,
        app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        match event {
            ConversationSelectionEvent::Changed => None,
            ConversationSelectionEvent::Activated {
                is_fullscreen,
                origin,
            } => {
                if !is_fullscreen {
                    Some(PolicyConfigUpdate::with_source(
                        InputConfig {
                            input_type: InputType::AI,
                            is_locked: true,
                        },
                        InputTypeAutoDetectionSource::InlineAgentViewEntry,
                    ))
                } else if matches!(origin, AgentViewEntryOrigin::ClearBuffer) {
                    let is_autodetection_enabled =
                        AISettings::as_ref(app).is_ai_autodetection_enabled(app);
                    Some(PolicyConfigUpdate::new(InputConfig {
                        input_type: current.input_type,
                        is_locked: !is_autodetection_enabled,
                    }))
                } else if self.ai_context_model.as_ref(app).has_locking_attachment() {
                    Some(PolicyConfigUpdate::with_source(
                        InputConfig {
                            input_type: InputType::AI,
                            is_locked: true,
                        },
                        InputTypeAutoDetectionSource::AttachmentForcedAi,
                    ))
                } else {
                    let is_autodetection_enabled =
                        AISettings::as_ref(app).is_ai_autodetection_enabled(app);
                    Some(PolicyConfigUpdate {
                        config: InputConfig {
                            input_type: InputType::AI,
                            is_locked: !is_autodetection_enabled,
                        },
                        decision_source: None,
                        temporarily_disable_autodetection: is_autodetection_enabled,
                    })
                }
            }
            ConversationSelectionEvent::Deactivated {
                is_exit_before_new_entrance,
                ..
            } => {
                if *is_exit_before_new_entrance {
                    return None;
                }
                let is_nld_in_terminal_enabled =
                    AISettings::as_ref(app).is_nld_in_terminal_enabled(app);
                Some(PolicyConfigUpdate::new(InputConfig {
                    input_type: InputType::Shell,
                    is_locked: !is_nld_in_terminal_enabled,
                }))
            }
        }
    }

    fn config_on_ai_settings_changed(
        &self,
        event: &AISettingsChangedEvent,
        current: InputConfig,
        is_autodetection_enabled_for_current_context: bool,
        app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        match event {
            AISettingsChangedEvent::AIAutoDetectionEnabled { .. }
                if FeatureFlag::AgentView.is_enabled() =>
            {
                if self
                    .conversation_selection
                    .as_ref(app)
                    .is_conversation_fullscreen(app)
                {
                    // Use context-specific check to determine if autodetection should be enabled
                    let is_nld_enabled = AISettings::as_ref(app).is_ai_autodetection_enabled(app);

                    // If autodetection is enabled, unlock the input.
                    Some(PolicyConfigUpdate::new(InputConfig {
                        is_locked: !is_nld_enabled,
                        input_type: InputType::AI,
                    }))
                } else {
                    None
                }
            }
            AISettingsChangedEvent::AIAutoDetectionEnabled { .. } => {
                // If autodetection is enabled, unlock the input.
                Some(PolicyConfigUpdate::new(InputConfig {
                    is_locked: !is_autodetection_enabled_for_current_context,
                    ..current
                }))
            }
            AISettingsChangedEvent::NLDInTerminalEnabled { .. }
                if FeatureFlag::AgentView.is_enabled()
                    && !self
                        .conversation_selection
                        .as_ref(app)
                        .is_conversation_active(app) =>
            {
                let is_nld_enabled = AISettings::as_ref(app).is_nld_in_terminal_enabled(app);
                Some(PolicyConfigUpdate::new(InputConfig {
                    is_locked: !is_nld_enabled,
                    input_type: InputType::Shell,
                }))
            }
            _ => None,
        }
    }
}
