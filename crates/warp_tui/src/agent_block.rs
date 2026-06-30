//! Simple input and streamed plain-text agent blocks for the TUI transcript.
use std::rc::Rc;

use warp::tui_export::{AIAgentTextSection, AIBlockModel};
use warpui_core::elements::tui::{
    Color, TuiColumn, TuiElement, TuiParentElement, TuiStyle, TuiText,
};
use warpui_core::{AppContext, Entity, TuiView};

const INPUT_COLOR: Color = Color::from_u32(0x8e8e8e);
const OUTPUT_COLOR: Color = Color::from_u32(0xf1f1f1);

/// Renderable pieces of an agent block; this will grow as we add tool calls and other sub-elements.
#[derive(Debug, Eq, PartialEq)]
enum TuiAgentBlockItem {
    Input(String),
    PlainText(String),
}

#[derive(Debug, Eq, PartialEq)]
struct AgentBlockContent {
    items: Vec<TuiAgentBlockItem>,
}

/// A simple TUI block backed by one agent exchange.
pub(super) struct TuiAgentBlockView {
    model: Rc<dyn AIBlockModel<View = Self>>,
}

impl TuiAgentBlockView {
    /// Creates a simple exchange-backed agent block.
    pub(super) fn new(model: Rc<dyn AIBlockModel<View = Self>>) -> Self {
        Self { model }
    }

    /// Replaces the exchange model backing this block.
    pub(super) fn set_model(&mut self, model: Rc<dyn AIBlockModel<View = Self>>) {
        self.model = model;
    }

    /// Returns this block's wrapped height at the given width.
    pub(super) fn desired_height(&self, width: u16, app: &AppContext) -> usize {
        self.content(app).desired_height(width)
    }

    fn content(&self, app: &AppContext) -> AgentBlockContent {
        let mut items = Vec::new();
        // Match the GUI block prompt text: displayable input queries separated by newlines.
        let input = self
            .model
            .inputs_to_render(app)
            .iter()
            .filter_map(|input| input.display_query())
            .collect::<Vec<_>>()
            .join("\n");
        if !input.is_empty() {
            items.push(TuiAgentBlockItem::Input(input));
        }

        if let Some(output) = self.model.status(app).output_to_render() {
            let output = output.get();
            items.extend(output.text_from_agent_output().flat_map(|text| {
                text.sections.iter().filter_map(|section| match section {
                    AIAgentTextSection::PlainText { text } => {
                        (!text.text().is_empty())
                            .then(|| TuiAgentBlockItem::PlainText(text.text().to_owned()))
                    }
                    // Add item variants here as the TUI learns to render richer sections.
                    AIAgentTextSection::Code { .. }
                    | AIAgentTextSection::Table { .. }
                    | AIAgentTextSection::Image { .. }
                    | AIAgentTextSection::MermaidDiagram { .. } => None,
                })
            }));
        }

        AgentBlockContent { items }
    }
}

impl AgentBlockContent {
    /// Returns this content's wrapped height at the given width.
    fn desired_height(&self, width: u16) -> usize {
        self.items
            .iter()
            .map(|item| usize::from(item.text_element().desired_height(width)))
            .sum::<usize>()
            .max(1)
    }

    /// Renders this content as a vertical list of agent block items.
    fn render(&self) -> Box<dyn TuiElement> {
        let mut column = TuiColumn::new();
        for item in &self.items {
            column = column.with_child(Box::new(item.text_element()));
        }
        Box::new(column)
    }
}

impl TuiAgentBlockItem {
    /// Renders this item as one styled TUI text element.
    fn text_element(&self) -> TuiText {
        let (text, color) = match self {
            Self::Input(text) => (text.clone(), INPUT_COLOR),
            Self::PlainText(text) => (text.clone(), OUTPUT_COLOR),
        };
        TuiText::new(text).with_style(TuiStyle::default().fg(color))
    }
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
        self.content(app).render()
    }
}
