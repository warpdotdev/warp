//! Repositories section UI for the vertical tabs panel (repo mode).

use std::cell::RefCell;
use std::collections::HashMap;

use pathfinder_color::ColorU;
use repo_mode::RepoEntryKind;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::elements::{
    Container, CrossAxisAlignment, Element, Flex, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, Padding, ParentElement, Text,
};
use warpui::{AppContext, SingletonEntity};

use super::repo_mode_model::RepoModeListEntry;
use super::Workspace;
use crate::appearance::Appearance;
use crate::workspace::WorkspaceAction;

/// Per-row mouse state for the Repositories section.
#[derive(Clone, Default)]
pub(super) struct RepoSidebarState {
    pub add_button: MouseStateHandle,
    pub all_row: MouseStateHandle,
    pub entry_rows: RefCell<HashMap<String, MouseStateHandle>>,
}

/// Renders the fixed Repositories block above the tab scroller.
pub(super) fn render_repo_sidebar(
    state: &RepoSidebarState,
    workspace: &Workspace,
    app: &AppContext,
) -> Box<dyn Element> {
    if !Workspace::repo_mode_enabled() {
        return Flex::column().finish();
    }

    let appearance = Appearance::as_ref(app);
    let entries = workspace.repo_mode_entries(app);
    let selected = workspace.selected_repo_root.as_deref();

    let header = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Text::new("Repositories", appearance.ui_font_family(), 11.)
                .with_color(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().background())
                        .into(),
                )
                .finish(),
        )
        .with_child(render_add_button(state.add_button.clone(), appearance))
        .finish();

    let mut column = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(
            Container::new(header)
                .with_padding(Padding::uniform(8.).with_bottom(4.))
                .finish(),
        )
        .with_child(render_all_row(
            state.all_row.clone(),
            selected.is_none(),
            appearance,
        ));

    for entry in entries {
        let key = entry.path.to_string_lossy().into_owned();
        let mouse = state
            .entry_rows
            .borrow_mut()
            .entry(key.clone())
            .or_default()
            .clone();
        let is_selected = selected == Some(key.as_str());
        column = column.with_child(render_entry_row(entry, mouse, is_selected, appearance));
    }

    Container::new(column.finish())
        .with_padding(Padding::uniform(0.).with_bottom(4.))
        .finish()
}

fn render_add_button(mouse: MouseStateHandle, app_appearance: &Appearance) -> Box<dyn Element> {
    let theme = app_appearance.theme();
    let accent = theme.accent();
    let font = app_appearance.ui_font_family();
    Hoverable::new(mouse, move |hover| {
        let background = if hover.is_hovered() {
            internal_colors::fg_overlay_2(theme)
        } else {
            ThemeFill::Solid(ColorU::transparent_black())
        };
        Container::new(
            Text::new("+ Add", font, 11.)
                .with_color(accent.into())
                .finish(),
        )
        .with_padding(Padding::uniform(4.))
        .with_background(background)
        .finish()
    })
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::AddLocalRepositoryOrFolder);
    })
    .finish()
}

fn render_all_row(
    mouse: MouseStateHandle,
    is_selected: bool,
    app_appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = app_appearance.theme();
    let font = app_appearance.ui_font_family();
    let color = theme.font_color(theme.background());
    Hoverable::new(mouse, move |hover| {
        let background = if is_selected {
            internal_colors::fg_overlay_3(theme)
        } else if hover.is_hovered() {
            internal_colors::fg_overlay_1(theme)
        } else {
            ThemeFill::Solid(ColorU::transparent_black())
        };
        Container::new(
            Text::new("All", font, 12.)
                .with_color(color.into())
                .finish(),
        )
        .with_padding(Padding::uniform(8.).with_top(4.).with_bottom(4.))
        .with_background(background)
        .finish()
    })
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::SelectRepoModeAll);
    })
    .finish()
}

fn render_entry_row(
    entry: RepoModeListEntry,
    mouse: MouseStateHandle,
    is_selected: bool,
    app_appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = app_appearance.theme();
    let font = app_appearance.ui_font_family();
    let color = if entry.is_dead {
        theme.sub_text_color(theme.background())
    } else {
        theme.font_color(theme.background())
    };
    let mut label = match entry.kind {
        RepoEntryKind::Repo => entry.display_name.clone(),
        RepoEntryKind::Folder => format!("{} (folder)", entry.display_name),
    };
    let is_dead = entry.is_dead;
    if is_dead {
        label = format!("{label} — Remove");
    }
    let path = entry.path;
    let path_for_remove = path.clone();
    Hoverable::new(mouse, move |hover| {
        let background = if is_selected {
            internal_colors::fg_overlay_3(theme)
        } else if hover.is_hovered() {
            internal_colors::fg_overlay_1(theme)
        } else {
            ThemeFill::Solid(ColorU::transparent_black())
        };
        Container::new(
            Text::new(label.clone(), font, 12.)
                .with_color(color.into())
                .finish(),
        )
        .with_padding(Padding::uniform(8.).with_top(4.).with_bottom(4.))
        .with_background(background)
        .finish()
    })
    .on_click(move |ctx, _, _| {
        if is_dead {
            ctx.dispatch_typed_action(WorkspaceAction::RemoveRepoModeEntry(path.clone()));
        } else {
            ctx.dispatch_typed_action(WorkspaceAction::SelectRepoModeEntry(path.clone()));
        }
    })
    .on_right_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::RemoveRepoModeEntry(
            path_for_remove.clone(),
        ));
    })
    .finish()
}
