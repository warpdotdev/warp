//! Stateless shortcuts projection for the shared read-only menu component.

use warpui_core::AppContext;
use warpui_core::keymap::Context;

use super::state::{TuiShortcut, TuiTerminalSessionState};
use crate::read_only_menu::{
    TuiReadOnlyMenu, TuiReadOnlyMenuRow, TuiReadOnlyMenuSection, TuiReadOnlyMenuText,
};
use crate::tui_builder::TuiUiBuilder;
fn entry(shortcut: &TuiShortcut, builder: &TuiUiBuilder) -> TuiReadOnlyMenuText {
    TuiReadOnlyMenuText::new([
        (format!("{} ", shortcut.key), builder.link_text_style()),
        (
            shortcut.description.to_owned(),
            builder.primary_text_style(),
        ),
    ])
}

/// Builds the keyboard-shortcuts menu opened by `?`.
///
/// The panel lists contextual keybindings grouped by section. Status
/// information is intentionally absent here — it lives in the dedicated
/// status menu opened by `/status`.
pub(super) fn menu(
    state: &TuiTerminalSessionState,
    context: &Context,
    builder: &TuiUiBuilder,
    ctx: &AppContext,
) -> TuiReadOnlyMenu {
    let sections = state.shortcut_sections(context, ctx);
    let sections = sections
        .iter()
        .map(|section| {
            let rows = section
                .shortcuts
                .chunks(2)
                .map(|shortcuts| {
                    let mut columns = shortcuts
                        .iter()
                        .map(|shortcut| entry(shortcut, builder))
                        .collect::<Vec<_>>();
                    if columns.len() == 1 {
                        columns.push(TuiReadOnlyMenuText::empty());
                    }
                    TuiReadOnlyMenuRow::new(columns)
                })
                .collect();
            TuiReadOnlyMenuSection::new(section.title, rows)
        })
        .collect();
    TuiReadOnlyMenu::new(sections)
}
