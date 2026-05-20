use std::path::PathBuf;

use warp_util::path::user_friendly_path;
use warpui::{
    elements::{Border, ChildView, Container, Hoverable, MouseStateHandle, Text},
    platform::Cursor,
    text_layout::ClipConfig,
    ui_components::components::UiComponentStyles,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    ai::persisted_workspace::{PersistedWorkspace, PersistedWorkspaceEvent},
    appearance::Appearance,
    tab_configs::PickerStyle,
    view_components::{DropdownItem, FilterableDropdown},
};

const DEFAULT_DROPDOWN_WIDTH: f32 = 380.;

/// Label for the sticky "Add new repo..." footer at the bottom of the picker.
const ADD_NEW_REPO_LABEL: &str = "+ Add new repo...";

/// A filterable dropdown listing known repos (from `PersistedWorkspace`), with a
/// sticky "+ Add new repo..." footer that is always visible even when scrolling.
///
/// Emits:
/// - [`RepoPickerEvent::Selected`] when the user picks a repo path.
/// - [`RepoPickerEvent::RequestAddRepo`] when the user clicks "+ Add new repo...".
pub struct RepoPicker {
    dropdown: ViewHandle<FilterableDropdown<RepoPickerAction>>,
    /// The currently selected repo path (updated by `handle_action`).
    selected: Option<String>,
    /// Mouse state for the sticky "Add new repo..." footer row.
    add_repo_mouse_state: MouseStateHandle,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RepoPickerAction {
    Select(String),
    AddNewRepo,
}

pub enum RepoPickerEvent {
    Selected(String),
    RequestAddRepo,
}

impl RepoPicker {
    /// Creates a new picker pre-populated with all known projects.
    ///
    /// `default_value` is pre-selected if it appears in the project list (or is
    /// added as an extra entry if it doesn't).
    pub fn new(default_value: Option<String>, ctx: &mut ViewContext<Self>) -> Self {
        Self::new_with_style(default_value, None, ctx)
    }

    pub fn new_with_style(
        default_value: Option<String>,
        style: Option<PickerStyle>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Subscribe to PersistedWorkspace so the list refreshes when the user
        // adds a repo via the folder picker.
        ctx.subscribe_to_model(&PersistedWorkspace::handle(ctx), |me, _, event, ctx| {
            if let PersistedWorkspaceEvent::WorkspaceAdded { path } = event {
                let path_str = path.to_string_lossy().to_string();
                me.refresh_items(Some(&path_str), ctx);
            }
        });

        let width = style.as_ref().map_or(DEFAULT_DROPDOWN_WIDTH, |s| s.width);
        let bg = style.and_then(|s| s.background);
        let dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_top_bar_max_width(width);
            dropdown.set_menu_width(width, ctx);
            if let Some(bg) = bg {
                dropdown.set_style(UiComponentStyles {
                    background: Some(bg.into()),
                    ..Default::default()
                });
            }
            dropdown
        });

        let mut picker = Self {
            dropdown,
            selected: None,
            add_repo_mouse_state: Default::default(),
        };

        // Attach the sticky footer. It stays visible while scrolling because it is
        // rendered below the scrollable items but inside the Menu's Dismiss
        // (via FilterableDropdown::set_footer → Menu::set_pinned_footer_builder).
        // Being inside the Dismiss means clicks on it do not trigger the dismiss
        // handler, so the standard on_click / LeftMouseUp path works correctly.
        let mouse_state = picker.add_repo_mouse_state.clone();
        picker.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_footer(
                move |app| {
                    let appearance = Appearance::as_ref(app);
                    let theme = appearance.theme();
                    let is_hovered = mouse_state.lock().unwrap().is_hovered();
                    let bg = if is_hovered {
                        theme.accent_button_color()
                    } else {
                        theme.surface_2()
                    };
                    let font_family = appearance.ui_font_family();
                    let font_size = appearance.ui_font_size();
                    let text_color = theme.main_text_color(bg);
                    let border_fill = theme.outline();
                    let mouse_state_clone = mouse_state.clone();
                    Hoverable::new(mouse_state_clone, move |_| {
                        Container::new(
                            Text::new_inline(ADD_NEW_REPO_LABEL, font_family, font_size)
                                .with_color(text_color.into())
                                .finish(),
                        )
                        .with_horizontal_padding(8.)
                        .with_vertical_padding(6.)
                        .with_background(bg)
                        .with_border(Border::top(1.).with_border_fill(border_fill))
                        .finish()
                    })
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(RepoPickerAction::AddNewRepo);
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish()
                },
                ctx,
            );
        });

        picker.refresh_items(default_value.as_deref(), ctx);
        picker
    }

    /// Refreshes the dropdown list from `PersistedWorkspace` and optionally
    /// pre-selects a specific path.
    pub fn refresh_and_select(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let path_str = path.to_string_lossy().to_string();
        self.refresh_items(Some(&path_str), ctx);
    }

    fn refresh_items(&mut self, select_path: Option<&str>, ctx: &mut ViewContext<Self>) {
        // workspaces() already returns entries sorted by most-recently-touched.
        // "+ Add new repo..." is a sticky footer (not a list item) so it is
        // not included here.
        //
        // Each item's `display_text` is the full user-friendly form
        // (`~`-prefixed). The dropdown clips it at render width via
        // `ClipConfig::start()`, so distinct paths with shared trailing
        // segments stay readable without character-count approximation.
        // The action carries the *raw* absolute path so consumers reading
        // `RepoPickerEvent::Selected` keep getting a real filesystem path.
        let home = dirs::home_dir().map(|p| p.display().to_string());
        let items: Vec<DropdownItem<RepoPickerAction>> = PersistedWorkspace::as_ref(ctx)
            .workspaces()
            .filter(|ws| ws.path.exists())
            .map(|ws| {
                let path_str = ws.path.to_string_lossy().into_owned();
                let display = user_friendly_path(&path_str, home.as_deref()).into_owned();
                DropdownItem::new(display, RepoPickerAction::Select(path_str.clone()))
                    .with_clip_config(ClipConfig::start())
                    .with_tooltip(path_str)
            })
            .collect();

        let raw_to_select = select_path
            .or(self.selected.as_deref())
            .map(|s| s.to_owned());

        // Mirror the raw path into `self.selected` so `selected_value()`
        // returns a real filesystem path even before the user explicitly
        // picks something. Load-bearing for `new_worktree_modal::on_open`,
        // which reads `repo_picker.selected_value()` at modal-open time when
        // its own `selected_repo` is still `None`.
        if let Some(ref raw) = raw_to_select {
            self.selected = Some(raw.clone());
        }

        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items, ctx);
            // Match by the action (which carries the raw absolute path) so two
            // repos that left-clip to identical-looking labels can't be
            // confused at preselection time.
            if let Some(ref raw) = raw_to_select {
                dropdown.set_selected_by_action(RepoPickerAction::Select(raw.clone()), ctx);
            }
        });

        ctx.notify();
    }

    pub fn toggle_dropdown(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.toggle_expanded(ctx);
        });
        self.dropdown.as_ref(ctx).is_expanded()
    }

    /// Returns the currently shown selected repo path (raw absolute path).
    ///
    /// `refresh_items` eagerly mirrors any pre-selected raw path into
    /// `self.selected`, so we never need to fall back to the dropdown's
    /// `selected_item_label` — that would return the `~`-abbreviated display
    /// string, not a usable filesystem path.
    pub fn selected_value(&self, _app: &AppContext) -> Option<String> {
        self.selected.clone()
    }
}

impl Entity for RepoPicker {
    type Event = RepoPickerEvent;
}

impl View for RepoPicker {
    fn ui_name() -> &'static str {
        "RepoPicker"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.dropdown).finish()
    }
}

impl TypedActionView for RepoPicker {
    type Action = RepoPickerAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RepoPickerAction::Select(value) => {
                self.selected = Some(value.clone());
                ctx.emit(RepoPickerEvent::Selected(value.clone()));
            }
            RepoPickerAction::AddNewRepo => {
                // Close the dropdown before the folder picker opens so the two
                // don't compete for focus.
                self.dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.close(ctx);
                });
                ctx.emit(RepoPickerEvent::RequestAddRepo);
            }
        }
    }
}
