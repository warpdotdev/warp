//! Stateless renderer for the dedicated `/status` menu.
//!
//! The status menu is a read-only panel opened by the `/status` slash command.
//! It owns the session/account data model (`TuiStatusInfo`) and the label/value
//! row helper (`render_field_row`). The outer panel container (`wrap_panel`)
//! is borrowed from the sibling `shortcuts` module so both panels share
//! identical background colour and horizontal padding without duplicating
//! styling code.

use warpui_core::AppContext;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{Modifier, TuiElement, TuiFlex, TuiParentElement, TuiText};

use super::shortcuts::wrap_panel;
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

/// Renders a single read-only label/value row for the status panel.
///
/// The label is left-padded to a fixed width so all value columns align
/// regardless of label length.
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
