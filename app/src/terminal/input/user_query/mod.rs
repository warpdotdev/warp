//! Inline menu for selecting user queries from a conversation.
//! Used by the `/fork-from` slash command to let users select which query to fork from.

mod data_source;
mod search_item;
mod view;

pub use data_source::SelectUserQuery;
pub use view::{UserQueryMenuEvent, UserQueryMenuView};
use warpui::keymap::Keystroke;
use warpui::platform::OperatingSystem;

use crate::terminal::input::inline_menu::{
    default_navigation_message_items, InlineMenuAction, InlineMenuMessageArgs, InlineMenuRowAction,
    InlineMenuType,
};
use crate::terminal::input::message_bar::{Message, MessageItem};

impl InlineMenuAction for SelectUserQuery {
    const MENU_TYPE: InlineMenuType = InlineMenuType::UserQueryMenu;

    fn produce_inline_menu_message<T>(args: InlineMenuMessageArgs<'_, Self, T>) -> Option<Message> {
        let InlineMenuMessageArgs {
            inline_menu_model, ..
        } = args;

        let mut items = Vec::new();

        if let Some(item) = inline_menu_model.selected_item() {
            let exchange_id = item.exchange_id;
            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(Keystroke {
                        key: "enter".to_owned(),
                        ..Default::default()
                    }),
                    MessageItem::text(" current pane"),
                ],
                move |ctx| {
                    ctx.dispatch_typed_action(InlineMenuRowAction::Accept {
                        item: SelectUserQuery { exchange_id },
                        cmd_or_ctrl_enter: false,
                    });
                },
                inline_menu_model.mouse_states().accept.clone(),
            ));

            let modifier_keystroke = if OperatingSystem::get().is_mac() {
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

            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(modifier_keystroke),
                    MessageItem::text(" new pane"),
                ],
                move |ctx| {
                    ctx.dispatch_typed_action(InlineMenuRowAction::Accept {
                        item: SelectUserQuery { exchange_id },
                        cmd_or_ctrl_enter: true,
                    });
                },
                inline_menu_model.mouse_states().accept_secondary.clone(),
            ));
        }

        items.extend(default_navigation_message_items(&args));
        Some(Message::new(items))
    }
}
