//! Shared message producers for displaying attached blocks/text context.

use warp_core::features::FeatureFlag;
use warpui::keymap::Keystroke;

use crate::ai::blocklist::agent_view::{AgentMessageBarMouseStates, AgentViewController};
use crate::ai::blocklist::{BlocklistAIContextModel, BlocklistAIInputModel};
use crate::terminal::input::InputAction;
use crate::terminal::input::buffer_model::InputBufferModel;
use crate::terminal::input::message_bar::{
    Message, MessageItem, MessageProvider, truncated_command_for_block,
};
use crate::terminal::model::TerminalModel;

/// Trait for message args that can provide attached context information.
/// Exposes the required dependencies for attached context message producers.
pub trait AttachedContextArgs {
    fn terminal_model(&self) -> &TerminalModel;
    fn input_buffer_model(&self) -> &InputBufferModel;
    fn input_model(&self) -> &BlocklistAIInputModel;
    fn agent_view_controller(&self) -> &AgentViewController;
    fn context_model(&self) -> &BlocklistAIContextModel;
    fn mouse_states(&self) -> &AgentMessageBarMouseStates;
}

/// Produces a message when blocks or selected text are attached as context.
pub struct AttachedBlocksMessageProducer;

impl<Args: AttachedContextArgs + Copy> MessageProvider<Args> for AttachedBlocksMessageProducer {
    fn produce_message(&self, args: Args) -> Option<Message> {
        // When AgentViewBlockContext is enabled, user-executed blocks are auto-attached
        // as context, so we don't need to show this message.
        if FeatureFlag::AgentViewBlockContext.is_enabled() {
            return None;
        }

        // In the agent view, only show the attached context message if in AI mode.
        if args.agent_view_controller().is_active()
            && !args.input_buffer_model().current_value().is_empty()
            && !args.input_model().is_ai_input_enabled()
        {
            return None;
        }

        let context_block_ids = args.context_model().pending_context_block_ids();
        if context_block_ids.is_empty() {
            return None;
        }

        let block_command = context_block_ids
            .iter()
            .find_map(|id| {
                args.terminal_model()
                    .block_list()
                    .block_with_id(id)
                    .map(|block| block.command_to_string())
            })
            .map(|cmd| truncated_command_for_block(&cmd))?;

        let message_text = if context_block_ids.len() == 1 {
            format!("`{}` attached as context", block_command)
        } else if context_block_ids.len() == 2 {
            format!(
                "`{}` and 1 other command attached as context",
                block_command
            )
        } else {
            format!(
                "`{}` and {} other commands attached as context",
                block_command,
                context_block_ids.len().saturating_sub(1)
            )
        };

        let mut items = vec![MessageItem::text(message_text)];

        // Always show ESC hint in agent view, make it clickable
        if args.agent_view_controller().is_active() {
            items.push(MessageItem::text(", "));
            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(Keystroke {
                        key: "escape".to_owned(),
                        ..Default::default()
                    }),
                    MessageItem::text(" to remove"),
                ],
                |ctx| {
                    ctx.dispatch_typed_action(InputAction::ClearAttachedContext);
                },
                args.mouse_states().clear_attached_context.clone(),
            ));
        }

        Some(Message::new(items))
    }
}

/// Produces a message when text selection is attached as context.
pub struct AttachedTextSelectionMessageProducer;

impl<Args: AttachedContextArgs + Copy> MessageProvider<Args>
    for AttachedTextSelectionMessageProducer
{
    fn produce_message(&self, args: Args) -> Option<Message> {
        // Only apply the visibility condition when agent view is active.
        // When inactive, always show the message.
        if args.agent_view_controller().is_active()
            && !args.input_buffer_model().current_value().is_empty()
            && !args.input_model().is_ai_input_enabled()
        {
            return None;
        }

        // Only show if there's selected text and no blocks attached
        // (blocks take precedence per requirements)
        if !args.context_model().pending_context_block_ids().is_empty() {
            return None;
        }

        let _ = args.context_model().pending_context_selected_text()?;

        let mut items = vec![MessageItem::text("selected text attached as context")];

        // Always show ESC hint in agent view, make it clickable
        if args.agent_view_controller().is_active() {
            items.push(MessageItem::text(", "));
            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(Keystroke {
                        key: "escape".to_owned(),
                        ..Default::default()
                    }),
                    MessageItem::text(" to remove"),
                ],
                |ctx| {
                    ctx.dispatch_typed_action(InputAction::ClearAttachedContext);
                },
                args.mouse_states().clear_attached_context.clone(),
            ));
        }

        Some(Message::new(items))
    }
}
