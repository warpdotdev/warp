use std::sync::Arc;

use parking_lot::FairMutex;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{Container, Element, Empty, MouseStateHandle};
use warpui::keymap::Keystroke;
use warpui::platform::OperatingSystem;
use warpui::{AppContext, Entity, ModelHandle, SingletonEntity, View, ViewContext};

use super::{AgentViewState, EphemeralMessageModel, EphemeralMessageModelEvent};
use crate::ai::agent::conversation::AIConversation;
use crate::ai::agent::{
    AIAgentExchangeId, AIAgentOutputStatus, FinishedAIAgentOutput, RenderableAIError,
};
use crate::ai::blocklist::agent_view::shortcuts::AgentShortcutViewModel;
use crate::ai::blocklist::agent_view::zero_state_block::render_ambient_credits_banner;
use crate::ai::blocklist::agent_view::{
    agent_view_bg_fill, is_in_cloud_context, AgentViewController, AgentViewControllerEvent,
};
use crate::ai::blocklist::{
    ai_brand_color, BlocklistAIContextEvent, BlocklistAIContextModel, BlocklistAIHistoryEvent,
    BlocklistAIInputEvent, BlocklistAIInputModel,
};
use crate::ai::document::ai_document_model::{AIDocumentModel, AIDocumentModelEvent};
use crate::ai::mcp::templatable_manager::{FigmaMcpStatus, TemplatableMCPServerManagerEvent};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::ai::request_usage_model::{
    AIRequestUsageModel, AIRequestUsageModelEvent, AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD,
};
use crate::search::slash_command_menu::static_commands::commands;
use crate::settings::AISettings;
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::message_bar::attached_context::{
    AttachedBlocksMessageProducer, AttachedContextArgs, AttachedTextSelectionMessageProducer,
};
use crate::terminal::input::message_bar::common::{
    disableable_message_item_color_overrides, render_standard_message_bar,
};
use crate::terminal::input::message_bar::{
    ChipHorizontalAlignment, EmptyMessageProducer, Message, MessageItem, MessageProvider,
};
use crate::terminal::input::slash_command_model::{SlashCommandEntryState, SlashCommandModel};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};
use crate::terminal::input::{HandoffComposeState, InputAction, SET_INPUT_MODE_AGENT_ACTION_NAME};
use crate::terminal::model::TerminalModel;
use crate::terminal::view::TerminalAction;
use crate::ui_components::blended_colors;
use crate::util::bindings::keybinding_name_to_keystroke;
use crate::workspace::tab_settings::{TabSettings, TabSettingsChangedEvent};
#[cfg(not(target_family = "wasm"))]
use crate::workspace::WorkspaceAction;
use crate::BlocklistAIHistoryModel;

const FIGMA_ICON_SIZE: f32 = 14.;

#[derive(Clone, Default)]
pub struct AgentMessageBarMouseStates {
    pub resume_conversation: MouseStateHandle,
    pub fork_from_last_known_good_state: MouseStateHandle,
    pub toggle_shortcuts: MouseStateHandle,
    pub toggle_slash_commands: MouseStateHandle,
    pub toggle_plan: MouseStateHandle,
    pub toggle_conversation_menu: MouseStateHandle,
    pub toggle_code_review: MouseStateHandle,
    pub handoff_to_cloud: MouseStateHandle,
    pub clear_attached_context: MouseStateHandle,
    /// Mouse state handle for the "Get Figma MCP" contextual button.
    pub figma_install_button: MouseStateHandle,
    /// Mouse state handle for the "Enable Figma MCP" contextual button.
    pub figma_enable_button: MouseStateHandle,
}

/// Renders contextual hint text at the bottom of the agent view status bar.
pub struct AgentMessageBar {
    agent_view_controller: ModelHandle<AgentViewController>,
    ephemeral_message_model: ModelHandle<EphemeralMessageModel>,
    shortcut_view_model: ModelHandle<AgentShortcutViewModel>,
    input_buffer_model: ModelHandle<InputBufferModel>,
    input_model: ModelHandle<BlocklistAIInputModel>,
    input_suggestions_model: ModelHandle<InputSuggestionsModeModel>,
    slash_command_model: ModelHandle<SlashCommandModel>,
    context_model: ModelHandle<BlocklistAIContextModel>,
    handoff_compose_state: ModelHandle<HandoffComposeState>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    mouse_states: AgentMessageBarMouseStates,
    /// Whether the word "figma" has been detected in the current input buffer or attached images.
    /// Only meaningful when `FeatureFlag::FigmaDetection` is enabled.
    figma_detected: bool,
}

impl Entity for AgentMessageBar {
    type Event = ();
}

impl AgentMessageBar {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_view_controller: ModelHandle<AgentViewController>,
        ephemeral_message_model: ModelHandle<EphemeralMessageModel>,
        shortcut_view_model: ModelHandle<AgentShortcutViewModel>,
        input_buffer_model: ModelHandle<InputBufferModel>,
        input_model: ModelHandle<BlocklistAIInputModel>,
        input_suggestions_model: ModelHandle<InputSuggestionsModeModel>,
        slash_command_model: ModelHandle<SlashCommandModel>,
        context_model: ModelHandle<BlocklistAIContextModel>,
        handoff_compose_state: ModelHandle<HandoffComposeState>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&agent_view_controller, |_, _, event, ctx| {
            if matches!(
                event,
                AgentViewControllerEvent::EnteredAgentView { .. }
                    | AgentViewControllerEvent::ExitedAgentView { .. }
            ) {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&ephemeral_message_model, |_, _, event, ctx| {
            if matches!(event, EphemeralMessageModelEvent::MessageChanged) {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&input_model, |_, _, event, ctx| {
            if matches!(
                event,
                BlocklistAIInputEvent::InputTypeChanged { .. }
                    | BlocklistAIInputEvent::LockChanged { .. }
            ) {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&input_buffer_model, |me, _, event, ctx| {
            let InputBufferUpdateEvent {
                old_content: old,
                new_content: new,
                ..
            } = event;
            // If the user inputs into the buffer, dismiss any explicit message if we have one.
            me.ephemeral_message_model
                .update(ctx, |m, ctx| m.try_dismiss_explicit_message(ctx));
            let empty_state_changed = old.is_empty() != new.is_empty();
            let in_shell_mode = !me.input_model.as_ref(ctx).is_ai_input_enabled();
            if empty_state_changed || in_shell_mode {
                ctx.notify();
            }
            me.update_figma_detected(ctx);
        });
        ctx.subscribe_to_model(&shortcut_view_model, |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.subscribe_to_model(&input_suggestions_model, |me, _, event, ctx| match event {
            InputSuggestionsModeEvent::ModeChanged {
                buffer_to_restore: _,
                input_config_to_restore: _,
            } => {
                if me.input_suggestions_model.as_ref(ctx).is_inline_menu_open() {
                    // When opening an inline menu, dismiss any explicit message if we have one.
                    me.ephemeral_message_model
                        .update(ctx, |m, ctx| m.try_dismiss_explicit_message(ctx));
                }
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&slash_command_model, |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.subscribe_to_model(&handoff_compose_state, |_, _, _, ctx| {
            ctx.notify();
        });

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |_, _, event, ctx| {
            if matches!(
                event,
                BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
            ) {
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&AIDocumentModel::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AIDocumentModelEvent::DocumentVisibilityChanged(_)) {
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&context_model, |me, _, event, ctx| {
            if let BlocklistAIContextEvent::UpdatedPendingContext { .. } = event {
                me.update_figma_detected(ctx);
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&TabSettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, TabSettingsChangedEvent::ShowCodeReviewButton { .. }) {
                ctx.notify();
            }
        });

        if FeatureFlag::FigmaDetection.is_enabled() {
            // When the state of the Figma MCP changes, re-render to update the Figma CTA button.
            ctx.subscribe_to_model(
                &TemplatableMCPServerManager::handle(ctx),
                |_, model, event, ctx| {
                    if let TemplatableMCPServerManagerEvent::StateChanged { uuid, .. } = event {
                        if let Some(figma_mcp_uuid) =
                            model.as_ref(ctx).get_figma_installation_uuid()
                        {
                            if uuid == &figma_mcp_uuid {
                                ctx.notify();
                            }
                        }
                    }
                },
            );
        }

        ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AIRequestUsageModelEvent::RequestUsageUpdated) {
                ctx.notify();
            }
        });

        Self {
            agent_view_controller,
            ephemeral_message_model,
            shortcut_view_model,
            input_buffer_model,
            input_model,
            input_suggestions_model,
            slash_command_model,
            context_model,
            handoff_compose_state,
            terminal_model,
            mouse_states: AgentMessageBarMouseStates::default(),
            figma_detected: false,
        }
    }
}

impl AgentMessageBar {
    /// Sets `figma_detected` by checking both the current input text and attached images.
    /// `figma_detected` is `true` when either the text contains "figma" (case-insensitive)
    /// or any attached image was exported from Figma.
    fn update_figma_detected(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::FigmaDetection.is_enabled() {
            return;
        }
        let text_has_figma = self
            .input_buffer_model
            .as_ref(ctx)
            .current_value()
            .to_lowercase()
            .contains("figma");
        let image_has_figma = self
            .context_model
            .as_ref(ctx)
            .pending_images()
            .iter()
            .any(|image| image.is_figma);
        let detected = text_has_figma || image_has_figma;
        if self.figma_detected != detected {
            self.figma_detected = detected;
            ctx.notify();
        }
    }

    /// Returns the Figma MCP status if the contextual button area should be rendered
    /// (i.e. when `FeatureFlag::FigmaDetection` is enabled and "figma" is detected in the input).
    fn figma_button_status(&self, app: &AppContext) -> Option<FigmaMcpStatus> {
        if FeatureFlag::FigmaDetection.is_enabled() && self.figma_detected {
            Some(TemplatableMCPServerManager::as_ref(app).get_figma_mcp_status())
        } else {
            None
        }
    }
}

impl View for AgentMessageBar {
    fn ui_name() -> &'static str {
        "AgentMessageBar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // If an inline menu is open, a 'message line' is rendered by the inline menu itself,
        // so defer to that.
        let input_suggestions_model = self.input_suggestions_model.as_ref(app);
        if input_suggestions_model.is_inline_menu_open() {
            return Empty::new().finish();
        }

        let shortcut_view_model = self.shortcut_view_model.as_ref(app);
        let input_buffer_model = self.input_buffer_model.as_ref(app);
        let input_model = self.input_model.as_ref(app);
        let agent_view_controller = self.agent_view_controller.as_ref(app);
        let context_model = self.context_model.as_ref(app);
        let slash_command_model = self.slash_command_model.as_ref(app);
        let handoff_compose_state = self.handoff_compose_state.as_ref(app);
        let terminal_model = self.terminal_model.lock();

        let appearance = Appearance::as_ref(app);

        let Some(active_conversation) = agent_view_controller
            .agent_view_state()
            .active_conversation_id()
            .and_then(|id| BlocklistAIHistoryModel::as_ref(app).conversation(&id))
        else {
            // This should never be hit, as the agent view requires there be an active conversation.
            return Empty::new().finish();
        };

        let ephemeral_message_model = self.ephemeral_message_model.as_ref(app);

        let args = AgentMessageArgs {
            active_conversation,
            agent_view_controller,
            ephemeral_message_model,
            shortcut_view_model,
            input_buffer_model,
            input_model,
            slash_command_model,
            context_model,
            handoff_compose_state,
            terminal_model: &terminal_model,
            appearance,
            app,
            mouse_states: &self.mouse_states,
        };

        // Ephemeral messages take highest priority.
        let Some(mut message) = ephemeral_message_model
            .produce_message(args)
            .or_else(|| BootstrappingMessageProducer.produce_message(args))
            .or_else(|| ForkSlashCommandMessageProducer.produce_message(args))
            .or_else(|| AttachedBlocksMessageProducer.produce_message(args))
            .or_else(|| AttachedTextSelectionMessageProducer.produce_message(args))
            .or_else(|| AutodetectedBashModeMessageProducer.produce_message(args))
            .or_else(|| ExitCloudHandoffModeMessageProducer.produce_message(args))
            .or_else(|| ExitBashModeMessageProducer.produce_message(args))
            .or_else(|| HideShortcutsMessageProducer.produce_message(args))
            .or_else(|| ZeroStateMessageProducer.produce_message(args))
            .or_else(|| EmptyMessageProducer.produce_message(args))
        else {
            return Empty::new().finish();
        };

        // Show credits banner when user has ambient credits remaining.
        let right_element = if cfg!(target_family = "wasm") {
            None
        } else if let Some(credits) =
            AIRequestUsageModel::as_ref(app).ambient_only_credits_remaining()
        {
            if credits >= AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD {
                Some(render_ambient_credits_banner(credits, app))
            } else {
                None
            }
        } else {
            None
        };

        // Append a Figma MCP chip to the message if applicable.
        match self.figma_button_status(app) {
            Some(FigmaMcpStatus::NotInstalled) => {
                message.items.push(figma_chip(
                    self.mouse_states.figma_install_button.clone(),
                    "Get Figma MCP",
                    Some(InputAction::FigmaAddButtonClicked),
                ));
            }
            Some(FigmaMcpStatus::Installed) => {
                message.items.push(figma_chip(
                    self.mouse_states.figma_enable_button.clone(),
                    "Enable Figma MCP",
                    Some(InputAction::FigmaEnableButtonClicked),
                ));
            }
            Some(FigmaMcpStatus::Enabling) => {
                message.items.push(
                    figma_chip(
                        self.mouse_states.figma_enable_button.clone(),
                        "Enabling...",
                        None,
                    )
                    .with_is_disabled(true),
                );
            }
            Some(FigmaMcpStatus::Running) | None => {}
        }

        let message_bar = render_standard_message_bar(message, right_element, app);
        if self.agent_view_controller.as_ref(app).is_inline() {
            Container::new(message_bar)
                .with_background(agent_view_bg_fill(app))
                .finish()
        } else {
            message_bar
        }
    }
}

/// Arguments for agent message producers.
#[derive(Copy, Clone)]
pub struct AgentMessageArgs<'a> {
    pub active_conversation: &'a AIConversation,
    pub agent_view_controller: &'a AgentViewController,
    pub ephemeral_message_model: &'a EphemeralMessageModel,
    pub shortcut_view_model: &'a AgentShortcutViewModel,
    pub input_buffer_model: &'a InputBufferModel,
    pub input_model: &'a BlocklistAIInputModel,
    pub slash_command_model: &'a SlashCommandModel,
    pub context_model: &'a BlocklistAIContextModel,
    pub handoff_compose_state: &'a HandoffComposeState,
    pub terminal_model: &'a TerminalModel,
    pub appearance: &'a Appearance,
    pub app: &'a AppContext,
    pub mouse_states: &'a AgentMessageBarMouseStates,
}

impl AttachedContextArgs for AgentMessageArgs<'_> {
    fn terminal_model(&self) -> &TerminalModel {
        self.terminal_model
    }

    fn input_buffer_model(&self) -> &InputBufferModel {
        self.input_buffer_model
    }

    fn input_model(&self) -> &BlocklistAIInputModel {
        self.input_model
    }

    fn agent_view_controller(&self) -> &AgentViewController {
        self.agent_view_controller
    }

    fn context_model(&self) -> &BlocklistAIContextModel {
        self.context_model
    }

    fn mouse_states(&self) -> &AgentMessageBarMouseStates {
        self.mouse_states
    }
}

/// Produces a message while the shell is still bootstrapping.
struct BootstrappingMessageProducer;

impl MessageProvider<AgentMessageArgs<'_>> for BootstrappingMessageProducer {
    fn produce_message(&self, args: AgentMessageArgs<'_>) -> Option<Message> {
        if args.terminal_model.block_list().is_bootstrapped()
            || args.terminal_model.is_dummy_cloud_mode_session()
            || args.terminal_model.is_shared_ambient_agent_session()
        {
            None
        } else {
            Some(Message::from_text("Starting shell..."))
        }
    }
}

/// Produces the zero state message
/// When a task is stopped, we also include "Cmd+Shift+R to resume conversation".
/// When a plan exists for the active conversation, we also include "cmd-alt-p to view plan".
struct ZeroStateMessageProducer;

impl MessageProvider<AgentMessageArgs<'_>> for ZeroStateMessageProducer {
    fn produce_message(&self, args: AgentMessageArgs<'_>) -> Option<Message> {
        let AgentMessageArgs {
            active_conversation,
            agent_view_controller,
            input_model,
            input_buffer_model,
            terminal_model,
            app,
            mouse_states,
            ..
        } = args;

        let is_locked_shell_input =
            !input_model.is_ai_input_enabled() && input_model.is_input_type_locked();
        if is_locked_shell_input {
            return None;
        }

        let AgentViewState::Active {
            original_conversation_length,
            ..
        } = agent_view_controller.agent_view_state()
        else {
            return None;
        };

        let mut items = Vec::new();

        let show_resume = !active_conversation.is_entirely_passive()
            && (active_conversation.status().is_cancelled()
                || active_conversation.status().is_error());
        if show_resume {
            let resume_keystroke = if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-shift-R").expect("keystroke should parse")
            } else {
                Keystroke::parse("ctrl-alt-r").expect("keystroke should parse")
            };
            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(resume_keystroke),
                    MessageItem::text("to resume conversation"),
                ],
                |ctx| {
                    ctx.dispatch_typed_action(TerminalAction::ResumeConversation);
                },
                mouse_states.resume_conversation.clone(),
            ));
        }

        // Override to disabled text color if the buffer is not empty, because
        // these shortcuts require the buffer be empty to take effect.
        let is_buffer_empty = input_buffer_model.current_value().is_empty();
        let (
            color_override_for_shortcuts_and_commands,
            bg_color_override_for_shortcuts_and_commands,
        ) = disableable_message_item_color_overrides(!is_buffer_empty, app);

        items.push(
            MessageItem::clickable(
                vec![
                    MessageItem::Keystroke {
                        keystroke: Keystroke {
                            key: "?".to_owned(),
                            ..Default::default()
                        },
                        color: color_override_for_shortcuts_and_commands,
                        background_color: bg_color_override_for_shortcuts_and_commands,
                    },
                    MessageItem::Text {
                        content: "for help".into(),
                        color: color_override_for_shortcuts_and_commands,
                    },
                ],
                |ctx| {
                    ctx.dispatch_typed_action(InputAction::ToggleAgentViewShortcuts);
                },
                mouse_states.toggle_shortcuts.clone(),
            )
            .with_is_disabled(!is_buffer_empty),
        );

        items.push(
            MessageItem::clickable(
                vec![
                    MessageItem::Keystroke {
                        keystroke: Keystroke {
                            key: "/".to_owned(),
                            ..Default::default()
                        },
                        color: color_override_for_shortcuts_and_commands,
                        background_color: bg_color_override_for_shortcuts_and_commands,
                    },
                    MessageItem::Text {
                        content: "for commands".into(),
                        color: color_override_for_shortcuts_and_commands,
                    },
                ],
                |ctx| {
                    ctx.dispatch_typed_action(InputAction::ToggleSlashCommandsMenu);
                },
                mouse_states.toggle_slash_commands.clone(),
            )
            .with_is_disabled(!is_buffer_empty),
        );

        let is_cloud_agent =
            is_in_cloud_context(agent_view_controller.agent_view_state(), terminal_model);
        let ai_settings = AISettings::as_ref(app);

        // Handoff to cloud only available for local agents.
        if !is_cloud_agent && ai_settings.is_ampersand_handoff_enabled(app) {
            items.push(
                MessageItem::clickable(
                    vec![
                        MessageItem::Keystroke {
                            keystroke: Keystroke {
                                key: "&".to_owned(),
                                ..Default::default()
                            },
                            color: color_override_for_shortcuts_and_commands,
                            background_color: bg_color_override_for_shortcuts_and_commands,
                        },
                        MessageItem::Text {
                            content: "send task to the cloud".into(),
                            color: color_override_for_shortcuts_and_commands,
                        },
                    ],
                    |ctx| {
                        ctx.dispatch_typed_action(InputAction::ActivateCloudHandoff);
                    },
                    mouse_states.handoff_to_cloud.clone(),
                )
                .with_is_disabled(!is_buffer_empty),
            );
        }

        let plan_count = AIDocumentModel::as_ref(app)
            .get_all_documents_for_conversation(active_conversation.id())
            .len();
        let has_plan = plan_count > 0;
        let has_conversation_been_updated_since_agent_view_entry =
            *original_conversation_length != active_conversation.exchange_count();

        if !is_cloud_agent && !has_conversation_been_updated_since_agent_view_entry {
            if let Some(conversations_keystroke) =
                keybinding_name_to_keystroke(commands::CONVERSATIONS.name, app)
            {
                items.push(MessageItem::clickable(
                    vec![
                        MessageItem::keystroke(conversations_keystroke),
                        MessageItem::text("open conversation"),
                    ],
                    |ctx| {
                        ctx.dispatch_typed_action(InputAction::ToggleConversationsMenu);
                    },
                    mouse_states.toggle_conversation_menu.clone(),
                ));
            }
        }

        // Code review only works locally.
        #[cfg(not(target_family = "wasm"))]
        if !is_cloud_agent
            && !ai_settings.is_cloud_handoff_enabled(app)
            && *TabSettings::as_ref(app).show_code_review_button
        {
            let code_review_keystroke = if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-shift-+").expect("keystroke should parse")
            } else {
                Keystroke::parse("ctrl-shift-+").expect("keystroke should parse")
            };
            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(code_review_keystroke),
                    MessageItem::text("for code review"),
                ],
                |ctx| {
                    ctx.dispatch_typed_action(WorkspaceAction::ToggleRightPanel);
                },
                mouse_states.toggle_code_review.clone(),
            ));
        }

        if has_plan {
            let is_plan_for_this_conversation_open = agent_view_controller
                .pane_group_id()
                .is_some_and(|pane_group_id| {
                    AIDocumentModel::as_ref(app).is_document_visible_by_conversation_in_pane_group(
                        &active_conversation.id(),
                        pane_group_id,
                    )
                });

            // If changing this text, ensure the logic is consistent with how TerminalAction::ToggleAIDocumentPane is handled.
            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(
                        Keystroke::parse("cmdorctrl-alt-p").expect("keystroke should parse"),
                    ),
                    MessageItem::text(if is_plan_for_this_conversation_open {
                        "to hide plan"
                    } else if plan_count > 1 {
                        "to view plans"
                    } else {
                        "to view plan"
                    }),
                ],
                |ctx| {
                    ctx.dispatch_typed_action(TerminalAction::ToggleAIDocumentPane);
                },
                mouse_states.toggle_plan.clone(),
            ));
        }
        if fork_from_last_known_good_state_exchange_id(active_conversation, terminal_model)
            .is_some()
        {
            let fork_keystroke = if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-alt-y").expect("keystroke should parse")
            } else {
                Keystroke::parse("ctrl-alt-y").expect("keystroke should parse")
            };
            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(fork_keystroke),
                    MessageItem::text("to fork and continue"),
                ],
                |ctx| {
                    ctx.dispatch_typed_action(
                        TerminalAction::ForkConversationFromLastKnownGoodState,
                    );
                },
                mouse_states.fork_from_last_known_good_state.clone(),
            ));
        }

        Some(Message::new(items))
    }
}

pub(crate) fn fork_from_last_known_good_state_exchange_id(
    active_conversation: &AIConversation,
    terminal_model: &TerminalModel,
) -> Option<AIAgentExchangeId> {
    if !should_fork_from_last_known_good_state(active_conversation, terminal_model) {
        return None;
    }

    active_conversation
        .exchanges_reversed()
        .filter(|exchange| exchange.has_user_query())
        // Assumes the failed latest exchange is in the root task; exchanges_reversed only
        // iterates root-task exchanges.
        // Skip the failed latest user query; fork from the nearest prior successful one.
        .skip(1)
        .find(|exchange| exchange.output_status.is_finished_and_successful())
        .map(|exchange| exchange.id)
}
fn should_fork_from_last_known_good_state(
    active_conversation: &AIConversation,
    terminal_model: &TerminalModel,
) -> bool {
    if terminal_model.is_conversation_transcript_viewer()
        || terminal_model.shared_session_status().is_viewer()
        || active_conversation.is_viewing_shared_session()
    {
        return false;
    }

    let Some(latest_exchange) = active_conversation.latest_exchange() else {
        return false;
    };

    let error = match &latest_exchange.output_status {
        AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Error { error, .. },
        } => error,
        _ => return false,
    };

    match error {
        RenderableAIError::QuotaLimit { .. }
        | RenderableAIError::ServerOverloaded
        | RenderableAIError::ContextWindowExceeded(_)
        | RenderableAIError::InvalidApiKey { .. }
        | RenderableAIError::AwsBedrockCredentialsExpiredOrInvalid { .. } => false,
        RenderableAIError::InternalWarpError => true,
        RenderableAIError::Other {
            will_attempt_resume,
            ..
        } => !will_attempt_resume,
    }
}

struct ForkSlashCommandMessageProducer;

impl MessageProvider<AgentMessageArgs<'_>> for ForkSlashCommandMessageProducer {
    fn produce_message(&self, args: AgentMessageArgs<'_>) -> Option<Message> {
        let SlashCommandEntryState::SlashCommand(detected_command) =
            args.slash_command_model.state()
        else {
            return None;
        };
        let command_name = detected_command.command.name;
        let is_fork_family = command_name == commands::FORK.name
            || command_name == commands::FORK_FROM.name
            || command_name == commands::FORK_AND_COMPACT.name;
        #[cfg(not(target_family = "wasm"))]
        let is_continue_locally = command_name == commands::CONTINUE_LOCALLY.name;
        #[cfg(target_family = "wasm")]
        let is_continue_locally = false;
        if !is_fork_family && !is_continue_locally {
            return None;
        }
        let modifier_keystroke = if cfg!(target_os = "macos") {
            Keystroke {
                key: "enter".to_owned(),
                cmd: true,
                ..Default::default()
            }
        } else {
            Keystroke {
                key: "enter".to_owned(),
                ctrl: true,
                shift: true,
                ..Default::default()
            }
        };

        // `/fork` and `/continue-locally` open in a new pane with Enter and a new tab with
        // Cmd/Ctrl+Enter. Other fork-like commands open in the current pane with Enter and a new
        // pane with Cmd/Ctrl+Enter.
        let primary_to_new_pane = command_name == commands::FORK.name || is_continue_locally;
        let (primary_label, secondary_label) = if primary_to_new_pane {
            (" new pane", " new tab")
        } else {
            (" current pane", " new pane")
        };

        Some(Message::new(vec![
            MessageItem::keystroke(Keystroke {
                key: "enter".to_owned(),
                ..Default::default()
            }),
            MessageItem::text(primary_label),
            MessageItem::keystroke(modifier_keystroke),
            MessageItem::text(secondary_label),
        ]))
    }
}

struct HideShortcutsMessageProducer;

impl MessageProvider<AgentMessageArgs<'_>> for HideShortcutsMessageProducer {
    fn produce_message(&self, args: AgentMessageArgs<'_>) -> Option<Message> {
        if !args.shortcut_view_model.is_shortcut_view_open() {
            return None;
        }

        Some(Message::new(vec![MessageItem::clickable(
            vec![
                MessageItem::keystroke(Keystroke {
                    key: "?".to_owned(),
                    ..Default::default()
                }),
                MessageItem::text("to hide help"),
            ],
            |ctx| {
                ctx.dispatch_typed_action(InputAction::ToggleAgentViewShortcuts);
            },
            args.mouse_states.toggle_shortcuts.clone(),
        )]))
    }
}

struct AutodetectedBashModeMessageProducer;

impl MessageProvider<AgentMessageArgs<'_>> for AutodetectedBashModeMessageProducer {
    fn produce_message(&self, args: AgentMessageArgs<'_>) -> Option<Message> {
        let AgentMessageArgs {
            input_buffer_model,
            input_model,
            appearance,
            app,
            slash_command_model,
            ..
        } = args;
        if input_model.is_ai_input_enabled()
            || input_model.is_input_type_locked()
            || input_buffer_model.current_value().is_empty()
            || slash_command_model.state().is_detected_command()
        {
            return None;
        }

        let message = match keybinding_name_to_keystroke(SET_INPUT_MODE_AGENT_ACTION_NAME, app) {
            Some(keystroke) => Message::new(vec![
                MessageItem::text("autodetected shell command, "),
                MessageItem::keystroke(keystroke),
                MessageItem::text(" to override"),
            ])
            .with_text_color(appearance.theme().ansi_fg_blue()),
            None => Message::from_text("autodetected shell command"),
        };

        Some(message)
    }
}

struct ExitCloudHandoffModeMessageProducer;

impl MessageProvider<AgentMessageArgs<'_>> for ExitCloudHandoffModeMessageProducer {
    fn produce_message(&self, args: AgentMessageArgs<'_>) -> Option<Message> {
        let AgentMessageArgs {
            input_buffer_model,
            handoff_compose_state,
            appearance,
            ..
        } = args;
        if !handoff_compose_state.is_active() {
            return None;
        }

        let active_color = ai_brand_color(appearance.theme());
        let is_buffer_empty = input_buffer_model.current_value().is_empty();
        let (dismiss_text_color, dismiss_key_color, dismiss_key_bg) = if is_buffer_empty {
            (active_color, None, None)
        } else {
            (
                Fill::from(active_color).with_opacity(60).into_solid(),
                Some(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().background())
                        .into_solid(),
                ),
                Some(blended_colors::neutral_1(appearance.theme())),
            )
        };

        Some(Message::new(vec![
            MessageItem::Keystroke {
                keystroke: Keystroke {
                    key: "enter".to_owned(),
                    ..Default::default()
                },
                color: None,
                background_color: None,
            },
            MessageItem::Text {
                content: "to hand off to cloud".into(),
                color: Some(active_color),
            },
            MessageItem::Keystroke {
                keystroke: Keystroke {
                    key: "backspace".to_owned(),
                    ..Default::default()
                },
                color: dismiss_key_color,
                background_color: dismiss_key_bg,
            },
            MessageItem::Text {
                content: "to dismiss".into(),
                color: Some(dismiss_text_color),
            },
        ]))
    }
}

struct ExitBashModeMessageProducer;

impl MessageProvider<AgentMessageArgs<'_>> for ExitBashModeMessageProducer {
    fn produce_message(&self, args: AgentMessageArgs<'_>) -> Option<Message> {
        let AgentMessageArgs {
            input_model,
            appearance,
            app,
            ..
        } = args;
        if input_model.is_ai_input_enabled() || !input_model.is_input_type_locked() {
            return None;
        }
        let set_input_mode_agent_keystroke =
            keybinding_name_to_keystroke(SET_INPUT_MODE_AGENT_ACTION_NAME, app)?;

        let text_color = appearance.theme().ansi_fg_blue();

        Some(
            Message::new(vec![
                MessageItem::Keystroke {
                    keystroke: set_input_mode_agent_keystroke,
                    color: None,
                    background_color: None,
                },
                MessageItem::text("to exit shell mode"),
            ])
            .with_text_color(text_color),
        )
    }
}

/// Creates a `MessageItem::Chip` for a Figma MCP contextual action.
/// When `action` is `Some`, the chip is interactive and dispatches that action on click.
/// When `action` is `None`, the chip is returned without an action (caller should disable it).
fn figma_chip(
    mouse_state: MouseStateHandle,
    label: &'static str,
    action: Option<InputAction>,
) -> MessageItem {
    let items = vec![
        MessageItem::Image {
            source: AssetSource::Bundled {
                path: "bundled/svg/figma-colored.svg",
            },
            width: FIGMA_ICON_SIZE,
            height: FIGMA_ICON_SIZE,
        },
        MessageItem::text(label),
    ];
    if let Some(action) = action {
        MessageItem::chip(
            items,
            move |ctx| ctx.dispatch_typed_action(action.clone()),
            mouse_state,
        )
        .with_horizontal_alignment(ChipHorizontalAlignment::Right)
    } else {
        MessageItem::Chip {
            items,
            action: Arc::new(|_| {}),
            mouse_state,
            disabled: true,
            horizontal_alignment: ChipHorizontalAlignment::Right,
        }
    }
}
