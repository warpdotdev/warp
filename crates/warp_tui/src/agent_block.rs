//! Simple input and streamed plain-text agent blocks for the TUI transcript.
use std::cell::RefCell;
use std::rc::Rc;

use warp::tui_export::{AIAgentTextSection, AIBlockModel};
use warpui_core::elements::tui::{
    Color, Modifier, TuiBuffer, TuiColumn, TuiConstraint, TuiContainer, TuiElement,
    TuiLayoutContext, TuiParentElement, TuiRect, TuiSize, TuiStyle, TuiText,
};
use warpui_core::{AppContext, Entity, EntityIdMap, TuiView};

const INPUT_PREFIX: &str = "≫ ";
const INPUT_TEXT_COLOR: Color = Color::from_u32(0xffffff);
const INPUT_BACKGROUND: Color = Color::from_u32(0x2c2d34);
const INPUT_OUTPUT_GAP_ROWS: u16 = 1;
const BLOCK_BOTTOM_PADDING_ROWS: u16 = 1;
const OUTPUT_COLOR: Color = Color::from_u32(0xf1f1f1);

/// Renderable pieces of an agent block; this will grow as we add tool calls and other sub-elements.
#[derive(Clone, Debug, Eq, PartialEq)]
enum TuiAgentBlockSection {
    Input(String),
    PlainText(String),
}

struct TuiAgentBlockElement {
    sections: Vec<TuiAgentBlockSection>,
    /// Child tree built during layout and reused during render because TUI child
    /// elements retain layout state needed for painting.
    content: RefCell<Option<Box<dyn TuiElement>>>,
}

/// A simple TUI block backed by one agent exchange.
pub(super) struct TuiAgentBlockView {
    model: Rc<dyn AIBlockModel<View = Self>>,
}

/// Extracts model state into renderable agent block sections.
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
        self.element(app).desired_height(width, app)
    }

    fn element(&self, app: &AppContext) -> TuiAgentBlockElement {
        let mut sections = Vec::new();
        // Match the GUI block prompt text: displayable input queries separated by newlines.
        let input = self
            .model
            .inputs_to_render(app)
            .iter()
            .filter_map(|input| input.display_query())
            .collect::<Vec<_>>()
            .join("\n");
        if !input.is_empty() {
            sections.push(TuiAgentBlockSection::Input(input));
        }

        if let Some(output) = self.model.status(app).output_to_render() {
            let output = output.get();
            sections.extend(output.text_from_agent_output().flat_map(|text| {
                text.sections.iter().filter_map(|section| match section {
                    AIAgentTextSection::PlainText { text } => (!text.text().is_empty())
                        .then(|| TuiAgentBlockSection::PlainText(text.text().to_owned())),
                    // Add item variants here as the TUI learns to render richer sections.
                    AIAgentTextSection::Code { .. }
                    | AIAgentTextSection::Table { .. }
                    | AIAgentTextSection::Image { .. }
                    | AIAgentTextSection::MermaidDiagram { .. } => None,
                })
            }));
        }

        TuiAgentBlockElement::new(sections)
    }
}

/// Lays out and paints the composed TUI element tree for an agent block.
impl TuiElement for TuiAgentBlockElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let mut content = self.render_element();
        let size = content.layout(constraint, ctx, app);
        *self.content.borrow_mut() = Some(content);
        size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        if let Some(content) = self.content.borrow().as_ref() {
            content.render(area, buffer, ctx);
        }
    }
}

/// Builds and measures the composed TUI element tree for all sections.
impl TuiAgentBlockElement {
    /// Creates an agent block element from logical render sections.
    fn new(sections: Vec<TuiAgentBlockSection>) -> Self {
        Self {
            sections,
            content: RefCell::new(None),
        }
    }

    /// Returns this element's laid-out height at the given width.
    fn desired_height(&self, width: u16, app: &AppContext) -> usize {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        usize::from(
            self.render_element()
                .layout(
                    TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
                    &mut ctx,
                    app,
                )
                .height,
        )
    }

    /// Builds the renderable element tree used by both layout and rendering.
    fn render_element(&self) -> Box<dyn TuiElement> {
        let mut column = TuiColumn::new();
        let mut should_gap_before_next = false;
        for section in &self.sections {
            let top_padding = if should_gap_before_next {
                INPUT_OUTPUT_GAP_ROWS
            } else {
                0
            };
            column = column.with_child(section.render_element(top_padding));
            should_gap_before_next = matches!(section, TuiAgentBlockSection::Input(_));
        }
        Box::new(
            TuiContainer::new(column).with_padding_bottom(
                u16::from(!self.sections.is_empty()) * BLOCK_BOTTOM_PADDING_ROWS,
            ),
        )
    }
}

/// Converts one logical section into a renderable TUI element.
impl TuiAgentBlockSection {
    fn render_element(&self, top_padding: u16) -> Box<dyn TuiElement> {
        match self {
            Self::Input(text) => Box::new(
                TuiContainer::new(
                    TuiText::new(format!("{INPUT_PREFIX}{text}")).with_style(
                        TuiStyle::default()
                            .fg(INPUT_TEXT_COLOR)
                            .bg(INPUT_BACKGROUND)
                            .add_modifier(Modifier::BOLD),
                    ),
                )
                .with_background(INPUT_BACKGROUND)
                .with_padding_top(top_padding),
            ),
            Self::PlainText(text) => Box::new(
                TuiContainer::new(
                    TuiText::new(text.clone()).with_style(TuiStyle::default().fg(OUTPUT_COLOR)),
                )
                .with_padding_top(top_padding),
            ),
        }
    }
}

#[cfg(test)]
#[path = "agent_block_tests.rs"]
mod tests;

/// Registers the view with the TUI runtime.
impl Entity for TuiAgentBlockView {
    type Event = ();
}

/// Renders the model-backed block as a TUI element.
impl TuiView for TuiAgentBlockView {
    fn ui_name() -> &'static str {
        "TuiAgentBlockView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        Box::new(self.element(app))
    }
}
