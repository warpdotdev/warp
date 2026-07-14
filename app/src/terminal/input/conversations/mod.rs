//! Inline conversation menu for selecting AI conversations, enabled
//! when `FeatureFlag::AgentView` is enabled.
mod data_source;
mod search_item;
mod view;

use pathfinder_color::ColorU;
pub use view::{InlineConversationMenuEvent, InlineConversationMenuView};
use warp_core::ui::appearance::Appearance;
use warpui::keymap::Keystroke;
use warpui::SingletonEntity;

use crate::ai::agent_conversations_model::AgentConversationEntryId;
use crate::terminal::input::inline_menu::{
    default_navigation_message_items, InlineMenuAction, InlineMenuMessageArgs, InlineMenuRowAction,
    InlineMenuType,
};
use crate::terminal::input::message_bar::{Message, MessageItem};

/// Tab identifiers for the inline conversation menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineConversationMenuTab {
    /// Show all conversations.
    All,
    /// Show only conversations whose most recent directory matches the session's CWD.
    CurrentDirectory,
}

/// Action emitted when enter is hit on a conversation the inline conversation menu.
#[derive(Clone, Debug)]
pub struct AcceptConversation {
    pub item_id: AgentConversationEntryId,
    pub is_open_elsewhere: bool,
}

impl InlineMenuAction for AcceptConversation {
    const MENU_TYPE: InlineMenuType = InlineMenuType::ConversationMenu;

    fn produce_inline_menu_message<T>(args: InlineMenuMessageArgs<'_, Self, T>) -> Option<Message> {
        let InlineMenuMessageArgs {
            inline_menu_model,
            app,
        } = args;

        let mut items = Vec::new();

        if let Some(item) = inline_menu_model.selected_item() {
            let text = if item.is_open_elsewhere {
                " go to conversation"
            } else {
                " continue in this pane"
            };

            let item_id = item.item_id;
            let is_open_elsewhere = item.is_open_elsewhere;
            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(Keystroke {
                        key: "enter".to_owned(),
                        ..Default::default()
                    }),
                    MessageItem::text(text),
                ],
                move |ctx| {
                    ctx.dispatch_typed_action(InlineMenuRowAction::Accept {
                        item: AcceptConversation {
                            item_id,
                            is_open_elsewhere,
                        },
                        cmd_or_ctrl_enter: false,
                    });
                },
                inline_menu_model.mouse_states().accept.clone(),
            ));
        } else {
            let theme = Appearance::as_ref(app).theme();
            let disabled_color = theme.disabled_text_color(theme.background()).into_solid();
            items.extend([
                MessageItem::Keystroke {
                    keystroke: Keystroke {
                        key: "enter".to_owned(),
                        ..Default::default()
                    },
                    color: Some(disabled_color),
                    background_color: Some(ColorU::transparent_black()),
                },
                MessageItem::Text {
                    content: " continue in this pane".into(),
                    color: Some(disabled_color),
                },
            ]);
        }

        items.extend(default_navigation_message_items(&args));
        Some(Message::new(items))
    }
}
