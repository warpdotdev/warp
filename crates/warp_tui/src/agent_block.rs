//! An agent block in the TUI transcript: one exchange rendered as the user's
//! submitted input followed by the agent's response.
use std::rc::Rc;

use warp::tui_export::{
    AIAgentExchangeId, AIAgentTextSection, AIBlockModel, AIConversationId, Appearance,
};
use warp_core::ui::color::blend::Blend;
// `ThemeFill` is the theme-layer color (it supports blend/opacity); `Fill` below
// is the element-layer color it converts into on its way to a terminal cell.
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Modifier, TuiColumn, TuiConstraint, TuiContainer, TuiElement, TuiLayoutContext,
    TuiParentElement, TuiSize, TuiStyle, TuiText,
};
use warpui_core::elements::Fill;
use warpui_core::{AppContext, Entity, EntityIdMap, TuiView};

const INPUT_PREFIX: &str = "≫ ";

/// Renderable pieces of an agent block; this will grow as we add tool calls and other sub-elements.
#[derive(Clone, Debug, Eq, PartialEq)]
enum TuiAIBlockSection {
    Input(String),
    PlainText(String),
}

/// A thin TUI rich-content view adapter backed by one agent exchange.
///
/// The rendering logic is mostly section extraction, but the shared block list
/// stores rich content by view id, so this remains a registered view.
pub(super) struct TuiAIBlock {
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
    model: Rc<dyn AIBlockModel<View = Self>>,
}

/// Extracts model state into renderable agent block sections.
impl TuiAIBlock {
    /// Creates a simple exchange-backed agent block.
    pub(super) fn new(
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
        model: Rc<dyn AIBlockModel<View = Self>>,
    ) -> Self {
        Self {
            conversation_id,
            exchange_id,
            model,
        }
    }

    /// Replaces the backing model when the same exchange is reassigned.
    pub(super) fn replace_model(
        &mut self,
        conversation_id: AIConversationId,
        model: Rc<dyn AIBlockModel<View = Self>>,
    ) {
        self.conversation_id = conversation_id;
        self.model = model;
    }

    /// Returns the conversation that currently owns this agent block.
    pub(super) fn conversation_id(&self) -> AIConversationId {
        self.conversation_id
    }

    /// Returns the exchange rendered by this agent block.
    pub(super) fn exchange_id(&self) -> AIAgentExchangeId {
        self.exchange_id
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
    fn sections(&self, app: &AppContext) -> Vec<TuiAIBlockSection> {
        let mut sections = Vec::new();
        let input = self
            .model
            .inputs_to_render(app)
            .iter()
            .filter_map(|input| input.display_query())
            .collect::<Vec<_>>()
            .join("\n");
        if !input.is_empty() {
            sections.push(TuiAIBlockSection::Input(input));
        }

        if let Some(output) = self.model.status(app).output_to_render() {
            let output = output.get();
            sections.extend(output.text_from_agent_output().flat_map(|text| {
                text.sections.iter().filter_map(|section| match section {
                    AIAgentTextSection::PlainText { text } => (!text.text().is_empty())
                        .then(|| TuiAIBlockSection::PlainText(text.text().to_owned())),
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
        let sections = self.sections(app);

        let mut column = TuiColumn::new();
        for (index, section) in sections.iter().enumerate() {
            // Output is many sections (one per text section), so top padding is
            // applied only to the section right after the input, giving a single
            // gap at the input→output boundary rather than before every line.
            let follows_input = index
                .checked_sub(1)
                .is_some_and(|prev| matches!(sections[prev], TuiAIBlockSection::Input(_)));
            column = column.with_child(section.render_element(u16::from(follows_input), app));
        }

        // No background of its own: the block shows the terminal's background,
        // matching the Figma where only the input line is highlighted.
        TuiContainer::new(column)
            .with_padding_bottom(u16::from(!sections.is_empty()))
            .finish()
    }
}

/// Converts one logical section into a renderable TUI element.
impl TuiAIBlockSection {
    fn render_element(&self, top_padding: u16, app: &AppContext) -> Box<dyn TuiElement> {
        let theme = Appearance::as_ref(app).theme();
        match self {
            Self::Input(text) => {
                let text_color = Fill::from(theme.foreground()).into();
                let accent = ThemeFill::from(theme.terminal_colors().normal.cyan);
                let background = Fill::from(
                    theme
                        .background()
                        .blend(&accent.with_opacity(10))
                        .blend(&accent.with_opacity(10)),
                )
                .into();
                // Only the first line carries the `≫` prompt marker; continuation
                // lines are indented to the marker's width so they align beneath it.
                let mut column = TuiColumn::new();
                for (index, line) in text.split('\n').enumerate() {
                    let line_text = if index == 0 {
                        format!("{INPUT_PREFIX}{line}")
                    } else {
                        format!("{}{line}", " ".repeat(INPUT_PREFIX.chars().count()))
                    };
                    column = column.child(
                        TuiText::new(line_text).with_style(
                            TuiStyle::default()
                                .fg(text_color)
                                .bg(background)
                                .add_modifier(Modifier::BOLD),
                        ),
                    );
                }
                TuiContainer::new(column)
                    .with_background(background)
                    .with_padding_top(top_padding)
                    .finish()
            }
            Self::PlainText(text) => {
                let text_color =
                    Fill::from(ThemeFill::from(theme.terminal_colors().normal.white)).into();
                TuiContainer::new(
                    TuiText::new(text.clone()).with_style(TuiStyle::default().fg(text_color)),
                )
                .with_padding_top(top_padding)
                .finish()
            }
        }
    }
}

/// Registers the view with the TUI runtime.
impl Entity for TuiAIBlock {
    type Event = ();
}

/// Renders the model-backed block as a TUI element.
impl TuiView for TuiAIBlock {
    fn ui_name() -> &'static str {
        "TuiAIBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        self.render_element(app)
    }
}

#[cfg(test)]
#[path = "agent_block_tests.rs"]
mod tests;
