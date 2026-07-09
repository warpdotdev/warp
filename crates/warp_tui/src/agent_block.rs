//! An agent block in the TUI transcript: one exchange rendered as the user's
//! submitted input followed by the agent's response.
//!
//! This module owns section extraction ([`TuiAIBlock::sections`]) and
//! composition ([`TuiAIBlock::render_element`]); the per-section render
//! functions live in [`crate::agent_block_sections`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use itertools::Itertools;
use parking_lot::FairMutex;
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    AIAgentExchangeId, AIAgentOutputMessageType, AIAgentTextSection, AIBlockModel,
    AIConversationId, BlockId, BlocklistAIActionEvent, BlocklistAIActionModel, MessageId,
    ModelEvent, ModelEventDispatcher, RequestCommandOutputResult, TerminalModel,
};
use warpui_core::elements::tui::{
    TuiChildView, TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiLayoutContext,
    TuiParentElement, TuiSize,
};
use warpui_core::elements::MouseStateHandle;
use warpui_core::{AppContext, Entity, EntityId, ModelHandle, TuiView, ViewContext, ViewHandle};

use super::tui_file_edits_view::TuiFileEditsView;
use crate::agent_block_sections::{
    render_fallback_tool_call_section, render_input_section, render_plain_text_section,
    render_thinking_section,
};
use crate::tool_call_labels::{CommandBlockState, ResolvedCommandBlock};
use crate::transcript_view::BLOCK_TOP_PADDING_ROWS;

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

/// A registered per-action child view for a stateful tool call.
///
/// Stateless tool calls render as pure elements in
/// [`TuiAIBlockSection::render_element`]; a tool type gets a variant here only
/// when it needs owned state or interactivity.
enum TuiToolCallView {
    FileEdits(ViewHandle<TuiFileEditsView>),
}

impl TuiToolCallView {
    /// The registered view's entity id, for [`TuiView::child_view_ids`].
    fn view_id(&self) -> EntityId {
        match self {
            Self::FileEdits(view) => view.id(),
        }
    }

    /// Renders the registered child view into the block's element tree.
    fn render_child(&self) -> TuiChildView {
        match self {
            Self::FileEdits(view) => TuiChildView::new(view),
        }
    }
}

/// Events emitted by an agent block to its transcript owner.
pub(super) enum TuiAIBlockEvent {
    LayoutInvalidated,
}

/// A thin TUI rich-content view adapter backed by one agent exchange.
///
/// The rendering logic is mostly section extraction, but the shared block list
/// stores rich content by view id, so this remains a registered view.
pub(super) struct TuiAIBlock {
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
    block_model: Rc<dyn AIBlockModel<View = Self>>,
    /// Source of truth for per-action execution status, consulted at render
    /// time to pick each tool-call row's text and styling.
    action_model: ModelHandle<BlocklistAIActionModel>,
    /// The owning surface's terminal model, used to read a command block's
    /// ground-truth state for agent-monitored commands (see
    /// [`Self::lrc_command_state`]). Locked only in short, render-time scopes.
    terminal_model: Arc<FairMutex<TerminalModel>>,
    /// Per-message UI state for this exchange's thinking blocks.
    thinking_states: ThinkingBlockStates,
    /// Every tool-call action id seen in this exchange's output, maintained by
    /// [`Self::sync_action_views`]. Mirrors the GUI `AIBlock`'s
    /// `requested_action_ids` so per-action-event lookups are a cheap set
    /// membership check instead of an output-message scan.
    action_ids: HashSet<AIAgentActionId>,
    /// Stateful per-action child views, keyed by tool-call action id.
    /// Populated by [`Self::sync_action_views`]; stateless tool calls never
    /// get entries here.
    action_views: HashMap<AIAgentActionId, TuiToolCallView>,
}

/// Extracts model state into renderable agent block sections.
impl TuiAIBlock {
    /// Creates an exchange-backed agent block. Like the GUI `AIBlock`, the
    /// block wires itself to its model at construction: it syncs per-action
    /// child views for tool calls already present, then re-syncs whenever the
    /// exchange's output updates (via `on_updated_output`).
    pub(super) fn new(
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
        block_model: Rc<dyn AIBlockModel<View = Self>>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        model_events: &ModelHandle<ModelEventDispatcher>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let mut block = Self {
            conversation_id,
            exchange_id,
            block_model,
            action_model: action_model.clone(),
            terminal_model,
            thinking_states: Default::default(),
            action_ids: HashSet::new(),
            action_views: HashMap::new(),
        };
        block.sync_action_views(&action_model, ctx);

        ctx.subscribe_to_model(
            &action_model,
            |me, _, event: &BlocklistAIActionEvent, ctx| {
                if me.renders_action(event.action_id()) {
                    me.invalidate_layout(ctx);
                }
            },
        );

        ctx.subscribe_to_model(model_events, |me, _, event, ctx| {
            let block_id = match event {
                ModelEvent::AfterBlockStarted { block_id, .. } => block_id,
                ModelEvent::BlockCompleted(completed) => &completed.block_id,
                _ => return,
            };
            if me
                .requested_command_action_id(block_id)
                .is_some_and(|action_id| me.renders_action(&action_id))
            {
                me.invalidate_layout(ctx);
            }
        });

        block.block_model.on_updated_output(
            Box::new(move |me, ctx| {
                me.sync_action_views(&action_model, ctx);
                ctx.notify();
            }),
            ctx,
        );
        block
    }

    /// Records the exchange's tool-call action ids and creates child views
    /// for stateful tool calls that don't have one yet. Rendering can't
    /// create views since it only sees `&AppContext`.
    fn sync_action_views(
        &mut self,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        let status = self.block_model.status(ctx);
        let mut file_edit_action_ids = Vec::new();
        if let Some(output) = status.output_to_render() {
            for message in &output.get().messages {
                let AIAgentOutputMessageType::Action(action) = &message.message else {
                    continue;
                };
                self.action_ids.insert(action.id.clone());
                if matches!(action.action, AIAgentActionType::RequestFileEdits { .. }) {
                    file_edit_action_ids.push(action.id.clone());
                }
            }
        }

        for action_id in file_edit_action_ids {
            if self.action_views.contains_key(&action_id) {
                continue;
            }
            let view =
                ctx.add_tui_view(|ctx| TuiFileEditsView::new(action_id.clone(), action_model, ctx));
            self.action_views
                .insert(action_id, TuiToolCallView::FileEdits(view));
            ctx.notify();
        }
    }

    /// Replaces the backing block model when the same exchange is reassigned.
    pub(super) fn replace_model(
        &mut self,
        conversation_id: AIConversationId,
        block_model: Rc<dyn AIBlockModel<View = Self>>,
    ) {
        self.conversation_id = conversation_id;
        self.block_model = block_model;
    }

    /// Returns the conversation that currently owns this agent block.
    pub(super) fn conversation_id(&self) -> AIConversationId {
        self.conversation_id
    }

    /// Returns the exchange rendered by this agent block.
    pub(super) fn exchange_id(&self) -> AIAgentExchangeId {
        self.exchange_id
    }

    /// Returns whether this block's output contains the tool call with the
    /// given action id. A set lookup over ids recorded by
    /// [`Self::sync_action_views`], so per-action-event checks stay cheap.
    fn renders_action(&self, action_id: &AIAgentActionId) -> bool {
        self.action_ids.contains(action_id)
    }

    /// Requests height remeasurement and redraws this block.
    fn invalidate_layout(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(TuiAIBlockEvent::LayoutInvalidated);
        ctx.notify();
    }

    /// Returns the requested-command action associated with a terminal block.
    fn requested_command_action_id(&self, block_id: &BlockId) -> Option<AIAgentActionId> {
        self.terminal_model
            .lock()
            .block_list()
            .block_with_id(block_id)
            .and_then(|block| block.requested_command_action_id().cloned())
    }

    /// Resolves the terminal block backing a shell-command tool call into
    /// its ground-truth state and executed command. When a block exists it
    /// supersedes the stored action status/result for execution states
    /// (mirroring the GUI's `RequestedCommandView`, which derives the row's
    /// icon and expandability from the block whenever one exists); the
    /// stored result only covers rows without a local block (e.g. viewers,
    /// restored sessions).
    fn resolve_command_block(
        &self,
        action: &AIAgentAction,
        status: Option<&AIActionStatus>,
    ) -> Option<ResolvedCommandBlock> {
        if !action.action.is_request_command_output() {
            return None;
        }
        // Long-running snapshot results carry the block id directly; used as
        // a fallback when the block can't be found by agent-interaction
        // metadata.
        let snapshot_block_id = match status
            .and_then(AIActionStatus::finished_result)
            .map(|result| &result.result)
        {
            Some(AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::LongRunningCommandSnapshot { block_id, .. },
            )) => Some(block_id),
            _ => None,
        };
        // Short-lived lock: the TUI layout/render pipeline drops its own model
        // guards before rich content measures or renders, so this never nests.
        let model = self.terminal_model.lock();
        let block_list = model.block_list();
        let block = block_list
            .block_for_ai_action_id(&action.id)
            .or_else(|| snapshot_block_id.and_then(|id| block_list.block_with_id(id)))?;
        // The block's command is the one actually executed (the streamed
        // command can be edited before acceptance), so surface it for display.
        let command = block
            .command_with_secrets_obfuscated(false)
            .trim()
            .to_owned();
        let state = if block.finished() {
            CommandBlockState::Finished {
                exit_code: block.exit_code(),
            }
        } else {
            CommandBlockState::Running
        };
        Some(ResolvedCommandBlock {
            command: (!command.is_empty()).then_some(command),
            state,
        })
    }

    /// Returns this block's wrapped height using the live layout context.
    pub(super) fn desired_height(
        &self,
        width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> usize {
        let mut element = self.render_element(app);
        usize::from(
            element
                .layout(
                    TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
                    ctx,
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
            .block_model
            .inputs_to_render(app)
            .iter()
            .filter_map(|input| input.display_query())
            .join("\n");
        if !input.is_empty() {
            sections.push(TuiAIBlockSection::Input(input));
        }

        // Walk output messages in order so tool-call rows interleave with text.
        if let Some(output) = self.block_model.status(app).output_to_render() {
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
        let output_streaming = self.block_model.status(app).is_streaming();
        let mut column = TuiFlex::column();
        let sections = self.sections(app);
        let last_index = sections.len().saturating_sub(1);
        for (index, section) in sections.iter().enumerate() {
            let element = match section {
                TuiAIBlockSection::Input(text) => render_input_section(text, app),
                TuiAIBlockSection::PlainText(text) => render_plain_text_section(text, app),
                // Stateful tool calls render their registered child view; every
                // other tool call stays a pure render fn.
                TuiAIBlockSection::ToolCall(action) => match self.action_views.get(&action.id) {
                    Some(view) => TuiContainer::new(Box::new(view.render_child())).finish(),
                    None => {
                        let status = self.action_model.as_ref(app).get_action_status(&action.id);
                        let block = self.resolve_command_block(action, status.as_ref());
                        render_fallback_tool_call_section(
                            action,
                            status.as_ref(),
                            output_streaming,
                            block.as_ref(),
                            app,
                        )
                    }
                },
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

            // One row of bottom padding separates sections; the last section
            // ends flush so blocks don't stack trailing and leading spacing.
            if index < last_index {
                column.add_child(TuiContainer::new(element).with_padding_bottom(1).finish());
            } else {
                column.add_child(element);
            }
        }
        // Blocks space themselves with blank rows on top — the same
        // `BLOCK_TOP_PADDING_ROWS` baked into terminal block heights — so
        // every adjacent block pair (terminal or agent) is separated by
        // exactly that many rows.
        TuiContainer::new(column.finish())
            .with_padding_top(BLOCK_TOP_PADDING_ROWS)
            .finish()
    }
}

/// Registers the view with the TUI runtime.
impl Entity for TuiAIBlock {
    type Event = TuiAIBlockEvent;
}

/// Renders the model-backed block as a TUI element.
impl TuiView for TuiAIBlock {
    fn ui_name() -> &'static str {
        "TuiAIBlock"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        self.action_views
            .values()
            .map(|view| view.view_id())
            .collect()
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        self.render_element(app)
    }
}

#[cfg(test)]
#[path = "agent_block_tests.rs"]
mod tests;
