//! Simple input and streamed plain-text agent blocks for the TUI transcript.

use warp::tui_export::{
    AIAgentExchangeId, AIAgentTextSection, AIConversationId, BlocklistAIHistoryModel,
};
use warpui_core::elements::tui::{
    Color, TuiColumn, TuiElement, TuiParentElement, TuiStyle, TuiText,
};
use warpui_core::{AppContext, Entity, SingletonEntity, TuiView};

const INPUT_COLOR: Color = Color::Rgb(0x8e, 0x8e, 0x8e);
const OUTPUT_COLOR: Color = Color::Rgb(0xf1, 0xf1, 0xf1);

struct AgentBlockContent {
    input: String,
    output: String,
}

/// A simple TUI block backed by one agent exchange.
pub(super) struct TuiAgentBlockView {
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
}

impl TuiAgentBlockView {
    /// Creates a simple exchange-backed agent block.
    pub(super) fn new(conversation_id: AIConversationId, exchange_id: AIAgentExchangeId) -> Self {
        Self {
            conversation_id,
            exchange_id,
        }
    }

    /// Updates the conversation containing this exchange.
    pub(super) fn set_conversation_id(&mut self, conversation_id: AIConversationId) {
        self.conversation_id = conversation_id;
    }

    /// Returns this block's wrapped height at the given width.
    pub(super) fn desired_height(&self, width: u16, app: &AppContext) -> usize {
        let content = self.content(app);
        desired_content_height(&content, width)
    }

    /// Renders the complete agent block.
    pub(super) fn render_full(&self, app: &AppContext) -> Box<dyn TuiElement> {
        render_content(&self.content(app))
    }

    fn content(&self, app: &AppContext) -> AgentBlockContent {
        let Some(exchange) = BlocklistAIHistoryModel::as_ref(app)
            .conversation(&self.conversation_id)
            .and_then(|conversation| conversation.exchange_with_id(self.exchange_id))
        else {
            return AgentBlockContent {
                input: String::new(),
                output: String::new(),
            };
        };

        let input = exchange.format_input_for_copy();
        let output = exchange
            .output_status
            .output()
            .map(|output| {
                let output = output.get();
                output
                    .text_from_agent_output()
                    .flat_map(|text| text.sections.iter())
                    .filter_map(|section| match section {
                        AIAgentTextSection::PlainText { text } => Some(text.text()),
                        AIAgentTextSection::Code { .. }
                        | AIAgentTextSection::Table { .. }
                        | AIAgentTextSection::Image { .. }
                        | AIAgentTextSection::MermaidDiagram { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        AgentBlockContent { input, output }
    }
}

fn desired_content_height(content: &AgentBlockContent, width: u16) -> usize {
    let input = TuiText::new(content.input.clone()).with_style(TuiStyle::default().fg(INPUT_COLOR));
    let output =
        TuiText::new(content.output.clone()).with_style(TuiStyle::default().fg(OUTPUT_COLOR));
    let input_height = usize::from(input.desired_height(width));
    let output_height = usize::from(output.desired_height(width));
    (input_height + output_height).max(1)
}

fn render_content(content: &AgentBlockContent) -> Box<dyn TuiElement> {
    Box::new(
        TuiColumn::new()
            .with_child(Box::new(
                TuiText::new(content.input.clone()).with_style(TuiStyle::default().fg(INPUT_COLOR)),
            ))
            .with_child(Box::new(
                TuiText::new(content.output.clone())
                    .with_style(TuiStyle::default().fg(OUTPUT_COLOR)),
            )),
    )
}

#[cfg(test)]
#[path = "agent_block_tests.rs"]
mod tests;

impl Entity for TuiAgentBlockView {
    type Event = ();
}

impl TuiView for TuiAgentBlockView {
    fn ui_name() -> &'static str {
        "TuiAgentBlockView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        self.render_full(app)
    }
}
