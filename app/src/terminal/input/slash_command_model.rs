use ai::skills::SkillReference;
use input_classifier::InputType;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_search_core::inline_menu::InputDrivenInlineMenuLifecycle;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::ai::blocklist::{
    BlocklistAIInputEvent, BlocklistAIInputModel, InputTypeAutoDetectionSource,
};
use crate::search::slash_command_menu::StaticCommand;
use crate::settings::InputSettings;
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::slash_commands::{
    GuiSlashCommandDataSource, SlashCommandDataSource as _,
};

/// Event emitted by the slash command model when its entry state is updated.
#[derive(Debug, Clone)]
pub struct UpdatedSlashCommandModel {
    /// The state before the update.
    pub old_state: SlashCommandEntryState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedCommand {
    /// The command in the input.
    pub command: StaticCommand,

    /// The space-delimited argument to the command, if any. Does not include the leading space.
    ///
    /// If there is no trailing space after the command, then `None`.
    pub argument: Option<String>,
}

/// A detected skill command in the input buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedSkillCommand {
    /// Either a path or a bundled_skill_id which uniquely identifies the skill
    pub reference: SkillReference,

    /// The skill name (without the leading '/').
    pub name: String,

    /// The space-delimited argument to the skill command (the user's prompt).
    pub argument: Option<String>,
}

/// Surface-neutral classification of the current slash command input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedSlashCommandInput {
    /// The input is not slash command composition.
    None,
    /// A slash command, saved prompt, or skill is being searched for.
    Composing {
        /// The suffix in the input after '/'.
        filter: String,
    },
    /// A valid static slash command is entered in the input.
    SlashCommand(DetectedCommand),
    /// A valid skill command is entered in the input.
    SkillCommand(DetectedSkillCommand),
}

#[derive(Debug, Clone)]
pub enum SlashCommandEntryState {
    /// The input contents have nothing to do with a slash command.
    None,
    /// '/' and a slash command is being composed.
    Composing {
        /// The suffix in the input after '/'.
        filter: String,
    },
    /// A valid slash command is entered in the input.
    SlashCommand(DetectedCommand),
    /// A valid skill command is entered in the input.
    SkillCommand(DetectedSkillCommand),
}

impl SlashCommandEntryState {
    pub fn detected_command(&self) -> Option<&StaticCommand> {
        match self {
            SlashCommandEntryState::SlashCommand(detected_command) => {
                Some(&detected_command.command)
            }
            _ => None,
        }
    }

    /// Returns `true` if this state has a detected slash command.
    pub fn is_detected_command(&self) -> bool {
        matches!(self, Self::SlashCommand(_))
    }

    /// Returns `true` if a slash command or skill command has been detected.
    pub fn is_detected_command_or_skill(&self) -> bool {
        matches!(self, Self::SlashCommand(_) | Self::SkillCommand(_))
    }

    /// Returns the byte length of the command prefix that should be highlighted
    /// in the input buffer, or `None` if no command/skill is detected.
    pub fn command_prefix_highlight_len(&self, buffer_text: &str) -> Option<usize> {
        match self {
            SlashCommandEntryState::SlashCommand(detected) => buffer_text
                .starts_with(detected.command.name)
                .then_some(detected.command.name.len()),
            SlashCommandEntryState::SkillCommand(detected) => {
                // Skill name doesn't include the leading '/', so we prefix it for matching.
                let prefix_len = 1 + detected.name.len();
                buffer_text
                    .get(..prefix_len)
                    .is_some_and(|p| p.starts_with('/') && p[1..] == *detected.name)
                    .then_some(prefix_len)
            }
            SlashCommandEntryState::None | SlashCommandEntryState::Composing { .. } => None,
        }
    }

    fn pending_command(&self) -> Option<&String> {
        match self {
            SlashCommandEntryState::Composing { filter } => Some(filter),
            _ => None,
        }
    }
}

pub fn slash_command_composition_filter(input: &str) -> Option<&str> {
    let pending_command = input.strip_prefix('/')?;
    let command_token = pending_command
        .split_once(' ')
        .map_or(pending_command, |(command, _)| command);
    if command_token.contains('/') {
        None
    } else {
        Some(pending_command)
    }
}

pub struct SlashCommandModel {
    input_buffer_model: ModelHandle<InputBufferModel>,
    ai_input_model: ModelHandle<BlocklistAIInputModel>,
    state: SlashCommandEntryState,
    lifecycle: InputDrivenInlineMenuLifecycle,
    data_source: ModelHandle<GuiSlashCommandDataSource>,
}

impl SlashCommandModel {
    pub fn new(
        buffer_model: &ModelHandle<InputBufferModel>,
        ai_input_model: &ModelHandle<BlocklistAIInputModel>,
        data_source: ModelHandle<GuiSlashCommandDataSource>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(buffer_model, |me, _, event, ctx| {
            me.handle_input_buffer_update(event, ctx);
        });

        if !FeatureFlag::AgentView.is_enabled() {
            // In the old modality, slash commands are disabled in locked shell mode.
            //
            // In the new modality, slash commands _are_ accessible in the terminal view, which is
            // in locked shell mode if NLD is disabled.
            ctx.subscribe_to_model(ai_input_model, |me, _, event, ctx| match event {
                BlocklistAIInputEvent::InputTypeChanged { config }
                | BlocklistAIInputEvent::LockChanged { config } => {
                    if config.is_locked {
                        if config.is_shell() && !me.is_disabled() {
                            let input_is_empty =
                                me.input_buffer_model.as_ref(ctx).current_value().is_empty();
                            me.disable_until_empty_buffer(input_is_empty, ctx);
                        } else if !config.is_shell()
                            && me.input_buffer_model.as_ref(ctx).current_value().is_empty()
                        {
                            me.lifecycle.input_changed(true, false);
                            let old_state =
                                std::mem::replace(&mut me.state, SlashCommandEntryState::None);
                            ctx.emit(UpdatedSlashCommandModel { old_state });
                        }
                    }
                }
            });
        }

        Self {
            input_buffer_model: buffer_model.clone(),
            ai_input_model: ai_input_model.clone(),
            data_source,
            state: SlashCommandEntryState::None,
            lifecycle: InputDrivenInlineMenuLifecycle::default(),
        }
    }

    /// Called by SlashCommandsMenu when menu is dismissed.
    /// Only `UserEscape` blocks future execution; `NoResults` allows it.
    pub fn disable(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_disabled() {
            return;
        }
        let input_is_empty = self
            .input_buffer_model
            .as_ref(ctx)
            .current_value()
            .is_empty();
        if input_is_empty {
            return;
        }

        // In the old modality, the input mode is always set to AI mode when a slash command
        // is being composed. We interpret slash command menu dismissal as intent to execute a
        // shell command.
        //
        // In the new modality, we don't implicitly tie slash command composition to a specific
        // input mode, so we shouldn't change the input mode based on slash command disablement.
        if !FeatureFlag::AgentView.is_enabled()
            && !self.ai_input_model.as_ref(ctx).is_input_type_locked()
        {
            self.ai_input_model.update(ctx, |input_model, ctx| {
                input_model.set_input_type(
                    InputType::Shell,
                    Some(InputTypeAutoDetectionSource::SlashCommand),
                    ctx,
                );
            });
        }

        self.disable_until_empty_buffer(input_is_empty, ctx);
    }

    /// Returns whether slash command execution should be allowed.
    pub fn is_disabled(&self) -> bool {
        !self.lifecycle.is_enabled()
    }

    pub fn state(&self) -> &SlashCommandEntryState {
        &self.state
    }

    fn disable_until_empty_buffer(&mut self, input_is_empty: bool, ctx: &mut ModelContext<Self>) {
        if self.is_disabled() {
            return;
        }
        self.lifecycle.disable_until_empty_buffer(input_is_empty);
        if self.lifecycle.is_enabled() {
            return;
        }
        let old_state = std::mem::replace(&mut self.state, SlashCommandEntryState::None);
        ctx.emit(UpdatedSlashCommandModel { old_state });
    }

    /// Parses `text` into a `SlashCommandEntryState` without mutating the
    /// model or emitting events.
    /// Use this when you have a prompt string and need to know whether it is
    /// a slash command, skill command, or plain text.
    pub fn detect_command(&self, text: &str, ctx: &AppContext) -> SlashCommandEntryState {
        match self.data_source.as_ref(ctx).parse_input(text, ctx) {
            ParsedSlashCommandInput::SlashCommand(detected) => {
                SlashCommandEntryState::SlashCommand(detected)
            }
            ParsedSlashCommandInput::SkillCommand(detected) => {
                SlashCommandEntryState::SkillCommand(detected)
            }
            ParsedSlashCommandInput::None | ParsedSlashCommandInput::Composing { .. } => {
                SlashCommandEntryState::None
            }
        }
    }

    fn handle_input_buffer_update(
        &mut self,
        event: &InputBufferUpdateEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let InputBufferUpdateEvent {
            new_content: new, ..
        } = event;
        self.lifecycle
            .input_changed(new.is_empty(), new.starts_with('/'));
        // AI-off is no longer a blanket disable: AI-dependent commands are filtered out
        // of `active_commands` via `Availability::AI_ENABLED`, so parsing still works for
        // non-AI commands like `/open-file`.
        if !FeatureFlag::AgentView.is_enabled() {
            let ai_input_model = self.ai_input_model.as_ref(ctx);
            if ai_input_model.is_input_type_locked() && !ai_input_model.is_ai_input_enabled() {
                self.disable_until_empty_buffer(new.is_empty(), ctx);
                return;
            }
        } else if !self.data_source.as_ref(ctx).is_agent_view_active(ctx)
            && !self.data_source.as_ref(ctx).is_cli_agent_input_open(ctx)
            && !*InputSettings::as_ref(ctx)
                .enable_slash_commands_in_terminal
                .value()
            && !self.is_disabled()
        {
            self.disable_until_empty_buffer(new.is_empty(), ctx);
            return;
        }

        if new.is_empty() {
            // The buffer was cleared, so reset state.
            let old_state = std::mem::replace(&mut self.state, SlashCommandEntryState::None);
            ctx.emit(UpdatedSlashCommandModel { old_state });
            return;
        }
        if self.is_disabled() {
            return;
        }

        let old_state = self.state.clone();
        match self.data_source.as_ref(ctx).parse_input(new, ctx) {
            ParsedSlashCommandInput::SlashCommand(detected_command) => {
                if let SlashCommandEntryState::SlashCommand(old_detected_command) = &self.state {
                    if *old_detected_command == detected_command {
                        return;
                    }
                }

                if !FeatureFlag::AgentView.is_enabled()
                    || detected_command.command.auto_enter_ai_mode
                {
                    // In the old modality, when there is a detected slash command, the input _must_ be in
                    // AI mode; we don't respect `StaticCommand::auto_enter_ai_mode = false`. That field is
                    // only used in the new modality.
                    //
                    // The fact that we've even detected a command implies that the input mode is in AI
                    // mode, either locked or unlocked; if the input were locked to shell mode then the
                    // shared lifecycle would have short-circuited above.
                    self.ai_input_model.update(ctx, |input_model, ctx| {
                        input_model.set_input_type(
                            InputType::AI,
                            Some(InputTypeAutoDetectionSource::SlashCommand),
                            ctx,
                        );
                    });
                }
                self.state = SlashCommandEntryState::SlashCommand(detected_command);
            }
            ParsedSlashCommandInput::SkillCommand(detected_skill) => {
                if let SlashCommandEntryState::SkillCommand(old_detected_skill) = &self.state {
                    if *old_detected_skill == detected_skill {
                        return;
                    }
                }

                // Skill commands always require AI mode
                self.ai_input_model.update(ctx, |input_model, ctx| {
                    input_model.set_input_type(
                        InputType::AI,
                        Some(InputTypeAutoDetectionSource::SlashCommand),
                        ctx,
                    );
                });
                self.state = SlashCommandEntryState::SkillCommand(detected_skill);
            }
            ParsedSlashCommandInput::Composing {
                filter: pending_command,
            } => {
                if self
                    .state
                    .pending_command()
                    .is_some_and(|command| command == &pending_command)
                {
                    return;
                }

                if !FeatureFlag::AgentView.is_enabled() {
                    // In the old modality, when composing a slash command, the input _must_ be in
                    // AI mode; we don't respect `StaticCommand::auto_enter_ai_mode = false`. That
                    // field is only used in the new modality.
                    //
                    // We don't even rely on the fact that the input is in AI mode while a slash
                    // command is being composed, its solely used to disable error underlining.
                    //
                    // In the new modality, slash commands declare whether or not they are
                    // available in terminal mode, and syntax highlighting/error underlining is
                    // handled appropriately. I am just making this change to preserve the existing
                    // product behavior (agent icon in NLD toggle becomes yellow).
                    self.ai_input_model.update(ctx, |input_model, ctx| {
                        input_model.set_input_type(
                            InputType::AI,
                            Some(InputTypeAutoDetectionSource::SlashCommand),
                            ctx,
                        );
                    });
                }
                self.state = SlashCommandEntryState::Composing {
                    filter: pending_command,
                };
            }
            ParsedSlashCommandInput::None => {
                self.state = SlashCommandEntryState::None;
            }
        }

        ctx.emit(UpdatedSlashCommandModel { old_state });
    }
}

impl Entity for SlashCommandModel {
    type Event = UpdatedSlashCommandModel;
}

#[cfg(test)]
#[path = "slash_command_model_tests.rs"]
mod tests;
