//! Inline menu for selecting rewind points in a conversation.
//! Used by the `/rewind` slash command to let users select which point to rewind to.

mod data_source;
mod search_item;
mod view;

pub use data_source::SelectRewindPoint;
pub use view::{RewindMenuEvent, RewindMenuView};
use warpui::keymap::Keystroke;

use crate::terminal::input::inline_menu::{
    default_navigation_message_items, InlineMenuAction, InlineMenuMessageArgs, InlineMenuType,
};
use crate::terminal::input::message_bar::{Message, MessageItem};

impl InlineMenuAction for SelectRewindPoint {
    const MENU_TYPE: InlineMenuType = InlineMenuType::RewindMenu;

    fn produce_inline_menu_message<T>(args: InlineMenuMessageArgs<'_, Self, T>) -> Option<Message> {
        let mut items = vec![
            MessageItem::keystroke(Keystroke {
                key: "enter".to_owned(),
                ..Default::default()
            }),
            MessageItem::text("rewind"),
        ];

        items.extend(default_navigation_message_items(&args));
        Some(Message::new(items))
    }
}
