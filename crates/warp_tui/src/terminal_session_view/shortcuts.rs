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

/// Session and account information displayed in the status section of the
/// shortcuts panel when `/status` is invoked or `?` is pressed.
pub(super) struct TuiStatusInfo {
    pub version: String,
    pub session: String,
    pub session_id: String,
    pub working_directory: String,
    pub org: String,
    pub email: String,
}

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

fn render_status_entry(label: &str, value: &str, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    TuiText::from_spans([
        (format!("{label:<19}"), builder.dim_text_style()),
        (value.to_owned(), builder.primary_text_style()),
    ])
    .truncate()
    .finish()
}

pub(super) fn render(
    state: &TuiTerminalSessionState,
    context: &Context,
    status_info: TuiStatusInfo,
    ctx: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(ctx);
    let sections = state.shortcut_sections(context, ctx);
    let mut panel = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    // Status section is always shown first in the panel.
    panel.add_child(
        TuiText::new("Status")
            .with_style(builder.primary_text_style().add_modifier(Modifier::BOLD))
            .truncate()
            .finish(),
    );
    for (label, value) in [
        ("Version", status_info.version.as_str()),
        ("Session", status_info.session.as_str()),
        ("Session ID", status_info.session_id.as_str()),
        ("Working directory", status_info.working_directory.as_str()),
        ("Org", status_info.org.as_str()),
        ("Email", status_info.email.as_str()),
    ] {
        panel.add_child(render_status_entry(label, value, &builder));
    }

    for section in sections.iter() {
        panel.add_child(TuiText::new(" ").finish());
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
