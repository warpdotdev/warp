use std::sync::Arc;

use ai::agent::action::{AIAgentActionType, ShellCommandDelay};
use parking_lot::FairMutex;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{CornerRadius, Radius};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, View, ViewContext,
};

use crate::ai::agent::icons;
use crate::ai::blocklist::block::cli_controller::LongRunningCommandControlState;
use crate::ai::blocklist::inline_action::inline_action_header::HeaderConfig;
use crate::ai::blocklist::{
    BlocklistAIActionModel, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
};
use crate::terminal::model::session::Sessions;
use crate::terminal::TerminalModel;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

/// A header rendered as rich content above the active block when Agent View is in inline mode.
pub struct InlineAgentViewHeader {
    terminal_view_id: EntityId,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    sessions_model: ModelHandle<Sessions>,
    action_model: ModelHandle<BlocklistAIActionModel>,
}

impl InlineAgentViewHeader {
    pub fn new(
        terminal_view_id: EntityId,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        sessions_model: ModelHandle<Sessions>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, move |me, _, event, ctx| {
            if event
                .terminal_view_id()
                .is_some_and(|id| id != me.terminal_view_id)
            {
                return;
            }
            match event {
                BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
                | BlocklistAIHistoryEvent::AppendedExchange { .. }
                | BlocklistAIHistoryEvent::StartedNewConversation { .. }
                | BlocklistAIHistoryEvent::SetActiveConversation { .. } => {
                    ctx.notify();
                }
                _ => (),
            }
        });

        ctx.subscribe_to_model(&action_model, |_, _, _, ctx| {
            ctx.notify();
        });

        Self {
            terminal_view_id,
            terminal_model,
            sessions_model,
            action_model,
        }
    }
}

impl Entity for InlineAgentViewHeader {
    type Event = ();
}

impl View for InlineAgentViewHeader {
    fn ui_name() -> &'static str {
        "InlineAgentViewHeader"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let active_conversation = history_model.active_conversation(self.terminal_view_id);
        let conversation_status = active_conversation.map(|conv| conv.status().clone());
        // Use active conversation's latest_exchange to include subtask exchanges (e.g., CLI subagent)
        let is_streaming = active_conversation
            .and_then(|conv| conv.latest_exchange())
            .map(|exchange| exchange.output_status.is_streaming())
            .unwrap_or(false);

        let (
            is_agent_tagged_in,
            is_agent_in_control,
            is_user_in_control,
            is_action_blocked,
            top_level_command,
        ) = {
            let terminal_model = self.terminal_model.lock();
            let active_block = terminal_model.block_list().active_block();
            let sessions = self.sessions_model.as_ref(app);
            (
                active_block.is_agent_tagged_in(),
                active_block
                    .long_running_control_state()
                    .is_some_and(LongRunningCommandControlState::is_agent_in_control),
                active_block
                    .long_running_control_state()
                    .is_some_and(LongRunningCommandControlState::is_user_in_control),
                active_block.is_agent_blocked(),
                active_block.top_level_command(sessions),
            )
        };
        if is_agent_tagged_in {
            let header_background = appearance.theme().surface_2();
            let icon = Icon::Oz.to_warpui_icon(
                blended_colors::text_main(appearance.theme(), header_background).into(),
            );
            let message = if let Some(command) = top_level_command.as_deref() {
                i18n::t("ai.inline_agent.prompt_interact_with_command")
                    .replace("{command}", command)
            } else {
                i18n::t("ai.inline_agent.prompt_interact_with_running_command")
            };
            return HeaderConfig::new(message, app)
                .with_icon(icon)
                .with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(8.)))
                .with_markdown()
                .render(app);
        }

        let action_model = self.action_model.as_ref(app);
        let action = action_model.get_async_running_action(app);
        let is_waiting_for_command_to_exit = action.as_ref().is_some_and(|action| {
            matches!(
                action.action,
                AIAgentActionType::ReadShellCommandOutput {
                    delay: Some(ShellCommandDelay::OnCompletion),
                    ..
                }
            )
        });
        let is_waiting_on_instructions =
            action.is_none() && !is_streaming && is_agent_in_control && !is_action_blocked;
        let message = if is_user_in_control {
            i18n::t("ai.inline_agent.user_in_control")
        } else if is_action_blocked {
            i18n::t("ai.inline_agent.agent_blocked")
        } else if is_waiting_for_command_to_exit {
            i18n::t("ai.inline_agent.waiting_for_command_exit")
        } else if is_waiting_on_instructions {
            i18n::t("ai.inline_agent.waiting_on_instructions")
        } else {
            i18n::t("ai.inline_agent.agent_in_control")
        };

        let icon = if is_user_in_control || is_waiting_on_instructions {
            icons::gray_stop_icon(appearance)
        } else if let Some(status) = &conversation_status {
            status.render_icon(appearance)
        } else {
            icons::in_progress_icon(appearance)
        };

        HeaderConfig::new(message, app)
            .with_icon(icon)
            .with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(8.)))
            .render(app)
    }
}
