//! Stateless shortcuts projection for the shared read-only menu component.

use warpui_core::AppContext;
use warpui_core::elements::tui::{TuiElement, TuiFlex, TuiText};
use warpui_core::keymap::Context;

use super::state::{TuiShortcut, TuiTerminalSessionState};
use crate::read_only_menu::{TuiReadOnlyMenu, TuiReadOnlyMenuSection};
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
    let sections = sections
        .iter()
        .map(|section| {
            let rows = section
                .shortcuts
                .chunks(2)
                .map(|shortcuts| {
                    let mut row = TuiFlex::row();
                    for shortcut in shortcuts {
                        row = row.flex_child(render_entry(shortcut, &builder));
                    }
                    if shortcuts.len() == 1 {
                        row = row.flex_child(TuiText::new("").finish());
                    }
                    row.finish()
                })
                .collect();
            TuiReadOnlyMenuSection::new(section.title, rows)
        })
        .collect();
    TuiReadOnlyMenu::new(sections).render(&builder)
}
