//! An agent block in the TUI transcript: one exchange rendered as the user's
//! submitted input followed by the agent's response.
//!
//! This module owns section extraction ([`TuiAIBlock::sections`]) and
//! composition ([`TuiAIBlock::render_element`]); the per-section render
//! functions live in [`crate::agent_block_sections`].

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use chrono::TimeDelta;
use itertools::Itertools;
use markdown_parser::{FormattedTable, FormattedText};
use parking_lot::FairMutex;
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionType, AIAgentExchangeId,
    AIAgentOutputMessageType, AIAgentText, AIAgentTextSection, AIAgentTodo, AIBlockModel,
    AIBlockModelHelper, AIBlockOutputStatus, AIConversationId, AuthStateProvider, BlockId,
    BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIHistoryModel, CancellationReason,
    FAILED_OUTPUT_USAGE_NOTICE_TEXT, FailedOutputPresentation, MessageId, ModelEvent,
    ModelEventDispatcher, ReceivedMessageDisplay, RenderableAIError, SummarizationType,
    TelemetryEvent, TerminalModel, TodoOperation, TodoStatus, TuiOnboardingMarker,
    TuiOnboardingMarkers, TuiOnboardingMarkersEvent, UserWorkspaces, failed_output_presentation,
    should_show_failed_output_usage_notice,
};
use warpui::SingletonEntity;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiBufferExt, TuiChildView, TuiConstraint, TuiContainer, TuiElement,
    TuiFlex, TuiHoverable, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiParentElement,
    TuiRect, TuiScreenPosition, TuiSelectionSpan, TuiSize, TuiText,
};
use warpui_core::{
    AppContext, Entity, EntityId, EntityIdMap, ModelHandle, TuiView, TypedActionView, ViewContext,
    ViewHandle,
};

use super::tui_ask_question_view::{TuiAskQuestionView, TuiAskQuestionViewEvent};
use super::tui_file_edits_view::{TuiFileEditsView, TuiFileEditsViewEvent};
use super::tui_generic_tool_call_view::{TuiGenericToolCallView, TuiGenericToolCallViewEvent};
use super::tui_shell_command_view::{TuiShellCommandView, TuiShellCommandViewEvent};
use crate::agent_block_sections::{
    render_completed_todos_section, render_fallback_tool_call_section, render_input_section,
    render_summarization_section, render_thinking_section, render_todo_list_section,
};
use crate::agent_message::render_agent_message;
use crate::orchestration_block::{TuiOrchestrationBlock, TuiOrchestrationBlockEvent};
use crate::orchestration_model::{TuiOrchestrationEvent, TuiOrchestrationModel};
use crate::terminal_session_view::BlockingInputSource;
use crate::transcript_view::BLOCK_TOP_PADDING_ROWS;
use crate::tui_builder::TuiUiBuilder;
use crate::tui_cli_subagent_view::TuiCLISubagentView;
use crate::tui_code_block_view::{TuiCodeBlockPayload, TuiCodeBlockView, TuiCodeBlockViewEvent};
use crate::tui_markdown::{
    TuiMarkdownBlockHooks, TuiMarkdownPalette, render_formatted_table, render_formatted_text,
};
use crate::tui_plan_view::{TuiPlanView, TuiPlanViewEvent};
use crate::tui_review_comments::render_review_comments_tool_call;
const OUT_OF_CREDITS_TITLE: &str = "I’m sorry, I couldn’t complete that request.";
const OUT_OF_CREDITS_DETAIL: &str =
    "In order to use Warp’s AI features, subscribe to a Warp plan or buy packs of credits.";
const OUT_OF_CREDITS_ACTION_LABEL: &str = "Get started with AI";
const OUT_OF_CREDITS_ACTION_HINT: &str = "(ctrl+o)";
const FIRST_CREDIT_GATE_TITLE: &str = "You need AI credits in order to use Warp’s agent.";
const FIRST_CREDIT_GATE_ACTION_LABEL: &str = "Start using AI";
const FIRST_CREDIT_GATE_ACTION_HINT: &str = "(ctrl+o).";
const FAILURE_WARNING_PREFIX: &str = "⚠ ";

pub(crate) fn upgrade_url(app: &AppContext) -> String {
    let user_id = AuthStateProvider::as_ref(app).get().user_id();
    UserWorkspaces::warp_agent_cli_upgrade_link(user_id)
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct TuiCodeBlockKey {
    message_id: MessageId,
    section_index: usize,
}

fn should_consume_first_credit_gate(
    is_restored: bool,
    presentation: Option<&FailedOutputPresentation>,
) -> bool {
    !is_restored
        && matches!(
            presentation,
            Some(FailedOutputPresentation::OutOfCredits { .. })
        )
}

fn render_first_credit_gate(
    out_of_credits_hover_state: &MouseStateHandle,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let primary_style = builder.primary_text_style();
    let upgrade_url = upgrade_url(app);
    let click_url = upgrade_url.clone();
    let action = TuiHoverable::new(
        out_of_credits_hover_state.clone(),
        TuiText::new(FIRST_CREDIT_GATE_ACTION_LABEL)
            .with_style(primary_style.add_modifier(Modifier::UNDERLINED))
            .finish(),
    )
    .on_click(move |_, app| app.open_url(&click_url))
    .finish();
    TuiFlex::column()
        .child(
            TuiText::new(FIRST_CREDIT_GATE_TITLE)
                .with_style(builder.attention_glyph_style())
                .finish(),
        )
        .child(
            TuiFlex::row()
                .child(action)
                .child(TuiText::new(" ").with_style(primary_style).finish())
                .child(
                    TuiText::new(FIRST_CREDIT_GATE_ACTION_HINT)
                        .with_style(builder.accent_text_style())
                        .finish(),
                )
                .finish(),
        )
        .child(TuiText::new(" ").finish())
        .child(TuiText::new(upgrade_url).with_style(primary_style).finish())
        .finish()
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
    /// A message delivered by another agent in the orchestration.
    AgentMessage(ReceivedMessageDisplay),
    Failure(FailedOutputPresentation),
    FirstCreditGate,
    UsageNotice,
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

fn render_failure_section(
    presentation: &FailedOutputPresentation,
    out_of_credits_hover_state: &MouseStateHandle,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let error_style = builder.error_text_style();
    let body_style = builder.muted_text_style();
    match presentation {
        FailedOutputPresentation::Message(message)
        | FailedOutputPresentation::AwsBedrockCredentialsExpiredOrInvalid {
            fallback_message: message,
        }
        | FailedOutputPresentation::GeminiEnterpriseCredentialsExpiredOrInvalid {
            fallback_message: message,
        } => TuiText::from_spans([
            (FAILURE_WARNING_PREFIX.to_owned(), error_style),
            (message.clone(), body_style),
        ])
        .finish(),
        FailedOutputPresentation::InvalidApiKey { title, detail } => TuiText::from_spans([
            (FAILURE_WARNING_PREFIX.to_owned(), error_style),
            (
                (*title).to_owned(),
                error_style.add_modifier(Modifier::BOLD),
            ),
            ("\n  ".to_owned(), body_style),
            (detail.clone(), body_style),
        ])
        .finish(),
        FailedOutputPresentation::OutOfCredits { .. } => {
            let primary_style = builder.primary_text_style();
            let link_style = primary_style.add_modifier(Modifier::UNDERLINED);
            let upgrade_url = upgrade_url(app);
            let click_url = upgrade_url.clone();
            let action = TuiHoverable::new(
                out_of_credits_hover_state.clone(),
                TuiText::new(OUT_OF_CREDITS_ACTION_LABEL)
                    .with_style(link_style)
                    .finish(),
            )
            .on_click(move |_, app| app.open_url(&click_url))
            .finish();
            let actions = TuiFlex::row()
                .child(TuiText::new("  ").with_style(primary_style).finish())
                .child(action)
                .child(TuiText::new(" ").with_style(primary_style).finish())
                .child(
                    TuiText::new(OUT_OF_CREDITS_ACTION_HINT)
                        .with_style(builder.accent_text_style())
                        .finish(),
                )
                .finish();
            TuiFlex::column()
                .child(
                    TuiText::from_spans([
                        (FAILURE_WARNING_PREFIX.to_owned(), error_style),
                        (OUT_OF_CREDITS_TITLE.to_owned(), primary_style),
                    ])
                    .finish(),
                )
                .child(
                    TuiContainer::new(
                        TuiText::new(OUT_OF_CREDITS_DETAIL)
                            .with_style(primary_style)
                            .finish(),
                    )
                    .with_padding_left(2)
                    .finish(),
                )
                .child(TuiText::new(" ").finish())
                .child(actions)
                .child(TuiText::new(" ").finish())
                .child(
                    TuiText::new(format!("  {upgrade_url}"))
                        .with_style(primary_style)
                        .finish(),
                )
                .finish()
        }
        FailedOutputPresentation::ContextWindowExceeded { message } => TuiText::from_spans([
            ("× ".to_owned(), error_style),
            (message.clone(), body_style),
        ])
        .finish(),
    }
}

fn render_usage_notice(app: &AppContext) -> Box<dyn TuiElement> {
    TuiText::new(FAILED_OUTPUT_USAGE_NOTICE_TEXT)
        .with_style(TuiUiBuilder::from_app(app).muted_text_style())
        .finish()
}

fn failure_text(presentation: &FailedOutputPresentation, app: &AppContext) -> String {
    match presentation {
        FailedOutputPresentation::Message(message)
        | FailedOutputPresentation::AwsBedrockCredentialsExpiredOrInvalid {
            fallback_message: message,
        }
        | FailedOutputPresentation::GeminiEnterpriseCredentialsExpiredOrInvalid {
            fallback_message: message,
        }
        | FailedOutputPresentation::ContextWindowExceeded { message } => message.clone(),
        FailedOutputPresentation::OutOfCredits { .. } => {
            let upgrade_url = upgrade_url(app);
            format!(
                "{OUT_OF_CREDITS_TITLE}\n  {OUT_OF_CREDITS_DETAIL}\n\n  \
                 {OUT_OF_CREDITS_ACTION_LABEL} {OUT_OF_CREDITS_ACTION_HINT}\n\n  {upgrade_url}"
            )
        }
        FailedOutputPresentation::InvalidApiKey { title, detail } => {
            format!("{title}\n{detail}")
        }
    }
}

/// A registered per-action child view for a stateful tool call.
///
/// Stateless tool calls render as pure elements in
/// [`TuiAIBlockSection::render_element`]; a tool type gets a variant here only
/// when it needs owned state or interactivity.
enum TuiToolCallView {
    AskQuestion(ViewHandle<TuiAskQuestionView>),
    FileEdits(ViewHandle<TuiFileEditsView>),
    Generic(ViewHandle<TuiGenericToolCallView>),
    Plan(ViewHandle<TuiPlanView>),
    ShellCommand(ViewHandle<TuiShellCommandView>),
    OrchestrationBlock(ViewHandle<TuiOrchestrationBlock>),
}

impl TuiToolCallView {
    /// The registered view's entity id, for [`TuiView::child_view_ids`].
    fn view_id(&self) -> EntityId {
        match self {
            Self::AskQuestion(view) => view.id(),
            Self::FileEdits(view) => view.id(),
            Self::Generic(view) => view.id(),
            Self::Plan(view) => view.id(),
            Self::ShellCommand(view) => view.id(),
            Self::OrchestrationBlock(view) => view.id(),
        }
    }

    /// Renders the registered child view into the block's element tree.
    fn render_child(&self) -> TuiChildView {
        match self {
            Self::AskQuestion(view) => TuiChildView::new(view),
            Self::FileEdits(view) => TuiChildView::new(view),
            Self::Generic(view) => TuiChildView::new(view),
            Self::Plan(view) => TuiChildView::new(view),
            Self::ShellCommand(view) => TuiChildView::new(view),
            Self::OrchestrationBlock(view) => TuiChildView::new(view),
        }
    }
}

/// Events emitted to the transcript that owns this rich-content block.
pub(super) enum TuiAIBlockEvent {
    /// The block's cached canonical height must be remeasured.
    LayoutInvalidated,
    /// A blocking child's focus/blocking state may have changed; the session
    /// surface re-derives the active blocker (input replacement).
    BlockingStateChanged,
    /// Replacement guidance submitted from a tool permission request.
    ReplacementGuidanceSubmitted {
        conversation_id: AIConversationId,
        text: String,
    },
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
    out_of_credits_hover_state: MouseStateHandle,
    first_credit_gate: bool,
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
    is_restored_for_telemetry: bool,
    time_to_first_token: OnceCell<TimeDelta>,
    time_to_last_token: Option<TimeDelta>,
    terminal_telemetry_emitted: bool,
    last_measured_width: Cell<Option<u16>>,
}

/// Extracts model state into renderable agent block sections.
impl TuiAIBlock {
    /// Creates an exchange-backed agent block. Like the GUI `AIBlock`, the
    /// block wires itself to its model at construction: it syncs per-action
    /// child views for tool calls already present, then re-syncs whenever the
    /// exchange's output updates (via `on_updated_output`).
    pub(super) fn new(
        identity: (AIConversationId, AIAgentExchangeId),
        block_model: Rc<dyn AIBlockModel<View = Self>>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        model_events: &ModelHandle<ModelEventDispatcher>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        is_restored_for_telemetry: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let (conversation_id, exchange_id) = identity;
        let mut block = Self {
            conversation_id,
            exchange_id,
            block_model,
            action_model: action_model.clone(),
            terminal_model,
            collapsible_states: Default::default(),
            out_of_credits_hover_state: MouseStateHandle::default(),
            first_credit_gate: false,
            action_ids: HashSet::new(),
            action_views: HashMap::new(),
            code_block_views: HashMap::new(),
            renders_todos: false,
            is_restored_for_telemetry,
            time_to_first_token: OnceCell::new(),
            time_to_last_token: None,
            terminal_telemetry_emitted: false,
            last_measured_width: Cell::new(None),
        };
        block.sync_action_views(&action_model, ctx);
        block.sync_code_block_views(ctx);
        block.sync_first_credit_gate(ctx);

        ctx.subscribe_to_model(
            &TuiOnboardingMarkers::handle(ctx),
            |block, _, event, ctx| match event {
                TuiOnboardingMarkersEvent::Loading => {}
                TuiOnboardingMarkersEvent::Ready => {
                    block.sync_first_credit_gate(ctx);
                    block.invalidate_layout(ctx);
                }
            },
        );

        ctx.subscribe_to_model(
            &action_model,
            |me, action_model, event: &BlocklistAIActionEvent, ctx| {
                if me.renders_action(event.action_id()) {
                    let materialized_active_blocker = if matches!(
                        event,
                        BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(_)
                    ) {
                        me.sync_action_views(&action_model, ctx)
                    } else {
                        false
                    };
                    if matches!(
                        event,
                        BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(_)
                            | BlocklistAIActionEvent::ExecutingAction(_)
                            | BlocklistAIActionEvent::FinishedAction { .. }
                    ) && !materialized_active_blocker
                    {
                        ctx.emit(TuiAIBlockEvent::BlockingStateChanged);
                    }
                    me.invalidate_action(event.action_id(), ctx);
                }
            },
        );

        if block.renders_agent_messages(ctx) && ctx.has_singleton_model::<TuiOrchestrationModel>() {
            ctx.subscribe_to_model(&TuiOrchestrationModel::handle(ctx), |me, _, event, ctx| {
                if let TuiOrchestrationEvent::RestoredRemoteChildStatusUpdated { conversation_id } =
                    event
                    && me.renders_agent_message_from(*conversation_id, ctx)
                {
                    ctx.notify();
                }
            });
        }

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
                if should_schedule_auto_expand
                    && let Some(TuiToolCallView::ShellCommand(view)) =
                        me.action_views.get(&action_id)
                {
                    view.update(ctx, |view, ctx| view.schedule_auto_expand(ctx));
                }
                me.invalidate_action(&action_id, ctx);
            }
        });
        block.block_model.on_updated_output(
            Box::new(move |me, ctx| {
                me.record_output_telemetry(ctx);
                me.sync_action_views(&action_model, ctx);
                me.sync_code_block_views(ctx);
                me.sync_first_credit_gate(ctx);
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

    fn sync_first_credit_gate(&mut self, ctx: &mut ViewContext<Self>) {
        if self.first_credit_gate {
            return;
        }
        let presentation = {
            let status = self.block_model.status(ctx);
            self.visible_failure(&status, ctx)
                .map(|(_, presentation)| presentation)
        };
        let is_out_of_credits =
            should_consume_first_credit_gate(self.block_model.is_restored(), presentation.as_ref());
        if is_out_of_credits {
            self.first_credit_gate = TuiOnboardingMarkers::handle(ctx)
                .update(ctx, |markers, ctx| {
                    markers.consume(TuiOnboardingMarker::FirstCreditGate, ctx)
                });
        }
    }

    fn record_output_telemetry(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_restored_for_telemetry || self.terminal_telemetry_emitted {
            return;
        }
        let status = self.block_model.status(ctx);
        if status.output_to_render().is_some()
            && let Some(latency) = self.block_model.time_since_request_start(ctx)
        {
            if self.time_to_first_token.set(latency).is_ok() {
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                    history.set_exchange_time_to_first_token(
                        self.conversation_id,
                        self.exchange_id,
                        latency.num_milliseconds(),
                    );
                });
            }
            self.time_to_last_token = Some(latency);
        }
        let (was_user_facing_error, cancelled) = match status {
            AIBlockOutputStatus::Pending | AIBlockOutputStatus::PartiallyReceived { .. } => return,
            AIBlockOutputStatus::Complete { .. } => (false, false),
            AIBlockOutputStatus::Cancelled { .. } => (false, true),
            AIBlockOutputStatus::Failed { .. } => (true, false),
        };
        self.terminal_telemetry_emitted = true;
        warp::send_telemetry_from_ctx!(
            TelemetryEvent::AgentModeCreatedAIBlock {
                client_exchange_id: self.exchange_id.to_string(),
                server_output_id: self.block_model.server_output_id(ctx),
                was_autodetected_ai_query: self.block_model.was_autodetected_ai_query(ctx),
                time_to_first_token_ms: self
                    .time_to_first_token
                    .get()
                    .map(|duration| duration.num_milliseconds() as u128),
                time_to_last_token_ms: self
                    .time_to_last_token
                    .map(|duration| duration.num_milliseconds() as u128),
                was_user_facing_error,
                cancelled,
                conversation_id: self.conversation_id,
                is_udi_enabled: false,
            },
            ctx
        );
    }

    /// Records the exchange's tool-call action ids and todo presence, and
    /// creates child views for stateful tool calls that don't have one yet.
    /// Rendering can't create views since it only sees `&AppContext`.
    fn sync_action_views(
        &mut self,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let mut materialized_active_blocker = false;
        let status = self.block_model.status(ctx);
        let output_streaming = status.is_streaming();
        // Whether this exchange's output already reached a terminal state
        // without producing a result for an action still streaming in it —
        // that action never reaches the queue and needs to render as
        // cancelled instead of sitting on its constructing/pending fallback
        // forever (see `TuiOrchestrationBlock::output_terminal_without_result`).
        let output_terminal_without_result = matches!(
            status,
            AIBlockOutputStatus::Cancelled { .. } | AIBlockOutputStatus::Failed { .. }
        );
        let mut ask_question_actions = Vec::new();
        let mut file_edit_actions = Vec::new();
        let mut generic_actions = Vec::new();
        let mut plan_actions = Vec::new();
        let mut shell_command_actions = Vec::new();
        let mut run_agents_actions = Vec::new();
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
                if let AIAgentActionType::AskUserQuestion { questions } = &action.action {
                    ask_question_actions.push((action.id.clone(), questions.clone()));
                } else if let AIAgentActionType::RequestFileEdits { file_edits, .. } =
                    &action.action
                {
                    file_edit_actions.push((action.id.clone(), file_edits.clone()));
                } else if matches!(
                    &action.action,
                    AIAgentActionType::CreateDocuments(_) | AIAgentActionType::EditDocuments(_)
                ) {
                    plan_actions.push(action.clone());
                } else if matches!(
                    &action.action,
                    AIAgentActionType::RequestCommandOutput { .. }
                ) {
                    shell_command_actions.push(action.clone());
                } else if matches!(&action.action, AIAgentActionType::RunAgents(_)) {
                    run_agents_actions.push(action.clone());
                } else if action_model
                    .as_ref(ctx)
                    .get_action_status(&action.id)
                    .is_some_and(|status| status.is_blocked())
                {
                    generic_actions.push(action.clone());
                }
            }
        }

        for (action_id, questions) in ask_question_actions {
            let needs_init = match self.action_views.get(&action_id) {
                Some(TuiToolCallView::AskQuestion(view)) => {
                    !view.as_ref(ctx).matches_action(&action_id, &questions)
                }
                Some(
                    TuiToolCallView::FileEdits(_)
                    | TuiToolCallView::Generic(_)
                    | TuiToolCallView::Plan(_)
                    | TuiToolCallView::ShellCommand(_)
                    | TuiToolCallView::OrchestrationBlock(_),
                )
                | None => true,
            };
            if !needs_init {
                continue;
            }
            let view_action_id = action_id.clone();
            let action_model = action_model.clone();
            let conversation_id = self.conversation_id;
            let view = ctx.add_typed_action_tui_view(move |ctx| {
                TuiAskQuestionView::new(
                    action_model,
                    conversation_id,
                    view_action_id,
                    questions,
                    ctx,
                )
            });
            ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
                TuiAskQuestionViewEvent::LayoutChanged => me.invalidate_layout(ctx),
            });
            materialized_active_blocker |= view.as_ref(ctx).is_awaiting_answers(ctx);
            self.action_views
                .insert(action_id, TuiToolCallView::AskQuestion(view));
            ctx.notify();
        }
        // Generic tool calls remain stateless until the shared action model
        // reports that one is the front-of-queue blocked action. Retain a view
        // only then so it can own the interactive permission prompt.
        for action in generic_actions {
            if let Some(TuiToolCallView::Generic(view)) = self.action_views.get(&action.id) {
                view.update(ctx, |view, ctx| {
                    view.update_action(action, output_streaming, ctx);
                });
                continue;
            }
            let action_id = action.id.clone();
            let action_model = action_model.clone();
            let conversation_id = self.conversation_id;
            let view = ctx.add_tui_view(|ctx| {
                TuiGenericToolCallView::new(
                    action,
                    output_streaming,
                    action_model,
                    conversation_id,
                    ctx,
                )
            });
            ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
                TuiGenericToolCallViewEvent::BlockingStateChanged => {
                    ctx.emit(TuiAIBlockEvent::BlockingStateChanged);
                    me.invalidate_layout(ctx);
                }
                TuiGenericToolCallViewEvent::LayoutChanged => me.invalidate_layout(ctx),
                TuiGenericToolCallViewEvent::ReplacementGuidanceSubmitted(text) => {
                    ctx.emit(TuiAIBlockEvent::ReplacementGuidanceSubmitted {
                        conversation_id: me.conversation_id,
                        text: text.clone(),
                    });
                }
            });
            materialized_active_blocker |= view.as_ref(ctx).active_permission_prompt(ctx).is_some();
            self.action_views
                .insert(action_id, TuiToolCallView::Generic(view));
            ctx.notify();
        }
        for (action_id, file_edits) in file_edit_actions {
            if self.action_views.contains_key(&action_id) {
                continue;
            }
            let view_action_id = action_id.clone();
            let conversation_id = self.conversation_id;
            let file_edits = file_edits.clone();
            let view = ctx.add_typed_action_tui_view(move |ctx| {
                TuiFileEditsView::new(
                    view_action_id,
                    conversation_id,
                    file_edits,
                    action_model,
                    ctx,
                )
            });
            ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
                TuiFileEditsViewEvent::BlockingStateChanged => {
                    ctx.emit(TuiAIBlockEvent::BlockingStateChanged);
                    me.invalidate_layout(ctx);
                }
                TuiFileEditsViewEvent::LayoutChanged => me.invalidate_layout(ctx),
                TuiFileEditsViewEvent::ReplacementGuidanceSubmitted(text) => {
                    ctx.emit(TuiAIBlockEvent::ReplacementGuidanceSubmitted {
                        conversation_id: me.conversation_id,
                        text: text.clone(),
                    });
                }
            });
            materialized_active_blocker |= view.as_ref(ctx).active_permission_prompt(ctx).is_some();
            self.action_views
                .insert(action_id, TuiToolCallView::FileEdits(view));
            ctx.notify();
        }

        for action in plan_actions {
            if let Some(TuiToolCallView::Plan(view)) = self.action_views.get(&action.id) {
                view.update(ctx, |view, ctx| {
                    view.sync_action(action, output_streaming, ctx);
                });
                continue;
            }
            let action_id = action.id.clone();
            let view = ctx.add_typed_action_tui_view(|ctx| {
                TuiPlanView::new(action, output_streaming, action_model, ctx)
            });
            ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
                TuiPlanViewEvent::LayoutChanged => me.invalidate_layout(ctx),
            });
            self.action_views
                .insert(action_id, TuiToolCallView::Plan(view));
            ctx.notify();
        }

        for action in shell_command_actions {
            if let Some(TuiToolCallView::ShellCommand(view)) = self.action_views.get(&action.id) {
                view.update(ctx, |view, ctx| {
                    view.update_action(action, output_streaming, ctx);
                });
                continue;
            }
            let action_id = action.id.clone();
            let action_model = action_model.clone();
            let conversation_id = self.conversation_id;
            let terminal_model = self.terminal_model.clone();
            let view = ctx.add_typed_action_tui_view(|ctx| {
                TuiShellCommandView::new(
                    action,
                    output_streaming,
                    action_model,
                    conversation_id,
                    terminal_model,
                    ctx,
                )
            });
            ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
                TuiShellCommandViewEvent::BlockingStateChanged => {
                    ctx.emit(TuiAIBlockEvent::BlockingStateChanged);
                    me.invalidate_layout(ctx);
                }
                TuiShellCommandViewEvent::LayoutChanged => me.invalidate_layout(ctx),
                TuiShellCommandViewEvent::ReplacementGuidanceSubmitted(text) => {
                    ctx.emit(TuiAIBlockEvent::ReplacementGuidanceSubmitted {
                        conversation_id: me.conversation_id,
                        text: text.clone(),
                    });
                }
            });
            materialized_active_blocker |= view.as_ref(ctx).active_permission_prompt(ctx).is_some();
            self.action_views
                .insert(action_id, TuiToolCallView::ShellCommand(view));
            ctx.notify();
        }

        // Create or update the interactive orchestration card for each
        // streamed RunAgents tool call.
        for action in run_agents_actions {
            let AIAgentActionType::RunAgents(request) = &action.action else {
                continue;
            };

            // Existing block: re-sync its edit state from the latest streamed
            // chunk (the request may have grown since the view was created).
            if let Some(TuiToolCallView::OrchestrationBlock(view)) =
                self.action_views.get(&action.id)
            {
                let request = request.clone();
                view.update(ctx, |view, ctx| {
                    view.update_request(&request, output_terminal_without_result, ctx)
                });
                continue;
            }
            // Read the active orchestration config for plan-inherited
            // resolution from the conversation, mirroring the GUI's
            // `ensure_run_agents_card_view`.
            let active_config = if request.plan_id.is_empty() {
                None
            } else {
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&self.conversation_id)
                    .and_then(|conversation| {
                        conversation
                            .orchestration_config_for_plan(&request.plan_id)
                            .map(|(config, status)| (config.clone(), status))
                    })
            };

            let action_id = action.id.clone();
            let request = request.clone();
            let card_action_model = action_model.clone();
            let run_agents_executor = action_model.as_ref(ctx).run_agents_executor(ctx);
            let fallback_base_model_id = self.block_model.base_model(ctx).map(|id| id.to_string());
            let is_restored = self.is_restored_for_telemetry;
            let conversation_id = self.conversation_id;
            let view = ctx.add_typed_action_tui_view(move |ctx| {
                TuiOrchestrationBlock::new(
                    conversation_id,
                    action,
                    &request,
                    active_config,
                    card_action_model,
                    run_agents_executor,
                    fallback_base_model_id,
                    is_restored,
                    output_terminal_without_result,
                    ctx,
                )
            });

            let action_id_for_events = action_id.clone();
            ctx.subscribe_to_view(&view, move |me, _, event, ctx| match event {
                TuiOrchestrationBlockEvent::RejectRequested => {
                    me.cancel_action(&action_id_for_events, ctx);
                }
                TuiOrchestrationBlockEvent::BlockingStateChanged => {
                    ctx.emit(TuiAIBlockEvent::BlockingStateChanged);
                    me.invalidate_layout(ctx);
                }
                TuiOrchestrationBlockEvent::LayoutInvalidated => me.invalidate_layout(ctx),
            });
            materialized_active_blocker |= view.as_ref(ctx).is_awaiting_confirmation(ctx);
            self.action_views
                .insert(action_id, TuiToolCallView::OrchestrationBlock(view));
            ctx.notify();
        }
        if materialized_active_blocker {
            ctx.emit(TuiAIBlockEvent::BlockingStateChanged);
        }
        materialized_active_blocker
    }

    /// Cancels a pending or running action as manually cancelled — the
    /// TUI counterpart of the GUI `AIBlock::cancel_action` reject path.
    fn cancel_action(&self, action_id: &AIAgentActionId, ctx: &mut ViewContext<Self>) {
        let conversation_id = self.conversation_id;
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.cancel_action_with_id(
                conversation_id,
                action_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
    }

    /// The front-of-queue blocking interaction owned by this block, if any:
    /// the conversation's front pending action when it is `Blocked`, rendered
    /// by one of this block's child views, and that view is still awaiting
    /// confirmation. Deriving from the action queue (not transcript order)
    /// keeps semantics identical to the GUI's `focus_subview_if_necessary`.
    pub(super) fn active_blocking_input_source(
        &self,
        ctx: &AppContext,
    ) -> Option<BlockingInputSource> {
        let action_model = self.action_model.as_ref(ctx);
        let pending = action_model.get_pending_action(ctx)?;
        let action_id = pending.id.clone();
        if !self.renders_action(&action_id) {
            return None;
        }
        if !matches!(
            action_model.get_action_status(&action_id),
            Some(AIActionStatus::Blocked)
        ) {
            return None;
        }
        match self.action_views.get(&action_id)? {
            TuiToolCallView::AskQuestion(view) => view
                .as_ref(ctx)
                .is_awaiting_answers(ctx)
                .then(|| BlockingInputSource::AskQuestion(view.clone())),
            TuiToolCallView::OrchestrationBlock(view) => view
                .as_ref(ctx)
                .is_awaiting_confirmation(ctx)
                .then(|| BlockingInputSource::Orchestration(view.clone())),
            TuiToolCallView::Generic(view) => view
                .as_ref(ctx)
                .active_permission_prompt(ctx)
                .map(BlockingInputSource::Permission),
            TuiToolCallView::FileEdits(view) => view
                .as_ref(ctx)
                .active_permission_prompt(ctx)
                .map(BlockingInputSource::Permission),
            TuiToolCallView::ShellCommand(view) => view
                .as_ref(ctx)
                .active_permission_prompt(ctx)
                .map(BlockingInputSource::Permission),
            // Plan tool views render inline and never replace the input.
            TuiToolCallView::Plan(_) => None,
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

    /// Returns whether this block renders any received-agent message.
    fn renders_agent_messages(&self, app: &AppContext) -> bool {
        let status = self.block_model.status(app);
        let Some(output) = status.output_to_render() else {
            return false;
        };
        output.get().messages.iter().any(|message| {
            matches!(
                &message.message,
                AIAgentOutputMessageType::MessagesReceivedFromAgents { messages }
                    if !messages.is_empty()
            )
        })
    }

    /// Returns whether this block renders a received message whose sender
    /// resolves to `conversation_id`.
    fn renders_agent_message_from(
        &self,
        conversation_id: AIConversationId,
        app: &AppContext,
    ) -> bool {
        let history = BlocklistAIHistoryModel::as_ref(app);
        let status = self.block_model.status(app);
        let Some(output) = status.output_to_render() else {
            return false;
        };
        output.get().messages.iter().any(|message| {
            let AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } =
                &message.message
            else {
                return false;
            };
            messages.iter().any(|message| {
                history.conversation_id_for_agent_id(&message.sender_agent_id)
                    == Some(conversation_id)
            })
        })
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

    fn latest_exposed_plan(&self, ctx: &AppContext) -> Option<ViewHandle<TuiPlanView>> {
        let status = self.block_model.status(ctx);
        let output = status.output_to_render()?;

        output.get().messages.iter().rev().find_map(|message| {
            let AIAgentOutputMessageType::Action(action) = &message.message else {
                return None;
            };
            let Some(TuiToolCallView::Plan(view)) = self.action_views.get(&action.id) else {
                return None;
            };
            view.as_ref(ctx).renders_rich_body().then(|| view.clone())
        })
    }
    pub(super) fn has_exposed_plan(&self, ctx: &AppContext) -> bool {
        self.latest_exposed_plan(ctx).is_some()
    }

    pub(super) fn toggle_latest_plan(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(plan) = self.latest_exposed_plan(ctx) else {
            return false;
        };
        plan.update(ctx, |plan, ctx| {
            plan.toggle_collapsed(ctx);
        });
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
            || self.action_views.values().any(|view| match view {
                TuiToolCallView::AskQuestion(_)
                | TuiToolCallView::FileEdits(_)
                | TuiToolCallView::Generic(_)
                | TuiToolCallView::Plan(_)
                | TuiToolCallView::OrchestrationBlock(_) => false,
                TuiToolCallView::ShellCommand(view) => {
                    view.as_ref(app).needs_continuous_height_measurement()
                }
            })
    }

    /// Records the width used for the latest height measurement.
    pub(super) fn record_height_measurement(&self, width: u16) {
        self.last_measured_width.set(Some(width));
    }

    /// Returns the failure that is visible in this block.
    ///
    /// This is the single source of truth for both failure rendering and
    /// contextual failure actions. Restored active failures remain visible;
    /// restoration only suppresses the separate usage notice.
    fn visible_failure<'a>(
        &self,
        status: &'a AIBlockOutputStatus,
        app: &AppContext,
    ) -> Option<(&'a RenderableAIError, FailedOutputPresentation)> {
        if !self.block_model.request_type(app).is_active() {
            return None;
        }
        let AIBlockOutputStatus::Failed { error, .. } = status else {
            return None;
        };
        failed_output_presentation(error, app).map(|presentation| (error, presentation))
    }

    pub(super) fn has_out_of_credits_failure(&self, app: &AppContext) -> bool {
        let status = self.block_model.status(app);
        matches!(
            self.visible_failure(&status, app),
            Some((_, FailedOutputPresentation::OutOfCredits { .. }))
        )
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
            collected.push(section_logical_text(section, app)?);
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
                let status = self.action_model.as_ref(app).get_action_status(&action.id);
                match &action.action {
                    AIAgentActionType::InsertCodeReviewComments { comments, .. } => {
                        render_review_comments_tool_call(
                            action,
                            comments,
                            status.as_ref(),
                            output_streaming,
                            app,
                        )
                    }
                    _ => {
                        if let Some(view) = self.action_views.get(&action.id) {
                            match view {
                                TuiToolCallView::Generic(view)
                                    if view.as_ref(app).active_permission_prompt(app).is_none() => {
                                }
                                TuiToolCallView::AskQuestion(_)
                                | TuiToolCallView::FileEdits(_)
                                | TuiToolCallView::Generic(_)
                                | TuiToolCallView::Plan(_)
                                | TuiToolCallView::ShellCommand(_)
                                | TuiToolCallView::OrchestrationBlock(_) => return None,
                            }
                        }
                        render_fallback_tool_call_section(
                            action,
                            status.as_ref(),
                            output_streaming,
                            None,
                            app,
                        )
                    }
                }
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
            TuiAIBlockSection::Summarization { message_id, body } => render_summarization_section(
                &self.collapsible_states,
                message_id,
                self.render_rich_text_sections(body, true, app),
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
            TuiAIBlockSection::AgentMessage(_) => return None,
            TuiAIBlockSection::Failure(presentation) => {
                render_failure_section(presentation, &self.out_of_credits_hover_state, app)
            }
            TuiAIBlockSection::FirstCreditGate => {
                render_first_credit_gate(&self.out_of_credits_hover_state, app)
            }
            TuiAIBlockSection::UsageNotice => render_usage_notice(app),
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
        let status = self.block_model.status(app);
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
        if let Some(output) = status.output_to_render() {
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
                        // WaitForEvents renders nothing, matching the GUI.
                        if !matches!(action.action, AIAgentActionType::WaitForEvents { .. }) {
                            sections.push(TuiAIBlockSection::ToolCall(Box::new(action.clone())));
                        }
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
                        summarization_type: SummarizationType::ConversationSummary,
                        ..
                    } => {
                        let body = Self::rich_text_sections(&message.id, text);
                        if !body.is_empty() {
                            sections.push(TuiAIBlockSection::Summarization {
                                message_id: message.id.clone(),
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
                    AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } => {
                        for received in messages {
                            sections.push(TuiAIBlockSection::AgentMessage(received.clone()));
                        }
                    }
                    // Event IDs contain no display detail. The sender's live
                    // conversation status is shown on rich message rows.
                    AIAgentOutputMessageType::EventsFromAgents { .. } => {}
                    // Other message kinds are not rendered by the TUI transcript yet.
                    AIAgentOutputMessageType::Summarization { .. }
                    | AIAgentOutputMessageType::Subagent(_)
                    | AIAgentOutputMessageType::WebSearch(_)
                    | AIAgentOutputMessageType::WebFetch(_)
                    | AIAgentOutputMessageType::CommentsAddressed { .. }
                    | AIAgentOutputMessageType::DebugOutput { .. }
                    | AIAgentOutputMessageType::ArtifactCreated(_)
                    | AIAgentOutputMessageType::SkillInvoked(_) => {}
                }
            }
        }

        if let Some((error, presentation)) = self.visible_failure(&status, app) {
            if self.first_credit_gate
                && matches!(presentation, FailedOutputPresentation::OutOfCredits { .. })
            {
                sections.push(TuiAIBlockSection::FirstCreditGate);
            } else {
                sections.push(TuiAIBlockSection::Failure(presentation));
            }
            if !self.first_credit_gate
                && should_show_failed_output_usage_notice(
                    error,
                    self.block_model
                        .is_latest_visible_exchange_in_root_task(app),
                    self.has_expanded_last_requested_command(app),
                    self.block_model.is_restored(),
                )
            {
                sections.push(TuiAIBlockSection::UsageNotice);
            }
        }

        sections
    }

    fn has_expanded_last_requested_command(&self, app: &AppContext) -> bool {
        let status = self.block_model.status(app);
        let Some(output) = status.output_to_render() else {
            return false;
        };
        let action_id = output.get().messages.iter().rev().find_map(|message| {
            let AIAgentOutputMessageType::Action(action) = &message.message else {
                return None;
            };
            matches!(
                &action.action,
                AIAgentActionType::RequestCommandOutput { .. }
            )
            .then(|| action.id.clone())
        });
        action_id
            .and_then(|action_id| self.action_views.get(&action_id))
            .is_some_and(|view| match view {
                TuiToolCallView::ShellCommand(view) => view.as_ref(app).is_expanded(),
                TuiToolCallView::AskQuestion(_)
                | TuiToolCallView::FileEdits(_)
                | TuiToolCallView::Generic(_)
                | TuiToolCallView::Plan(_)
                | TuiToolCallView::OrchestrationBlock(_) => false,
            })
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

        // Keep the view registered so a streaming exchange can gain visible
        // sections later, but do not reserve inter-block padding while every
        // message in this exchange is intentionally hidden.
        let sections = self.sections(app);
        if sections.is_empty() {
            return TuiFlex::column().finish();
        }

        let mut column = TuiFlex::column();
        let last_index = sections.len().saturating_sub(1);
        for (index, section) in sections.iter().enumerate() {
            let element = match section {
                TuiAIBlockSection::Input(text) => render_input_section(text, app),
                TuiAIBlockSection::RichText(section) => {
                    self.render_rich_text_section(section, false, app)
                }
                // Stateful tool calls render their registered child view; every
                // other tool call stays a pure render fn.
                TuiAIBlockSection::ToolCall(action) => match &action.action {
                    AIAgentActionType::InsertCodeReviewComments { comments, .. } => {
                        let status = self.action_model.as_ref(app).get_action_status(&action.id);
                        render_review_comments_tool_call(
                            action,
                            comments,
                            status.as_ref(),
                            output_streaming,
                            app,
                        )
                    }
                    _ => match self.action_views.get(&action.id) {
                        Some(TuiToolCallView::Plan(view))
                            if !view.as_ref(app).renders_rich_body() =>
                        {
                            let status =
                                self.action_model.as_ref(app).get_action_status(&action.id);
                            render_fallback_tool_call_section(
                                action,
                                status.as_ref(),
                                output_streaming,
                                None,
                                app,
                            )
                        }
                        Some(TuiToolCallView::Generic(view))
                            if view.as_ref(app).active_permission_prompt(app).is_none() =>
                        {
                            let status =
                                self.action_model.as_ref(app).get_action_status(&action.id);
                            render_fallback_tool_call_section(
                                action,
                                status.as_ref(),
                                output_streaming,
                                None,
                                app,
                            )
                        }
                        Some(view) => TuiContainer::new(Box::new(view.render_child())).finish(),
                        None => {
                            let status =
                                self.action_model.as_ref(app).get_action_status(&action.id);
                            render_fallback_tool_call_section(
                                action,
                                status.as_ref(),
                                output_streaming,
                                None,
                                app,
                            )
                        }
                    },
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
                TuiAIBlockSection::Summarization { message_id, body } => {
                    render_summarization_section(
                        &self.collapsible_states,
                        message_id,
                        self.render_rich_text_sections(body, true, app),
                        app,
                    )
                }
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
                TuiAIBlockSection::AgentMessage(message) => render_agent_message(
                    &self.collapsible_states,
                    message,
                    self.conversation_id,
                    app,
                ),
                TuiAIBlockSection::Failure(presentation) => {
                    render_failure_section(presentation, &self.out_of_credits_hover_state, app)
                }
                TuiAIBlockSection::FirstCreditGate => {
                    render_first_credit_gate(&self.out_of_credits_hover_state, app)
                }
                TuiAIBlockSection::UsageNotice => render_usage_notice(app),
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
/// clean logical form (tool calls, reasoning, summaries, todo lists, or agent
/// messages), which fall back to per-row grid text.
fn section_logical_text(section: &TuiAIBlockSection, app: &AppContext) -> Option<String> {
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
        | TuiAIBlockSection::CompletedTodos { .. }
        | TuiAIBlockSection::AgentMessage(_) => None,
        TuiAIBlockSection::Failure(presentation) => Some(failure_text(presentation, app)),
        TuiAIBlockSection::FirstCreditGate => {
            let upgrade_url = upgrade_url(app);
            Some(format!(
                "{FIRST_CREDIT_GATE_TITLE}\n{FIRST_CREDIT_GATE_ACTION_LABEL} \
                 {FIRST_CREDIT_GATE_ACTION_HINT}\n\n{upgrade_url}"
            ))
        }
        TuiAIBlockSection::UsageNotice => Some(FAILED_OUTPUT_USAGE_NOTICE_TEXT.to_owned()),
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
