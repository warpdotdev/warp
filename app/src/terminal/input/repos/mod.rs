//! Inline repo switcher menu showing indexed repos with git status.
mod data_source;
#[cfg(feature = "local_fs")]
mod search_item;
mod view;

use std::path::PathBuf;

pub use view::{InlineReposMenuEvent, InlineReposMenuView};
use warpui::keymap::Keystroke;

use crate::terminal::input::inline_menu::{
    default_navigation_message_items, InlineMenuAction, InlineMenuMessageArgs, InlineMenuRowAction,
    InlineMenuType,
};
use crate::terminal::input::message_bar::common::disableable_message_item_color_overrides;
use crate::terminal::input::message_bar::{Message, MessageItem};

/// Action emitted when a repo is accepted in the inline repos menu.
#[derive(Clone, Debug)]
pub struct AcceptRepo {
    pub path: PathBuf,
}

impl InlineMenuAction for AcceptRepo {
    const MENU_TYPE: InlineMenuType = InlineMenuType::IndexedReposMenu;

    fn produce_inline_menu_message<T>(args: InlineMenuMessageArgs<'_, Self, T>) -> Option<Message> {
        let mut items = Vec::new();

        let path = args
            .inline_menu_model
            .selected_item()
            .map(|item| item.path.clone());
        let has_path = path.is_some();

        let (
            color_override_for_shortcuts_and_commands,
            bg_color_override_for_shortcuts_and_commands,
        ) = disableable_message_item_color_overrides(!has_path, args.app);

        items.push(
            MessageItem::clickable(
                vec![
                    MessageItem::Keystroke {
                        keystroke: Keystroke {
                            key: "enter".to_owned(),
                            ..Default::default()
                        },
                        color: color_override_for_shortcuts_and_commands,
                        background_color: bg_color_override_for_shortcuts_and_commands,
                    },
                    MessageItem::Text {
                        content: " cd to repo".into(),
                        color: color_override_for_shortcuts_and_commands,
                    },
                ],
                move |ctx| {
                    if let Some(path) = path.clone() {
                        ctx.dispatch_typed_action(InlineMenuRowAction::Accept {
                            item: AcceptRepo { path },
                            cmd_or_ctrl_enter: false,
                        });
                    }
                },
                args.inline_menu_model.mouse_states().accept.clone(),
            )
            .with_is_disabled(!has_path),
        );

        items.extend(default_navigation_message_items(&args));
        Some(Message::new(items))
    }
}
