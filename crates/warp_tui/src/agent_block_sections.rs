//! Pure render functions for each agent block section kind.
//!
//! Each render function takes a section's data (plus the block's collapsible
//! section states for collapse/hover state) and returns its element. Spacing
//! between sections is owned by the composer in `agent_block.rs`, not by these
//! renderers.

use std::time::Duration;

use warp::tui_export::{
    format_elapsed_seconds, AIActionStatus, AIAgentAction, AIAgentTodo, AIAgentTodoList, MessageId,
    TodoStatus,
};
use warpui_core::elements::tui::{
    Modifier, TuiContainer, TuiElement, TuiFlex, TuiParentElement, TuiStyle, TuiText,
};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::AppContext;

use crate::agent_block::{CollapsibleSectionStates, TuiAIBlockAction};
use crate::tool_call_labels::{
    tool_call_display_state, tool_call_glyph, tool_call_label, ResolvedCommandBlock,
    ToolCallDisplayState,
};
use crate::tui_builder::TuiUiBuilder;

const INPUT_PREFIX: &str = "> ";

/// Task-list header glyph. Visually interchangeable with the design's `☰`,
/// whose inconsistent cell width across terminals leaves ghost cells behind.
const TASK_LIST_HEADER_GLYPH: &str = "≡";

/// Renders the input section: the user's submitted query on a highlighted
/// background with a `>` prompt marker.
pub(crate) fn render_input_section(text: &str, app: &AppContext) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let text_style = builder.input_text_style();
    let prefix_style = builder.input_prefix_style();

    // Only the first line carries the `>` prompt marker; continuation
    // lines are indented to the marker's width so they align beneath it.
    // The column stretches to the full offered width so the highlighted
    // background spans the whole row, not just the text.
    let mut column = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (index, line) in text.split('\n').enumerate() {
        let row = if index == 0 {
            TuiFlex::row()
                .child(TuiText::new(INPUT_PREFIX).with_style(prefix_style).finish())
                .child(
                    TuiText::new(line.to_owned())
                        .with_style(text_style)
                        .finish(),
                )
                .finish()
        } else {
            TuiFlex::row()
                .child(
                    TuiText::new(" ".repeat(INPUT_PREFIX.chars().count()))
                        .with_style(text_style)
                        .finish(),
                )
                .child(
                    TuiText::new(line.to_owned())
                        .with_style(text_style)
                        .finish(),
                )
                .finish()
        };
        column = column.child(row);
    }
    TuiContainer::new(column.finish())
        .with_background(builder.input_background())
        .finish()
}

/// Shared leading-glyph style for all rich and fallback TUI tool-call rows.
pub(crate) fn tool_call_glyph_style(
    state: ToolCallDisplayState,
    builder: &TuiUiBuilder,
) -> TuiStyle {
    match state {
        ToolCallDisplayState::Constructing | ToolCallDisplayState::Pending => {
            builder.dim_text_style()
        }
        ToolCallDisplayState::AwaitingApproval | ToolCallDisplayState::Running => {
            builder.attention_glyph_style()
        }
        ToolCallDisplayState::Succeeded => builder.success_glyph_style(),
        ToolCallDisplayState::Failed => builder.error_text_style(),
        ToolCallDisplayState::Cancelled => builder.muted_text_style(),
    }
}

/// Shared label style for all rich and fallback TUI tool-call rows.
pub(crate) fn tool_call_label_style(
    state: ToolCallDisplayState,
    builder: &TuiUiBuilder,
) -> TuiStyle {
    match state {
        ToolCallDisplayState::Constructing | ToolCallDisplayState::Pending => {
            builder.dim_text_style()
        }
        ToolCallDisplayState::AwaitingApproval
        | ToolCallDisplayState::Running
        | ToolCallDisplayState::Succeeded
        | ToolCallDisplayState::Failed
        | ToolCallDisplayState::Cancelled => builder.primary_text_style(),
    }
}

/// Renders a plain-text response section.
pub(crate) fn render_plain_text_section(text: &str, app: &AppContext) -> Box<dyn TuiElement> {
    TuiText::new(text.to_owned())
        .with_style(TuiUiBuilder::from_app(app).primary_text_style())
        .finish()
}

/// Renders the fallback plain-text status row for an agent tool call, used
/// for every tool call without a richer registered child view (the GUI's
/// view-based action rendering has no TUI equivalent for these yet): a
/// colored state glyph in a two-cell gutter (mirroring the GUI's inline
/// action icons), then per-tool, per-state label text that wraps with a
/// hanging indent under itself. State lives in the glyph, so labels keep the
/// normal foreground except in-flight rows, which stay dim until execution
/// starts. `output_streaming` marks tool calls whose arguments are still
/// streaming in (see `ToolCallDisplayState::Constructing`); `block` carries
/// the terminal block's ground truth for shell-command tool calls (see
/// `ResolvedCommandBlock`).
pub(crate) fn render_fallback_tool_call_section(
    action: &AIAgentAction,
    status: Option<&AIActionStatus>,
    output_streaming: bool,
    block: Option<&ResolvedCommandBlock>,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let state = tool_call_display_state(status, output_streaming, block.map(|block| block.state));
    let glyph_style = tool_call_glyph_style(state, &builder);
    let label_style = tool_call_label_style(state, &builder);
    let label = tool_call_label(action, status, output_streaming, block);
    TuiFlex::row()
        .child(
            TuiText::new(format!("{} ", tool_call_glyph(state)))
                .with_style(glyph_style)
                .finish(),
        )
        .child(TuiText::new(label).with_style(label_style).finish())
        .finish()
}

/// Renders a reasoning message as a collapsible thinking block.
pub(crate) fn render_thinking_section(
    states: &CollapsibleSectionStates,
    message_id: &MessageId,
    finished_duration: Option<Duration>,
    body: &str,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let header = match finished_duration {
        Some(duration) => format!("Thought for {}", format_elapsed_seconds(duration)),
        None => "Thinking...".to_owned(),
    };
    render_collapsible_message_section(
        states,
        message_id,
        header,
        finished_duration.is_some(),
        body,
        builder.muted_text_style(),
        app,
    )
}

/// Renders a streamed conversation summary with the same persistent
/// collapse/hover behavior as a reasoning section.
pub(crate) fn render_summarization_section(
    states: &CollapsibleSectionStates,
    message_id: &MessageId,
    finished: bool,
    body: &str,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    render_collapsible_message_section(
        states,
        message_id,
        "Conversation summarized".to_owned(),
        finished,
        body,
        TuiUiBuilder::from_app(app).primary_text_style(),
        app,
    )
}

fn render_collapsible_message_section(
    states: &CollapsibleSectionStates,
    message_id: &MessageId,
    header: String,
    finished: bool,
    body: &str,
    body_style: TuiStyle,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    // Indent the body so every wrapped line aligns beneath the header.
    let body_element = TuiContainer::new(
        TuiText::new(body.to_owned())
            .with_style(body_style)
            .finish(),
    )
    .with_padding_left(4);

    let collapsed = states.is_collapsed(message_id, finished);
    let toggle_message_id = message_id.clone();
    builder.collapsible(
        collapsed,
        header,
        states.hover_state(message_id),
        body_element.finish(),
        move |event_ctx, _app| {
            event_ctx.dispatch_typed_action(TuiAIBlockAction::SetSectionCollapsed {
                message_id: toggle_message_id.clone(),
                collapsed: !collapsed,
            });
        },
    )
}

/// Renders the agent's task list as a collapsible block: a bold
/// `≡ Tasks N` header (prominent, unlike the muted thinking header, per the
/// TUI Figma design) over one status row per task. Statuses are resolved by
/// the caller from the conversation's todo history; like tool-call rows,
/// state lives in each row's glyph.
///
/// Task lists default to expanded and collapse only on manual toggle (the
/// GUI's `TodoListElementState` behavior), so the default passed to the
/// state map is never "collapsed" — unlike thinking blocks, which
/// auto-collapse on finish.
pub(crate) fn render_todo_list_section(
    states: &CollapsibleSectionStates,
    message_id: &MessageId,
    todos: &[(String, TodoStatus)],
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);

    let mut rows = TuiFlex::column();
    for (title, status) in todos {
        let (glyph, glyph_style) = todo_glyph(status, &builder);
        let title_style = match status {
            TodoStatus::Pending | TodoStatus::InProgress | TodoStatus::Completed => {
                builder.primary_text_style()
            }
            // Cancelled items are struck through in the GUI; stopped items
            // just read as de-emphasized.
            TodoStatus::Cancelled => builder
                .muted_text_style()
                .add_modifier(Modifier::CROSSED_OUT),
            TodoStatus::Stopped => builder.muted_text_style(),
        };
        rows.add_child(
            TuiFlex::row()
                .child(
                    TuiText::new(format!("{glyph} "))
                        .with_style(glyph_style)
                        .finish(),
                )
                .child(TuiText::new(title.clone()).with_style(title_style).finish())
                .finish(),
        );
    }
    // Indent rows so the status glyphs sit under the header label.
    let body = TuiContainer::new(rows.finish()).with_padding_left(2);

    let collapsed = states.is_collapsed(message_id, false);
    let toggle_message_id = message_id.clone();
    builder.prominent_collapsible(
        collapsed,
        TASK_LIST_HEADER_GLYPH,
        format!("Tasks {}", todos.len()),
        states.hover_state(message_id),
        body.finish(),
        move |event_ctx, _app| {
            event_ctx.dispatch_typed_action(TuiAIBlockAction::SetSectionCollapsed {
                message_id: toggle_message_id.clone(),
                collapsed: !collapsed,
            });
        },
    )
}

/// The status glyph and its style for one task row. Pending and in-progress
/// glyphs follow the TUI Figma design; terminal states reuse the tool-call
/// glyph vocabulary (see `tool_call_glyph`).
fn todo_glyph(status: &TodoStatus, builder: &TuiUiBuilder) -> (&'static str, TuiStyle) {
    match status {
        TodoStatus::Pending => ("◌", builder.primary_text_style()),
        TodoStatus::InProgress => ("•", builder.attention_glyph_style()),
        TodoStatus::Completed => ("✓", builder.success_glyph_style()),
        TodoStatus::Cancelled | TodoStatus::Stopped => ("■", builder.muted_text_style()),
    }
}

/// Renders the compact completion row for todos the agent just marked done:
/// `✓ Completed <title> (n/m)`, muted like the GUI's sub-text treatment.
/// `active_list` supplies each item's `(n/m)` position; the position is
/// omitted for items no longer in the active list.
pub(crate) fn render_completed_todos_section(
    completed: &[AIAgentTodo],
    active_list: Option<&AIAgentTodoList>,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let style = builder.muted_text_style();
    TuiFlex::row()
        .child(TuiText::new("✓ ").with_style(style).finish())
        .child(
            TuiText::new(completed_todos_label(completed, active_list))
                .with_style(style)
                .finish(),
        )
        .finish()
}

/// Builds the `Completed a (1/3), b (2/3)` label text, a port of the GUI's
/// `render_completed_todo_items` text logic.
pub(crate) fn completed_todos_label(
    completed: &[AIAgentTodo],
    active_list: Option<&AIAgentTodoList>,
) -> String {
    let mut label = String::new();
    for (index, item) in completed.iter().enumerate() {
        let position = active_list
            .and_then(|list| {
                list.get_item_index(&item.id)
                    .map(|i| format!(" ({}/{})", i + 1, list.len()))
            })
            .unwrap_or_default();
        if index == 0 {
            label += &format!("Completed {}{position}", item.title);
        } else {
            label += &format!(", {}{position}", item.title);
        }
    }
    label
}
