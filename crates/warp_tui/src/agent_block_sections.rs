//! Pure render functions for each agent block section kind.
//!
//! Each render function takes a section's data (plus the block's thinking
//! block states for collapse/hover state) and returns its element. Spacing
//! between sections is owned by the composer in `agent_block.rs`, not by these
//! renderers.

use std::time::Duration;

use warp::tui_export::{format_elapsed_seconds, Appearance, MessageId};
use warp_core::ui::color::blend::Blend;
// `ThemeFill` is the theme-layer color (it supports blend/opacity); `Fill` below
// is the element-layer color it converts into on its way to a terminal cell.
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    tui_collapsible, Modifier, TuiContainer, TuiElement, TuiFlex, TuiStyle, TuiText,
};
use warpui_core::elements::Fill;
use warpui_core::AppContext;

use crate::agent_block::ThinkingBlockStates;

const INPUT_PREFIX: &str = "≫ ";

/// Renders the input section: the user's submitted query on a highlighted
/// background with a `≫` prompt marker.
pub(crate) fn render_input_section(text: &str, app: &AppContext) -> Box<dyn TuiElement> {
    let theme = Appearance::as_ref(app).theme();
    let text_color = Fill::from(theme.foreground()).into();
    let accent = ThemeFill::from(theme.terminal_colors().normal.cyan);
    let background = Fill::from(theme.background().blend(&accent.with_opacity(20))).into();

    // Only the first line carries the `≫` prompt marker; continuation
    // lines are indented to the marker's width so they align beneath it.
    let mut column = TuiFlex::column();
    for (index, line) in text.split('\n').enumerate() {
        let line_text = if index == 0 {
            format!("{INPUT_PREFIX}{line}")
        } else {
            format!("{}{line}", " ".repeat(INPUT_PREFIX.chars().count()))
        };
        column = column.child(
            TuiText::new(line_text)
                .with_style(
                    TuiStyle::default()
                        .fg(text_color)
                        .bg(background)
                        .add_modifier(Modifier::BOLD),
                )
                .finish(),
        );
    }
    TuiContainer::new(column.finish())
        .with_background(background)
        .finish()
}

/// Renders a plain-text response section.
pub(crate) fn render_plain_text_section(text: &str, app: &AppContext) -> Box<dyn TuiElement> {
    let theme = Appearance::as_ref(app).theme();
    let text_color = Fill::from(ThemeFill::from(theme.terminal_colors().normal.white)).into();
    TuiText::new(text.to_owned())
        .with_style(TuiStyle::default().fg(text_color))
        .finish()
}

/// Renders a dim status row standing in for an agent tool call.
// TODO: add richer rendering for each tool call type. This is just a rendering stub to build off of.
pub(crate) fn render_tool_call_section(app: &AppContext) -> Box<dyn TuiElement> {
    let theme = Appearance::as_ref(app).theme();
    let text_color = Fill::from(ThemeFill::from(theme.terminal_colors().bright.black)).into();
    TuiText::new("executed a tool call")
        .with_style(
            TuiStyle::default()
                .fg(text_color)
                .add_modifier(Modifier::DIM),
        )
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
    let theme = Appearance::as_ref(app).theme();
    let text_color = Fill::from(ThemeFill::from(theme.terminal_colors().bright.black)).into();
    let style = TuiStyle::default().fg(text_color);
    let hover_color = Fill::from(ThemeFill::from(theme.terminal_colors().normal.white)).into();
    let hover_style = TuiStyle::default()
        .fg(hover_color)
        .add_modifier(Modifier::BOLD);

    let header = match finished_duration {
        Some(duration) => format!("Thought for {}", format_elapsed_seconds(duration)),
        None => "Thinking...".to_owned(),
    };
    // Indent the reasoning body so every wrapped line aligns beneath the header.
    let body_element = TuiContainer::new(TuiText::new(body.to_owned()).with_style(style).finish())
        .with_padding_left(4);

    let collapsed = states.is_collapsed(message_id, finished_duration.is_some());
    let toggle_states = states.clone();
    let toggle_message_id = message_id.clone();
    tui_collapsible(
        collapsed,
        header,
        style,
        hover_style,
        states.hover_state(message_id),
        body_element.finish(),
        move |event_ctx, _app| {
            toggle_states.set_collapsed(toggle_message_id.clone(), !collapsed);
            event_ctx.notify();
        },
    )
}
