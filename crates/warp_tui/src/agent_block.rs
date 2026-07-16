//! An agent block in the TUI transcript: one exchange rendered as the user's
//! submitted input followed by the agent's response.
//!
//! This module owns section extraction ([`TuiAIBlock::sections`]) and
//! composition ([`TuiAIBlock::render_element`]); the per-section render
//! functions live in [`crate::agent_block_sections`].

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use itertools::Itertools;
use markdown_parser::{FormattedTable, FormattedText};
use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIAgentExchangeId, AIAgentOutputMessageType,
    AIAgentText, AIAgentTextSection, AIAgentTodo, AIBlockModel, AIConversationId, BlockId,
    BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIHistoryModel, MessageId, ModelEvent,
    ModelEventDispatcher, SummarizationType, TerminalModel, TodoOperation, TodoStatus,
};
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiBufferExt, TuiChildView, TuiConstraint, TuiContainer, TuiElement,
    TuiFlex, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiParentElement, TuiRect,
    TuiScreenPosition, TuiSelectionSpan, TuiSize, TuiText,
};
use warpui_core::elements::MouseStateHandle;
use warpui_core::{
    AppContext, Entity, EntityId, EntityIdMap, ModelHandle, TuiView, TypedActionView, ViewContext,
    ViewHandle,
};

use super::tui_file_edits_view::{TuiFileEditsView, TuiFileEditsViewEvent};
use super::tui_shell_command_view::{TuiShellCommandView, TuiShellCommandViewEvent};
use crate::agent_block_sections::{
    render_completed_todos_section, render_fallback_tool_call_section, render_input_section,
    render_summarization_section, render_thinking_section, render_todo_list_section,
};
use crate::transcript_view::BLOCK_TOP_PADDING_ROWS;
use crate::tui_builder::TuiUiBuilder;
use crate::tui_cli_subagent_view::TuiCLISubagentView;
use crate::tui_code_block_view::{TuiCodeBlockPayload, TuiCodeBlockView, TuiCodeBlockViewEvent};
use crate::tui_markdown::{
    render_formatted_table, render_formatted_text, TuiMarkdownBlockHooks, TuiMarkdownPalette,
};

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct TuiCodeBlockKey {
    message_id: MessageId,
    section_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TuiRichTextSection {
    Markdown(Arc<FormattedText>),
    PlainText(String),
    Code(TuiCodeBlockKey),
    Table {
        structured: Option<FormattedTable>,
        fallback: String,
    },
    Image {
        alt_text: String,
        source: String,
    },
}

/// Renderable pieces of an agent block; this will grow as we render richer sections.
#[derive(Clone, Debug, Eq, PartialEq)]
enum TuiAIBlockSection {
    Input(String),
    RichText(TuiRichTextSection),
    /// An agent tool call, rendered by a registered rich child view when one
    /// exists and by the fallback status row otherwise.
    ToolCall(Box<AIAgentAction>),
    /// A reasoning ("thinking") segment, rendered as a collapsible block.
    Thinking {
        message_id: MessageId,
        finished_duration: Option<Duration>,
        body: Vec<TuiRichTextSection>,
    },
    Summarization {
        message_id: MessageId,
        finished: bool,
        body: Vec<TuiRichTextSection>,
    },
    /// The agent's task list (todo list), rendered as a collapsible block.
    TodoList {
        message_id: MessageId,
        todos: Vec<AIAgentTodo>,
    },
    /// A compact completion row for todos the agent just marked done.
    CompletedTodos {
        completed: Vec<AIAgentTodo>,
    },
}

/// Per-message UI state for collapsible sections (thinking blocks,
/// conversation summaries, and task lists), keyed by the owning output
/// message.
#[derive(Default)]
pub(crate) struct CollapsibleSectionStates {
    states: RefCell<HashMap<MessageId, CollapsibleSectionState>>,
}

/// UI state for a single collapsible section.
#[derive(Default)]
struct CollapsibleSectionState {
    /// Manual collapse override. `None` means the section's default (supplied
    /// per render by the caller: thinking blocks default to collapsed once
    /// finished, task lists default to expanded) — a recorded override wins
    /// permanently.
    collapse_override: Option<bool>,
    /// Hover state for the section header. Owned here (not created inline
    /// during render) so it survives element-tree rebuilds, following the
    /// GUI's `MouseStateHandle` pattern.
    hover_state: MouseStateHandle,
}

impl CollapsibleSectionStates {
    /// Whether the section for `message_id` is collapsed: the manual override
    /// if one was recorded, else `default_collapsed`.
    pub(crate) fn is_collapsed(&self, message_id: &MessageId, default_collapsed: bool) -> bool {
        self.states
            .borrow()
            .get(message_id)
            .and_then(|state| state.collapse_override)
            .unwrap_or(default_collapsed)
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
    ShellCommand(ViewHandle<TuiShellCommandView>),
}

impl TuiToolCallView {
    /// The registered view's entity id, for [`TuiView::child_view_ids`].
    fn view_id(&self) -> EntityId {
        match self {
            Self::FileEdits(view) => view.id(),
            Self::ShellCommand(view) => view.id(),
        }
    }

    /// Renders the registered child view into the block's element tree.
    fn render_child(&self) -> TuiChildView {
        match self {
            Self::FileEdits(view) => TuiChildView::new(view),
            Self::ShellCommand(view) => TuiChildView::new(view),
        }
    }
}

/// Events emitted to the transcript that owns this rich-content block.
pub(super) enum TuiAIBlockEvent {
    /// The block's cached canonical height must be remeasured.
    LayoutInvalidated,
}

/// User interactions handled by the owning agent block.
#[derive(Clone, Debug)]
pub(crate) enum TuiAIBlockAction {
    SetSectionCollapsed {
        message_id: MessageId,
        collapsed: bool,
    },
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
    /// Per-message UI state for this exchange's collapsible sections
    /// (thinking blocks and task lists).
    collapsible_states: CollapsibleSectionStates,
    /// Every tool-call action id seen in this exchange's output, maintained by
    /// [`Self::sync_action_views`]. Mirrors the GUI `AIBlock`'s
    /// `requested_action_ids` so per-action-event lookups are a cheap set
    /// membership check instead of an output-message scan.
    action_ids: HashSet<AIAgentActionId>,
    /// Stateful per-action child views, keyed by tool-call action id.
    /// Populated by [`Self::sync_action_views`]; stateless tool calls never
    /// get entries here.
    action_views: HashMap<AIAgentActionId, TuiToolCallView>,
    /// Persistent editor-backed children for code and Mermaid sections.
    code_block_views: HashMap<TuiCodeBlockKey, ViewHandle<TuiCodeBlockView>>,
    /// Whether the exchange's output contains any todo-operation message,
    /// maintained by [`Self::sync_action_views`]. Lets the transcript scope
    /// conversation-wide todo/status invalidations to the blocks whose
    /// rendering can actually change.
    renders_todos: bool,
    last_measured_width: Cell<Option<u16>>,
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
            collapsible_states: Default::default(),
            action_ids: HashSet::new(),
            action_views: HashMap::new(),
            code_block_views: HashMap::new(),
            renders_todos: false,
            last_measured_width: Cell::new(None),
        };
        block.sync_action_views(&action_model, ctx);
        block.sync_code_block_views(ctx);

        ctx.subscribe_to_model(
            &action_model,
            |me, _, event: &BlocklistAIActionEvent, ctx| {
                if me.renders_action(event.action_id()) {
                    me.invalidate_action(event.action_id(), ctx);
                }
            },
        );

        ctx.subscribe_to_model(model_events, |me, _, event, ctx| {
            let (block_id, should_schedule_auto_expand) = match event {
                ModelEvent::AfterBlockStarted { block_id, .. } => (block_id, true),
                ModelEvent::BlockCompleted(completed) => (&completed.block_id, false),
                _ => return,
            };
            let Some(action_id) = me.requested_command_action_id(block_id) else {
                return;
            };
            if me.renders_action(&action_id) {
                if should_schedule_auto_expand {
                    if let Some(TuiToolCallView::ShellCommand(view)) =
                        me.action_views.get(&action_id)
                    {
                        view.update(ctx, |view, ctx| view.schedule_auto_expand(ctx));
                    }
                }
                me.invalidate_action(&action_id, ctx);
            }
        });
        block.block_model.on_updated_output(
            Box::new(move |me, ctx| {
                me.sync_action_views(&action_model, ctx);
                me.sync_code_block_views(ctx);
                // The presenter caches this block's rendered element; new
                // output must invalidate both the view and its canonical
                // block-list height or scrolling keeps a stale extent after
                // the response stops streaming.
                me.invalidate_layout(ctx);
            }),
            ctx,
        );
        block
    }

    /// Records the exchange's tool-call action ids and todo presence, and
    /// creates child views for stateful tool calls that don't have one yet.
    /// Rendering can't create views since it only sees `&AppContext`.
    fn sync_action_views(
        &mut self,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        let status = self.block_model.status(ctx);
        let output_streaming = status.is_streaming();
        let mut file_edit_action_ids = Vec::new();
        let mut shell_command_actions = Vec::new();
        if let Some(output) = status.output_to_render() {
            for message in &output.get().messages {
                if matches!(&message.message, AIAgentOutputMessageType::TodoOperation(_)) {
                    self.renders_todos = true;
                    continue;
                }
                let AIAgentOutputMessageType::Action(action) = &message.message else {
                    continue;
                };
                self.action_ids.insert(action.id.clone());
                if matches!(&action.action, AIAgentActionType::RequestFileEdits { .. }) {
                    file_edit_action_ids.push(action.id.clone());
                } else if matches!(
                    &action.action,
                    AIAgentActionType::RequestCommandOutput { .. }
                ) {
                    shell_command_actions.push(action.clone());
                }
            }
        }

        for action_id in file_edit_action_ids {
            if self.action_views.contains_key(&action_id) {
                continue;
            }
            let view_action_id = action_id.clone();
            let view = ctx.add_typed_action_tui_view(move |ctx| {
                TuiFileEditsView::new(view_action_id, action_model, ctx)
            });
            ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
                TuiFileEditsViewEvent::LayoutChanged => me.invalidate_layout(ctx),
            });
            self.action_views
                .insert(action_id, TuiToolCallView::FileEdits(view));
            ctx.notify();
        }

        for action in shell_command_actions {
            if let Some(TuiToolCallView::ShellCommand(view)) = self.action_views.get(&action.id) {
                view.update(ctx, |view, ctx| {
                    view.update_action(action, output_streaming);
                    ctx.notify();
                });
                continue;
            }
            let action_id = action.id.clone();
            let action_model = action_model.clone();
            let terminal_model = self.terminal_model.clone();
            let view = ctx.add_typed_action_tui_view(|_| {
                TuiShellCommandView::new(action, output_streaming, action_model, terminal_model)
            });
            ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
                TuiShellCommandViewEvent::LayoutChanged => me.invalidate_layout(ctx),
            });
            self.action_views
                .insert(action_id, TuiToolCallView::ShellCommand(view));
            ctx.notify();
        }
    }

    /// Reconciles persistent code children from the latest rendered output.
    /// Keys remain stable while a message's section position survives; a
    /// streaming boundary change naturally drops stale children and creates
    /// the newly semantic section.
    fn sync_code_block_views(&mut self, ctx: &mut ViewContext<Self>) {
        let mut descriptors = Vec::new();
        if let Some(output) = self.block_model.status(ctx).output_to_render() {
            for message in &output.get().messages {
                let text = match &message.message {
                    AIAgentOutputMessageType::Text(text)
                    | AIAgentOutputMessageType::Reasoning { text, .. } => Some(text),
                    AIAgentOutputMessageType::Summarization {
                        text,
                        summarization_type: SummarizationType::ConversationSummary,
                        ..
                    } => Some(text),
                    AIAgentOutputMessageType::Action(_)
                    | AIAgentOutputMessageType::TodoOperation(_)
                    | AIAgentOutputMessageType::Subagent(_)
                    | AIAgentOutputMessageType::Summarization { .. }
                    | AIAgentOutputMessageType::WebSearch(_)
                    | AIAgentOutputMessageType::WebFetch(_)
                    | AIAgentOutputMessageType::CommentsAddressed { .. }
                    | AIAgentOutputMessageType::DebugOutput { .. }
                    | AIAgentOutputMessageType::ArtifactCreated(_)
                    | AIAgentOutputMessageType::SkillInvoked(_)
                    | AIAgentOutputMessageType::MessagesReceivedFromAgents { .. }
                    | AIAgentOutputMessageType::EventsFromAgents { .. } => None,
                };
                let Some(text) = text else {
                    continue;
                };
                for (section_index, section) in text.sections.iter().enumerate() {
                    let payload = match section {
                        AIAgentTextSection::Code { code, language, .. } => {
                            Some(TuiCodeBlockPayload::new(
                                code.clone(),
                                language.as_ref().map(|language| language.display_name()),
                            ))
                        }
                        AIAgentTextSection::MermaidDiagram { diagram } => {
                            Some(TuiCodeBlockPayload::new(
                                diagram.source.clone(),
                                Some("mermaid".to_owned()),
                            ))
                        }
                        AIAgentTextSection::PlainText { .. }
                        | AIAgentTextSection::Table { .. }
                        | AIAgentTextSection::Image { .. } => None,
                    };
                    if let Some(payload) = payload {
                        descriptors.push((
                            TuiCodeBlockKey {
                                message_id: message.id.clone(),
                                section_index,
                            },
                            payload,
                        ));
                    }
                }
            }
        }

        let active_keys = descriptors
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<HashSet<_>>();
        self.code_block_views
            .retain(|key, _| active_keys.contains(key));

        for (key, payload) in descriptors {
            if let Some(view) = self.code_block_views.get(&key) {
                view.update(ctx, |view, ctx| {
                    view.sync(payload, ctx);
                });
                continue;
            }
            let view = ctx.add_tui_view(move |ctx| TuiCodeBlockView::new(payload, ctx));
            ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
                TuiCodeBlockViewEvent::LayoutChanged | TuiCodeBlockViewEvent::SyntaxUpdated => {
                    me.invalidate_layout(ctx)
                }
            });
            self.code_block_views.insert(key, view);
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

    /// Returns whether this block's output contains any todo-operation
    /// message (a task list or a completion row) — the only content whose
    /// styling depends on conversation-wide todo and status state.
    pub(super) fn renders_todos(&self) -> bool {
        self.renders_todos
    }
    pub(super) fn set_cli_subagent_view(
        &mut self,
        action_id: &AIAgentActionId,
        cli_subagent_view: Option<ViewHandle<TuiCLISubagentView>>,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(TuiToolCallView::ShellCommand(view)) = self.action_views.get(action_id) else {
            return false;
        };
        view.update(ctx, |view, ctx| {
            view.set_cli_subagent_view(cli_subagent_view, ctx);
        });
        self.invalidate_layout(ctx);
        true
    }

    /// Invalidates this block and its stateful command child after an owned
    /// action status or backing terminal block changes.
    fn invalidate_action(&mut self, action_id: &AIAgentActionId, ctx: &mut ViewContext<Self>) {
        if let Some(TuiToolCallView::ShellCommand(view)) = self.action_views.get(action_id) {
            view.update(ctx, |_, ctx| ctx.notify());
        }
        self.invalidate_layout(ctx);
    }

    /// Requests canonical height remeasurement and redraws this block.
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

    /// Whether the cached height is stale at `width`.
    pub(super) fn needs_height_measurement(&self, width: u16, app: &AppContext) -> bool {
        self.last_measured_width.get() != Some(width)
            || self.block_model.status(app).is_streaming()
            || self.action_views.values().any(|view| match view {
                TuiToolCallView::FileEdits(_) => false,
                TuiToolCallView::ShellCommand(view) => {
                    view.as_ref(app).needs_continuous_height_measurement()
                }
            })
    }

    /// Records the width used for the latest height measurement.
    pub(super) fn record_height_measurement(&self, width: u16) {
        self.last_measured_width.set(Some(width));
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

    /// Logical (unwrapped) text for a selection over this block's text
    /// sections — the user's query and the agent's textual responses.
    ///
    /// Copy would otherwise reconstruct the text from the rendered cell grid,
    /// inserting a newline at every soft-wrap boundary, capturing wrap/quote
    /// indentation, and dropping rows beyond what was rendered. Sourcing from
    /// the model returns the text exactly as authored. Each section's row span
    /// at `width` is derived from the same composition `render_element` uses
    /// (one blank `BLOCK_TOP_PADDING_ROWS` on top, one padding row between
    /// sections), so the selection can be mapped back to whole sections.
    ///
    /// Returns `None` — so the caller falls back to per-row grid text — when the
    /// selection only partially covers a section, covers a section with no clean
    /// logical form (a tool call, reasoning, summary, or todo list), or the
    /// block contains a child-view tool call whose height can't be measured
    /// here. That keeps partial selections and non-text content on the existing
    /// path (the diagram-style fallback).
    pub(super) fn selection_logical_text(
        &self,
        selection: TuiSelectionSpan,
        block_top: usize,
        width: u16,
        app: &AppContext,
    ) -> Option<String> {
        if selection.start.row < block_top {
            return None;
        }
        let output_streaming = self.block_model.status(app).is_streaming();
        let sections = self.sections(app);
        if sections.is_empty() {
            return None;
        }
        let last_index = sections.len().saturating_sub(1);
        let end_row_exclusive = if selection.end.col == 0 {
            selection.end.row
        } else {
            selection.end.row.saturating_add(1)
        };

        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let mut section_top = block_top.saturating_add(usize::from(BLOCK_TOP_PADDING_ROWS));
        let mut collected = Vec::new();
        let mut overlapped_any = false;
        for (index, section) in sections.iter().enumerate() {
            let mut element = self.measurable_section_element(section, output_streaming, app)?;
            let height = usize::from(
                element
                    .layout(
                        TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
                        &mut ctx,
                        app,
                    )
                    .height,
            );
            let start = section_top;
            let end = section_top.saturating_add(height);
            // One padding row separates sections; the last section ends flush.
            section_top = if index < last_index {
                end.saturating_add(1)
            } else {
                end
            };
            if height == 0 {
                continue;
            }
            let overlaps = start < end_row_exclusive && end > selection.start.row;
            if !overlaps {
                continue;
            }
            overlapped_any = true;
            // The section must be covered from its first column through its last
            // rendered glyph; any partial-column or partial-row overlap falls
            // back. Otherwise a selection ending mid-way through the final
            // wrapped row would still return the whole logical section and copy
            // unselected trailing text.
            let covers_start = selection.start.row < start
                || (selection.start.row == start && selection.start.col == 0);
            let last_row = end.saturating_sub(1);
            let covers_end = selection.end.row >= end
                || (selection.end.row == last_row
                    && usize::from(selection.end.col)
                        >= last_row_content_width(&mut element, width, height));
            if !covers_start || !covers_end {
                return None;
            }
            collected.push(section_logical_text(section)?);
        }
        overlapped_any.then(|| collected.join("\n"))
    }

    /// Rebuilds a section's element for standalone height measurement, mirroring
    /// `render_element`'s per-section construction. Returns `None` for a tool
    /// call backed by a registered child view, whose height can't be measured
    /// without the presenter's `rendered_views`.
    fn measurable_section_element(
        &self,
        section: &TuiAIBlockSection,
        output_streaming: bool,
        app: &AppContext,
    ) -> Option<Box<dyn TuiElement>> {
        Some(match section {
            TuiAIBlockSection::Input(text) => render_input_section(text, app),
            TuiAIBlockSection::RichText(section) => {
                if matches!(section, TuiRichTextSection::Code(_)) {
                    return None;
                }
                self.render_rich_text_section(section, false, app)
            }
            TuiAIBlockSection::ToolCall(action) => {
                if self.action_views.contains_key(&action.id) {
                    return None;
                }
                let status = self.action_model.as_ref(app).get_action_status(&action.id);
                render_fallback_tool_call_section(
                    action,
                    status.as_ref(),
                    output_streaming,
                    None,
                    app,
                )
            }
            TuiAIBlockSection::Thinking {
                message_id,
                finished_duration,
                body,
            } => render_thinking_section(
                &self.collapsible_states,
                message_id,
                *finished_duration,
                self.render_rich_text_sections(body, true, app),
                app,
            ),
            TuiAIBlockSection::Summarization {
                message_id,
                finished,
                body,
            } => render_summarization_section(
                &self.collapsible_states,
                message_id,
                *finished,
                self.render_rich_text_sections(body, false, app),
                app,
            ),
            TuiAIBlockSection::TodoList { message_id, todos } => {
                let history = BlocklistAIHistoryModel::as_ref(app);
                let rows: Vec<(String, TodoStatus)> = todos
                    .iter()
                    .map(|todo| {
                        (
                            todo.title.clone(),
                            history
                                .todo_status(&self.conversation_id, &todo.id)
                                .unwrap_or(TodoStatus::Cancelled),
                        )
                    })
                    .collect();
                render_todo_list_section(&self.collapsible_states, message_id, &rows, app)
            }
            TuiAIBlockSection::CompletedTodos { completed } => {
                let history = BlocklistAIHistoryModel::as_ref(app);
                render_completed_todos_section(
                    completed,
                    history.active_todo_list(&self.conversation_id),
                    app,
                )
            }
        })
    }
    fn rich_text_sections(message_id: &MessageId, text: &AIAgentText) -> Vec<TuiRichTextSection> {
        text.sections
            .iter()
            .enumerate()
            .filter(|(_, section)| !section.is_empty())
            .map(|(section_index, section)| match section {
                AIAgentTextSection::PlainText { text } => text
                    .formatted_text_arc()
                    .map(TuiRichTextSection::Markdown)
                    .unwrap_or_else(|| TuiRichTextSection::PlainText(text.text().to_owned())),
                AIAgentTextSection::Code { .. } | AIAgentTextSection::MermaidDiagram { .. } => {
                    TuiRichTextSection::Code(TuiCodeBlockKey {
                        message_id: message_id.clone(),
                        section_index,
                    })
                }
                AIAgentTextSection::Table { table } => TuiRichTextSection::Table {
                    structured: table.structured_table().cloned(),
                    fallback: table.rendered_lines().join("\n"),
                },
                AIAgentTextSection::Image { image } => TuiRichTextSection::Image {
                    alt_text: image.alt_text.clone(),
                    source: image.source.clone(),
                },
            })
            .collect()
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
                            Self::rich_text_sections(&message.id, text)
                                .into_iter()
                                .map(TuiAIBlockSection::RichText),
                        );
                    }
                    AIAgentOutputMessageType::Action(action) => {
                        sections.push(TuiAIBlockSection::ToolCall(Box::new(action.clone())));
                    }
                    AIAgentOutputMessageType::Reasoning {
                        text,
                        finished_duration,
                    } => {
                        let body = Self::rich_text_sections(&message.id, text);
                        // Some providers intentionally emit duration/signature-only reasoning
                        // records for conversation continuity when no user-visible summary exists;
                        // omit them because they have no content to render.
                        if !body.is_empty() {
                            sections.push(TuiAIBlockSection::Thinking {
                                message_id: message.id.clone(),
                                finished_duration: *finished_duration,
                                body,
                            });
                        }
                    }
                    AIAgentOutputMessageType::Summarization {
                        text,
                        finished_duration,
                        summarization_type: SummarizationType::ConversationSummary,
                        ..
                    } => {
                        let body = Self::rich_text_sections(&message.id, text);
                        if !body.is_empty() {
                            sections.push(TuiAIBlockSection::Summarization {
                                message_id: message.id.clone(),
                                finished: finished_duration.is_some(),
                                body,
                            });
                        }
                    }
                    AIAgentOutputMessageType::TodoOperation(operation) => match operation {
                        TodoOperation::UpdateTodos { todos } if !todos.is_empty() => {
                            sections.push(TuiAIBlockSection::TodoList {
                                message_id: message.id.clone(),
                                todos: todos.clone(),
                            });
                        }
                        TodoOperation::MarkAsCompleted { completed_todos }
                            if !completed_todos.is_empty() =>
                        {
                            sections.push(TuiAIBlockSection::CompletedTodos {
                                completed: completed_todos.clone(),
                            });
                        }
                        // Empty operations carry nothing to render (matching
                        // the GUI's guards).
                        TodoOperation::UpdateTodos { .. }
                        | TodoOperation::MarkAsCompleted { .. } => {}
                    },
                    // Other message kinds are not rendered by the TUI transcript yet.
                    AIAgentOutputMessageType::Summarization { .. }
                    | AIAgentOutputMessageType::Subagent(_)
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

    fn markdown_palette(app: &AppContext, muted: bool) -> TuiMarkdownPalette {
        let builder = TuiUiBuilder::from_app(app);
        let mut palette = TuiMarkdownPalette::from_builder(&builder);
        if muted {
            let style = builder.muted_text_style();
            palette.body = style;
            palette.muted = style;
            palette.heading = style.add_modifier(Modifier::BOLD);
            palette.marker = style;
            palette.link = style.add_modifier(Modifier::UNDERLINED);
            palette.inline_code = style;
            palette.rule = style;
            palette.code = style;
            palette.table_header = style.add_modifier(Modifier::BOLD);
            palette.fallback = style.add_modifier(Modifier::ITALIC);
        }
        palette
    }

    fn render_rich_text_section(
        &self,
        section: &TuiRichTextSection,
        muted: bool,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        let palette = Self::markdown_palette(app, muted);
        match section {
            TuiRichTextSection::Markdown(formatted) => {
                render_formatted_text(formatted, palette, &TuiMarkdownBlockHooks::default())
            }
            TuiRichTextSection::PlainText(text) => {
                TuiText::new(text.clone()).with_style(palette.body).finish()
            }
            TuiRichTextSection::Code(key) => self
                .code_block_views
                .get(key)
                .map(|view| TuiChildView::new(view).finish())
                .unwrap_or_else(|| {
                    TuiText::new("[Code block unavailable]")
                        .with_style(palette.fallback)
                        .finish()
                }),
            TuiRichTextSection::Table {
                structured: Some(table),
                ..
            } => render_formatted_table(table, palette),
            TuiRichTextSection::Table {
                structured: None,
                fallback,
            } => TuiText::new(fallback.clone())
                .with_style(palette.body)
                .finish(),
            TuiRichTextSection::Image { alt_text, source } => {
                let label = if alt_text.is_empty() {
                    "Image".to_owned()
                } else {
                    format!("Image: {alt_text}")
                };
                TuiText::from_spans([
                    (label, palette.fallback),
                    (format!(" ({source})"), palette.link),
                ])
                .finish()
            }
        }
    }

    fn render_rich_text_sections(
        &self,
        sections: &[TuiRichTextSection],
        muted: bool,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        let mut column = TuiFlex::column();
        for section in sections {
            column.add_child(self.render_rich_text_section(section, muted, app));
        }
        column.finish()
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
                TuiAIBlockSection::RichText(section) => {
                    self.render_rich_text_section(section, false, app)
                }
                // Stateful tool calls render their registered child view; every
                // other tool call stays a pure render fn.
                TuiAIBlockSection::ToolCall(action) => match self.action_views.get(&action.id) {
                    Some(view) => TuiContainer::new(Box::new(view.render_child())).finish(),
                    None => {
                        let status = self.action_model.as_ref(app).get_action_status(&action.id);
                        render_fallback_tool_call_section(
                            action,
                            status.as_ref(),
                            output_streaming,
                            None,
                            app,
                        )
                    }
                },
                TuiAIBlockSection::Thinking {
                    message_id,
                    finished_duration,
                    body,
                } => render_thinking_section(
                    &self.collapsible_states,
                    message_id,
                    *finished_duration,
                    self.render_rich_text_sections(body, true, app),
                    app,
                ),
                TuiAIBlockSection::Summarization {
                    message_id,
                    finished,
                    body,
                } => render_summarization_section(
                    &self.collapsible_states,
                    message_id,
                    *finished,
                    self.render_rich_text_sections(body, false, app),
                    app,
                ),
                TuiAIBlockSection::TodoList { message_id, todos } => {
                    // Statuses resolve against the conversation's todo
                    // history at render time, so superseded lists restyle
                    // without needing a dedicated invalidation. Items the
                    // conversation no longer knows belong to a superseded
                    // list (matching the GUI's fallback).
                    let history = BlocklistAIHistoryModel::as_ref(app);
                    let rows: Vec<(String, TodoStatus)> = todos
                        .iter()
                        .map(|todo| {
                            (
                                todo.title.clone(),
                                history
                                    .todo_status(&self.conversation_id, &todo.id)
                                    .unwrap_or(TodoStatus::Cancelled),
                            )
                        })
                        .collect();
                    render_todo_list_section(&self.collapsible_states, message_id, &rows, app)
                }
                TuiAIBlockSection::CompletedTodos { completed } => {
                    let history = BlocklistAIHistoryModel::as_ref(app);
                    render_completed_todos_section(
                        completed,
                        history.active_todo_list(&self.conversation_id),
                        app,
                    )
                }
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

/// The number of columns occupied by a section's final rendered row, used to
/// decide whether a selection ending on that row reaches the section's last
/// glyph (full coverage) or stops short of it (partial — fall back). Renders the
/// already-laid-out section element to a cell grid and measures the last row's
/// trimmed content; text-only sections need no registered child views.
fn last_row_content_width(element: &mut Box<dyn TuiElement>, width: u16, height: usize) -> usize {
    if height == 0 {
        return 0;
    }
    let buffer_height = u16::try_from(height).unwrap_or(u16::MAX);
    let mut rendered_views = EntityIdMap::default();
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, width, buffer_height));
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    {
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
    }
    buffer
        .to_lines()
        .get(height.saturating_sub(1))
        .map(|line| line.trim_end().chars().count())
        .unwrap_or(0)
}

/// The copy-able logical text for a section, or `None` for section kinds with no
/// clean logical form (tool calls, reasoning, summaries, todo lists), which fall
/// back to per-row grid text.
fn section_logical_text(section: &TuiAIBlockSection) -> Option<String> {
    match section {
        TuiAIBlockSection::Input(text) => Some(text.clone()),
        TuiAIBlockSection::RichText(TuiRichTextSection::Markdown(formatted)) => {
            Some(formatted.raw_text().trim_end_matches('\n').to_owned())
        }
        TuiAIBlockSection::RichText(TuiRichTextSection::PlainText(text)) => Some(text.clone()),
        TuiAIBlockSection::RichText(
            TuiRichTextSection::Code(_)
            | TuiRichTextSection::Table { .. }
            | TuiRichTextSection::Image { .. },
        ) => None,
        TuiAIBlockSection::ToolCall(_)
        | TuiAIBlockSection::Thinking { .. }
        | TuiAIBlockSection::Summarization { .. }
        | TuiAIBlockSection::TodoList { .. }
        | TuiAIBlockSection::CompletedTodos { .. } => None,
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
            .chain(self.code_block_views.values().map(|view| view.id()))
            .collect()
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        self.render_element(app)
    }
}

impl TypedActionView for TuiAIBlock {
    type Action = TuiAIBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiAIBlockAction::SetSectionCollapsed {
                message_id,
                collapsed,
            } => {
                self.collapsible_states
                    .set_collapsed(message_id.clone(), *collapsed);
                self.invalidate_layout(ctx);
            }
        }
    }
}

#[cfg(test)]
#[path = "agent_block_tests.rs"]
mod tests;
