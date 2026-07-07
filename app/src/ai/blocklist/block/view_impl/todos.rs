//! Rendering logic for todo list components in AI blocks.

use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, Highlight,
    ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::Properties;
use warpui::text_layout::TextStyle;
use warpui::{AppContext, Element, SingletonEntity};

use super::WithContentItemSpacing;
use crate::ai::agent::conversation::{AIConversation, TodoStatus};
use crate::ai::agent::icons::{
    gray_stop_icon, in_progress_icon, pending_icon, succeeded_icon, todo_list_icon,
};
use crate::ai::agent::todos::AIAgentTodoList;
use crate::ai::agent::{AIAgentTodo, MessageId};
use crate::ai::blocklist::block::{AIBlockAction, TodoListElementState};
use crate::ai::blocklist::inline_action::inline_action_header::{
    ExpandedConfig, HeaderConfig, InteractionMode, INLINE_ACTION_HORIZONTAL_PADDING,
};
use crate::ai::blocklist::inline_action::inline_action_icons::{cancelled_icon, icon_size};
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

pub(super) fn render_todos(
    id: &MessageId,
    todos: &[AIAgentTodo],
    conversation: &AIConversation,
    state: &TodoListElementState,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    // Add collapsible header.
    let id = id.clone();
    let mut header_config = HeaderConfig::new("Tasks", app)
        .with_interaction_mode(InteractionMode::ManuallyExpandable(
            ExpandedConfig::new(state.is_expanded, state.header_toggle_mouse_state.clone())
                .with_toggle_callback(move |ctx| {
                    ctx.dispatch_typed_action(AIBlockAction::ToggleTodoListExpanded(id.clone()));
                }),
        ))
        .with_icon(todo_list_icon(appearance));

    let mut has_cancelled_todo = false;
    let mut rendered_todos = vec![];
    for todo in todos.iter() {
        let status = conversation
            .todo_status(&todo.id)
            .unwrap_or(TodoStatus::Cancelled);
        if status.is_cancelled() {
            has_cancelled_todo = true;
        }
        rendered_todos.push(render_todo(todo, status, app));
    }

    let is_list_outdated = has_cancelled_todo
        || todos.len() != conversation.active_todo_list().map_or(0, |list| list.len());
    if is_list_outdated {
        header_config = header_config.with_badge("Outdated".to_string());
    }

    let header_element = header_config.render(app);

    let mut container = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(header_element);

    // Render todo list items.
    if state.is_expanded {
        container.add_child(
            Container::new(Flex::column().with_children(rendered_todos).finish())
                .with_padding_top(12.)
                .with_border(
                    Border::new(1.)
                        .with_sides(false, true, true, true)
                        .with_border_fill(theme.outline()),
                )
                .with_background_color(theme.background().into_solid())
                .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                .finish(),
        );
    }

    container
        .finish()
        .with_agent_output_item_spacing(app)
        .finish()
}

fn render_todo(todo: &AIAgentTodo, status: TodoStatus, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let text_color = blended_colors::text_main(theme, theme.surface_1());
    let icon = match status {
        TodoStatus::Pending => pending_icon(appearance),
        TodoStatus::InProgress => in_progress_icon(appearance),
        TodoStatus::Completed => succeeded_icon(appearance),
        TodoStatus::Cancelled => cancelled_icon(appearance),
        TodoStatus::Stopped => gray_stop_icon(appearance),
    };
    let item_icon = Container::new(
        ConstrainedBox::new(icon.finish())
            .with_width(icon_size(app) - 4.)
            .with_height(icon_size(app) - 4.)
            .finish(),
    )
    .with_margin_right(12.)
    .finish();

    let mut item_text = Text::new(
        todo.title.clone(),
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_style(Properties::default().weight(appearance.monospace_font_weight()));

    if status.is_cancelled() {
        let title = todo.title.clone();
        let highlight_indices = (0..title.chars().count()).collect();
        let strikethrough_highlight = Highlight::new().with_text_style(
            TextStyle::new()
                .with_show_strikethrough(true)
                .with_foreground_color(blended_colors::neutral_5(theme)),
        );

        item_text = item_text.with_single_highlight(strikethrough_highlight, highlight_indices);
    } else {
        item_text = item_text.with_color(text_color);
    }

    let item_text = item_text.finish();

    let item_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(item_icon)
        .with_child(Shrinkable::new(1., item_text).finish())
        .finish();

    Container::new(item_row)
        .with_margin_left(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_margin_bottom(12.)
        .finish()
}

/// Renders a completed todo item with a check mark and a divider line.
pub(super) fn render_completed_todo_items(
    completed_items: &[AIAgentTodo],
    current_todo_list: Option<&AIAgentTodoList>,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let sub_text_color = blended_colors::text_sub(theme, theme.surface_1());

    // Create a check mark icon
    let check_icon = Container::new(
        ConstrainedBox::new(
            warpui::elements::Icon::new(
                Icon::Check.into(),
                warp_core::ui::theme::Fill::Solid(sub_text_color),
            )
            .finish(),
        )
        .with_width(icon_size(app) - 4.)
        .with_height(icon_size(app) - 4.)
        .finish(),
    )
    .with_margin_right(6.)
    .finish();

    // Create the content row
    let mut content_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(check_icon);

    let mut completed_text = "".to_owned();
    for (i, completed_item) in completed_items.iter().enumerate() {
        let index_and_len = current_todo_list.and_then(|list| {
            list.get_item_index(&completed_item.id)
                .map(|i| (i, list.len()))
        });

        if i == 0 {
            if let Some((index, list_len)) = index_and_len {
                completed_text += format!(
                    "Completed {} ({}/{})",
                    completed_item.title,
                    index + 1,
                    list_len
                )
                .as_str()
            } else {
                completed_text += format!("Completed {}", completed_item.title).as_str()
            }
        } else if let Some((index, list_len)) = index_and_len {
            completed_text +=
                format!(", {} ({}/{})", completed_item.title, index + 1, list_len).as_str()
        } else {
            completed_text += format!(", {}", completed_item.title).as_str()
        }
    }
    if completed_text.is_empty() {
        return None;
    }
    content_row.add_child(
        Shrinkable::new(
            1.,
            Text::new(
                completed_text,
                appearance.ui_font_family(),
                (appearance.ui_font_size() - 2.) * appearance.monospace_ui_scalar(),
            )
            .with_color(sub_text_color)
            .with_style(Properties::default().weight(appearance.monospace_font_weight()))
            .finish(),
        )
        .finish(),
    );

    // Create a divider line that extends to the full width using negative margins
    let divider = Container::new(
        ConstrainedBox::new(Empty::new().finish())
            .with_height(1.)
            .finish(),
    )
    .with_background_color(theme.outline().into_solid())
    .with_margin_top(6.)
    .with_margin_bottom(6.)
    .with_margin_left(-20.)
    .with_margin_right(-20.)
    .finish();

    // Combine content and divider - structure to allow full-width divider
    let complete_item = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(content_row.finish())
        .with_child(divider)
        .finish();

    Some(complete_item.with_agent_output_item_spacing(app).finish())
}
