//! Stateless renderer for the contextual shortcuts projection.
//!
//! The panel has no focus, actions, retained state, or independent lifecycle,
//! so it intentionally remains an element renderer rather than a child view.
//! Its contents are rebuilt from the session model's live snapshot and the
//! current keymap context on every parent render.
//!
//! The read-only row rendering helpers (`render_field_row`, `shortcuts_background`
//! wrapper) are also used by the sibling `status_menu` module to provide visual
//! consistency between the two panels without duplicating the styling.

use warpui_core::AppContext;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    Modifier, TuiContainer, TuiElement, TuiFlex, TuiParentElement, TuiText,
};
use warpui_core::keymap::Context;

use super::state::{TuiShortcut, TuiTerminalSessionState};
use crate::tui_builder::TuiUiBuilder;

/// Session and account information displayed in the dedicated status menu
/// opened when the user invokes the `/status` slash command.
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

/// Renders a single read-only label/value row in the same style as the
/// shortcuts panel's section rows. Shared with `status_menu` so both panels
/// use identical visual structure without duplicating the styling.
pub(super) fn render_field_row(
    label: &str,
    value: &str,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    TuiText::from_spans([
        (format!("{label:<19}"), builder.dim_text_style()),
        (value.to_owned(), builder.primary_text_style()),
    ])
    .truncate()
    .finish()
}

/// Wraps `inner` with the standard panel container: horizontal padding and
/// the shortcuts background colour. Used by `status_menu` so both panels share
/// identical outer styling.
pub(super) fn wrap_panel(
    inner: Box<dyn TuiElement>,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    TuiContainer::new(inner)
        .with_padding_x(1)
        .with_background(builder.shortcuts_background())
        .finish()
}

/// Renders the keyboard-shortcuts panel (opened by `?`).
///
/// The panel lists contextual keybindings grouped by section. Status
/// information is intentionally absent here — it lives in the dedicated
/// status menu opened by `/status`.
pub(super) fn render(
    state: &TuiTerminalSessionState,
    context: &Context,
    ctx: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(ctx);
    let sections = state.shortcut_sections(context, ctx);
    let mut panel = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    let mut first_section = true;
    for section in sections.iter() {
        if !first_section {
            panel.add_child(TuiText::new(" ").finish());
        }
        first_section = false;
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
    wrap_panel(panel.finish(), &builder)
}
