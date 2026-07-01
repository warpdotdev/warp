//! Simple input and streamed plain-text agent blocks for the TUI transcript.
use std::rc::Rc;

use warp::tui_export::{AIAgentTextSection, AIBlockModel, Appearance};
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Color, Modifier, TuiColumn, TuiConstraint, TuiContainer, TuiElement, TuiLayoutContext,
    TuiParentElement, TuiSize, TuiStyle, TuiText,
};
use warpui_core::elements::Fill;
use warpui_core::{AppContext, Entity, EntityIdMap, TuiView};

const INPUT_PREFIX: &str = "≫ ";
const INPUT_OUTPUT_GAP_ROWS: u16 = 1;
const BLOCK_BOTTOM_PADDING_ROWS: u16 = 1;

/// Renderable pieces of an agent block; this will grow as we add tool calls and other sub-elements.
#[derive(Clone, Debug, Eq, PartialEq)]
enum TuiAgentBlockSection {
    Input(Vec<String>),
    PlainText(String),
}

/// A thin TUI rich-content view adapter backed by one agent exchange.
///
/// The rendering logic is mostly section extraction, but the shared block list
/// stores rich content by view id, so this remains a registered view.
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
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let mut element = self.render_element(app);
        usize::from(
            element
                .layout(
                    TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
                    &mut ctx,
                    app,
                )
                .height,
        )
    }

    /// Extracts this exchange's visible input/output into logical render sections.
    fn sections(&self, app: &AppContext) -> Vec<TuiAgentBlockSection> {
        let mut sections = Vec::new();
        let input_lines = self
            .model
            .inputs_to_render(app)
            .iter()
            .filter_map(|input| input.display_query())
            .flat_map(display_input_lines)
            .collect::<Vec<_>>();
        if input_lines.iter().any(|line| !line.is_empty()) {
            sections.push(TuiAgentBlockSection::Input(input_lines));
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

        sections
    }
    /// Builds this block's generic TUI element tree.
    fn render_element(&self, app: &AppContext) -> Box<dyn TuiElement> {
        Self::render_sections(&self.sections(app), app)
    }

    /// Builds the generic TUI element tree for logical render sections.
    fn render_sections(sections: &[TuiAgentBlockSection], app: &AppContext) -> Box<dyn TuiElement> {
        let theme = Appearance::as_ref(app).theme();
        let background: Color = theme.surface_1().into();
        let mut column = TuiColumn::new();
        let mut should_gap_before_next = false;
        for section in sections {
            let top_padding = if should_gap_before_next {
                INPUT_OUTPUT_GAP_ROWS
            } else {
                0
            };
            column = column.with_child(section.render_element(top_padding, app));
            should_gap_before_next = matches!(section, TuiAgentBlockSection::Input(_));
        }
        Box::new(
            TuiContainer::new(column)
                .with_background(background)
                .with_padding_bottom(u16::from(!sections.is_empty()) * BLOCK_BOTTOM_PADDING_ROWS),
        )
    }
}

/// Splits an agent input query into display lines while preserving blank lines.
fn display_input_lines(query: &str) -> impl Iterator<Item = String> + '_ {
    query.split('\n').map(str::to_owned)
}
/// Converts one logical section into a renderable TUI element.
impl TuiAgentBlockSection {
    fn render_element(&self, top_padding: u16, app: &AppContext) -> Box<dyn TuiElement> {
        let theme = Appearance::as_ref(app).theme();
        match self {
            Self::Input(lines) => {
                let text_color: Color = theme.foreground().into();
                let accent = Fill::from(theme.terminal_colors().normal.cyan);
                let background: Color = theme
                    .background()
                    .blend(&accent.with_opacity(10))
                    .blend(&accent.with_opacity(10))
                    .into();
                let mut column = TuiColumn::new();
                for line in lines {
                    column = column.child(
                        TuiText::new(format!("{INPUT_PREFIX}{line}")).with_style(
                            TuiStyle::default()
                                .fg(text_color)
                                .bg(background)
                                .add_modifier(Modifier::BOLD),
                        ),
                    );
                }
                Box::new(
                    TuiContainer::new(column)
                    .with_background(background)
                    .with_padding_top(top_padding),
                )
            }
            Self::PlainText(text) => {
                let text_color: Color = Fill::from(theme.terminal_colors().normal.white).into();
                Box::new(
                    TuiContainer::new(
                        TuiText::new(text.clone()).with_style(TuiStyle::default().fg(text_color)),
                    )
                    .with_padding_top(top_padding),
                )
            }
        }
    }
}


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
        self.render_element(app)
    }
}

#[cfg(test)]
#[path = "agent_block_tests.rs"]
mod tests;
