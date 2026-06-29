//! Simple input and streamed plain-text agent blocks for the TUI transcript.

use std::ops::Range;

use warp::tui_export::{
    AIAgentExchangeId, AIAgentTextSection, AIConversationId, BlocklistAIHistoryModel,
};
use warpui_core::elements::tui::{
    Color, RenderedViewportItem, TuiClipped, TuiColumn, TuiElement, TuiParentElement, TuiStyle,
    TuiText,
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

    /// Renders the requested logical rows and reports the full wrapped height.
    pub(super) fn render_visible_rows(
        &self,
        visible_rows: Range<usize>,
        width: u16,
        app: &AppContext,
    ) -> RenderedViewportItem {
        let content = self.content(app);
        render_content_visible_rows(&content, visible_rows, width)
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

fn render_content_visible_rows(
    content: &AgentBlockContent,
    visible_rows: Range<usize>,
    width: u16,
) -> RenderedViewportItem {
    let input = TuiText::new(content.input.clone()).with_style(TuiStyle::default().fg(INPUT_COLOR));
    let output =
        TuiText::new(content.output.clone()).with_style(TuiStyle::default().fg(OUTPUT_COLOR));
    let input_height = usize::from(input.desired_height(width));
    let output_height = usize::from(output.desired_height(width));
    let full_height = (input_height + output_height).max(1);
    let mut children = Vec::<Box<dyn TuiElement>>::new();

    push_visible_text(&mut children, input, 0..input_height, visible_rows.clone());
    push_visible_text(
        &mut children,
        output,
        input_height..input_height + output_height,
        visible_rows,
    );
    if children.is_empty() {
        children.push(Box::new(TuiText::new(" ")));
    }

    RenderedViewportItem {
        element: Box::new(TuiColumn::new().with_children(children)),
        measured_full_height: Some(full_height),
    }
}

#[cfg(test)]
#[path = "agent_block_tests.rs"]
mod tests;

fn push_visible_text(
    children: &mut Vec<Box<dyn TuiElement>>,
    text: TuiText,
    text_rows: Range<usize>,
    visible_rows: Range<usize>,
) {
    let start = text_rows.start.max(visible_rows.start);
    let end = text_rows.end.min(visible_rows.end);
    if start < end {
        children.push(Box::new(
            TuiClipped::new(text).with_vertical_offset(start.saturating_sub(text_rows.start)),
        ));
    }
}

impl Entity for TuiAgentBlockView {
    type Event = ();
}

impl TuiView for TuiAgentBlockView {
    fn ui_name() -> &'static str {
        "TuiAgentBlockView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let content = self.content(app);
        Box::new(
            TuiColumn::new()
                .with_child(Box::new(
                    TuiText::new(content.input).with_style(TuiStyle::default().fg(INPUT_COLOR)),
                ))
                .with_child(Box::new(
                    TuiText::new(content.output).with_style(TuiStyle::default().fg(OUTPUT_COLOR)),
                )),
        )
    }
}
