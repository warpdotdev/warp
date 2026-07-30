//! Stateless status projection for the shared read-only menu component.

use crate::read_only_menu::{
    TuiReadOnlyMenu, TuiReadOnlyMenuRow, TuiReadOnlyMenuSection, TuiReadOnlyMenuText,
};
use crate::tui_builder::TuiUiBuilder;

/// Session and account information displayed by the `/status` menu.
pub(super) struct TuiStatusInfo {
    pub version: String,
    pub session: String,
    pub conversation_id: String,
    pub working_directory: String,
    pub org: String,
    pub email: String,
}

fn field_row(label: &str, value: &str, builder: &TuiUiBuilder) -> TuiReadOnlyMenuRow {
    TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([
        (format!("{label:<19}"), builder.read_only_menu_label_style()),
        (value.to_owned(), builder.primary_text_style()),
    ])])
}

/// Builds the dedicated status menu opened by `/status`.
pub(super) fn menu(status_info: TuiStatusInfo, builder: &TuiUiBuilder) -> TuiReadOnlyMenu {
    let rows = [
        ("Version", status_info.version.as_str()),
        ("Session", status_info.session.as_str()),
        ("Conversation ID", status_info.conversation_id.as_str()),
        ("Working directory", status_info.working_directory.as_str()),
        ("Org", status_info.org.as_str()),
        ("Email", status_info.email.as_str()),
    ]
    .into_iter()
    .map(|(label, value)| field_row(label, value, builder))
    .collect();
    TuiReadOnlyMenu::new(vec![TuiReadOnlyMenuSection::new("Status", rows)])
}
