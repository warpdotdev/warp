//! Pure render functions for each agent block section kind, plus the thinking
//! section's collapse-state semantics ([`ThinkingOverrides`]).
//!
//! Each render function takes a section's data (plus the block's thinking
//! overrides for collapse state) and returns its element. Spacing between
//! sections is owned by the composer in `agent_block.rs`, not by these
//! renderers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use warp::tui_export::{format_elapsed_seconds, Appearance, MessageId};
use warp_core::ui::color::blend::Blend;
// `ThemeFill` is the theme-layer color (it supports blend/opacity); `Fill` below
// is the element-layer color it converts into on its way to a terminal cell.
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    tui_collapsible, Modifier, TuiColumn, TuiContainer, TuiElement, TuiStyle, TuiText,
};
use warpui_core::elements::Fill;
use warpui_core::AppContext;

const INPUT_PREFIX: &str = "≫ ";
/// Left indent (cells) applied to a thinking block's reasoning body so every
/// wrapped line aligns beneath the header.
const THINKING_BODY_INDENT: u16 = 4;

/// Manual collapse overrides for thinking blocks, keyed by reasoning message.
/// Absence means the default: collapsed iff reasoning has finished, so a block
/// streams expanded and auto-collapses on finish unless the user has toggled
/// it — a recorded override wins permanently.
#[derive(Clone, Default)]
pub(crate) struct ThinkingOverrides {
    overrides: Rc<RefCell<HashMap<MessageId, bool>>>,
}

impl ThinkingOverrides {
    /// Whether the thinking block for `message_id` is collapsed: the manual
    /// override if one was recorded, else collapsed iff `finished`.
    pub(crate) fn is_collapsed(&self, message_id: &MessageId, finished: bool) -> bool {
        self.overrides
            .borrow()
            .get(message_id)
            .copied()
            .unwrap_or(finished)
    }

    /// Records a manual collapse override for `message_id`.
    pub(crate) fn set(&self, message_id: MessageId, collapsed: bool) {
        self.overrides.borrow_mut().insert(message_id, collapsed);
    }
}

/// Renders the input section: the user's submitted query on a highlighted
/// background with a `≫` prompt marker.
pub(crate) fn render_input_section(text: &str, app: &AppContext) -> Box<dyn TuiElement> {
    let theme = Appearance::as_ref(app).theme();
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

/// Renders a reasoning message as a collapsible thinking block.
pub(crate) fn render_thinking_section(
    overrides: &ThinkingOverrides,
    message_id: &MessageId,
    finished_duration: Option<Duration>,
    body: &str,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let theme = Appearance::as_ref(app).theme();
    let text_color = Fill::from(ThemeFill::from(theme.terminal_colors().bright.black)).into();
    let style = TuiStyle::default().fg(text_color);

    let header = match finished_duration {
        Some(duration) => format!("Thought for {}", format_elapsed_seconds(duration)),
        None => "Thinking...".to_owned(),
    };

    // Indent the whole reasoning body beneath the header via left padding so
    // every wrapped line aligns, not just the first. An empty body renders
    // nothing, so a just-started block shows only the header.
    let body_element = TuiContainer::new(TuiText::new(body.to_owned()).with_style(style).finish())
        .with_padding_left(THINKING_BODY_INDENT);

    let collapsed = overrides.is_collapsed(message_id, finished_duration.is_some());
    let toggle_overrides = overrides.clone();
    let toggle_message_id = message_id.clone();
    tui_collapsible(
        collapsed,
        header,
        style,
        body_element.finish(),
        move |event_ctx, _app| {
            toggle_overrides.set(toggle_message_id.clone(), !collapsed);
            event_ctx.notify();
        },
    )
}
