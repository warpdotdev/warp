//! Pure render functions for each agent block section kind.
//!
//! Each render function takes a section's data (plus the block's thinking
//! block states for collapse/hover state) and returns its element. Spacing
//! between sections is owned by the composer in `agent_block.rs`, not by these
//! renderers.

use std::time::Duration;

use warp::tui_export::{format_elapsed_seconds, AIActionStatus, AIAgentAction, MessageId};
use warpui_core::elements::tui::{TuiContainer, TuiElement, TuiFlex, TuiText};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::AppContext;

use crate::agent_block::ThinkingBlockStates;
use crate::tool_call_labels::{
    tool_call_display_state, tool_call_glyph, tool_call_label, ResolvedCommandBlock,
    ToolCallDisplayState,
};
use crate::tui_builder::TuiUiBuilder;

const INPUT_PREFIX: &str = "≫ ";

/// Renders the input section: the user's submitted query on a highlighted
/// background with a `≫` prompt marker.
pub(crate) fn render_input_section(text: &str, app: &AppContext) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let style = builder.input_text_style();

    // Only the first line carries the `≫` prompt marker; continuation
    // lines are indented to the marker's width so they align beneath it.
    // The column stretches to the full offered width so the highlighted
    // background spans the whole row, not just the text.
    let mut column = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (index, line) in text.split('\n').enumerate() {
        let line_text = if index == 0 {
            format!("{INPUT_PREFIX}{line}")
        } else {
            format!("{}{line}", " ".repeat(INPUT_PREFIX.chars().count()))
        };
        column = column.child(TuiText::new(line_text).with_style(style).finish());
    }
    TuiContainer::new(column.finish())
        .with_background(builder.input_background())
        .finish()
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
    let glyph_style = match state {
        ToolCallDisplayState::Constructing | ToolCallDisplayState::Pending => {
            builder.dim_text_style()
        }
        ToolCallDisplayState::AwaitingApproval | ToolCallDisplayState::Running => {
            builder.attention_glyph_style()
        }
        ToolCallDisplayState::Succeeded => builder.success_glyph_style(),
        ToolCallDisplayState::Failed => builder.error_text_style(),
        ToolCallDisplayState::Cancelled => builder.muted_text_style(),
    };
    let label_style = match state {
        ToolCallDisplayState::Constructing | ToolCallDisplayState::Pending => {
            builder.dim_text_style()
        }
        ToolCallDisplayState::AwaitingApproval
        | ToolCallDisplayState::Running
        | ToolCallDisplayState::Succeeded
        | ToolCallDisplayState::Failed
        | ToolCallDisplayState::Cancelled => builder.primary_text_style(),
    };
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

/// Renders a reasoning message as a collapsible thinking block. The header
/// turns white and bold while the block's hover state reports it hovered.
pub(crate) fn render_thinking_section(
    states: &ThinkingBlockStates,
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
    // Indent the reasoning body so every wrapped line aligns beneath the header.
    let body_element = TuiContainer::new(
        TuiText::new(body.to_owned())
            .with_style(builder.muted_text_style())
            .finish(),
    )
    .with_padding_left(4);

    let collapsed = states.is_collapsed(message_id, finished_duration.is_some());
    let toggle_states = states.clone();
    let toggle_message_id = message_id.clone();
    builder.collapsible(
        collapsed,
        header,
        states.hover_state(message_id),
        body_element.finish(),
        move |event_ctx, _app| {
            toggle_states.set_collapsed(toggle_message_id.clone(), !collapsed);
            event_ctx.notify();
        },
    )
}
