use warpui::elements::{Container, Element};
use warpui::keymap::Keystroke;
use warpui::{AppContext, Entity, ModelHandle, View, ViewContext};

use super::message_bar::common::render_terminal_message;
use super::message_bar::{Message, MessageItem, MessageProvider};
use crate::terminal::input::inline_history::{AcceptHistoryItem, HistoryTab};
use crate::terminal::input::inline_menu::{InlineMenuModel, InlineMenuModelEvent};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};

/// Renders contextual hint text at the bottom of the terminal input when `FeatureFlag::AgentView`
/// is enabled.
pub struct TerminalInputMessageBar {
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    inline_history_model: ModelHandle<InlineMenuModel<AcceptHistoryItem, HistoryTab>>,
}

impl Entity for TerminalInputMessageBar {
    type Event = ();
}

impl TerminalInputMessageBar {
    pub fn new(
        suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
        inline_history_model: ModelHandle<InlineMenuModel<AcceptHistoryItem, HistoryTab>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&suggestions_mode_model, |_, _, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged { .. } = event;
            ctx.notify();
        });
        ctx.subscribe_to_model(&inline_history_model, |_, _, event, ctx| {
            if let InlineMenuModelEvent::UpdatedSelectedItem = event {
                ctx.notify();
            }
        });

        Self {
            suggestions_mode_model,
            inline_history_model,
        }
    }
}

impl View for TerminalInputMessageBar {
    fn ui_name() -> &'static str {
        "TerminalInputMessageBar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_history_menu()
        {
            let selected = self.inline_history_model.as_ref(app).selected_item();
            let message = InlineHistoryMessageProducer
                .produce_message(selected)
                .unwrap_or_default();
            return Container::new(render_terminal_message(message, app))
                .with_padding_bottom(8.)
                .with_padding_right(8.)
                .finish();
        }

        Container::new(render_terminal_message(Message::default(), app))
            .with_padding_bottom(8.)
            .with_padding_right(8.)
            .finish()
    }
}

struct InlineHistoryMessageProducer;
impl MessageProvider<Option<&AcceptHistoryItem>> for InlineHistoryMessageProducer {
    fn produce_message(&self, selected: Option<&AcceptHistoryItem>) -> Option<Message> {
        let enter = MessageItem::keystroke(Keystroke {
            key: "enter".to_owned(),
            ..Default::default()
        });
        let items = match selected {
            Some(AcceptHistoryItem::Command { .. }) => {
                vec![enter, MessageItem::text(" to execute")]
            }
            Some(AcceptHistoryItem::AIPrompt { .. }) => {
                vec![enter, MessageItem::text(" to send")]
            }
            Some(AcceptHistoryItem::Conversation { title, .. }) => {
                vec![enter, MessageItem::text(format!(" to open '{title}'"))]
            }
            None => {
                vec![MessageItem::text("")]
            }
        };
        Some(Message::new(items))
    }
}
