//! Pure render functions for each agent block section kind.
//!
//! Each render function takes a section's data (plus the block's thinking
//! block states for collapse/hover state) and returns its element. Spacing
//! between sections is owned by the composer in `agent_block.rs`, not by these
//! renderers.

use std::time::Duration;

use warp::tui_export::{format_elapsed_seconds, MessageId};
use warpui_core::elements::tui::{TuiContainer, TuiElement, TuiFlex, TuiText};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::AppContext;

use crate::agent_block::ThinkingBlockStates;
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

/// Renders a dim status row standing in for an agent tool call.
// TODO: add richer rendering for each tool call type. This is just a rendering stub to build off of.
pub(crate) fn render_tool_call_section(app: &AppContext) -> Box<dyn TuiElement> {
    TuiText::new("executed a tool call")
        .with_style(TuiUiBuilder::from_app(app).dim_text_style())
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
