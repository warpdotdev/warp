//! Stateless status projection for the shared read-only menu component.

use warpui_core::AppContext;
use warpui_core::elements::tui::{TuiElement, TuiText};

use crate::read_only_menu::{TuiReadOnlyMenu, TuiReadOnlyMenuSection};
use crate::tui_builder::TuiUiBuilder;

/// Session and account information displayed by the `/status` menu.
pub(super) struct TuiStatusInfo {
    pub version: String,
    pub session: String,
    pub session_id: String,
    pub working_directory: String,
    pub org: String,
    pub email: String,
}

fn render_field_row(label: &str, value: &str, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    TuiText::from_spans([
        (format!("{label:<19}"), builder.dim_text_style()),
        (value.to_owned(), builder.primary_text_style()),
    ])
    .truncate()
    .finish()
}

/// Renders the dedicated status menu opened by `/status`.
pub(super) fn render(status_info: TuiStatusInfo, ctx: &AppContext) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(ctx);
    let rows = [
        ("Version", status_info.version.as_str()),
        ("Session", status_info.session.as_str()),
        ("Session ID", status_info.session_id.as_str()),
        ("Working directory", status_info.working_directory.as_str()),
        ("Org", status_info.org.as_str()),
        ("Email", status_info.email.as_str()),
    ]
    .into_iter()
    .map(|(label, value)| render_field_row(label, value, &builder))
    .collect();
    TuiReadOnlyMenu::new(vec![TuiReadOnlyMenuSection::new("Status", rows)]).render(&builder)
}
