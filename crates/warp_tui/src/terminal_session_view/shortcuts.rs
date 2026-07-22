//! Stateless renderer for the contextual shortcuts projection.
//!
//! The panel has no focus, actions, retained state, or independent lifecycle,
//! so it intentionally remains an element renderer rather than a child view.
//! Its contents are rebuilt from the session model's live snapshot and the
//! current keymap context on every parent render.

use warpui_core::AppContext;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    Modifier, TuiContainer, TuiElement, TuiFlex, TuiParentElement, TuiText,
};
use warpui_core::keymap::Context;

use super::state::{TuiShortcut, TuiTerminalSessionState};
use crate::tui_builder::TuiUiBuilder;

fn render_entry(shortcut: &TuiShortcut, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    TuiText::from_spans([
        (format!("{} ", shortcut.key), builder.link_text_style()),
        (
            shortcut.description.to_owned(),
            builder.primary_text_style(),
        ),
    ])
    .truncate()
    .finish()
}

pub(super) fn render(
    state: &TuiTerminalSessionState,
    context: &Context,
    ctx: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(ctx);
    let sections = state.shortcut_sections(context, ctx);
    let mut panel = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (section_index, section) in sections.iter().enumerate() {
        if section_index > 0 {
            panel.add_child(TuiText::new(" ").finish());
        }
        panel.add_child(
            TuiText::new(section.title)
                .with_style(builder.primary_text_style().add_modifier(Modifier::BOLD))
                .truncate()
                .finish(),
        );
        for shortcuts in section.shortcuts.chunks(2) {
            let mut row = TuiFlex::row();
            for shortcut in shortcuts {
                row = row.flex_child(render_entry(shortcut, &builder));
            }
            if shortcuts.len() == 1 {
                row = row.flex_child(TuiText::new("").finish());
            }
            panel.add_child(row.finish());
        }
    }
    TuiContainer::new(panel.finish())
        .with_padding_x(1)
        .with_background(builder.shortcuts_background())
        .finish()
}
