//! Stateless active-TODO projection for the shared read-only menu component.

use warp::tui_export::{AIConversation, TodoStatus};
use warpui_core::elements::tui::Modifier;

use crate::agent_block_sections::todo_status_glyph;
use crate::read_only_menu::{
    TuiReadOnlyMenu, TuiReadOnlyMenuRow, TuiReadOnlyMenuSection, TuiReadOnlyMenuText,
};
use crate::tui_builder::TuiUiBuilder;

fn todo_row(title: &str, status: TodoStatus, builder: &TuiUiBuilder) -> TuiReadOnlyMenuRow {
    let (glyph, glyph_style) = todo_status_glyph(&status, builder);
    let title_style = match status {
        TodoStatus::Pending | TodoStatus::Completed => builder.muted_text_style(),
        TodoStatus::InProgress => builder.primary_text_style(),
        TodoStatus::Cancelled => builder
            .muted_text_style()
            .add_modifier(Modifier::CROSSED_OUT),
        TodoStatus::Stopped => builder.muted_text_style(),
    };
    TuiReadOnlyMenuRow::new([TuiReadOnlyMenuText::new([
        (format!("{glyph} "), glyph_style),
        (title.to_owned(), title_style),
    ])])
}

pub(super) fn active_todo_menu(
    conversation: &AIConversation,
    builder: &TuiUiBuilder,
) -> Option<TuiReadOnlyMenu> {
    let todo_list = conversation
        .active_todo_list()
        .filter(|todo_list| !todo_list.is_empty())?;
    let completed = todo_list.completed_items().len();
    let rows = todo_list
        .completed_items()
        .iter()
        .chain(todo_list.pending_items())
        .filter_map(|item| {
            conversation
                .todo_status(&item.id)
                .map(|status| todo_row(&item.title, status, builder))
        })
        .collect();
    Some(TuiReadOnlyMenu::new(vec![TuiReadOnlyMenuSection::new(
        format!("Tasks {completed}/{}", todo_list.len()),
        rows,
    )]))
}
