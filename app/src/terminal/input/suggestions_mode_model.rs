use warpui::{Entity, ModelContext, ModelHandle};

use super::{BufferState, DynamicEnumSuggestionStatus, InputConfig, InputSuggestionsMode};
use crate::ai::agent::conversation::AIConversationId;
use crate::terminal::input::buffer_model::InputBufferModel;
use crate::terminal::input::inline_menu::InlineMenuType;

/// Model responsible for managing the input suggestions mode state.
pub struct InputSuggestionsModeModel {
    mode: InputSuggestionsMode,
    /// Buffer state saved when an inline menu is opened, so it can be restored on dismiss.
    buffer_to_restore: Option<BufferState>,
    /// Handle to the input buffer model, used to snapshot buffer state when opening menus.
    buffer_model: ModelHandle<InputBufferModel>,
}

impl InputSuggestionsModeModel {
    pub fn new(buffer_model: ModelHandle<InputBufferModel>) -> Self {
        Self {
            mode: InputSuggestionsMode::Closed,
            buffer_to_restore: None,
            buffer_model,
        }
    }

    pub fn mode(&self) -> &InputSuggestionsMode {
        &self.mode
    }

    pub fn set_mode(&mut self, mode: InputSuggestionsMode, ctx: &mut ModelContext<Self>) {
        if self.mode == mode {
            return;
        }

        let input_config_to_restore = self.mode.input_config_to_restore();

        // If we're setting a new non-closed mode while the current mode is also non-closed,
        // first emit a mode change for the implicit close before transitioning to the new mode.
        if self.is_visible() && !matches!(mode, InputSuggestionsMode::Closed) {
            self.mode = InputSuggestionsMode::Closed;
            ctx.emit(InputSuggestionsModeEvent::ModeChanged {
                buffer_to_restore: None,
                input_config_to_restore,
            });
        }

        // Snapshot the buffer state when opening a mode that supports buffer restoration.
        if mode.should_snapshot_and_restore_buffer() {
            let buffer_model = self.buffer_model.as_ref(ctx);
            self.buffer_to_restore = Some(BufferState::new(
                buffer_model.current_value().to_owned(),
                buffer_model.cursor_point(),
            ));
        }

        // When closing via set_mode, we always discard saved buffer state.
        // To restore buffer state, callers should use close_and_restore_buffer.
        if matches!(mode, InputSuggestionsMode::Closed) {
            self.buffer_to_restore = None;
        }

        self.mode = mode;
        ctx.emit(InputSuggestionsModeEvent::ModeChanged {
            buffer_to_restore: None,
            input_config_to_restore: None,
        });
    }

    /// Closes the current menu, restoring the buffer if it was snapshotted on open.
    pub fn close_and_restore_buffer(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_closed() {
            return;
        }

        let buffer_to_restore = self.buffer_to_restore.take();
        let input_config_to_restore = self.mode.input_config_to_restore();
        self.mode = InputSuggestionsMode::Closed;
        ctx.emit(InputSuggestionsModeEvent::ModeChanged {
            buffer_to_restore,
            input_config_to_restore,
        });
    }

    pub fn set_dynamic_enum_status(
        &mut self,
        status: DynamicEnumSuggestionStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        if let InputSuggestionsMode::DynamicWorkflowEnumSuggestions {
            dynamic_enum_status,
            ..
        } = &mut self.mode
        {
            *dynamic_enum_status = status;
            ctx.emit(InputSuggestionsModeEvent::ModeChanged {
                buffer_to_restore: None,
                input_config_to_restore: None,
            });
        }
    }

    pub fn is_visible(&self) -> bool {
        self.mode.is_visible()
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::Closed)
    }

    pub fn is_history_up(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::HistoryUp { .. })
    }

    pub fn is_completion_suggestions(&self) -> bool {
        matches!(
            self.mode,
            InputSuggestionsMode::CompletionSuggestions { .. }
        )
    }

    pub fn is_static_workflow_enum_suggestions(&self) -> bool {
        matches!(
            self.mode,
            InputSuggestionsMode::StaticWorkflowEnumSuggestions { .. }
        )
    }

    pub fn is_dynamic_workflow_enum_suggestions(&self) -> bool {
        matches!(
            self.mode,
            InputSuggestionsMode::DynamicWorkflowEnumSuggestions { .. }
        )
    }

    pub fn is_ai_context_menu(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::AIContextMenu { .. })
    }

    pub fn is_slash_commands(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::SlashCommands)
    }

    pub fn is_conversation_menu(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::ConversationMenu)
    }

    pub fn is_inline_model_selector(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::ModelSelector)
    }

    pub fn is_profile_selector(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::ProfileSelector)
    }

    pub fn is_prompts_menu(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::PromptsMenu)
    }

    pub fn is_skill_menu(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::SkillMenu)
    }

    pub fn is_user_query_menu(&self) -> bool {
        matches!(
            self.mode,
            InputSuggestionsMode::UserQueryMenu {
                action: super::UserQueryMenuAction::ForkFrom,
                ..
            }
        )
    }

    pub fn is_rewind_menu(&self) -> bool {
        matches!(
            self.mode,
            InputSuggestionsMode::UserQueryMenu {
                action: super::UserQueryMenuAction::Rewind,
                ..
            }
        )
    }

    /// Returns the conversation_id if the current mode is UserQueryMenu (ForkFrom).
    pub fn user_query_conversation_id(&self) -> Option<AIConversationId> {
        match &self.mode {
            InputSuggestionsMode::UserQueryMenu {
                action: super::UserQueryMenuAction::ForkFrom,
                conversation_id,
            } => Some(*conversation_id),
            _ => None,
        }
    }

    /// Returns the conversation_id if the current mode is RewindMenu.
    pub fn rewind_conversation_id(&self) -> Option<AIConversationId> {
        match &self.mode {
            InputSuggestionsMode::UserQueryMenu {
                action: super::UserQueryMenuAction::Rewind,
                conversation_id,
            } => Some(*conversation_id),
            _ => None,
        }
    }

    pub fn is_inline_history_menu(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::InlineHistoryMenu { .. })
    }

    pub fn is_repos_menu(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::IndexedReposMenu)
    }

    pub fn is_plan_menu(&self) -> bool {
        matches!(self.mode, InputSuggestionsMode::PlanMenu { .. })
    }

    /// Returns the conversation_id if the current mode is PlanMenu.
    pub fn plan_menu_conversation_id(&self) -> Option<AIConversationId> {
        match &self.mode {
            InputSuggestionsMode::PlanMenu { conversation_id } => Some(*conversation_id),
            _ => None,
        }
    }

    pub fn inline_menu_type(&self) -> Option<InlineMenuType> {
        InlineMenuType::from_suggestions_mode(&self.mode)
    }

    pub fn is_inline_menu_open(&self) -> bool {
        self.mode.is_inline_menu()
    }
}

impl Entity for InputSuggestionsModeModel {
    type Event = InputSuggestionsModeEvent;
}

pub enum InputSuggestionsModeEvent {
    ModeChanged {
        /// The saved buffer state to restore, if this mode change is an inline menu closing.
        /// `None` for all other transitions.
        buffer_to_restore: Option<BufferState>,
        /// The saved input config to restore, if this mode change closes inline history menu
        /// without accepting the temporary preview state.
        input_config_to_restore: Option<InputConfig>,
    },
}
