mod cloud_mode_v2_view;
mod data_source;
mod search_item;
pub(super) mod view;

#[cfg(feature = "local_fs")]
use std::path::PathBuf;

use ai::skills::SkillReference;
pub use cloud_mode_v2_view::{CloudModeV2SlashCommandView, Section as CloudModeV2Section};
pub use data_source::*;
pub use view::{CloseReason, InlineSlashCommandView, SlashCommandsEvent};
#[cfg(not(target_family = "wasm"))]
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::AnsiColorIdentifier;
#[cfg(feature = "local_fs")]
use warp_util::path::{CleanPathResult, LineAndColumnArg};
use warpui::clipboard::ClipboardContent;
use warpui::{AppContext, SingletonEntity, ViewContext};

#[cfg(not(target_family = "wasm"))]
use crate::ai::agent::conversation::AIConversationId;
#[cfg(not(target_family = "wasm"))]
use crate::ai::agent_conversations_model::AgentConversationsModel;
#[cfg(not(target_family = "wasm"))]
use crate::ai::agent_management::telemetry::AgentManagementTelemetryEvent;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::ambient_agents::telemetry::HandoffEntryPoint;
use crate::ai::blocklist::agent_view::{
    AgentViewEntryOrigin, DismissalStrategy, EphemeralMessage, ENTER_OR_EXIT_CONFIRMATION_WINDOW,
};
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::blocklist::handoff::PendingCloudLaunch;
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, InputTypeAutoDetectionSource, QueuedQuery, QueuedQueryModel,
    QueuedQueryOrigin, SlashCommandRequest,
};
use crate::cloud_object::model::persistence::CloudModel;
use crate::code_review::telemetry_event::CodeReviewPaneEntrypoint;
use crate::search::slash_command_menu::static_commands::commands::{self, COMMAND_REGISTRY};
use crate::search::slash_command_menu::static_commands::Availability;
use crate::search::slash_command_menu::{SlashCommandId, StaticCommand};
use crate::server::ids::SyncId;
use crate::server::telemetry::SlashCommandAcceptedDetails;
use crate::settings::AISettings;
use crate::tab::SelectedTabColor;
use crate::terminal::input::decorations::InputBackgroundJobOptions;
use crate::terminal::input::inline_menu::{InlineMenuAction, InlineMenuType};
use crate::terminal::input::message_bar::Message;
use crate::terminal::input::slash_command_model::{
    SlashCommandEntryState, UpdatedSlashCommandModel,
};
use crate::terminal::input::{
    CompletionsTrigger, Event, Input, InputAction, InputSuggestionsMode, UserQueryMenuAction,
};
#[cfg(feature = "local_fs")]
use crate::terminal::model::session::Session;
use crate::terminal::view::TerminalAction;
use crate::ui_components::color_dot;
use crate::view_components::DismissibleToast;
use crate::workflows::{WorkflowSelectionSource, WorkflowSource, WorkflowType};
use crate::workspace::{ForkedConversationDestination, ToastStack, WorkspaceAction};
use crate::TelemetryEvent;

#[derive(Debug, Clone)]
pub enum AcceptSlashCommandOrSavedPrompt {
    SlashCommand {
        id: SlashCommandId,
    },
    SavedPrompt {
        id: SyncId,
    },
    /// A skill selected from browse or search. Contains name (for display/insertion) and path/bundled_skill_id (for execution).
    Skill {
        reference: SkillReference,
        name: String,
    },
}
impl InlineMenuAction for AcceptSlashCommandOrSavedPrompt {
    const MENU_TYPE: InlineMenuType = InlineMenuType::SlashCommands;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SlashCommandTrigger {
    Input { cmd_or_ctrl_enter: bool },
    Keybinding,
}

impl SlashCommandTrigger {
    fn cmd_or_ctrl_enter() -> Self {
        Self::Input {
            cmd_or_ctrl_enter: true,
        }
    }

    pub fn input() -> Self {
        Self::Input {
            cmd_or_ctrl_enter: false,
        }
    }

    pub(super) fn keybinding() -> Self {
        Self::Keybinding
    }

    pub fn is_keybinding(&self) -> bool {
        matches!(self, Self::Keybinding)
    }

    fn is_cmd_or_ctrl_enter(&self) -> bool {
        matches!(
            self,
            Self::Input {
                cmd_or_ctrl_enter: true
            }
        )
    }
}

#[cfg(feature = "local_fs")]
fn open_file_command_path(
    session: &Session,
    current_dir: &str,
    raw_arg: &str,
) -> (PathBuf, Option<LineAndColumnArg>) {
    let parsed_path = CleanPathResult::with_line_and_column_number(raw_arg.trim());
    // The argument may contain shell-escaped characters (e.g. `\ ` for spaces) from auto-suggest.
    // Unescape them so the path matches the actual filesystem entry.
    let unescaped_path = session.shell_family().unescape(&parsed_path.path);
    // Expand `~` to the user's home directory.
    let expanded_path = shellexpand::tilde(&unescaped_path);

    let shell_path = session
        .convert_directory_to_typed_path_buf(current_dir.to_owned())
        .join(session.convert_directory_to_typed_path_buf(expanded_path.into_owned()))
        .normalize();
    let file_path = session
        .maybe_convert_to_native_path(&shell_path.to_path())
        .unwrap_or_else(|err| {
            log::warn!("unable to convert /open-file path to native path: {err:?}");
            PathBuf::from(shell_path.to_string_lossy().into_owned())
        });

    (file_path, parsed_path.line_and_column_num)
}

impl Input {
    fn is_slash_command_available(&self, command: &StaticCommand, ctx: &AppContext) -> bool {
        let slash_command_data_source = if self.is_cloud_mode_input_v2_composing(ctx) {
            let Some(data_source) = self.cloud_mode_composer_slash_command_data_source.as_ref()
            else {
                return false;
            };
            data_source
        } else {
            &self.slash_command_data_source
        };
        slash_command_data_source
            .as_ref(ctx)
            .command_is_active(command, ctx)
    }

    pub(super) fn select_slash_command(
        &mut self,
        command: &StaticCommand,
        trigger: SlashCommandTrigger,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.is_slash_command_available(command, ctx) {
            return;
        }
        if command.argument.as_ref().is_none() {
            self.execute_slash_command(
                command, None, trigger, /*is_queued_prompt*/ false, ctx,
            );
        } else if command
            .argument
            .as_ref()
            .is_some_and(|arg| arg.should_execute_on_selection)
        {
            // TODO (zachbai): this is a hack for Oz launch. Caller
            // should probably be invoking `execute_slash_command` in this case.
            let argument = if !self.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                let trimmed = self.buffer_text(ctx).trim().to_owned();
                (!trimmed.is_empty()).then_some(trimmed)
            } else {
                None
            };
            self.execute_slash_command(
                command,
                argument.as_ref(),
                trigger,
                /*is_queued_prompt*/ false,
                ctx,
            );
        } else {
            self.editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&format!("{} ", command.name), ctx);
            });
        }
    }

    pub(super) fn close_slash_commands_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.suggestions_mode_model.update(ctx, |model, ctx| {
            model.set_mode(InputSuggestionsMode::Closed, ctx);
        });
        ctx.notify();
    }

    pub(super) fn handle_slash_command_model_event(
        &mut self,
        event: &UpdatedSlashCommandModel,
        ctx: &mut ViewContext<Self>,
    ) {
        // Refresh decorations if the slash command detection state changed, since
        // detected commands affect syntax highlighting.
        let new_state = self.slash_command_model.as_ref(ctx).state();
        if event.old_state.is_detected_command() != new_state.is_detected_command() {
            let _ = self
                .debounce_input_background_tx
                .try_send(InputBackgroundJobOptions::default().with_command_decoration());
        }

        match self.slash_command_model.as_ref(ctx).state().clone() {
            SlashCommandEntryState::None | SlashCommandEntryState::DisabledUntilEmptyBuffer => {
                if self.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                    self.close_slash_commands_menu(ctx);
                }
            }
            SlashCommandEntryState::Composing { .. } => {
                if self.suggestions_mode_model.as_ref(ctx).is_closed() {
                    self.open_slash_commands_menu(ctx);
                } else if !self.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                    self.slash_command_model.update(ctx, |model, ctx| {
                        model.disable(ctx);
                    });
                }
            }
            SlashCommandEntryState::SlashCommand(detected_command) => {
                // If there is only one result (or zero, but that should be impossible if there is
                // a valid command in the input) OR if the user has started typing arguments, hide
                // the menu.
                if self.suggestions_mode_model.as_ref(ctx).is_slash_commands()
                    && (self
                        .inline_slash_commands_view
                        .as_ref(ctx)
                        .result_count(ctx)
                        < 2
                        || detected_command.argument.is_some())
                {
                    self.close_slash_commands_menu(ctx);
                }

                if detected_command.command.auto_enter_ai_mode
                    || !FeatureFlag::AgentView.is_enabled()
                {
                    self.enter_ai_mode(Some(InputTypeAutoDetectionSource::SlashCommand), ctx);
                }

                if detected_command.command.name == commands::EDIT.name
                    && detected_command
                        .argument
                        .as_ref()
                        .is_some_and(|argument| argument.is_empty())
                    && self.suggestions_mode_model.as_ref(ctx).is_closed()
                {
                    self.open_completion_suggestions(CompletionsTrigger::Keybinding, ctx);
                }
            }
            SlashCommandEntryState::SkillCommand(detected_skill) => {
                // Hide the menu once the user has started typing the prompt
                if self.suggestions_mode_model.as_ref(ctx).is_slash_commands()
                    && (self
                        .inline_slash_commands_view
                        .as_ref(ctx)
                        .result_count(ctx)
                        < 2
                        || detected_skill.argument.is_some())
                {
                    self.close_slash_commands_menu(ctx);
                }

                // Skill commands always require AI mode
                self.enter_ai_mode(Some(InputTypeAutoDetectionSource::SlashCommand), ctx);
            }
        }
    }

    pub(crate) fn handle_slash_commands_menu_event(
        &mut self,
        event: &SlashCommandsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SlashCommandsEvent::Close(reason) => {
                if reason.is_manual_dismissal() {
                    self.slash_command_model.update(ctx, |model, ctx| {
                        model.disable(ctx);
                    });
                }

                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
                ctx.notify();
            }
            SlashCommandsEvent::SelectedSavedPrompt { id } => {
                let Some(workflow) = CloudModel::as_ref(ctx).get_workflow(id).cloned() else {
                    log::warn!("Tried to execute workflow for id {id:?} but it does not exist");
                    return;
                };
                let is_in_agent_view = FeatureFlag::AgentView.is_enabled()
                    && self.agent_view_controller.as_ref(ctx).is_fullscreen();
                send_telemetry_from_ctx!(
                    TelemetryEvent::SlashCommandAccepted {
                        command_details: SlashCommandAcceptedDetails::SavedPrompt,
                        is_in_agent_view,
                    },
                    ctx
                );

                self.show_workflows_info_box_on_workflow_selection(
                    WorkflowType::Cloud(Box::new(workflow)),
                    WorkflowSource::WarpAI,
                    WorkflowSelectionSource::SlashMenu,
                    None,
                    ctx,
                );
            }
            SlashCommandsEvent::SelectedStaticCommand {
                id,
                cmd_or_ctrl_enter,
            } => {
                let Some(command) = COMMAND_REGISTRY.get_command(id) else {
                    return;
                };
                self.select_slash_command(
                    command,
                    SlashCommandTrigger::Input {
                        cmd_or_ctrl_enter: *cmd_or_ctrl_enter,
                    },
                    ctx,
                );
            }
            SlashCommandsEvent::SelectedSkill { name, reference: _ } => {
                // Insert /{skill-name} into the buffer
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(format!("/{name} ").as_str(), ctx);
                });
                self.close_slash_commands_menu(ctx);
            }
        }
    }

    /// Executes the given `command` with `argument`, if any.
    ///
    /// When `is_queued_prompt` is true, this is the first send of a previously queued prompt:
    /// the input buffer is left alone so the user doesn't lose anything they've typed while
    /// the agent was busy.
    ///
    /// Returns `true` if execution was 'handled' (whether or not it resulted in success or failure).
    pub(super) fn execute_slash_command(
        &mut self,
        command: &StaticCommand,
        argument: Option<&String>,
        trigger: SlashCommandTrigger,
        is_queued_prompt: bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        fn show_error_toast(message: String, ctx: &mut ViewContext<Input>) {
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_ephemeral_toast(DismissibleToast::error(message), window_id, ctx);
            });
        }

        // Safety net: commands whose availability requires AI should not execute when AI is
        // globally disabled. They're normally filtered out of the slash command menu, but this
        // protects keybinding-triggered execution where a bound key may still address the command.
        if command.availability.contains(Availability::AI_ENABLED)
            && !AISettings::as_ref(ctx).is_any_ai_enabled(ctx)
        {
            show_error_toast(format!("{} requires AI to be enabled", command.name), ctx);
            return true;
        }

        // Handle the slash command action based on its kind
        match command.name {
            add_mcp if command.name == commands::ADD_MCP.name => {
                ctx.dispatch_typed_action(&TerminalAction::OpenAddMCPPane);
            }
            add_prompt if command.name == commands::ADD_PROMPT.name => {
                ctx.dispatch_typed_action(&TerminalAction::OpenAddPromptPane);
            }
            add_rule if command.name == commands::ADD_RULE.name => {
                ctx.dispatch_typed_action(&TerminalAction::OpenAddRulePane);
            }
            agent_or_new
                if command.name == commands::NEW.name || command.name == commands::AGENT.name =>
            {
                if !self
                    .ai_context_model
                    .as_ref(ctx)
                    .can_start_new_conversation()
                {
                    self.ephemeral_message_model.update(ctx, |model, ctx| {
                        let appearance = Appearance::handle(ctx).as_ref(ctx);
                        let message = Message::from_text(
                            "cannot start new conversation while terminal command is running",
                        )
                        .with_text_color(appearance.theme().ansi_fg_red());
                        model.show_ephemeral_message(
                            EphemeralMessage::new(
                                message,
                                DismissalStrategy::Timer(ENTER_OR_EXIT_CONFIRMATION_WINDOW),
                            ),
                            ctx,
                        );
                    });
                    return true;
                }
                // Keybindings can be triggered reflexively while users are already in an active
                // conversation, so we gate only this path behind a second-press confirmation.
                // Typed `/agent`/`/new` and slash-menu execution stay single-step by design.
                if trigger.is_keybinding() && self.agent_view_controller.as_ref(ctx).is_active() {
                    let should_start_new_conversation =
                        self.agent_view_controller.update(ctx, |controller, ctx| {
                            controller
                                .should_start_new_conversation_for_keybinding(command.name, ctx)
                        });
                    if !should_start_new_conversation {
                        // Keep the current input/conversation untouched on first press; only the
                        // ephemeral confirmation prompt should change.
                        return true;
                    }
                }

                let prompt = argument.and_then(|argument| {
                    let trimmed = argument.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_owned())
                    }
                });

                ctx.emit(Event::EnterAgentView {
                    initial_prompt: prompt,
                    conversation_id: None,
                    origin: AgentViewEntryOrigin::SlashCommand { trigger },
                });
            }
            cloud_agent if command.name == commands::CLOUD_AGENT.name => {
                let prompt = argument.and_then(|argument| {
                    let trimmed = argument.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_owned())
                    }
                });

                ctx.emit(Event::EnterCloudAgentView {
                    initial_prompt: prompt,
                });
            }
            create_docker_sandbox if command.name == commands::CREATE_DOCKER_SANDBOX.name => {
                ctx.emit(Event::CreateDockerSandbox);
            }
            conversations if command.name == commands::CONVERSATIONS.name => {
                if self.is_cloud_mode_input_v2_composing(ctx) {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.set_mode(InputSuggestionsMode::Closed, ctx);
                    });
                    self.clear_buffer_and_reset_undo_stack(ctx);
                    if let Some(view) = self.cloud_mode_v2_history_menu_view.clone() {
                        view.update(ctx, |v, ctx| {
                            v.arm_initial_buffer_sync(ctx);
                        });
                    }
                    ctx.dispatch_typed_action_deferred(InputAction::OpenInlineHistoryMenu);
                    return true;
                } else if FeatureFlag::AgentView.is_enabled() {
                    self.open_conversation_menu(ctx);
                } else {
                    ctx.dispatch_typed_action(&TerminalAction::OpenConversationsPalette);
                }
            }
            rename_tab if command.name == commands::RENAME_TAB.name => {
                let Some(name) = argument
                    .map(|name| name.trim())
                    .filter(|name| !name.is_empty())
                else {
                    show_error_toast(
                        "Please provide a tab name after /rename-tab".to_owned(),
                        ctx,
                    );
                    return true;
                };

                ctx.dispatch_typed_action(&WorkspaceAction::SetActiveTabName(name.to_owned()));
            }
            set_tab_color if command.name == commands::SET_TAB_COLOR.name => {
                let supported_options = || {
                    color_dot::TAB_COLOR_OPTIONS
                        .iter()
                        .map(|c| c.to_string().to_ascii_lowercase())
                        .chain(std::iter::once("none".to_owned()))
                        .collect::<Vec<_>>()
                        .join(", ")
                };

                let Some(arg) = argument
                    .map(|name| name.trim())
                    .filter(|name| !name.is_empty())
                else {
                    show_error_toast(
                        format!(
                            "Please provide a color after /set-tab-color ({})",
                            supported_options()
                        ),
                        ctx,
                    );
                    return true;
                };

                let color = if arg.eq_ignore_ascii_case("none") {
                    SelectedTabColor::Cleared
                } else {
                    let parsed = arg
                        .parse::<AnsiColorIdentifier>()
                        .ok()
                        .filter(|c| color_dot::TAB_COLOR_OPTIONS.contains(c));
                    match parsed {
                        Some(c) => SelectedTabColor::Color(c),
                        None => {
                            show_error_toast(
                                format!(
                                    "Unknown tab color '{arg}'. Use one of: {}.",
                                    supported_options()
                                ),
                                ctx,
                            );
                            return true;
                        }
                    }
                };

                ctx.dispatch_typed_action(&WorkspaceAction::SetActiveTabColor(color));
            }
            create_env if command.name == commands::CREATE_ENVIRONMENT.name => {
                // If the user included args after the slash command, treat them as repo paths/URLs.
                let repos = argument
                    .map(|arg| {
                        arg.split_whitespace()
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();

                ctx.emit(Event::TriggerEnvironmentSetup { repos });
            }
            create_project if command.name == commands::CREATE_NEW_PROJECT.name => {
                if argument.is_none_or(|args| args.is_empty()) {
                    show_error_toast(
                        "Please describe the project you want to create after /create-new-project"
                            .to_owned(),
                        ctx,
                    );
                    return true;
                }

                let args = argument.expect("args are Some()");
                self.initiate_create_new_project(args.to_owned(), ctx);
            }
            edit if command.name == commands::EDIT.name => {
                #[cfg(feature = "local_fs")]
                match argument {
                    Some(args) if !args.is_empty() => {
                        let Some(session_id) = self.active_block_session_id() else {
                            return false;
                        };

                        let Some(session) = self.sessions.as_ref(ctx).get(session_id) else {
                            return false;
                        };

                        if !session.is_local() {
                            let window_id = ctx.window_id();
                            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                                toast_stack.add_ephemeral_toast(
                                    DismissibleToast::error(
                                        "The /open-file command is only available for local sessions"
                                            .to_owned(),
                                    ),
                                    window_id,
                                    ctx,
                                );
                            });
                            return false;
                        }

                        let current_dir = self
                            .active_block_metadata
                            .as_ref()
                            .and_then(|metadata| metadata.current_working_directory())
                            .map(str::to_owned);

                        let Some(current_dir) = current_dir else {
                            return false;
                        };

                        let (file_path, line_col) =
                            open_file_command_path(&session, &current_dir, args);

                        match std::fs::metadata(&file_path) {
                            Ok(metadata) if metadata.is_file() => {
                                use crate::util::file::external_editor;

                                ctx.dispatch_typed_action(&TerminalAction::OpenCodeInWarp {
                                    path: file_path,
                                    layout: external_editor::settings::EditorLayout::SplitPane,
                                    line_col,
                                });
                            }
                            Ok(_) => {
                                show_error_toast(
                                    "The /open-file command only works for files, not directories"
                                        .to_owned(),
                                    ctx,
                                );
                                return true;
                            }
                            Err(_) => {
                                show_error_toast(
                                    format!("File not found: {}", file_path.display()),
                                    ctx,
                                );
                                return true;
                            }
                        }
                    }
                    _ => {
                        use crate::server::telemetry::PaletteSource;

                        ctx.emit(Event::OpenFilesPalette {
                            source: PaletteSource::Keybinding,
                        });
                    }
                }
                #[cfg(not(feature = "local_fs"))]
                {
                    show_error_toast(
                        "The /open-file command is not supported in this build".to_owned(),
                        ctx,
                    );
                    return true;
                }
            }
            export_to_clipboard if command.name == commands::EXPORT_TO_CLIPBOARD.name => {
                let history = BlocklistAIHistoryModel::handle(ctx);
                let Some(conversation) = history
                    .as_ref(ctx)
                    .active_conversation(self.terminal_view_id)
                else {
                    show_error_toast("No active conversation to export".to_owned(), ctx);
                    return true;
                };

                let action_model = self.ai_action_model.as_ref(ctx);
                let conversation_text = conversation.export_to_markdown(Some(action_model));

                ctx.clipboard()
                    .write(ClipboardContent::plain_text(conversation_text));

                // Show a toast to confirm the export
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::default(String::from(
                        "Conversation exported to clipboard",
                    ));
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
            }
            export_to_file if command.name == commands::EXPORT_TO_FILE.name => {
                #[cfg(not(target_family = "wasm"))]
                {
                    self.export_conversation_to_file(
                        argument.map(|filename| filename.to_owned()),
                        ctx,
                    );
                }
                #[cfg(target_family = "wasm")]
                {
                    show_error_toast(
                        "Export conversation to file unsupported in web".to_owned(),
                        ctx,
                    );
                    return true;
                }
            }
            index if command.name == commands::INDEX.name => {
                ctx.dispatch_typed_action(&TerminalAction::IndexProjectSpeedbump);
            }
            init if command.name == commands::INIT.name => {
                ctx.dispatch_typed_action(&TerminalAction::InitProject);
            }
            changelog if command.name == commands::CHANGELOG.name => {
                if !FeatureFlag::Changelog.is_enabled() {
                    return false;
                }
                ctx.dispatch_typed_action(&WorkspaceAction::ViewLatestChangelog);
            }
            feedback if command.name == commands::FEEDBACK.name => {
                ctx.dispatch_typed_action(&WorkspaceAction::SendFeedback);
            }
            open_code_review if command.name == commands::OPEN_CODE_REVIEW.name => {
                ctx.dispatch_typed_action(&TerminalAction::ToggleCodeReviewPane {
                    entrypoint: CodeReviewPaneEntrypoint::SlashCommand,
                });
            }
            open_mcp_servers if command.name == commands::OPEN_MCP_SERVERS.name => {
                ctx.dispatch_typed_action(&TerminalAction::OpenViewMCPPane);
            }
            open_settings_file if command.name == commands::OPEN_SETTINGS_FILE.name => {
                if !FeatureFlag::SettingsFile.is_enabled() || !cfg!(feature = "local_fs") {
                    return false;
                }
                ctx.dispatch_typed_action(&WorkspaceAction::OpenSettingsFile);
            }
            open_project_rules if command.name == commands::OPEN_PROJECT_RULES.name => {
                ctx.dispatch_typed_action(&TerminalAction::OpenProjectRulesPane);
            }
            open_rules if command.name == commands::OPEN_RULES.name => {
                ctx.dispatch_typed_action(&TerminalAction::OpenRulesPane);
            }
            edit_skill if command.name == commands::EDIT_SKILL.name => {
                if !FeatureFlag::ListSkills.is_enabled() {
                    return false;
                }
                // Open the skill selector menu - user will select a skill from the inline menu
                self.open_skill_selector(ctx);
            }
            invoke_skill if command.name == commands::INVOKE_SKILL.name => {
                if !FeatureFlag::ListSkills.is_enabled() {
                    return false;
                }
                if self.is_cloud_mode_input_v2_composing(ctx) {
                    self.apply_v2_slash_section_filter(CloudModeV2Section::Skills, ctx);
                    return true;
                }
                // Open the skill selector menu for invocation - skill command will be inserted into buffer
                self.open_invoke_skill_selector(ctx);
            }
            host if command.name == commands::HOST.name => {
                if !self.is_cloud_mode_input_v2_composing(ctx) {
                    return false;
                }
                // Only open the host selector when a default host is configured.
                if self
                    .host_selector()
                    .is_none_or(|h| !h.as_ref(ctx).has_default_host())
                {
                    return false;
                }
                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
                self.clear_buffer_and_reset_undo_stack(ctx);
                self.open_v2_host_selector(ctx);
                return true;
            }
            harness if command.name == commands::HARNESS.name => {
                if !self.is_cloud_mode_input_v2_composing(ctx) {
                    // Defensive: the command is registered only when the V2 flag is on and its
                    // availability requires CLOUD_AGENT_V2, so this branch should be unreachable.
                    return false;
                }
                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
                self.clear_buffer_and_reset_undo_stack(ctx);
                self.open_v2_harness_selector(ctx);
                return true;
            }
            environment if command.name == commands::ENVIRONMENT.name => {
                if !self.is_cloud_mode_input_v2_composing(ctx) {
                    return false;
                }
                self.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
                self.clear_buffer_and_reset_undo_stack(ctx);
                self.open_v2_environment_selector(ctx);
                return true;
            }
            models if command.name == commands::MODEL.name => {
                if self.is_cloud_mode_input_v2_composing(ctx) {
                    self.suggestions_mode_model.update(ctx, |model, ctx| {
                        model.set_mode(InputSuggestionsMode::Closed, ctx);
                    });
                    self.clear_buffer_and_reset_undo_stack(ctx);
                    self.agent_input_footer.update(ctx, |footer, ctx| {
                        footer.open_v2_model_selector(ctx);
                    });
                    return true;
                } else {
                    self.open_model_selector(ctx);
                }
            }
            profiles if command.name == commands::PROFILE.name => {
                if !FeatureFlag::InlineProfileSelector.is_enabled() {
                    return false;
                }

                self.open_profile_selector(ctx);
            }
            prompts if command.name == commands::PROMPTS.name => {
                if self.is_cloud_mode_input_v2_composing(ctx) {
                    self.apply_v2_slash_section_filter(CloudModeV2Section::Prompts, ctx);
                    return true;
                }
                if FeatureFlag::AgentView.is_enabled() {
                    self.open_prompts_menu(ctx);
                } else {
                    return false;
                }
            }
            rewind if command.name == commands::REWIND.name => {
                self.open_rewind_menu(ctx);
            }
            pr_comments if command.name == commands::PR_COMMENTS.name => {
                if !FeatureFlag::PRCommentsSlashCommand.is_enabled() {
                    return false;
                }

                let Some(repo_path) = self
                    .active_session_path_if_local(ctx)
                    .map(|path| path.to_path_buf())
                    .map(|path| path.to_string_lossy().to_string())
                else {
                    log::error!("Expected a valid working directory since /pr-comments is only available from the terminal");
                    return false;
                };

                self.ai_controller.update(ctx, move |controller, ctx| {
                    controller.send_slash_command_request(
                        SlashCommandRequest::FetchReviewComments { repo_path },
                        ctx,
                    )
                });
            }
            usage if command.name == commands::USAGE.name => {
                ctx.dispatch_typed_action(&TerminalAction::OpenBillingAndUsagePane);
            }
            remote_control if command.name == commands::REMOTE_CONTROL.name => {
                if !FeatureFlag::CreatingSharedSessions.is_enabled()
                    || !FeatureFlag::HOARemoteControl.is_enabled()
                {
                    return false;
                }
                if self
                    .model
                    .lock()
                    .shared_session_status()
                    .is_sharer_or_viewer()
                {
                    show_error_toast("Session is already being shared".to_owned(), ctx);
                    return true;
                }
                ctx.emit(Event::StartRemoteControl);
            }
            cost if command.name == commands::COST.name => {
                let history = BlocklistAIHistoryModel::handle(ctx);
                let conversation = history
                    .as_ref(ctx)
                    .active_conversation(self.terminal_view_id);
                if conversation.is_none() {
                    show_error_toast(
                        "Cannot show conversation cost: no active conversation".to_owned(),
                        ctx,
                    );
                } else if conversation.is_some_and(|c| c.is_empty()) {
                    show_error_toast(
                        "Cannot show conversation cost: conversation is empty".to_owned(),
                        ctx,
                    );
                } else if conversation.is_some_and(|c| !c.status().is_done()) {
                    show_error_toast(
                        "Cannot show conversation cost: conversation is in progress".to_owned(),
                        ctx,
                    );
                } else {
                    ctx.dispatch_typed_action(&TerminalAction::ToggleUsageFooter);
                }
            }
            #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
            move_to_cloud if command.name == commands::MOVE_TO_CLOUD.name => {
                if !AISettings::as_ref(ctx)
                    .is_cloud_handoff_enabled_for_terminal_view(self.terminal_view_id, ctx)
                {
                    return false;
                }
                let prompt = argument
                    .map(|argument| argument.trim())
                    .filter(|argument| !argument.is_empty())
                    .map(str::to_owned);
                if let Some(prompt) = prompt {
                    // `/handoff query` auto-submits, same as `& query`.
                    let attachments = self.collect_cloud_launch_attachments(ctx);
                    let launch = PendingCloudLaunch {
                        prompt,
                        attachments,
                    };
                    ctx.dispatch_typed_action_deferred(
                        WorkspaceAction::OpenLocalToCloudHandoffPane {
                            launch: Some(launch),
                            environment_id: None,
                            entry_point: HandoffEntryPoint::SlashCommand,
                        },
                    );
                } else {
                    // `/handoff` with no query enters `&` compose mode,
                    // same as the footer chip.
                    self.activate_cloud_handoff_compose(HandoffEntryPoint::SlashCommand, ctx);
                }
            }
            fork if command.name == commands::FORK.name => {
                let Some(conversation_id) = self
                    .ai_context_model
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
                else {
                    show_error_toast("/fork requires an active conversation".to_owned(), ctx);
                    return true;
                };

                let destination = if trigger.is_cmd_or_ctrl_enter() {
                    ForkedConversationDestination::NewTab
                } else {
                    ForkedConversationDestination::SplitPane
                };

                ctx.dispatch_typed_action(&WorkspaceAction::ForkAIConversation {
                    conversation_id,
                    fork_from_exchange: None,
                    summarize_after_fork: false,
                    summarization_prompt: None,
                    initial_prompt: argument.cloned(),
                    destination,
                });
            }
            fork_from if command.name == commands::FORK_FROM.name => {
                self.open_user_query_menu(UserQueryMenuAction::ForkFrom, ctx);
                return true;
            }
            #[cfg(not(target_family = "wasm"))]
            continue_locally if command.name == commands::CONTINUE_LOCALLY.name => {
                let Some(conversation_id) = self
                    .ai_context_model
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
                else {
                    show_error_toast(
                        "/continue-locally requires an active conversation".to_owned(),
                        ctx,
                    );
                    return true;
                };

                if !conversation_is_cloud_oz_for_slash_command(conversation_id, ctx) {
                    show_error_toast(
                        "/continue-locally is only available for cloud Oz conversations".to_owned(),
                        ctx,
                    );
                    return true;
                }

                let destination = if trigger.is_cmd_or_ctrl_enter() {
                    ForkedConversationDestination::NewTab
                } else {
                    ForkedConversationDestination::SplitPane
                };

                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::SlashCommandContinueLocally,
                    ctx
                );

                ctx.dispatch_typed_action(&WorkspaceAction::ForkAIConversation {
                    conversation_id,
                    fork_from_exchange: None,
                    summarize_after_fork: false,
                    summarization_prompt: None,
                    initial_prompt: argument.cloned(),
                    destination,
                });
            }
            fork_and_compact if command.name == commands::FORK_AND_COMPACT.name => {
                let Some(conversation_id) = self
                    .ai_context_model
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
                else {
                    show_error_toast(
                        "/fork-and-compact requires an active conversation".to_owned(),
                        ctx,
                    );
                    return true;
                };

                let destination = if trigger.is_cmd_or_ctrl_enter() {
                    ForkedConversationDestination::SplitPane
                } else {
                    ForkedConversationDestination::CurrentPane
                };

                ctx.dispatch_typed_action(&WorkspaceAction::ForkAIConversation {
                    conversation_id,
                    fork_from_exchange: None,
                    summarize_after_fork: true,
                    summarization_prompt: None,
                    initial_prompt: argument.cloned(),
                    destination,
                });
            }
            compact_and if command.name == commands::COMPACT_AND.name => {
                if self
                    .ai_context_model
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
                    .is_none()
                {
                    show_error_toast(
                        "/compact-and requires an active conversation".to_owned(),
                        ctx,
                    );
                    return true;
                };

                ctx.dispatch_typed_action(&WorkspaceAction::SummarizeAIConversation {
                    prompt: None,
                    initial_prompt: argument.cloned(),
                });
            }
            queue if command.name == commands::QUEUE.name => {
                let Some(conversation_id) = self
                    .ai_context_model
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
                else {
                    show_error_toast("/queue requires an active conversation".to_owned(), ctx);
                    return true;
                };

                let Some(prompt) = argument.filter(|a| !a.is_empty()).cloned() else {
                    show_error_toast("/queue requires a prompt argument".to_owned(), ctx);
                    return true;
                };

                let history = BlocklistAIHistoryModel::handle(ctx);
                let is_in_progress = history
                    .as_ref(ctx)
                    .conversation(&conversation_id)
                    .is_some_and(|c| c.status().is_in_progress() || c.status().is_blocked());

                if is_in_progress {
                    QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                        model.append(
                            conversation_id,
                            QueuedQuery::new(prompt, QueuedQueryOrigin::QueueSlashCommand),
                            ctx,
                        );
                    });
                } else {
                    self.submit_queued_prompt(prompt, ctx);
                }
            }
            open_repo if command.name == commands::OPEN_REPO.name => {
                if !FeatureFlag::InlineRepoMenu.is_enabled() {
                    return false;
                }
                self.open_repos_menu(ctx);
            }
            command_that_just_sends_ai_request_with_prefix
                if command.name == commands::COMPACT.name
                    || command.name == commands::PLAN.name
                    || command.name == commands::ORCHESTRATE.name =>
            {
                // These slash commands just send AI requests with the slash command text as a
                // prefix, and special handling is done downstream as an implementation detail
                // of handling user queries with specific slash command prefixes.
                return false;
            }
            _ => {
                debug_assert!(
                    false,
                    "Attempted to execute slash command with no handler: {}",
                    command.name
                );
                return false;
            }
        }

        // Leave the buffer alone when re-sending a queued prompt (the user may have typed
        // new input while the agent was busy).
        if !is_queued_prompt {
            self.editor.update(ctx, |editor, ctx| {
                editor.clear_buffer(ctx);
            });
        }

        // If the command must be executed in AI mode, and we're not already in an agent view,
        // enter the agent view.
        if FeatureFlag::AgentView.is_enabled()
            && command.auto_enter_ai_mode
            && !self.agent_view_controller.as_ref(ctx).is_active()
        {
            self.agent_view_controller.update(ctx, |controller, ctx| {
                let _ = controller.try_enter_agent_view(
                    None,
                    AgentViewEntryOrigin::SlashCommand {
                        trigger: SlashCommandTrigger::input(),
                    },
                    ctx,
                );
            });
        }

        let is_in_agent_view = FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(ctx).is_active();
        send_telemetry_from_ctx!(
            TelemetryEvent::SlashCommandAccepted {
                command_details: SlashCommandAcceptedDetails::StaticCommand {
                    command_name: command.name.to_owned(),
                },
                is_in_agent_view,
            },
            ctx
        );
        true
    }

    /// Handles cmd+enter (Mac) / ctrl+enter (Linux/Windows) for slash commands.
    ///
    /// Returns `true` if the keypress was handled.
    pub(super) fn maybe_handle_cmd_or_ctrl_shift_enter_for_slash_command(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        // If slash command menu is open, accept the selected item with cmd_or_ctrl_enter=true.
        if matches!(
            self.suggestions_mode_model.as_ref(ctx).mode(),
            InputSuggestionsMode::SlashCommands
        ) {
            if self.is_cloud_mode_input_v2_composing(ctx) {
                if let Some(view) = self.cloud_mode_v2_slash_commands_view.clone() {
                    view.update(ctx, |view, ctx| {
                        view.accept_selected_item(true, ctx);
                    });
                }
            } else {
                self.inline_slash_commands_view.update(ctx, |view, ctx| {
                    view.accept_selected_item(true, ctx);
                });
            }
            return true;
        }

        // If no menu but slash command detected in buffer, execute with cmd_or_ctrl_enter=true
        match self.slash_command_model.as_ref(ctx).state() {
            SlashCommandEntryState::SlashCommand(detected_command) => {
                let command = detected_command.command.clone();
                let argument = detected_command.argument.clone();
                if !self.is_slash_command_available(&command, ctx) {
                    return false;
                }
                self.execute_slash_command(
                    &command,
                    argument.as_ref(),
                    SlashCommandTrigger::cmd_or_ctrl_enter(),
                    /*is_queued_prompt*/ false,
                    ctx,
                )
            }
            SlashCommandEntryState::SkillCommand(_)
                if self.is_cloud_mode_input_v2_composing(ctx) =>
            {
                false
            }
            SlashCommandEntryState::SkillCommand(detected_skill) => {
                let reference = detected_skill.reference.clone();
                let user_query = detected_skill.argument.clone();
                self.execute_skill_command(
                    reference, user_query, /*is_queued_prompt*/ false, ctx,
                )
            }
            SlashCommandEntryState::None
            | SlashCommandEntryState::Composing { .. }
            | SlashCommandEntryState::DisabledUntilEmptyBuffer => false,
        }
    }

    fn apply_v2_slash_section_filter(
        &mut self,
        section: CloudModeV2Section,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text("/", ctx);
        });
        if let Some(view) = self.cloud_mode_v2_slash_commands_view.clone() {
            view.update(ctx, |v, ctx| {
                v.set_section_filter(Some(section), ctx);
            });
        }
    }

    pub(super) fn maybe_clear_v2_slash_section_filter(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !self.is_cloud_mode_input_v2_composing(ctx) {
            return false;
        }
        let Some(view) = self.cloud_mode_v2_slash_commands_view.clone() else {
            return false;
        };
        let has_filter = view.as_ref(ctx).has_section_filter();
        if !has_filter {
            return false;
        }
        view.update(ctx, |v, ctx| {
            v.set_section_filter(None, ctx);
        });
        true
    }

    /// Executes a slash command on `enter` keypress.
    ///
    /// If the slash command menu is open, then "accepts" the slash command:
    ///   * If the slash command does not take arguments, executes it
    ///   * If the slash command does take arguments, inserts it into the input.
    ///
    /// If the slash command menu is not open, then "executes" the slash command in the input, if
    /// there is one.
    ///
    /// Returns `true` if the enter keypress was 'handled', else upstream enter keypress handling
    /// logic should continue.
    pub(super) fn maybe_handle_enter_for_slash_command(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if matches!(
            self.suggestions_mode_model.as_ref(ctx).mode(),
            InputSuggestionsMode::SlashCommands
        ) {
            if self.is_cloud_mode_input_v2_composing(ctx) {
                if let Some(view) = self.cloud_mode_v2_slash_commands_view.clone() {
                    view.update(ctx, |view, ctx| {
                        view.accept_selected_item(false, ctx);
                    });
                }
            } else {
                self.inline_slash_commands_view.update(ctx, |view, ctx| {
                    view.accept_selected_item(false, ctx);
                });
            }
            return true;
        }

        match self.slash_command_model.as_ref(ctx).state() {
            SlashCommandEntryState::SlashCommand(detected_command) => {
                let command = detected_command.command.clone();
                let argument = detected_command.argument.clone();
                if !self.is_slash_command_available(&command, ctx) {
                    return false;
                }
                self.execute_slash_command(
                    &command,
                    argument.as_ref(),
                    SlashCommandTrigger::input(),
                    /*is_queued_prompt*/ false,
                    ctx,
                )
            }
            SlashCommandEntryState::SkillCommand(_)
                if self.is_cloud_mode_input_v2_composing(ctx) =>
            {
                false
            }
            SlashCommandEntryState::SkillCommand(detected_skill) => {
                let reference = detected_skill.reference.clone();
                let user_query = detected_skill.argument.clone();
                self.execute_skill_command(
                    reference, user_query, /*is_queued_prompt*/ false, ctx,
                )
            }
            SlashCommandEntryState::None
            | SlashCommandEntryState::Composing { .. }
            | SlashCommandEntryState::DisabledUntilEmptyBuffer => false,
        }
    }
}

/// Returns true when the conversation with `conversation_id` is associated with a cloud Oz
/// `AmbientAgentTask`. Used as the defensive runtime gate for `/continue-locally` so a
/// keybinding-triggered execution can't fall through onto a non-cloud-Oz conversation after
/// the menu has been recomputed. Mirrors `SlashCommandDataSource::active_conversation_is_cloud_oz`.
#[cfg(not(target_family = "wasm"))]
fn conversation_is_cloud_oz_for_slash_command(
    conversation_id: AIConversationId,
    ctx: &AppContext,
) -> bool {
    let history = BlocklistAIHistoryModel::as_ref(ctx);
    let Some(conversation) = history.conversation(&conversation_id) else {
        return false;
    };
    let Some(task_id) = conversation.task_id() else {
        return false;
    };

    let Some(task) = AgentConversationsModel::as_ref(ctx).get_task_data(&task_id) else {
        // Permissive: not yet fetched. Matches the data-source default so the command isn't
        // wrongly blocked while the task fetch is in flight.
        return true;
    };

    match task
        .agent_config_snapshot
        .as_ref()
        .and_then(|s| s.harness.as_ref())
    {
        Some(config) => config.harness_type == Harness::Oz,
        None => true,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
