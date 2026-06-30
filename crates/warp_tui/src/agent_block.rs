//! Simple input and streamed plain-text agent blocks for the TUI transcript.
use std::rc::Rc;

use warp::tui_export::{AIAgentTextSection, AIBlockModel};
use warpui_core::elements::tui::{
    Color, TuiColumn, TuiElement, TuiParentElement, TuiStyle, TuiText,
};
use warpui_core::{AppContext, Entity, TuiView};

const INPUT_COLOR: Color = Color::Rgb(0x8e, 0x8e, 0x8e);
const OUTPUT_COLOR: Color = Color::Rgb(0xf1, 0xf1, 0xf1);

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
        let content = self.content(app);
        desired_content_height(&content, width)
    }

    /// Renders the complete agent block.
    pub(super) fn render_full(&self, app: &AppContext) -> Box<dyn TuiElement> {
        render_content(&self.content(app))
    }

    fn content(&self, app: &AppContext) -> AgentBlockContent {
        let mut items = Vec::new();
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
                    AIAgentTextSection::PlainText { text } if !text.text().is_empty() => {
                        Some(TuiAgentBlockItem::PlainText(text.text().to_owned()))
                    }
                    AIAgentTextSection::PlainText { .. }
                    | AIAgentTextSection::Code { .. }
                    | AIAgentTextSection::Table { .. }
                    | AIAgentTextSection::Image { .. }
                    | AIAgentTextSection::MermaidDiagram { .. } => None,
                })
            }));
        }

        AgentBlockContent { items }
    }
}

fn desired_content_height(content: &AgentBlockContent, width: u16) -> usize {
    content
        .items
        .iter()
        .map(|item| usize::from(item.text_element().desired_height(width)))
        .sum::<usize>()
        .max(1)
}

fn render_content(content: &AgentBlockContent) -> Box<dyn TuiElement> {
    let mut column = TuiColumn::new();
    for item in &content.items {
        column = column.with_child(Box::new(item.text_element()));
    }
    Box::new(column)
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
        self.render_full(app)
    }
}
