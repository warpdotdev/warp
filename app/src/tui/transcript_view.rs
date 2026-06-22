//! [`TuiTranscriptView`]: a read-only projection of the active TUI agent
//! conversation. Submitted prompts and streamed responses live in
//! [`BlocklistAIHistoryModel`], so the transcript never mirrors conversation
//! state into a separate buffer.

use warpui::SingletonEntity;
use warpui_core::elements::tui::{Color, Modifier, TuiColumn, TuiElement, TuiStyle, TuiText};
use warpui_core::{AppContext, Entity, TuiView};

use super::{CoreTuiModel, TuiToolActionModel};
use crate::ai::blocklist::BlocklistAIHistoryModel;

/// Near-white transcript text (`#f1f1f1`).
const TEXT_COLOR: Color = Color::Rgb(0xf1, 0xf1, 0xf1);

#[derive(Default)]
pub struct TuiTranscriptView;

impl Entity for TuiTranscriptView {
    type Event = ();
}

impl TuiView for TuiTranscriptView {
    fn ui_name() -> &'static str {
        "TuiTranscriptView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let Some(conversation_id) = CoreTuiModel::as_ref(ctx).active_conversation_id() else {
            return Box::new(());
        };
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return Box::new(());
        };

        let user_style = TuiStyle::default()
            .fg(TEXT_COLOR)
            .add_modifier(Modifier::BOLD);
        let agent_style = TuiStyle::default().fg(TEXT_COLOR);
        let mut children = Vec::<Box<dyn TuiElement>>::new();

        for exchange in conversation.root_task_exchanges() {
            let input = exchange.format_input_for_copy();
            let output = exchange.format_output_for_copy(None);
            let has_input = !input.is_empty();
            let has_output = !output.is_empty();

            if has_input {
                children.push(Box::new(TuiText::new(input).with_style(user_style)));
            }
            if has_output {
                children.push(Box::new(TuiText::new(output).with_style(agent_style)));
            }
            if let Some(output) = exchange.output_status.output() {
                for action in output.get().actions() {
                    if let Some(card) =
                        TuiToolActionModel::as_ref(ctx).card_for_action(conversation_id, &action.id)
                    {
                        children.push(Box::new(
                            TuiText::new(format!("[ {} ]", card.title)).with_style(agent_style),
                        ));
                        for line in &card.lines {
                            children.push(Box::new(
                                TuiText::new(format!("  {line}")).with_style(agent_style),
                            ));
                        }
                    }
                }
            }
            if has_input || has_output {
                children.push(Box::new(TuiText::new(" ")));
            }
        }

        Box::new(TuiColumn::with_children(children))
    }
}
