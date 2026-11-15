//! Model-layer AI input state management logic.
//!
//! The primary export of this module is `BlocklistAIInputModel`, which is a terminal-surface-scoped
//! model managing input "type" state (whether the input is in AI or shell mode).

use std::sync::Arc;

use instant::Instant;
use parking_lot::FairMutex;
use serde::{Deserialize, Serialize};
use session_sharing_protocol::common::{InputMode, InputType as ProtocolInputType};
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

/// The source of the final input type decision applied to the user input.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputTypeAutoDetectionSource {
    /// User explicitly toggled the input type via cmd + I.
    ManualToggle,
    /// `!` shell prefix force-locked the input to Shell mode.
    ShellPrefix,
    /// Image / file attachment in progress force-locked AI mode.
    AttachmentForcedAi,
    /// Inline history menu / history-up suggestion selection set the input type.
    HistorySelection,
    /// Inserting a workflow into the input set the input type based on workflow kind.
    WorkflowInsertion,
    /// Accepting a shell command autosuggestion forced Shell mode.
    CommandAutosuggestionAccepted,
    /// Accepting an Agent Mode query autosuggestion forced AI mode.
    AgentQueryAutosuggestionAccepted,
    /// Empty-buffer conversation context render forced AI so conversation context renders.
    ConversationContextRender,
    /// "Continue conversation" button forced AI mode.
    ContinueConversation,
    /// Onboarding tutorial agent prompt forced AI mode.
    OnboardingAgentPrompt,
    /// Locking the input for a CLI subagent's terminal-control handoff.
    /// Distinguishes an agent-installed AI lock from a user-forced one so the
    /// post-agent reset only restores autodetection when the lock was installed
    /// for agent terminal control.
    AgentTerminalControl,
    /// Starting a new agent conversation forced AI mode.
    StartNewConversation,
    /// Ask-AI flow (text/block selection, programmatic Ask-AI lock) forced AI mode.
    AskAi,
    /// Detected/composing slash or skill command forced AI mode.
    SlashCommand,
    /// Entering inline agent view force-locked AI without an explicit user toggle.
    InlineAgentViewEntry,
    /// Activating cloud handoff compose (`&` prefix or programmatic) force-locked AI.
    CloudHandoffEnter,
    /// Exiting cloud handoff compose restored AI / unlocked-if-autodetect.
    CloudHandoffExit,
    /// Legacy non-AgentView `?` AI prefix path force-locked AI.
    AgentModePrefix,
    /// Inline code review send overrode the input mode to AI.
    InlineCodeReviewSend,
    /// External input config update from session sharing applied.
    SessionSharingApply,
    /// Fullscreen AgentView inline history command cycling force-locked Shell.
    FullscreenInlineHistoryCycling,
    /// Closing history suggestions restored the previously saved config.
    RestoreSavedConfig,
    /// `set_input_config_for_classic_mode` reset (CtrlC, delete-all-left, etc.).
    ClassicModeReset,
    /// Toggling voice input forced AI mode.
    VoiceInputToggle,
    /// Inserting from the AI `@` context menu forced AI mode.
    AtContextMenuInsert,
}

use warp_errors::report_if_error;

use super::ConversationSelectionHandle;
use super::context_model::BlocklistAIContextModel;
use super::input_mode_policy::{InputModePolicyHandle, PolicyConfigUpdate};
pub use super::input_type::InputType;
use crate::settings::{AISettings, AISettingsChangedEvent, InputBoxType, InputSettings};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::terminal::TerminalModel;

/// Configuration for the terminal pane's input.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputConfig {
    /// The type of the terminal input.
    pub input_type: InputType,

    /// If `true`, we will not attempt to auto-detect the best input type.
    pub is_locked: bool,
}

impl InputConfig {
    /// Create a sensible default InputConfig based on user's auto-detection setting.
    pub fn new(app: &AppContext) -> Self {
        let ai_settings = AISettings::as_ref(app);
        let is_autodetection_enabled = ai_settings.is_ai_autodetection_enabled(app);

        InputConfig {
            input_type: InputType::Shell,
            is_locked: !is_autodetection_enabled, // Locked if auto-detection disabled
        }
    }

    pub fn with_toggled_type(self) -> Self {
        let input_type = if self.input_type.is_ai() {
            InputType::Shell
        } else {
            InputType::AI
        };
        Self { input_type, ..self }
    }

    pub fn with_shell_type(self) -> Self {
        Self {
            input_type: InputType::Shell,
            ..self
        }
    }

    pub fn with_input_type(self, input_type: InputType) -> Self {
        Self { input_type, ..self }
    }

    pub fn unlocked_if_autodetection_enabled(
        self,
        is_in_fullscreen_agent_view: bool,
        app: &AppContext,
    ) -> Self {
        Self {
            is_locked: if !FeatureFlag::AgentView.is_enabled() || is_in_fullscreen_agent_view {
                !AISettings::as_ref(app).is_ai_autodetection_enabled(app)
            } else {
                !AISettings::as_ref(app).is_nld_in_terminal_enabled(app)
            },
            ..self
        }
    }

    pub fn locked(self) -> Self {
        Self {
            is_locked: true,
            ..self
        }
    }

    pub fn is_ai(&self) -> bool {
        self.input_type == InputType::AI
    }

    pub fn is_shell(&self) -> bool {
        self.input_type == InputType::Shell
    }
}

impl From<InputConfig> for InputMode {
    fn from(config: InputConfig) -> Self {
        let protocol_input_type = match config.input_type {
            InputType::Shell => ProtocolInputType::Shell,
            InputType::AI => ProtocolInputType::AI,
        };

        InputMode::new(protocol_input_type, config.is_locked)
    }
}

/// Terminal-surface-scoped model responsible for managing AI input state.
#[derive(Clone)]
pub struct BlocklistAIInputModel {
    input_config: InputConfig,

    /// The timestamp of the last time the input mode was switched, if the switch was to AI mode and
    /// it was autodetected. Else, `None`.
    last_ai_autodetection_ts: Option<Instant>,
    /// the latest input type classification decision source
    last_ai_autodetection_source: Option<InputTypeAutoDetectionSource>,

    /// Whether the input buffer was empty at the time the lock was set.  This will be true
    /// if a persistent lock is in place and a buffer is submitted.
    was_lock_set_with_empty_buffer: bool,

    conversation_selection: ConversationSelectionHandle,

    /// Handle to the per-surface context model. Used to read pending image / file attachments
    /// when deciding whether to force-lock the input to AI mode (see
    /// [`BlocklistAIContextModel::has_locking_attachment`]).
    ai_context_model: ModelHandle<BlocklistAIContextModel>,

    /// View-supplied policy for decisions the model cannot make view-agnostically
    /// (lock gating, autodetection context, reactive config transitions).
    policy: InputModePolicyHandle,
    model: Arc<FairMutex<TerminalModel>>,
}

impl BlocklistAIInputModel {
    /// Creates input state for a terminal surface.
    pub fn new(
        model: Arc<FairMutex<TerminalModel>>,
        conversation_selection: ConversationSelectionHandle,
        ai_context_model: ModelHandle<BlocklistAIContextModel>,
        policy: InputModePolicyHandle,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Reactively restore input config when CLI agent rich input closes.
        #[cfg(not(feature = "local_terminal"))]
        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            move |me, _, event, ctx| {
                let CLIAgentSessionsModelEvent::InputSessionChanged {
                    terminal_view_id: event_view_id,
                    previous_input_state,
                    ..
                } = event
                else {
                    return;
                };
                // CLI agent sessions are keyed by terminal view id; GUI surfaces use the
                // view id as their surface id, so this filters events to our surface.
                if *event_view_id != terminal_surface_id {
                    return;
                }
                if let CLIAgentInputState::Open {
                    previous_input_config,
                    previous_was_lock_set_with_empty_buffer,
                    ..
                } = previous_input_state
                {
                    me.restore_input_config(
                        *previous_input_config,
                        *previous_was_lock_set_with_empty_buffer,
                        ctx,
                    );
                }
            },
        );

        #[cfg(not(feature = "local_terminal"))]
        ctx.subscribe_to_model(&AISettings::handle(ctx), move |me, _, event, ctx| {
            // Computing the guarded autodetection state takes the terminal-model
            // lock, so only compute it for the one event whose handling can need
            // it; policies must not rely on it for any other event.
            let is_autodetection_enabled_for_current_context =
                matches!(event, AISettingsChangedEvent::AIAutoDetectionEnabled { .. })
                    && me.is_autodetection_enabled_for_current_context(ctx);
            if let Some(update) = me.policy.config_on_ai_settings_changed(
                event,
                me.input_config(),
                is_autodetection_enabled_for_current_context,
                ctx,
            ) {
                me.apply_policy_update(update, ctx);
            }
        });

        #[cfg(not(feature = "local_terminal"))]
        ctx.subscribe_to_model(&conversation_selection, |me, _, event, ctx| {
            if let Some(update) =
                me.policy
                    .config_on_conversation_selection_changed(event, me.input_config(), ctx)
            {
                me.apply_policy_update(update, ctx);
            }
        });

        let input_config = policy.initial_config(ctx);
        Self {
            input_config,
            conversation_selection,
            ai_context_model,
            policy,
            last_ai_autodetection_ts: None,
            last_ai_autodetection_source: None,
            was_lock_set_with_empty_buffer: false,
            model,
        }
    }

    /// Builds a self-contained input model for tests, usable from other crates
    /// via the `test-util` feature: a mock terminal model, an inert
    /// conversation selection, and no production subscriptions.
    #[cfg(any(test, feature = "test-util"))]
    pub fn mock(policy: InputModePolicyHandle, ctx: &mut AppContext) -> ModelHandle<Self> {
        use super::conversation_selection::{ConversationSelection, MockConversationSelection};

        let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let conversation_selection = ctx
            .add_model(|_| Box::new(MockConversationSelection) as Box<dyn ConversationSelection>);
        let context_conversation_selection = conversation_selection.clone();
        let context_terminal_model = model.clone();
        let ai_context_model = ctx.add_model(|_| {
            BlocklistAIContextModel::new_for_test(
                context_terminal_model,
                EntityId::new(),
                context_conversation_selection,
            )
        });
        let input_config = policy.initial_config(ctx);
        ctx.add_model(|_| Self {
            input_config,
            conversation_selection,
            ai_context_model,
            policy,
            last_ai_autodetection_ts: None,
            last_ai_autodetection_source: None,
            was_lock_set_with_empty_buffer: false,
            model,
        })
    }

    /// Returns whether the surface presents a selected conversation as active.
    fn is_conversation_active(&self, app: &AppContext) -> bool {
        self.conversation_selection
            .as_ref(app)
            .is_conversation_active(app)
    }

    /// Convenience wrapper around `BlocklistAIContextModel::has_locking_attachment`.
    fn has_locking_attachment(&self, app: &AppContext) -> bool {
        self.ai_context_model.as_ref(app).has_locking_attachment()
    }

    /// Returns the InputType enum which specifies how we will handle the terminal input.
    pub fn input_type(&self) -> InputType {
        self.input_config.input_type
    }

    /// Whether the input type is locked. Does not take user autodetection setting or feature flags
    /// into account.
    pub fn is_input_type_locked(&self) -> bool {
        self.input_config.is_locked
    }

    pub fn is_ai_input_enabled(&self) -> bool {
        matches!(self.input_config.input_type, InputType::AI)
    }

    pub fn input_config(&self) -> InputConfig {
        self.input_config
    }

    pub fn is_terminal_use_active_or_pending(&self) -> bool {
        let model = self.model.lock();
        let active_block = model.block_list().active_block();
        // Keep AI input locked while an agent-requested command is waiting for its
        // CLI subagent, while the user has tagged the agent in, or while the user
        // can hand control of an active monitored command back to the agent.
        active_block.is_agent_driving_command()
            || active_block.is_agent_tagged_in()
            || active_block.is_eligible_for_agent_handoff()
    }
    pub fn last_ai_autodetection_source(&self) -> Option<InputTypeAutoDetectionSource> {
        self.last_ai_autodetection_source
    }

    pub fn last_ai_autodetection_ts(&self) -> Option<Instant> {
        self.last_ai_autodetection_ts
    }

    /// Sets the input config iff the input is in classic mode (i.e. not UDI).
    pub fn set_input_config_for_classic_mode(
        &mut self,
        new_config: InputConfig,
        ctx: &mut ModelContext<Self>,
    ) {
        // When agent view is active, the input should behave like Universal mode
        // even if Classic mode is selected (e.g. when PS1 is enabled).
        if FeatureFlag::AgentView.is_enabled() && self.is_conversation_active(ctx) {
            return;
        }

        let input_type = InputSettings::as_ref(ctx).input_type(ctx);
        if !matches!(input_type, InputBoxType::Classic) {
            return;
        }
        self.set_input_config_internal(
            new_config,
            Some(InputTypeAutoDetectionSource::ClassicModeReset),
            ctx,
        );
    }

    /// Swaps between Agent/Shell input types while preserving lock state.
    pub fn set_input_type(
        &mut self,
        input_type: InputType,
        decision_source: Option<InputTypeAutoDetectionSource>,
        ctx: &mut ModelContext<Self>,
    ) {
        let current_config = self.input_config();
        self.set_input_config_internal(
            current_config.with_input_type(input_type),
            decision_source,
            ctx,
        );
    }

    /// Applies a policy-produced config update via the internal setter.
    fn apply_policy_update(&mut self, update: PolicyConfigUpdate, ctx: &mut ModelContext<Self>) {
        self.set_input_config_internal(update.config, update.decision_source, ctx);
    }

    fn set_input_config_internal(
        &mut self,
        new_config: InputConfig,
        decision_source: Option<InputTypeAutoDetectionSource>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if cfg!(feature = "local_terminal") {
            return false;
        }

        // Locking the input to AI is only allowed when the view's policy permits it (e.g. the
        // GUI only allows it inside an agent view or an open CLI agent rich input session).
        if new_config.input_type.is_ai()
            && new_config.is_locked
            && !self.policy.allows_locked_ai_input(ctx)
        {
            return false;
        }

        if self.input_config == new_config {
            self.last_ai_autodetection_source = decision_source;
            return false;
        }

        let old_config = self.input_config;

        if !new_config.is_locked && new_config.input_type.is_ai() {
            self.last_ai_autodetection_ts = Some(Instant::now());
        } else {
            self.last_ai_autodetection_ts = None;
        }

        if new_config.input_type.is_ai() {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let new_num_times = *settings.entered_agent_mode_num_times + 1;
                report_if_error!(
                    settings
                        .entered_agent_mode_num_times
                        .set_value(new_num_times, ctx)
                );
            });
        }

        self.input_config = new_config;
        self.last_ai_autodetection_source = decision_source;

        // Emit specific events for what actually changed
        if old_config.input_type != new_config.input_type {
            ctx.emit(BlocklistAIInputEvent::InputTypeChanged { config: new_config });
        }

        if old_config.is_locked != new_config.is_locked {
            ctx.emit(BlocklistAIInputEvent::LockChanged { config: new_config });
        }

        true
    }

    /// Allows you to set the input config and mutate the lock state.
    pub fn set_input_config(
        &mut self,
        new_config: InputConfig,
        is_input_buffer_empty: bool,
        decision_source: Option<InputTypeAutoDetectionSource>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.set_input_config_internal(new_config, decision_source, ctx);
        self.was_lock_set_with_empty_buffer = self.is_input_type_locked() && is_input_buffer_empty;
    }

    /// Restores a previous input config without recomputing whether the lock was set while the
    /// buffer was empty.
    fn restore_input_config(
        &mut self,
        new_config: InputConfig,
        was_lock_set_with_empty_buffer: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.set_input_config_internal(new_config, None, ctx);
        self.was_lock_set_with_empty_buffer = was_lock_set_with_empty_buffer;
    }

    /// Returns `false` if the input type is locked and we will not attempt to automatically detect
    /// and change the input type.
    pub fn should_run_input_autodetection(&self, app: &AppContext) -> bool {
        FeatureFlag::AgentMode.is_enabled()
            && self.is_autodetection_enabled_for_current_context(app)
            && !self.input_config.is_locked
    }

    /// Returns whether autodetection is enabled for the current context, layering view-agnostic
    /// guards (agent in control, pending attachments) over the view policy's setting lookup.
    pub fn is_autodetection_enabled_for_current_context(&self, app: &AppContext) -> bool {
        // If the agent is in control or tagged in, don't run autodetection.
        if self.is_terminal_use_active_or_pending() {
            return false;
        }

        // Defense in depth: while there is a pending image / file attachment, the classifier
        // must never have a chance to flip the input back to shell mode, even per-keystroke.
        // The conversation-activation subscriber and `set_input_mode_agent` already lock at entry;
        // this guard protects the window if any future caller forgets.
        if self.has_locking_attachment(app) {
            return false;
        }

        self.policy.is_autodetection_enabled(app)
    }

    pub fn enable_autodetection(&mut self, input_type: InputType, ctx: &mut ModelContext<Self>) {
        self.set_input_config_internal(
            InputConfig {
                input_type,
                is_locked: false,
            },
            None,
            ctx,
        );
    }

    /// Handles the input buffer being submitted.
    pub fn handle_input_buffer_submitted(&mut self, ctx: &mut ModelContext<Self>) {
        // If the agent is still in control of a long-running command, keep the input locked to AI mode.
        let is_terminal_use_active_or_pending = self.is_terminal_use_active_or_pending();

        let new_config = if is_terminal_use_active_or_pending {
            InputConfig {
                input_type: InputType::AI,
                is_locked: true,
            }
        } else {
            // If NLD is enabled and input is currently locked, unlock it, as we want to
            // resume autodetection for the next input.
            InputConfig {
                is_locked: !self.policy.is_autodetection_enabled(ctx),
                ..self.input_config
            }
        };

        self.set_input_config(
            new_config,
            // We know the buffer is currently empty, as it was just submitted.
            true, None, ctx,
        );
    }

    pub fn was_lock_set_with_empty_buffer(&self) -> bool {
        self.was_lock_set_with_empty_buffer
    }

}

#[derive(Debug, Clone)]
pub enum BlocklistAIInputEvent {
    /// Emitted when the terminal input type is updated.
    InputTypeChanged {
        /// The new input config.
        config: InputConfig,
    },
    /// Emitted when the input lock state is updated.
    LockChanged {
        /// The new input config.
        config: InputConfig,
    },
}

impl BlocklistAIInputEvent {
    pub fn did_update_input_config(&self) -> bool {
        match self {
            BlocklistAIInputEvent::InputTypeChanged { .. }
            | BlocklistAIInputEvent::LockChanged { .. } => true,
        }
    }

    pub fn updated_config(&self) -> &InputConfig {
        match self {
            BlocklistAIInputEvent::InputTypeChanged { config }
            | BlocklistAIInputEvent::LockChanged { config } => config,
        }
    }
}

impl Entity for BlocklistAIInputModel {
    type Event = BlocklistAIInputEvent;
}

#[cfg(test)]
#[path = "input_model_tests.rs"]
mod tests;
