//! Inline plan menu for selecting among multiple AI document plans.
mod data_source;
mod search_item;
mod view;

use ai::document::AIDocumentId;
pub use view::{InlinePlanMenuEvent, InlinePlanMenuView};
use warpui::keymap::Keystroke;

use crate::ai::document::ai_document_model::AIDocumentVersion;
use crate::terminal::input::inline_menu::{
    InlineMenuAction, InlineMenuMessageArgs, InlineMenuRowAction, InlineMenuType,
    default_navigation_message_items,
};
use crate::terminal::input::message_bar::{Message, MessageItem};

/// Action emitted when a plan is selected from the inline plan menu.
#[derive(Clone, Debug)]
pub struct AcceptPlan {
    pub document_id: AIDocumentId,
    pub document_version: AIDocumentVersion,
}

impl InlineMenuAction for AcceptPlan {
    const MENU_TYPE: InlineMenuType = InlineMenuType::PlanMenu;

    fn produce_inline_menu_message<T>(args: InlineMenuMessageArgs<'_, Self, T>) -> Option<Message> {
        let InlineMenuMessageArgs {
            inline_menu_model, ..
        } = args;

        let mut items = Vec::new();

        if inline_menu_model.selected_item().is_some() {
            items.push(MessageItem::clickable(
                vec![
                    MessageItem::keystroke(Keystroke {
                        key: "enter".to_owned(),
                        ..Default::default()
                    }),
                    MessageItem::text(" open plan"),
                ],
                move |ctx| {
                    ctx.dispatch_typed_action(InlineMenuRowAction::<AcceptPlan>::AcceptSelected {
                        cmd_or_ctrl_enter: false,
                    });
                },
                inline_menu_model.mouse_states().accept.clone(),
            ));
        }

        items.extend(default_navigation_message_items(&args));
        Some(Message::new(items))
    }
}
