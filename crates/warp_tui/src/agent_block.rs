//! An agent block in the TUI transcript: one exchange rendered as the user's
//! submitted input followed by the agent's response.
//!
//! This module owns section extraction ([`TuiAIBlock::sections`]) and
//! composition ([`TuiAIBlock::render_element`]); the per-section render
//! functions live in [`crate::agent_block_sections`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use itertools::Itertools;
use warp::tui_export::{
    AIAgentAction, AIAgentExchangeId, AIAgentOutputMessageType, AIAgentTextSection, AIBlockModel,
    AIConversationId, MessageId,
};
use warpui_core::elements::tui::{
    TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiLayoutContext, TuiParentElement, TuiSize,
};
use warpui_core::elements::MouseStateHandle;
use warpui_core::{AppContext, Entity, EntityIdMap, TuiView};

use crate::agent_block_sections::{
    render_input_section, render_plain_text_section, render_thinking_section,
    render_tool_call_section,
};

/// Renderable pieces of an agent block; this will grow as we render richer sections.
#[derive(Clone, Debug, Eq, PartialEq)]
enum TuiAIBlockSection {
    Input(String),
    PlainText(String),
    /// A lightweight status row standing in for an agent tool call.
    ToolCall(Box<AIAgentAction>),
    /// A reasoning ("thinking") segment, rendered as a collapsible block.
    Thinking {
        message_id: MessageId,
        finished_duration: Option<Duration>,
        body: String,
    },
}

/// Per-message UI state for thinking blocks, keyed by reasoning message.
#[derive(Clone, Default)]
pub(crate) struct ThinkingBlockStates {
    states: Rc<RefCell<HashMap<MessageId, ThinkingBlockState>>>,
}

/// UI state for a single thinking block.
#[derive(Default)]
struct ThinkingBlockState {
    /// Manual collapse override. `None` means the default: collapsed iff
    /// reasoning has finished, so a block streams expanded and auto-collapses
    /// on finish unless the user has toggled it — a recorded override wins
    /// permanently.
    collapse_override: Option<bool>,
    /// Hover state for the thinking header. Owned here (not created inline
    /// during render) so it survives element-tree rebuilds, following the
    /// GUI's `MouseStateHandle` pattern.
    hover_state: MouseStateHandle,
}

impl ThinkingBlockStates {
    /// Whether the thinking block for `message_id` is collapsed: the manual
    /// override if one was recorded, else collapsed iff `finished`.
    pub(crate) fn is_collapsed(&self, message_id: &MessageId, finished: bool) -> bool {
        self.states
            .borrow()
            .get(message_id)
            .and_then(|state| state.collapse_override)
            .unwrap_or(finished)
    }

    /// Records a manual collapse override for `message_id`.
    pub(crate) fn set_collapsed(&self, message_id: MessageId, collapsed: bool) {
        self.states
            .borrow_mut()
            .entry(message_id)
            .or_default()
            .collapse_override = Some(collapsed);
    }

    /// Returns the persistent hover state handle for `message_id`.
    pub(crate) fn hover_state(&self, message_id: &MessageId) -> MouseStateHandle {
        self.states
            .borrow_mut()
            .entry(message_id.clone())
            .or_default()
            .hover_state
            .clone()
    }
}

/// A thin TUI rich-content view adapter backed by one agent exchange.
///
/// The rendering logic is mostly section extraction, but the shared block list
/// stores rich content by view id, so this remains a registered view.
pub(super) struct TuiAIBlock {
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
    model: Rc<dyn AIBlockModel<View = Self>>,
    /// Per-message UI state for this exchange's thinking blocks.
    thinking_states: ThinkingBlockStates,
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
            thinking_states: Default::default(),
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

    /// Extracts this exchange's visible input/output into logical render sections,
    /// preserving message order so reasoning interleaves with plain-text output.
    fn sections(&self, app: &AppContext) -> Vec<TuiAIBlockSection> {
        let mut sections = Vec::new();
        let input = self
            .model
            .inputs_to_render(app)
            .iter()
            .filter_map(|input| input.display_query())
            .join("\n");
        if !input.is_empty() {
            sections.push(TuiAIBlockSection::Input(input));
        }

        // Walk output messages in order so tool-call rows interleave with text.
        if let Some(output) = self.model.status(app).output_to_render() {
            let output = output.get();
            for message in &output.messages {
                match &message.message {
                    AIAgentOutputMessageType::Text(text) => {
                        sections.extend(
                            text.sections
                                .iter()
                                .filter_map(|section| match section {
                                    AIAgentTextSection::PlainText { text } => Some(text.text()),
                                    // The TUI can't render these section kinds yet.
                                    AIAgentTextSection::Code { .. }
                                    | AIAgentTextSection::Table { .. }
                                    | AIAgentTextSection::Image { .. }
                                    | AIAgentTextSection::MermaidDiagram { .. } => None,
                                })
                                .filter(|line| !line.is_empty())
                                .map(|line| TuiAIBlockSection::PlainText(line.to_owned())),
                        );
                    }
                    AIAgentOutputMessageType::Action(action) => {
                        sections.push(TuiAIBlockSection::ToolCall(Box::new(action.clone())));
                    }
                    AIAgentOutputMessageType::Reasoning {
                        text,
                        finished_duration,
                    } => {
                        sections.push(TuiAIBlockSection::Thinking {
                            message_id: message.id.clone(),
                            finished_duration: *finished_duration,
                            body: text
                                .sections
                                .iter()
                                .filter_map(|section| match section {
                                    AIAgentTextSection::PlainText { text } => Some(text.text()),
                                    // The TUI can't render these section kinds yet.
                                    AIAgentTextSection::Code { .. }
                                    | AIAgentTextSection::Table { .. }
                                    | AIAgentTextSection::Image { .. }
                                    | AIAgentTextSection::MermaidDiagram { .. } => None,
                                })
                                .join("\n"),
                        });
                    }
                    // Other message kinds are not rendered by the TUI transcript yet.
                    AIAgentOutputMessageType::Summarization { .. }
                    | AIAgentOutputMessageType::Subagent(_)
                    | AIAgentOutputMessageType::TodoOperation(_)
                    | AIAgentOutputMessageType::WebSearch(_)
                    | AIAgentOutputMessageType::WebFetch(_)
                    | AIAgentOutputMessageType::CommentsAddressed { .. }
                    | AIAgentOutputMessageType::DebugOutput { .. }
                    | AIAgentOutputMessageType::ArtifactCreated(_)
                    | AIAgentOutputMessageType::SkillInvoked(_)
                    | AIAgentOutputMessageType::MessagesReceivedFromAgents { .. }
                    | AIAgentOutputMessageType::EventsFromAgents { .. } => {}
                }
            }
        }

        sections
    }

    /// Builds this block's generic TUI element tree.
    fn render_element(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let mut column = TuiFlex::column();
        for section in &self.sections(app) {
            let element = match section {
                TuiAIBlockSection::Input(text) => render_input_section(text, app),
                TuiAIBlockSection::PlainText(text) => render_plain_text_section(text, app),
                TuiAIBlockSection::ToolCall(_) => render_tool_call_section(app),
                TuiAIBlockSection::Thinking {
                    message_id,
                    finished_duration,
                    body,
                } => render_thinking_section(
                    &self.thinking_states,
                    message_id,
                    *finished_duration,
                    body,
                    app,
                ),
            };

            // One row of bottom padding gives uniform spacing between sections
            // and after the last one.
            column.add_child(TuiContainer::new(element).with_padding_bottom(1).finish());
        }
        column.finish()
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
