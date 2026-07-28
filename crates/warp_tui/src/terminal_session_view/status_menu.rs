//! Stateless renderer for the dedicated `/status` menu.
//!
//! The status menu is a read-only panel opened by the `/status` slash command.
//! It reuses the field-row rendering helpers and panel-container wrapper from
//! the sibling `shortcuts` module so both panels share identical visual
//! structure (background colour, horizontal padding, label/value alignment)
//! without duplicating any styling code.  The analogy mirrors how `inline_menu`
//! provides a shared structure used by slash commands, models, and
//! conversations: `shortcuts::render_field_row` and `shortcuts::wrap_panel` are
//! the shared structure here, consumed by both `shortcuts::render` (for
//! keyboard-shortcut rows) and this module (for status rows).

use warpui_core::AppContext;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{Modifier, TuiElement, TuiFlex, TuiParentElement, TuiText};

use super::shortcuts::{TuiStatusInfo, render_field_row, wrap_panel};
use crate::tui_builder::TuiUiBuilder;

/// Renders the dedicated status menu (opened by `/status`).
///
/// The menu displays six read-only session and account fields using the same
/// label/value row styling as the shortcuts panel.  It does not include any
/// keyboard-shortcut entries; those live in the shortcuts panel (`?`).
pub(super) fn render(status_info: TuiStatusInfo, ctx: &AppContext) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(ctx);
    let mut panel = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

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
        panel.add_child(render_field_row(label, value, &builder));
    }

    wrap_panel(panel.finish(), &builder)
}
