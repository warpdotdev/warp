//! Model-layer AI input state management logic.
//!
//! The primary export of this module is `BlocklistAIInputModel`, which is a terminal pane-scoped
//! model managing input "type" state (whether the input is in AI or shell mode). This model also
//! exposes methods for running query autodetection, where an algorithm determines if the current
//! input contents are an AI query or shell command, which is then used to update the input mode.

use std::sync::Arc;

use futures::stream::AbortHandle;
use input_classifier::util::{is_agent_follow_up_input, is_one_off_natural_language_word};
use instant::Instant;
use parking_lot::FairMutex;
use serde::{Deserialize, Serialize};
use session_sharing_protocol::common::{InputMode, InputType as ProtocolInputType};
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

pub use input_classifier::InputType;

use super::agent_view::{AgentViewController, AgentViewControllerEvent, AgentViewEntryOrigin};
use super::context_model::BlocklistAIContextModel;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::PrivacySettings;
use warp_completer::completer::CompletionContext;

use crate::{
    input_classifier::InputClassifierModel,
    report_if_error, send_telemetry_from_ctx,
    settings::{AISettings, AISettingsChangedEvent, InputBoxType, InputSettings},
    terminal::{
        input::decorations::ParsedTokensSnapshot,
        model::{rich_content::RichContentType, session::SessionId},
        History, TerminalModel,
    },
    TelemetryEvent,
};

use super::telemetry_banner::should_collect_ai_ugc_telemetry;

/// Cutoff score for deciding an user input matches a history command entry.
const HISTORY_ENTRY_MATCH_CUTOFF: f32 = 0.9;

/// Duration to temporarily disable autodetection during operations like history selection.
const AUTODETECTION_DISABLE_DURATION_MS: u64 = 250;

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

/// Terminal pane-scoped model responsible for managing AI input state.
#[derive(Clone)]
pub struct BlocklistAIInputModel {
    input_config: InputConfig,

    /// The timestamp of the last time the input mode was switched, if the switch was to AI mode and
    /// it was autodetected. Else, `None`.
    last_ai_autodetection_ts: Option<Instant>,

    /// Timestamp of the last time the input type was explicitly set.
    last_explicit_input_type_set_at: Option<Instant>,

    /// Whether the input buffer was empty at the time the lock was set.  This will be true
    /// if a persistent lock is in place and a buffer is submitted.
    was_lock_set_with_empty_buffer: bool,

    agent_view_controller: ModelHandle<AgentViewController>,

    /// Handle to the per-pane context model. Used to read pending image / file attachments
    /// when deciding whether to force-lock the input to AI mode (see
    /// [`BlocklistAIContextModel::has_locking_attachment`]).
    ai_context_model: ModelHandle<BlocklistAIContextModel>,

    terminal_view_id: EntityId,

    autodetect_abort_handle: Option<AbortHandle>,
    model: Arc<FairMutex<TerminalModel>>,
}

impl BlocklistAIInputModel {
    pub fn new(
        model: Arc<FairMutex<TerminalModel>>,
        agent_view_controller: ModelHandle<AgentViewController>,
        ai_context_model: ModelHandle<BlocklistAIContextModel>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Reactively restore input config when CLI agent rich input closes.
        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            move |me, event, ctx| {
                let CLIAgentSessionsModelEvent::InputSessionChanged {
                    terminal_view_id: event_view_id,
                    previous_input_state,
                    ..
                } = event
                else {
                    return;
                };
                if *event_view_id != terminal_view_id {
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

        ctx.subscribe_to_model(&AISettings::handle(ctx), move |me, event, ctx| {
            match event {
                AISettingsChangedEvent::AIAutoDetectionEnabled { .. }
                    if FeatureFlag::AgentView.is_enabled() =>
                {
                    if me.agent_view_controller.as_ref(ctx).is_fullscreen() {
                        // Use context-specific check to determine if autodetection should be enabled
                        let is_nld_enabled =
                            AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx);

                        // If autodetection is enabled, unlock the input.
                        me.set_input_config_internal(
                            InputConfig {
                                is_locked: !is_nld_enabled,
                                input_type: InputType::AI,
                            },
                            ctx,
                        );
                    }
                }
                AISettingsChangedEvent::AIAutoDetectionEnabled { .. } => {
                    // Use context-specific check to determine if autodetection should be enabled
                    let is_autodetection_enabled =
                        me.is_autodetection_enabled_for_current_context(ctx);

                    // If autodetection is enabled, unlock the input.
                    me.set_input_config_internal(
                        InputConfig {
                            is_locked: !is_autodetection_enabled,
                            ..me.input_config()
                        },
                        ctx,
                    );
                }
                AISettingsChangedEvent::NLDInTerminalEnabled { .. }
                    if FeatureFlag::AgentView.is_enabled()
                        && !me.agent_view_controller.as_ref(ctx).is_active() =>
                {
                    let is_nld_enabled = AISettings::as_ref(ctx).is_nld_in_terminal_enabled(ctx);
                    me.set_input_config_internal(
                        InputConfig {
                            is_locked: !is_nld_enabled,
                            input_type: InputType::Shell,
                        },
                        ctx,
                    );
                }
                _ => (),
            }
        });

        if FeatureFlag::AgentView.is_enabled() {
            ctx.subscribe_to_model(&agent_view_controller, |me, event, ctx| match event {
                AgentViewControllerEvent::EnteredAgentView {
                    display_mode,
                    origin,
                    ..
                } => {
                    if display_mode.is_inline() {
                        me.set_input_config_internal(
                            InputConfig {
                                input_type: InputType::AI,
                                is_locked: true,
                            },
                            ctx,
                        );
                    } else if matches!(origin, AgentViewEntryOrigin::ClearBuffer) {
                        let is_autodetection_enabled =
                            AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx);
                        me.set_input_config_internal(
                            InputConfig {
                                input_type: me.input_config().input_type,
                                is_locked: !is_autodetection_enabled,
                            },
                            ctx,
                        );
                    } else if me.has_locking_attachment(ctx) {
                        // Interaction patterns that should fully bypass NLD on
                        // entry: image / file attachment in progress / attached.
                        // Force-lock to AI regardless of the user's NLD setting so the
                        // classifier never gets a chance to drop the buffer back to shell.
                        me.set_input_config_internal(
                            InputConfig {
                                input_type: InputType::AI,
                                is_locked: true,
                            },
                            ctx,
                        );
                    } else {
                        let is_autodetection_enabled =
                            AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx);
                        if is_autodetection_enabled {
                            // Upon entering the agent view, temporarily disable autodetection as
                            // the existing buffer contents, if any are now most likely intended to
                            // be sent to the agent, and if the input would otherwise trigger a
                            // false-negative classification, we'd drop the user right into shell
                            // mode.
                            me.temporarily_disable_autodetection();
                        }
                        me.set_input_config_internal(
                            InputConfig {
                                input_type: InputType::AI,
                                is_locked: !is_autodetection_enabled,
                            },
                            ctx,
                        );
                    }
                }
                AgentViewControllerEvent::ExitedAgentView {
                    is_exit_before_new_entrance,
                    ..
                } => {
                    if !is_exit_before_new_entrance {
                        // When truly exiting agent view, use the terminal-specific NLD setting
                        // since the user is returning to terminal mode.
                        let is_nld_in_terminal_enabled =
                            AISettings::as_ref(ctx).is_nld_in_terminal_enabled(ctx);
                        me.set_input_config_internal(
                            InputConfig {
                                input_type: InputType::Shell,
                                is_locked: !is_nld_in_terminal_enabled,
                            },
                            ctx,
                        );
                    }
                }
                _ => (),
            });
        }

        let is_autodetection_enabled = if FeatureFlag::AgentView.is_enabled() {
            AISettings::as_ref(ctx).is_nld_in_terminal_enabled(ctx)
        } else {
            AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx)
        };
        Self {
            input_config: InputConfig {
                input_type: InputType::Shell,
                is_locked: !is_autodetection_enabled,
            },
            agent_view_controller,
            ai_context_model,
            terminal_view_id,
            last_ai_autodetection_ts: None,
            last_explicit_input_type_set_at: None,
            was_lock_set_with_empty_buffer: false,
            autodetect_abort_handle: None,
            model,
        }
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
        if FeatureFlag::AgentView.is_enabled() && self.agent_view_controller.as_ref(ctx).is_active()
        {
            return;
        }

        let input_type = InputSettings::as_ref(ctx).input_type(ctx);
        if !matches!(input_type, InputBoxType::Classic) {
            return;
        }
        self.set_input_config_internal(new_config, ctx);
    }

    /// Swaps between Agent/Shell input types while preserving lock state. Temporarily disables
    /// autodetection.
    pub fn set_input_type(&mut self, input_type: InputType, ctx: &mut ModelContext<Self>) {
        self.temporarily_disable_autodetection();
        let current_config = self.input_config();
        self.set_input_config_internal(current_config.with_input_type(input_type), ctx);
    }

    /// Does not disable autodetection.
    fn set_input_config_internal(
        &mut self,
        new_config: InputConfig,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        // When `AgentView` is enabled, AI input mode can only be set in the top-level terminal
        // mode via autodetection; it cannot be locked to AI input mode unless there is an active
        // agent view or a CLI agent rich input session is open. In the agent view case, executing
        // autodetected AI input will trigger entering the agent view with that query. In the CLI
        // agent rich input case, the input must be in AI mode to suppress shell decorations
        // (syntax highlighting, error underlining).
        if FeatureFlag::AgentView.is_enabled()
            && !self.agent_view_controller.as_ref(ctx).is_active()
            && new_config.input_type.is_ai()
            && new_config.is_locked
            && !CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id)
        {
            return false;
        }

        if self.input_config == new_config {
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
                report_if_error!(settings
                    .entered_agent_mode_num_times
                    .set_value(new_num_times, ctx));
            });
        }

        self.input_config = new_config;

        // Emit specific events for what actually changed
        if old_config.input_type != new_config.input_type {
            ctx.emit(BlocklistAIInputEvent::InputTypeChanged { config: new_config });
        }

        if old_config.is_locked != new_config.is_locked {
            ctx.emit(BlocklistAIInputEvent::LockChanged { config: new_config });
        }

        true
    }

    /// Allows you to set the input config and mutate the lock state. Temporarily disables autodetection.
    pub fn set_input_config(
        &mut self,
        new_config: InputConfig,
        is_input_buffer_empty: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.temporarily_disable_autodetection();
        self.set_input_config_internal(new_config, ctx);
        if new_config.is_locked {
            self.abort_in_progress_detection();
        }
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
        self.temporarily_disable_autodetection();
        self.set_input_config_internal(new_config, ctx);
        self.abort_in_progress_detection();
        self.was_lock_set_with_empty_buffer = was_lock_set_with_empty_buffer;
    }

    /// Returns `false` if the input type is locked and we will not attempt to automatically detect
    /// and change the input type.
    pub fn should_run_input_autodetection(&self, app: &AppContext) -> bool {
        FeatureFlag::AgentMode.is_enabled()
            && self.is_autodetection_enabled_for_current_context(app)
            && !self.input_config.is_locked
    }

    /// Returns whether autodetection is enabled for the current context.
    /// When AgentView is enabled, this checks whether we're in agent view or terminal mode
    /// and returns the appropriate setting.
    pub fn is_autodetection_enabled_for_current_context(&self, app: &AppContext) -> bool {
        // If the agent is in control or tagged in, don't run autodetection.
        if self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_agent_in_control_or_tagged_in()
        {
            return false;
        }

        // Defense in depth: while there is a pending image / file attachment, the classifier
        // must never have a chance to flip the input back to shell mode, even per-keystroke.
        // The `EnteredAgentView` subscriber and `set_input_mode_agent` already lock at entry;
        // this guard protects the window if any future caller forgets.
        if self.has_locking_attachment(app) {
            return false;
        }

        let ai_settings = AISettings::as_ref(app);
        if FeatureFlag::AgentView.is_enabled() {
            if self.agent_view_controller.as_ref(app).is_fullscreen() {
                ai_settings.is_ai_autodetection_enabled(app)
            } else {
                ai_settings.is_nld_in_terminal_enabled(app)
            }
        } else {
            // AgentView not enabled: use the main autodetection setting
            ai_settings.is_ai_autodetection_enabled(app)
        }
    }

    /// Temporarily disable autodetection for a fixed duration.
    /// Useful for operations like history selection where we don't want
    /// autodetection to interfere with the manual input type setting.
    fn temporarily_disable_autodetection(&mut self) {
        self.last_explicit_input_type_set_at = Some(Instant::now());
    }

    pub fn enable_autodetection(&mut self, input_type: InputType, ctx: &mut ModelContext<Self>) {
        self.set_input_config_internal(
            InputConfig {
                input_type,
                is_locked: false,
            },
            ctx,
        );
        // The goal of this function is to allow autodetection to run, but if we
        // don't clear this, it may be suppressed for a short duration.
        self.last_explicit_input_type_set_at = None;
    }

    /// Handles the input buffer being submitted.
    pub fn handle_input_buffer_submitted(&mut self, ctx: &mut ModelContext<Self>) {
        // If the agent is still in control of a long-running command, keep the input locked to AI mode.
        let is_agent_in_control_or_tagged_in = self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_agent_in_control_or_tagged_in();

        let new_config = if is_agent_in_control_or_tagged_in {
            InputConfig {
                input_type: InputType::AI,
                is_locked: true,
            }
        } else {
            // If NLD is enabled and input is currently locked, unlock it, as we want to
            // resume autodetection for the next input.
            self.input_config.unlocked_if_autodetection_enabled(
                self.agent_view_controller.as_ref(ctx).is_fullscreen(),
                ctx,
            )
        };

        self.set_input_config(
            new_config,
            // We know the buffer is currently empty, as it was just submitted.
            true, ctx,
        );
    }

    pub fn was_lock_set_with_empty_buffer(&self) -> bool {
        self.was_lock_set_with_empty_buffer
    }

    /// Aborts any in progress work for autodetection.
    pub fn abort_in_progress_detection(&mut self) {
        if let Some(handle) = self.autodetect_abort_handle.take() {
            handle.abort();
        }
    }

    /// If the input type is unlocked, analyze the input and set the input type to the type we
    /// detected. Emits an event if the input mode changed. If the input mode is locked, do
    /// nothing.
    ///
    /// When `session_id` is `Some`, history matching is performed. The `completion_context`
    /// is always used for alias expansion (callers without a live session should pass an
    /// `EmptyCompletionContext`).
    pub fn detect_and_set_input_type<C: CompletionContext + Clone + Send + 'static>(
        &mut self,
        input: ParsedTokensSnapshot,
        completion_context: C,
        session_id: Option<SessionId>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Abort the last autodetect handle if exists.
        self.abort_in_progress_detection();

        // If the input mode is locked, there's no point in running autodetection.
        if !self.should_run_input_autodetection(ctx) {
            return;
        }

        if self
            .last_explicit_input_type_set_at
            .map(|last_explicitly_set_at| {
                Instant::now()
                    < last_explicitly_set_at
                        + std::time::Duration::from_millis(AUTODETECTION_DISABLE_DURATION_MS)
            })
            .unwrap_or(false)
        {
            return;
        }

        let first_token_str = input.parsed_tokens.first().map(|t| t.token.clone());
        let Some(first_token_str) = first_token_str else {
            // If the buffer is empty, short-circuit and leave input type unchanged.
            //
            // We don't know enough (anything) to be able to change the input type one way or
            // another.
            return;
        };

        let denylist: Vec<&str> = AISettings::as_ref(ctx)
            .autodetection_command_denylist
            .value()
            .split(',')
            .collect();

        // Early return if the first token is included in the denylist. No need to parse history.
        if denylist.contains(&first_token_str.as_str()) {
            self.set_input_config_internal(
                InputConfig {
                    input_type: InputType::Shell,
                    ..self.input_config()
                },
                ctx,
            );
            return;
        }

        // If we have a session, gather history entries for matching.
        let history_entries = session_id.map(|sid| {
            History::as_ref(ctx)
                .commands(sid)
                .into_iter()
                .flatten()
                .filter_map(|entry| {
                    if entry
                        .exit_code
                        .is_some_and(|code| code.was_command_not_found())
                    {
                        return None;
                    }
                    Some(entry.command.to_string())
                })
                .collect::<Vec<String>>()
        });

        let buffer_cloned = input.buffer_text.clone();
        let other_buffer_cloned = buffer_cloned.clone();
        let current_input_type = self.input_type();

        let is_udi_enabled = InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);

        // Determine if the input is a follow-up to an AI block.
        let is_agent_follow_up = {
            let model = self.model.lock();
            let block_list = model.block_list();
            let block_index = block_list.last_non_hidden_block_by_index();
            match block_list.last_non_hidden_rich_content_block_after_block(block_index) {
                Some((_, content)) => content.content_type == Some(RichContentType::AIBlock),
                _ => false,
            }
        };

        let classifier = InputClassifierModel::as_ref(ctx).classifier();
        let handle = ctx
            .spawn(
                async move {
                    // First check if the token is a natural language word, if current input type is AI
                    if matches!(current_input_type, InputType::AI)
                        && is_one_off_natural_language_word(first_token_str.to_lowercase().as_str())
                    {
                        return InputType::AI;
                    }

                    // If this is clearly intended to be a follow-up to an AI block, classify it as AI.
                    if is_agent_follow_up
                        && is_agent_follow_up_input(&buffer_cloned.trim().to_lowercase())
                    {
                        return InputType::AI;
                    }

                    // If we have history entries (i.e., a live session), check for
                    // close matches to short-circuit as shell input.
                    if let Some(history_entries) = history_entries {
                        if has_any_close_matches(
                            &buffer_cloned,
                            history_entries.iter().map(AsRef::as_ref),
                            HISTORY_ENTRY_MATCH_CUTOFF,
                        )
                        .await
                        {
                            return InputType::Shell;
                        }
                    }

                    // Yield so that an attempt to abort the classification is handled.  We do this periodically
                    // so that we can skip doing additional expensive work if the classification is aborted.
                    futures_lite::future::yield_now().await;

                    let input =
                        warp_completer::util::expand_aliases(input, &completion_context).await;

                    futures_lite::future::yield_now().await;

                    let context = input_classifier::Context {
                        current_input_type,
                        is_agent_follow_up,
                    };
                    let new_input_type =
                        classifier.detect_input_type(input.clone(), &context).await;

                    futures_lite::future::yield_now().await;

                    new_input_type
                },
                move |me, new_input_type, ctx| {
                    // In theory, we shouldn't need to check this, as we only run autodetection if the input
                    // is not locked, and we should abort the autodetect future if the input is locked, but
                    // we do it anyway out of an abundance of caution.
                    if !me.should_run_input_autodetection(ctx) {
                        return;
                    }
                    // If the autodetect abort handle is none, it means we aborted autodetection.
                    // It's possible that the future already completed before we aborted, and then we reach this callback after abort.
                    // In this case, don't set the input type.
                    if me.autodetect_abort_handle.is_none() {
                        return;
                    }
                    me.set_input_config_internal(
                        InputConfig {
                            input_type: new_input_type,
                            ..me.input_config()
                        },
                        ctx,
                    );
                    if current_input_type != new_input_type {
                        let buffer_length = other_buffer_cloned.len();
                        let input_buffer_text_for_telemetry = should_collect_ai_ugc_telemetry(
                            ctx,
                            PrivacySettings::as_ref(ctx).is_telemetry_enabled,
                        )
                        .then_some(other_buffer_cloned);
                        send_telemetry_from_ctx!(
                            TelemetryEvent::AgentModeChangedInputType {
                                input: input_buffer_text_for_telemetry,
                                buffer_length,
                                is_manually_changed: false,
                                new_input_type,
                                active_block_id: me
                                    .model
                                    .lock()
                                    .block_list()
                                    .active_block_id()
                                    .clone(),
                                is_udi_enabled,
                            },
                            ctx
                        );
                    }
                },
            )
            .abort_handle();
        self.autodetect_abort_handle = Some(handle);
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

/// Returns whether the set of possibilities contains any close matches
/// to the provided word, using the given similarity threshold.
///
/// Adapted from [`difflib::get_close_matches`], but an async function with
/// periodic yields such that the operation can be aborted if necessary.  Also,
/// unlike the original function, this returns as soon as it finds any match
/// above the threshold, instead of finding _all_ matches above the threshold
/// and returning the top N matches.
async fn has_any_close_matches<'a>(
    word: &str,
    possibilities: impl Iterator<Item = &'a str>,
    cutoff: f32,
) -> bool {
    const BATCH_SIZE: usize = 50;

    if !(0.0..=1.0).contains(&cutoff) {
        panic!("Cutoff must be greater than 0.0 and lower than 1.0");
    }
    let mut matcher = difflib::sequencematcher::SequenceMatcher::new("", word);
    for (idx, i) in possibilities.enumerate() {
        // Periodically, yield to the executor so this task can be aborted if
        // requested.
        if idx % BATCH_SIZE == 0 {
            futures_lite::future::yield_now().await;
        }

        matcher.set_first_seq(i);
        // The fast ratio computations produce an upper bound on the value of
        // ratio, so if a faster check fails, the slower checks are guaranteed
        // to also fail.
        if matcher.real_quick_ratio() >= cutoff && matcher.ratio() >= cutoff {
            return true;
        }
    }

    false
}
