use std::path::Path;

use warpui::elements::{
    Container, CrossAxisAlignment, Flex, MouseStateHandle, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::geometry::vector::Vector2F;
use warpui::{Element, EventContext};

use crate::appearance::Appearance;
use crate::tab_configs::session_config::SessionType;
use crate::tab_configs::session_config_rendering;
use crate::view_components::callout_bubble::{
    callout_background_fill, callout_body_color, callout_title_color,
};

const SECTION_GAP: f32 = 16.;

pub struct TabConfigFormState<'a> {
    pub session_types: &'a [SessionType],
    pub selected_session_type_index: usize,
    pub session_pill_mouse_states: &'a [MouseStateHandle],
    pub selected_directory: &'a Path,
    pub directory_button_mouse_state: MouseStateHandle,
    pub enable_worktree: bool,
    pub is_git_repo: bool,
    pub worktree_checkbox_mouse_state: MouseStateHandle,
    pub worktree_tooltip_mouse_state: MouseStateHandle,
    pub autogenerate_worktree_branch_name: bool,
    pub autogenerate_checkbox_mouse_state: MouseStateHandle,
    pub autogenerate_tooltip_mouse_state: MouseStateHandle,
}

pub struct TabConfigFormHandlers<F1, F2, F3, F4> {
    pub on_select_session_type: F1,
    pub on_open_directory_picker: F2,
    pub on_toggle_worktree: F3,
    pub on_toggle_autogenerate: F4,
}

/// Renders the tab config form content (session type + directory + worktree).
///
/// This is the body of Step 4, without the surrounding popover chrome.
/// Action callbacks are passed in so the caller owns the dispatch logic.
pub fn render_tab_config_form<F1, F2, F3, F4>(
    state: TabConfigFormState<'_>,
    handlers: TabConfigFormHandlers<F1, F2, F3, F4>,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F1: Fn(usize, &mut EventContext, Vector2F) + 'static,
    F2: Fn(&mut EventContext, Vector2F) + 'static,
    F3: Fn(&mut EventContext, Vector2F) + 'static,
    F4: Fn(&mut EventContext, Vector2F) + 'static,
{
    let callout_bg = callout_background_fill(appearance).into_solid();
    let title = Text::new(
        "Create your first tab config",
        appearance.ui_font_family(),
        16.,
    )
    .with_color(callout_title_color(appearance))
    .with_style(Properties::default().weight(Weight::Bold))
    .finish();

    let description = Text::new(
        "Set up a reusable starting point for your tabs. Pick a repo, choose a session type, and optionally attach a worktree. Use it whenever you want to open a tab with this setup.",
        appearance.ui_font_family(),
        14.,
    )
    .with_color(callout_body_color(appearance))
    .finish();

    let session_type_section = session_config_rendering::render_session_type_pills_with_background(
        state.session_types,
        state.selected_session_type_index,
        state.session_pill_mouse_states,
        handlers.on_select_session_type,
        Some(callout_bg),
        appearance,
    );

    let directory_section = session_config_rendering::render_directory_picker_with_background(
        state.selected_directory,
        state.directory_button_mouse_state,
        handlers.on_open_directory_picker,
        Some(callout_bg),
        appearance,
    );

    let worktree_section = session_config_rendering::render_worktree_checkbox_with_background(
        state.enable_worktree,
        state.is_git_repo,
        state.worktree_checkbox_mouse_state,
        state.worktree_tooltip_mouse_state,
        handlers.on_toggle_worktree,
        Some(callout_bg),
        appearance,
    );

    let autogenerate_section =
        session_config_rendering::render_autogenerate_worktree_branch_name_checkbox_with_background(
            state.autogenerate_worktree_branch_name,
            state.enable_worktree,
            state.autogenerate_checkbox_mouse_state,
            state.autogenerate_tooltip_mouse_state,
            handlers.on_toggle_autogenerate,
            Some(callout_bg),
            appearance,
        );

    Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(title)
        .with_child(Container::new(description).with_margin_top(8.).finish())
        .with_child(
            Container::new(session_type_section)
                .with_margin_top(SECTION_GAP)
                .finish(),
        )
        .with_child(
            Container::new(directory_section)
                .with_margin_top(SECTION_GAP)
                .finish(),
        )
        .with_child(
            Container::new(worktree_section)
                .with_margin_top(SECTION_GAP)
                .finish(),
        )
        .with_child(
            Container::new(autogenerate_section)
                .with_margin_top(8.)
                .finish(),
        )
        .finish()
}
